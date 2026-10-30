//! `/relay`: the devices' outbound WebSocket (architecture §16.4, §18.3).
//! A device proves its Ed25519 key on a fresh challenge; the first device
//! is admitted by the one-time bind code given at `--bind`.

use crate::app::{AuthStatus, CallResult, Session, Shared};
use crate::state::{ConnRec, DeviceRec};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::Response;
use chrono::Utc;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use tokio::sync::mpsc;
use workroam_ipc::relay::{self as proto, FromDevice, ToDevice};

/// How long a queued remote push or pull waits for its device.
pub const QUEUE_TTL: Duration = Duration::from_secs(24 * 3600);

pub async fn ws(State(app): State<Shared>, up: WebSocketUpgrade) -> Response {
    up.max_message_size(4 << 20)
        .on_upgrade(move |s| session(app, s))
}

pub fn verify(pubkey_hex: &str, msg: &[u8], sig_hex: &str) -> bool {
    let Ok(pk) = hex::decode(pubkey_hex) else {
        return false;
    };
    let Ok(pk) = <[u8; 32]>::try_from(pk.as_slice()) else {
        return false;
    };
    let Ok(vk) = VerifyingKey::from_bytes(&pk) else {
        return false;
    };
    let Ok(sig) = hex::decode(sig_hex) else {
        return false;
    };
    let Ok(sig) = <[u8; 64]>::try_from(sig.as_slice()) else {
        return false;
    };
    vk.verify(msg, &Signature::from_bytes(&sig)).is_ok()
}

fn text(m: &ToDevice) -> Message {
    Message::Text(serde_json::to_string(m).unwrap_or_default().into())
}

async fn session(app: Shared, sock: WebSocket) {
    let (mut wr, mut rd) = sock.split();
    let Some(relay_id) = app.persist.lock().unwrap().relay_id.clone() else {
        let _ = wr
            .send(text(&ToDevice::Error {
                message: "this relay is not bound yet: start it with workroam-server --bind <code>"
                    .into(),
            }))
            .await;
        return;
    };
    let nonce = hex::encode(workroam_core::crypto::random_bytes::<24>());
    if wr
        .send(text(&ToDevice::Challenge {
            relay_id: relay_id.clone(),
            nonce: nonce.clone(),
        }))
        .await
        .is_err()
    {
        return;
    }
    // The first message must authenticate, within 20 seconds.
    let first = tokio::time::timeout(Duration::from_secs(20), rd.next()).await;
    let Ok(Some(Ok(Message::Text(t)))) = first else {
        return;
    };
    let Ok(FromDevice::Auth {
        device_id,
        device_name,
        pubkey,
        signature,
        ..
    }) = serde_json::from_str::<FromDevice>(&t)
    else {
        return;
    };
    if !verify(
        &pubkey,
        &proto::auth_message(&relay_id, &device_id, &nonce),
        &signature,
    ) {
        let _ = wr
            .send(text(&ToDevice::Error {
                message: "signature check failed".into(),
            }))
            .await;
        return;
    }
    let admitted = app.update(|p| {
        if let Some(d) = p.devices.iter_mut().find(|d| d.id == device_id) {
            if d.pubkey != pubkey {
                return false;
            }
            d.name = device_name.clone();
            d.last_seen = Some(Utc::now());
            return true;
        }
        let bound = p.bind.as_ref().is_some_and(|b| {
            b.device_id == device_id && b.pubkey == pubkey && b.expires >= Utc::now().timestamp()
        });
        if bound {
            p.bind = None;
            p.devices.push(DeviceRec {
                id: device_id.clone(),
                name: device_name.clone(),
                pubkey: pubkey.clone(),
                bound_at: Utc::now(),
                last_seen: Some(Utc::now()),
            });
            tracing::info!("bound to device {device_name}");
            println!("bound to device {device_name}");
        }
        bound
    });
    if !admitted {
        let _ = wr
            .send(text(&ToDevice::Error {
                message: "this device is not bound to this relay (the bind code may have expired)"
                    .into(),
            }))
            .await;
        return;
    }
    let (tx, mut rx) = mpsc::unbounded_channel::<ToDevice>();
    let devices: Vec<String> = app
        .persist
        .lock()
        .unwrap()
        .devices
        .iter()
        .map(|d| d.name.clone())
        .collect();
    let _ = tx.send(ToDevice::Welcome {
        relay_id: relay_id.clone(),
        public_url: app.base(),
        devices,
        features: app.features(),
    });
    {
        let mut live = app.live.lock().unwrap();
        // Replay what waited for this device.
        if let Some(q) = live.queue.remove(&device_id) {
            for item in q.into_iter().filter(|q| q.at.elapsed() < QUEUE_TTL) {
                let _ = tx.send(item.msg);
            }
        }
        // Authorizations still waiting for any device.
        for r in live
            .auth
            .values()
            .filter(|r| matches!(r.status, AuthStatus::Pending))
        {
            let _ = tx.send(ToDevice::Authorize {
                request_id: r.poll_secret.clone(),
                connection_id: r.connection_id.clone(),
                agent: r.agent.clone(),
                agent_name: r.client_name.clone(),
                code: r.code.clone(),
            });
        }
        live.sessions.insert(
            device_id.clone(),
            Session {
                tx: tx.clone(),
                name: device_name.clone(),
                since: Utc::now(),
                projects: vec![],
            },
        );
    }
    tracing::info!("device {device_name} connected");
    let writer = tokio::spawn(async move {
        let mut ping = tokio::time::interval(Duration::from_secs(30));
        loop {
            tokio::select! {
                m = rx.recv() => match m {
                    Some(m) => if wr.send(text(&m)).await.is_err() { break },
                    None => break,
                },
                _ = ping.tick() => if wr.send(Message::Ping(Vec::new().into())).await.is_err() { break },
            }
        }
    });
    while let Some(Ok(frame)) = rd.next().await {
        let Message::Text(t) = frame else {
            if matches!(frame, Message::Close(_)) {
                break;
            }
            continue;
        };
        match serde_json::from_str::<FromDevice>(&t) {
            Ok(m) => handle(&app, &device_id, &relay_id, m),
            Err(e) => tracing::debug!("bad frame from {device_name}: {e}"),
        }
    }
    writer.abort();
    {
        let mut live = app.live.lock().unwrap();
        if live
            .sessions
            .get(&device_id)
            .is_some_and(|s| s.tx.same_channel(&tx))
        {
            live.sessions.remove(&device_id);
        }
    }
    app.update(|p| {
        if let Some(d) = p.devices.iter_mut().find(|d| d.id == device_id) {
            d.last_seen = Some(Utc::now());
        }
    });
    tracing::info!("device {device_name} disconnected");
}

fn handle(app: &Shared, device_id: &str, relay_id: &str, m: FromDevice) {
    match m {
        FromDevice::Auth { .. } => {}
        FromDevice::Result {
            call_id,
            ok,
            result,
            error,
        } => {
            let mut live = app.live.lock().unwrap();
            if let Some(tx) = live.calls.remove(&call_id) {
                let _ = tx.send(CallResult { ok, result, error });
            }
            live.detached.retain(|c| c != &call_id);
        }
        FromDevice::Grant {
            request_id,
            connection_id,
            access,
            projects,
            signature,
        } => {
            let pubkey = app
                .persist
                .lock()
                .unwrap()
                .device(device_id)
                .map(|d| d.pubkey.clone());
            let Some(pubkey) = pubkey else { return };
            if !verify(
                &pubkey,
                &proto::grant_message(relay_id, &connection_id, &access, &projects),
                &signature,
            ) {
                tracing::warn!("grant with a bad signature from {device_id}");
                return;
            }
            let req = {
                let live = app.live.lock().unwrap();
                live.auth.get(&request_id).cloned()
            };
            let Some(req) = req.filter(|r| {
                r.connection_id == connection_id && matches!(r.status, AuthStatus::Pending)
            }) else {
                return;
            };
            app.update(|p| {
                p.connections.retain(|c| c.connection_id != connection_id);
                p.connections.push(ConnRec {
                    connection_id: connection_id.clone(),
                    client_id: req.client_id.clone(),
                    agent: req.agent.clone(),
                    agent_name: req.client_name.clone(),
                    access: access.clone(),
                    projects: projects.clone(),
                    device_id: device_id.to_string(),
                    signature: signature.clone(),
                    created_at: Utc::now(),
                    revoked: false,
                });
            });
            let code = crate::oauth::new_secret("wrc_");
            let mut redirect = url::Url::parse(&req.redirect_uri).expect("checked at authorize");
            redirect.query_pairs_mut().append_pair("code", &code);
            if let Some(s) = &req.state {
                redirect.query_pairs_mut().append_pair("state", s);
            }
            let mut live = app.live.lock().unwrap();
            live.codes.insert(
                code,
                crate::app::AuthCode {
                    connection_id,
                    client_id: req.client_id.clone(),
                    redirect_uri: req.redirect_uri.clone(),
                    code_challenge: req.code_challenge.clone(),
                    created: std::time::Instant::now(),
                },
            );
            if let Some(r) = live.auth.get_mut(&request_id) {
                r.status = AuthStatus::Granted {
                    redirect: redirect.to_string(),
                };
            }
        }
        FromDevice::Deny { request_id } => {
            if let Some(r) = app.live.lock().unwrap().auth.get_mut(&request_id) {
                if matches!(r.status, AuthStatus::Pending) {
                    r.status = AuthStatus::Denied;
                }
            }
        }
        FromDevice::Revoke { connection_id } => {
            app.update(|p| {
                for c in p
                    .connections
                    .iter_mut()
                    .filter(|c| c.connection_id == connection_id)
                {
                    c.revoked = true;
                }
                p.tokens.retain(|t| t.connection_id != connection_id);
            });
            tracing::info!("connection {connection_id} revoked");
        }
        FromDevice::AddDevice {
            device_id: id,
            device_name,
            pubkey,
        } => {
            if hex::decode(&pubkey).map(|b| b.len()) != Ok(32) {
                return;
            }
            app.update(|p| {
                p.devices.retain(|d| d.id != id);
                p.devices.push(DeviceRec {
                    id,
                    name: device_name,
                    pubkey,
                    bound_at: Utc::now(),
                    last_seen: None,
                });
            });
        }
        FromDevice::Presence { projects } => {
            if let Some(s) = app.live.lock().unwrap().sessions.get_mut(device_id) {
                s.projects = projects;
            }
        }
    }
}
