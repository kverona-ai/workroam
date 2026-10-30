//! IPC client used by the CLI, `workroam mcp`, the desktop app and the
//! device relay. Calls are matched to responses by id; notifications go to
//! an event queue.

use crate::protocol::{
    ClientKind, Hello, HelloReply, Message, Notification, Request, Response, PROTOCOL,
};
use crate::transport::{self, Stream};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};
use workroam_core::{Code, Error, Result};

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Response>>>>;

pub struct Client {
    tx: mpsc::UnboundedSender<String>,
    pending: Pending,
    next: AtomicU64,
    events: tokio::sync::Mutex<mpsc::UnboundedReceiver<Notification>>,
    pub hello: HelloReply,
}

fn unavailable(e: impl std::fmt::Display) -> Error {
    Error::new(
        Code::DaemonUnavailable,
        format!("the Workroam background service is not running ({e})"),
    )
    .with_hint("start it with `workroamd` or open the Workroam app")
}

impl Client {
    /// Connect to the daemon at the default endpoint.
    pub async fn connect(kind: ClientKind, agent: Option<String>) -> Result<Client> {
        Self::connect_to(&workroam_core::paths::socket_path(), kind, agent).await
    }

    pub async fn connect_to(
        path: &Path,
        kind: ClientKind,
        agent: Option<String>,
    ) -> Result<Client> {
        let s = transport::connect(path).await.map_err(unavailable)?;
        Self::over(s, kind, agent).await
    }

    /// Run the protocol over any byte stream (in-process daemon, tests).
    pub async fn over(stream: Stream, kind: ClientKind, agent: Option<String>) -> Result<Client> {
        let (rd, mut wr) = tokio::io::split(stream);
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        let (ev_tx, ev_rx) = mpsc::unbounded_channel();
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        tokio::spawn(async move {
            while let Some(line) = rx.recv().await {
                if wr.write_all(line.as_bytes()).await.is_err() {
                    break;
                }
                let _ = wr.flush().await;
            }
        });
        let p2 = pending.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(rd).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                match Message::parse(&line) {
                    Ok(Message::Response(r)) => {
                        if let Some(id) = r.id.as_u64() {
                            if let Some(s) = p2.lock().unwrap().remove(&id) {
                                let _ = s.send(r);
                            }
                        }
                    }
                    Ok(Message::Notification(n)) => {
                        let _ = ev_tx.send(n);
                    }
                    _ => {}
                }
            }
            // Connection closed: fail every waiting call.
            p2.lock().unwrap().clear();
        });
        let mut c = Client {
            tx,
            pending,
            next: AtomicU64::new(1),
            events: tokio::sync::Mutex::new(ev_rx),
            hello: HelloReply {
                protocol: PROTOCOL,
                version: String::new(),
                device: String::new(),
                configured: false,
            },
        };
        let reply = c
            .call(
                "hello",
                serde_json::to_value(Hello {
                    protocol: PROTOCOL,
                    client: kind,
                    version: env!("CARGO_PKG_VERSION").into(),
                    agent,
                })?,
            )
            .await?;
        c.hello = serde_json::from_value(reply)?;
        Ok(c)
    }

    pub async fn call(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (s, r) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, s);
        let line = Message::Request(Request::new(id, method, params)).to_line();
        self.tx
            .send(line)
            .map_err(|_| unavailable("connection closed"))?;
        match r.await {
            Ok(resp) => resp.into_result(),
            Err(_) => Err(unavailable("connection closed")),
        }
    }

    /// Typed convenience wrapper around [`Client::call`].
    pub async fn call_as<T: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        params: Value,
    ) -> Result<T> {
        Ok(serde_json::from_value(self.call(method, params).await?)?)
    }

    /// Next server event; `None` once the connection is closed.
    pub async fn next_event(&self) -> Option<Notification> {
        self.events.lock().await.recv().await
    }
}
