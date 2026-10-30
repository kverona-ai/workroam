//! Serving IPC connections: `hello` first, then requests run concurrently;
//! `subscribe` forwards server events on the same connection.

use crate::{Ctx, Daemon};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;
use workroam_core::{Code, Error};
use workroam_ipc::transport::{Listener, Stream};
use workroam_ipc::{ClientKind, Hello, HelloReply, Message, Response, PROTOCOL};

pub async fn serve_conn(d: Arc<Daemon>, stream: Stream) {
    let (rd, mut wr) = tokio::io::split(stream);
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let writer = tokio::spawn(async move {
        while let Some(line) = rx.recv().await {
            if wr.write_all(line.as_bytes()).await.is_err() {
                break;
            }
            let _ = wr.flush().await;
        }
    });
    let mut lines = BufReader::new(rd).lines();
    let mut ctx: Option<Ctx> = None;
    let mut forwarding: Option<tokio::task::JoinHandle<()>> = None;
    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        let req = match Message::parse(&line) {
            Ok(Message::Request(r)) => r,
            Ok(_) => continue,
            Err(e) => {
                let _ = tx.send(Message::Response(Response::err(Value::Null, &e)).to_line());
                continue;
            }
        };
        let id = req.id.clone().unwrap_or(Value::Null);
        if req.method == "hello" {
            let reply = match serde_json::from_value::<Hello>(req.params.clone()) {
                Ok(h) if h.protocol == PROTOCOL => {
                    ctx = Some(Ctx {
                        client: h.client,
                        agent: h.agent.clone(),
                        origin: None,
                    });
                    let device = crate::load_machine().map(|m| m.name).unwrap_or_default();
                    Response::ok(
                        id,
                        serde_json::to_value(HelloReply {
                            protocol: PROTOCOL,
                            version: env!("CARGO_PKG_VERSION").into(),
                            device,
                            configured: crate::is_configured(),
                        })
                        .unwrap_or_default(),
                    )
                }
                Ok(h) => Response::err(
                    id,
                    &Error::invalid(format!(
                        "protocol {} is not supported (this service speaks {PROTOCOL})",
                        h.protocol
                    ))
                    .with_hint("update Workroam on this computer"),
                ),
                Err(e) => Response::err(id, &Error::invalid(format!("bad hello: {e}"))),
            };
            let _ = tx.send(Message::Response(reply).to_line());
            continue;
        }
        let Some(c) = ctx.clone() else {
            let _ = tx.send(
                Message::Response(Response::err(id, &Error::invalid("send hello first"))).to_line(),
            );
            continue;
        };
        let mut c = c;
        // Only the relay client may speak for a remote agent.
        if let Some(o) = req.origin.clone() {
            if c.client != ClientKind::Relay {
                let _ = tx.send(
                    Message::Response(Response::err(
                        id,
                        &Error::new(Code::Denied, "origin is reserved for the device relay"),
                    ))
                    .to_line(),
                );
                continue;
            }
            c.origin = Some(o);
        }
        if req.method == "subscribe" {
            if forwarding.is_none() {
                let mut ev = d.events.subscribe();
                let tx2 = tx.clone();
                forwarding = Some(tokio::spawn(async move {
                    loop {
                        match ev.recv().await {
                            Ok(n) => {
                                if tx2.send(Message::Notification(n).to_line()).is_err() {
                                    break;
                                }
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                            Err(_) => break,
                        }
                    }
                }));
            }
            let _ =
                tx.send(Message::Response(Response::ok(id, json!({"subscribed": true}))).to_line());
            continue;
        }
        let d2 = d.clone();
        let tx2 = tx.clone();
        tokio::spawn(async move {
            let resp = match crate::handlers::handle(&d2, &c, &req.method, req.params).await {
                Ok(v) => Response::ok(id, v),
                Err(e) => Response::err(id, &e),
            };
            let _ = tx2.send(Message::Response(resp).to_line());
        });
    }
    if let Some(f) = forwarding {
        f.abort();
    }
    drop(tx);
    let _ = writer.await;
}

/// Accept connections forever.
pub async fn listen(d: Arc<Daemon>, path: &std::path::Path) -> std::io::Result<()> {
    let mut l = Listener::bind(path).await?;
    tracing::info!("listening on {}", path.display());
    loop {
        let s = l.accept().await?;
        tokio::spawn(serve_conn(d.clone(), s));
    }
}

/// An in-process daemon for one CLI command (no background service).
pub async fn in_process(
    kind: ClientKind,
    agent: Option<String>,
) -> workroam_core::Result<workroam_ipc::Client> {
    let d = Daemon::new(false);
    let (a, b) = tokio::io::duplex(1 << 20);
    tokio::spawn(serve_conn(d, Box::new(b)));
    workroam_ipc::Client::over(Box::new(a), kind, agent).await
}
