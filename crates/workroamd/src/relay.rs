//! Device relay client (architecture §16.4, §18.3) and `agent.connect` /
//! `agent.disconnect`. The daemon keeps one outbound WebSocket to the
//! relay, authenticates with its Ed25519 device key, answers remote tool
//! calls after checking its own copy of the grant, and asks the user on a
//! card for every connection and every remote push or pull.

use crate::approvals::Approval;
use crate::{p_req, p_str, Ctx, Daemon};
use chrono::Utc;
use ed25519_dalek::{Signer, SigningKey};
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use workroam_core::config::ProjectConfig;
use workroam_core::model::{Grant, HeadState, RelayRecord};
use workroam_core::{Code, Error, Result};
use workroam_ipc::relay::{self as proto, BindCode, FromDevice, ToDevice};
use workroam_ipc::{ClientKind, Origin};
use workroam_platform::secrets;
use workroam_store::control::keys;

#[derive(Debug, Clone, Default, Serialize)]
pub struct RelayStatus {
    pub url: Option<String>,
    pub relay_id: Option<String>,
    pub connected: bool,
    pub last_error: Option<String>,
    pub public_url: Option<String>,
    /// Full-mode features of the server (`pair`, `notify`, `web`).
    pub features: Vec<String>,
}

#[derive(Default)]
pub struct RelayState {
    status: Mutex<RelayStatus>,
    tx: Mutex<Option<mpsc::UnboundedSender<FromDevice>>>,
    running: AtomicBool,
}

impl RelayState {
    pub fn status(&self) -> RelayStatus {
        self.status.lock().unwrap().clone()
    }
    fn send(&self, m: FromDevice) -> bool {
        self.tx
            .lock()
            .unwrap()
            .as_ref()
            .map(|t| t.send(m).is_ok())
            .unwrap_or(false)
    }
}

/// This device's relay key, created on first use and kept in the keychain.
pub fn device_key() -> Result<SigningKey> {
    if let Some(h) = secrets::get(secrets::keys::RELAY_KEY)? {
        if let Ok(b) = hex::decode(h.trim()) {
            if let Ok(seed) = <[u8; 32]>::try_from(b.as_slice()) {
                return Ok(SigningKey::from_bytes(&seed));
            }
        }
    }
    let seed: [u8; 32] = workroam_core::crypto::random_bytes();
    secrets::set(secrets::keys::RELAY_KEY, &hex::encode(seed))?;
    Ok(SigningKey::from_bytes(&seed))
}

pub fn pubkey_hex(k: &SigningKey) -> String {
    hex::encode(k.verifying_key().to_bytes())
}

fn ws_url(base: &str) -> String {
    let b = base.trim_end_matches('/');
    let b = if let Some(r) = b.strip_prefix("https://") {
        format!("wss://{r}")
    } else if let Some(r) = b.strip_prefix("http://") {
        format!("ws://{r}")
    } else {
        b.to_string()
    };
    format!("{b}/relay")
}

/// Start the relay loop when this device has a relay configured.
pub fn ensure_running(d: &Arc<Daemon>) {
    let Ok(m) = crate::load_machine() else { return };
    if m.remote.relay_url.is_empty() || d.relay.running.swap(true, Ordering::SeqCst) {
        return;
    }
    let d = d.clone();
    tokio::spawn(async move {
        let mut backoff = 1u64;
        while let Ok(m) = crate::load_machine() {
            if m.remote.relay_url.is_empty() {
                break;
            }
            {
                let mut s = d.relay.status.lock().unwrap();
                s.url = Some(m.remote.relay_url.clone());
                s.relay_id = Some(m.remote.relay_id.clone());
            }
            match run_once(&d, &m.remote.relay_url, &m.remote.relay_id).await {
                Ok(()) => backoff = 1,
                Err(e) => {
                    tracing::warn!("relay: {}", e.message);
                    d.relay.status.lock().unwrap().last_error = Some(e.message);
                }
            }
            d.relay.status.lock().unwrap().connected = false;
            *d.relay.tx.lock().unwrap() = None;
            tokio::time::sleep(Duration::from_secs(backoff)).await;
            backoff = (backoff * 2).min(60);
        }
        d.relay.running.store(false, Ordering::SeqCst);
    });
}

async fn run_once(d: &Arc<Daemon>, url: &str, relay_id: &str) -> Result<()> {
    let key = device_key()?;
    let m = crate::load_machine()?;
    let (sock, _) = tokio_tungstenite::connect_async(ws_url(url))
        .await
        .map_err(|e| Error::network(format!("cannot reach the relay at {url}: {e}")))?;
    let (mut wr, mut rd) = sock.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<FromDevice>();
    *d.relay.tx.lock().unwrap() = Some(tx.clone());
    let writer = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            let text = serde_json::to_string(&msg).unwrap_or_default();
            if wr
                .send(tokio_tungstenite::tungstenite::Message::Text(text.into()))
                .await
                .is_err()
            {
                break;
            }
        }
    });
    while let Some(frame) = rd.next().await {
        let frame = frame.map_err(|e| Error::network(format!("relay connection: {e}")))?;
        let text = match frame {
            tokio_tungstenite::tungstenite::Message::Text(t) => t.to_string(),
            tokio_tungstenite::tungstenite::Message::Close(_) => break,
            _ => continue,
        };
        let Ok(msg) = serde_json::from_str::<ToDevice>(&text) else {
            continue;
        };
        match msg {
            ToDevice::Challenge {
                relay_id: rid,
                nonce,
            } => {
                if !relay_id.is_empty() && rid != relay_id {
                    return Err(Error::new(Code::Denied, "the relay has another identity"));
                }
                let sig = key.sign(&proto::auth_message(&rid, &m.device_id, &nonce));
                let _ = tx.send(FromDevice::Auth {
                    device_id: m.device_id.clone(),
                    device_name: m.name.clone(),
                    pubkey: pubkey_hex(&key),
                    signature: hex::encode(sig.to_bytes()),
                    bind_code: None,
                });
            }
            ToDevice::Welcome {
                public_url,
                features,
                ..
            } => {
                let mut s = d.relay.status.lock().unwrap();
                s.connected = true;
                s.last_error = None;
                s.public_url = Some(public_url);
                s.features = features;
                drop(s);
                let _ = tx.send(FromDevice::Presence {
                    projects: remote_projects().unwrap_or_default(),
                });
            }
            ToDevice::Call {
                call_id,
                connection_id,
                agent,
                tool,
                args,
            } => {
                let d = d.clone();
                let tx = tx.clone();
                tokio::spawn(async move {
                    let r = call_tool(&d, &connection_id, &agent, &tool, &args).await;
                    let msg = match r {
                        Ok(v) => FromDevice::Result {
                            call_id,
                            ok: true,
                            result: v,
                            error: None,
                        },
                        Err(e) => FromDevice::Result {
                            call_id,
                            ok: false,
                            result: json!({"code": e.code.exit_code()}),
                            error: Some(e.message),
                        },
                    };
                    let _ = tx.send(msg);
                });
            }
            ToDevice::Authorize {
                request_id,
                connection_id,
                agent,
                agent_name,
                code,
            } => {
                let d = d.clone();
                let tx = tx.clone();
                let rid = relay_id.to_string();
                tokio::spawn(async move {
                    match authorize(&d, &rid, &connection_id, &agent, &agent_name, &code).await {
                        Ok((access, projects, signature)) => {
                            let _ = tx.send(FromDevice::Grant {
                                request_id,
                                connection_id,
                                access,
                                projects,
                                signature,
                            });
                        }
                        Err(_) => {
                            let _ = tx.send(FromDevice::Deny { request_id });
                        }
                    }
                });
            }
            ToDevice::Error { message } => {
                d.relay.status.lock().unwrap().last_error = Some(message);
            }
        }
    }
    writer.abort();
    Ok(())
}

/// Whether the server this device is bound to offers a full-mode feature.
pub fn has_feature(d: &Daemon, f: &str) -> bool {
    d.relay.status().features.iter().any(|x| x == f)
}

/// A device-signed request to the full-mode server.
pub async fn signed_request(base: &str, method: &str, path: &str, body: &Value) -> Result<Value> {
    let key = device_key()?;
    let m = crate::load_machine()?;
    let raw = serde_json::to_vec(body)?;
    let t = Utc::now().timestamp();
    let sig = key.sign(&proto::http_message(method, path, t, &raw));
    let url = format!("{}{path}", base.trim_end_matches('/'));
    let m2 = reqwest::Method::from_bytes(method.as_bytes())
        .map_err(|e| Error::internal(e.to_string()))?;
    let r = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| Error::internal(e.to_string()))?
        .request(m2, &url)
        .header(proto::H_DEVICE, &m.device_id)
        .header(proto::H_TIME, t.to_string())
        .header(proto::H_SIGNATURE, hex::encode(sig.to_bytes()))
        .header("content-type", "application/json")
        .body(raw)
        .send()
        .await
        .map_err(|e| Error::network(format!("cannot reach the server at {base}: {e}")))?;
    let status = r.status();
    let v: Value = r.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        let msg = v["error"].as_str().unwrap_or("request failed").to_string();
        return Err(Error::network(format!(
            "server: {msg} (HTTP {})",
            status.as_u16()
        )));
    }
    Ok(v)
}

/// The bound server's base URL.
pub fn server_base(d: &Daemon) -> Option<String> {
    let s = d.relay.status();
    s.public_url.or(s.url).filter(|u| !u.is_empty())
}

/// Projects on this device with remote access turned on.
fn remote_projects() -> Result<Vec<String>> {
    let m = crate::load_machine()?;
    Ok(m.projects
        .iter()
        .filter(|(_, b)| {
            ProjectConfig::load(&workroam_core::paths::expand_tilde(&b.path))
                .ok()
                .flatten()
                .map(|c| c.remote.enabled)
                .unwrap_or(false)
        })
        .map(|(n, _)| n.clone())
        .collect())
}

/// The device's own check of a remote call (never trusts the relay).
pub async fn check_grant(
    d: &Arc<Daemon>,
    connection_id: &str,
    project: &str,
    write: bool,
) -> Result<()> {
    let ws = d.ws().await?;
    let g = ws
        .control
        .get::<Grant>(&keys::grant(connection_id))
        .await?
        .map(|(g, _)| g)
        .filter(|g| !g.revoked)
        .ok_or_else(|| Error::new(Code::Denied, "this connection is not authorized"))?;
    if !g.projects.iter().any(|p| p == project) {
        return Err(Error::new(
            Code::Denied,
            format!("this connection may not access {project}"),
        ));
    }
    if write && g.access != "sync" {
        return Err(Error::new(Code::Denied, "this connection is read-only"));
    }
    if let Some(b) = ws.machine.projects.get(project) {
        let on = ProjectConfig::load(&workroam_core::paths::expand_tilde(&b.path))?
            .map(|c| c.remote.enabled)
            .unwrap_or(false);
        if !on {
            return Err(
                Error::new(Code::Denied, format!("remote access is off for {project}")).with_hint(
                    format!("workroam config set remote.enabled true -p {project}"),
                ),
            );
        }
    }
    Ok(())
}

async fn call_tool(
    d: &Arc<Daemon>,
    connection_id: &str,
    agent: &str,
    tool: &str,
    args: &Value,
) -> Result<Value> {
    let ctx = Ctx {
        client: ClientKind::Relay,
        agent: Some(agent.to_string()),
        origin: Some(Origin {
            connection_id: connection_id.to_string(),
            agent: agent.to_string(),
        }),
    };
    let ws = d.ws().await?;
    if tool == "list_projects" {
        let g = ws
            .control
            .get::<Grant>(&keys::grant(connection_id))
            .await?
            .map(|(g, _)| g)
            .filter(|g| !g.revoked)
            .ok_or_else(|| Error::new(Code::Denied, "this connection is not authorized"))?;
        let mut out = Vec::new();
        for name in &g.projects {
            let pid = ws.project_id(name);
            let head = ws.head(name).await?.map(|(h, _)| h);
            let lease = workroam_lease::read(&ws.control, &pid)
                .await?
                .map(|(l, _)| l);
            out.push(json!({
                "project": name,
                "archived": head.as_ref().map(|h| h.state == HeadState::Archived).unwrap_or(false),
                "last_sync": head.as_ref().map(|h| h.created_at),
                "last_device": head.as_ref().map(|h| h.device_name.clone()),
                "lease_holder": lease.filter(|l| !workroam_lease::is_stale(l)).map(|l| l.holder.name),
            }));
        }
        return Ok(json!(out));
    }
    let project = p_req(args, "project")?;
    let write = matches!(tool, "request_push" | "request_pull");
    check_grant(d, connection_id, &project, write).await?;
    let pa = json!({"project": project});
    match tool {
        "status" => {
            let s = crate::handlers::handle(d, &ctx, "status", pa).await?;
            let p = s["projects"].get(0).cloned().unwrap_or(Value::Null);
            Ok(json!({
                "project": project,
                "device_online": true,
                "lease": p["lease"],
                "head": p["head"],
                "local_changes": p["local_changes"],
                "incoming": p["incoming"],
                "handoff": {"exists": p["handoff"]["exists"], "time": p["handoff"]["time"]},
            }))
        }
        "history" => {
            let mut a = pa.clone();
            a["limit"] = args.get("limit").cloned().unwrap_or(json!(10));
            crate::handlers::handle(d, &ctx, "history.list", a).await
        }
        "read_handoff" => {
            let head = ws
                .head(&project)
                .await?
                .and_then(|(h, _)| h.snapshot)
                .ok_or_else(|| Error::invalid("the project has no snapshot yet"))?;
            let snap = ws.snap.clone();
            let text = workroam_engine::blocking(move || {
                snap.read_file(&head, workroam_adapters::handoff::PATH)
            })?
            .map(|b| String::from_utf8_lossy(&b).to_string())
            .unwrap_or_default();
            Ok(json!({"project": project, "handoff": text}))
        }
        "request_push" | "request_pull" => {
            let method = if tool == "request_push" {
                "push.plan"
            } else {
                "pull.plan"
            };
            let mut a = pa.clone();
            a["release"] = args.get("release").cloned().unwrap_or(json!(false));
            a["trigger"] = json!("remote");
            let plans = crate::handlers::handle(d, &ctx, method, a).await?;
            let first = plans.get(0).cloned().unwrap_or(Value::Null);
            if let Some(b) = first.get("blocker").filter(|b| !b.is_null()) {
                return Err(Error::new(
                    Code::Conflict,
                    b["message"].as_str().unwrap_or("blocked").to_string(),
                ));
            }
            let job = first["job"].as_str().unwrap_or_default().to_string();
            crate::jobs::start(d, &ctx, &job, true, true).await
        }
        other => Err(Error::invalid(format!("unknown tool {other}"))),
    }
}

/// OAuth confirmation card (§16.4 steps 2 – 4). Returns the signed grant.
async fn authorize(
    d: &Arc<Daemon>,
    relay_id: &str,
    connection_id: &str,
    agent: &str,
    agent_name: &str,
    code: &str,
) -> Result<(String, Vec<String>, String)> {
    let ws = d.ws().await?;
    let projects: Vec<String> = ws
        .projects()
        .await?
        .into_iter()
        .filter(|p| !p.archived)
        .map(|p| p.name)
        .collect();
    let a = Approval {
        id: workroam_core::crypto::random_id("ap"),
        job_id: None,
        kind: "connect".into(),
        project: None,
        initiator: agent_name.to_string(),
        remote: true,
        title: format!("Connect {agent_name}"),
        details: json!({"agent": agent, "projects": projects, "access_options": ["read-only", "sync"]}),
        code: Some(code.to_string()),
        created_at: Utc::now(),
        timeout_s: 600,
    };
    let ans = d.approvals.ask(d, a).await?;
    let access = ans.access.clone().unwrap_or_else(|| "read-only".into());
    if !matches!(access.as_str(), "read-only" | "sync") {
        return Err(Error::invalid("access is read-only or sync"));
    }
    let chosen: Vec<String> = ans
        .projects
        .into_iter()
        .filter(|p| projects.contains(p))
        .collect();
    let key = device_key()?;
    let sig = key.sign(&proto::grant_message(
        relay_id,
        connection_id,
        &access,
        &chosen,
    ));
    let url = crate::load_machine()?.remote.relay_url;
    let g = Grant {
        connection_id: connection_id.to_string(),
        agent: agent.to_string(),
        agent_name: agent_name.to_string(),
        relay_url: url,
        relay_id: relay_id.to_string(),
        access: access.clone(),
        projects: chosen.clone(),
        created_at: Utc::now(),
        created_by: ws.device(),
        revoked: false,
        webhook_url: None,
    };
    ws.control.put(&keys::grant(connection_id), &g).await?;
    Ok((access, chosen, hex::encode(sig.to_bytes())))
}

const INSTALL_SERVER: &str = "curl -fsSL https://github.com/kverona-ai/workroam/releases/latest/download/install-server.sh | sh\nexport PATH=\"$HOME/.local/bin:$PATH\"";

/// `agent.connect`: local agents get their files; cloud agents get a relay.
pub async fn connect(d: &Arc<Daemon>, _ctx: &Ctx, p: &Value) -> Result<Value> {
    let id = p_req(p, "agent")?;
    let a = workroam_adapters::registry::by_id(&id).ok_or_else(|| {
        Error::invalid(format!("unknown agent {id}")).with_hint("workroam agent list")
    })?;
    if a.is_local() {
        return crate::handlers::connect_local(d, &a).await;
    }
    let ws = d.ws().await?;
    ws.ensure_writable()?;
    let mut m = crate::load_machine()?;
    if let Some(url) = p_str(p, "relay") {
        let url = url.trim_end_matches('/').to_string();
        if !url.starts_with("https://")
            && !url.starts_with("http://127.0.0.1")
            && !url.starts_with("http://localhost")
        {
            return Err(Error::invalid("the relay address must be https://")
                .with_hint("use the public HTTPS address the relay printed"));
        }
        if m.remote.relay_id.is_empty() {
            m.remote.relay_id = workroam_core::crypto::random_id("r");
        }
        let rid = m.remote.relay_id.clone();
        let u2 = url.clone();
        d.update_machine(move |x| {
            x.remote.relay_url = u2;
            x.remote.relay_id = rid;
        })
        .await?;
        let rec = RelayRecord {
            relay_id: m.remote.relay_id.clone(),
            url: url.clone(),
            bound_at: Utc::now(),
            devices: vec![ws.machine.device_id.clone()],
            default_device: Some(ws.machine.device_id.clone()),
            remote_projects: vec![],
        };
        ws.control.put(&keys::relay(&rec.relay_id), &rec).await?;
        ensure_running(d);
        for _ in 0..20 {
            if d.relay.status().connected {
                break;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        m = crate::load_machine()?;
    }
    if m.remote.relay_url.is_empty() {
        // No relay yet: deployment instructions with a bind code (§18.3).
        if m.remote.relay_id.is_empty() {
            let rid = workroam_core::crypto::random_id("r");
            m.remote.relay_id = rid.clone();
            d.update_machine(move |x| x.remote.relay_id = rid).await?;
        }
        let key = device_key()?;
        let code = BindCode {
            relay_id: m.remote.relay_id.clone(),
            device_id: m.device_id.clone(),
            device_name: m.name.clone(),
            pubkey: pubkey_hex(&key),
            expires: (Utc::now() + chrono::Duration::minutes(30)).timestamp(),
        }
        .encode();
        let text = format!(
            "Please deploy my Workroam relay on your cloud computer and keep it running:\n\n{INSTALL_SERVER}\nnohup workroam-server --bind {code} --tunnel > ~/.workroam-server.log 2>&1 &\nsleep 15; cat ~/.workroam-server.log\n\nReport the public https:// address it prints. If the computer restarts, start it again with `workroam-server --tunnel`; tell me if the address changes."
        );
        return Ok(json!({
            "agent": a.id,
            "needs_relay": true,
            "bind_code": code,
            "instructions": text,
            "next": format!("workroam agent connect {} --relay <address>", a.id),
        }));
    }
    let base = d
        .relay
        .status()
        .public_url
        .unwrap_or(m.remote.relay_url.clone());
    let mcp_url = format!("{}/mcp", base.trim_end_matches('/'));
    let prompt = a
        .connect
        .setup
        .chat_prompt
        .clone()
        .unwrap_or_else(|| format!("Add a remote MCP connector named Workroam with the URL {mcp_url} and sign in when asked."));
    Ok(json!({
        "agent": a.id,
        "needs_relay": false,
        "relay": m.remote.relay_url,
        "connected": d.relay.status().connected,
        "mcp_url": mcp_url,
        "open_url": a.connect.setup.open_url,
        "instructions": prompt.replace("{url}", &mcp_url),
        "skill": workroam_adapters::SKILL_CLOUD_MD,
        "next": "approve the connection card that appears when the agent signs in",
    }))
}

pub async fn disconnect(d: &Arc<Daemon>, p: &Value) -> Result<Value> {
    let id = p_req(p, "agent")?;
    let a = workroam_adapters::registry::by_id(&id)
        .ok_or_else(|| Error::invalid(format!("unknown agent {id}")))?;
    if a.is_local() {
        return crate::handlers::disconnect_local(d, &a).await;
    }
    let ws = d.ws().await?;
    let mut revoked = Vec::new();
    for (k, mut g) in ws.control.get_all::<Grant>(keys::GRANTS).await? {
        if g.agent == id && !g.revoked {
            g.revoked = true;
            ws.control.put(&k, &g).await?;
            d.relay.send(FromDevice::Revoke {
                connection_id: g.connection_id.clone(),
            });
            revoked.push(g.connection_id);
        }
    }
    Ok(json!({"agent": id, "revoked": revoked}))
}
