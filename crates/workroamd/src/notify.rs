//! Phone notifications (ntfy) and agent webhooks (architecture §16.6):
//! posted directly by the device that produced the event; failures are
//! logged, retried at most 3 times and never affect syncing.

use crate::jobs::JobInfo;
use crate::Daemon;
use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::Sha256;
use workroam_core::model::Grant;
use workroam_platform::secrets;
use workroam_store::control::keys;

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_default()
}

async fn post(url: &str, body: Vec<u8>, headers: &[(&str, String)]) {
    for attempt in 0..3 {
        let mut r = http().post(url).body(body.clone());
        for (k, v) in headers {
            r = r.header(*k, v);
        }
        match r.send().await {
            Ok(resp) if resp.status().is_success() => return,
            Ok(resp) => tracing::warn!("notify {url}: HTTP {}", resp.status()),
            Err(e) => tracing::warn!("notify {url}: {e}"),
        }
        tokio::time::sleep(std::time::Duration::from_secs(2 << attempt)).await;
    }
}

pub fn sign(secret: &str, body: &[u8]) -> String {
    let mut m =
        <Hmac<Sha256> as hmac::KeyInit>::new_from_slice(secret.as_bytes()).expect("any key length");
    m.update(body);
    format!("sha256={}", hex::encode(m.finalize().into_bytes()))
}

/// `event`: finished, failed, arrival.
pub async fn send(d: &Daemon, event: &str, project: &str, summary: Value) {
    let Ok(m) = crate::load_machine() else {
        return;
    };
    let payload = json!({
        "project": project,
        "event": event,
        "device": m.name,
        "time": chrono::Utc::now(),
        "summary": summary,
    });
    let via_server = crate::relay::has_feature(d, "notify")
        .then(|| crate::relay::server_base(d))
        .flatten();
    if (via_server.is_some() || !m.notify.ntfy.is_empty())
        && (event != "finished" || m.notify.on_success)
    {
        let title = match event {
            "finished" => format!("Workroam: {project} synced"),
            "failed" => format!("Workroam: {project} failed"),
            _ => format!("Workroam: new progress in {project}"),
        };
        let text = summary
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or(event)
            .to_string();
        let url = m.notify.ntfy.clone();
        // A full-mode server forwards to its ntfy topic; otherwise post directly.
        if let Some(base) = via_server {
            let body = json!({"title": title, "message": text, "tags": "workroam", "priority": if event == "failed" { "high" } else { "default" }});
            tokio::spawn(async move {
                if let Err(e) = crate::relay::signed_request(&base, "POST", "/notify", &body).await
                {
                    tracing::warn!("notify via server: {}", e.message);
                }
            });
        } else {
            tokio::spawn(async move {
                post(
                    &url,
                    text.into_bytes(),
                    &[("Title", title), ("Tags", "workroam".into())],
                )
                .await
            });
        }
    }
    // Webhooks of remote agents allowed on this project.
    let Ok(ws) = d.ws().await else { return };
    let Ok(grants) = ws.control.get_all::<Grant>(keys::GRANTS).await else {
        return;
    };
    for (_, g) in grants {
        let Some(url) = g.webhook_url.clone() else {
            continue;
        };
        if g.revoked || !(g.projects.is_empty() || g.projects.iter().any(|p| p == project)) {
            continue;
        }
        let body = serde_json::to_vec(&payload).unwrap_or_default();
        let secret = secrets::get(&secrets::keys::webhook(&g.agent))
            .ok()
            .flatten()
            .unwrap_or_default();
        let sig = sign(&secret, &body);
        tokio::spawn(async move {
            post(
                &url,
                body,
                &[
                    ("Content-Type", "application/json".into()),
                    ("X-Workroam-Signature", sig),
                ],
            )
            .await
        });
    }
}

pub async fn job_event(d: &Daemon, event: &str, info: &JobInfo) {
    if !matches!(info.kind.as_str(), "push" | "pull") {
        return;
    }
    let message = match (event, &info.error) {
        ("failed", Some(e)) => e.message.clone(),
        _ => format!("{} {}", info.kind, info.project),
    };
    let summary = info
        .result
        .as_ref()
        .and_then(|r| r.get("summary"))
        .cloned()
        .unwrap_or(Value::Null);
    send(
        d,
        event,
        &info.project,
        json!({"message": message, "kind": info.kind, "changes": summary}),
    )
    .await;
}
