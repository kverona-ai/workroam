//! The connection to `workroamd`. The app starts the daemon it ships with
//! (a Tauri sidecar next to the executable) when none is running, keeps one
//! connection, and forwards every daemon event to the windows as the
//! `daemon` event (`{event, data}`).

use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{watch, Mutex};
use workroam_core::{Code, Error, Result};
use workroam_ipc::{Client, ClientKind};

pub struct Bridge {
    client: Mutex<Option<Arc<Client>>>,
    up: watch::Sender<bool>,
}

impl Default for Bridge {
    fn default() -> Self {
        Bridge {
            client: Mutex::new(None),
            up: watch::channel(false).0,
        }
    }
}

fn daemon_binary() -> Option<std::path::PathBuf> {
    let name = if cfg!(windows) {
        "workroamd.exe"
    } else {
        "workroamd"
    };
    let beside = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.join(name)))
        .filter(|p| p.exists());
    beside.or_else(|| {
        std::env::var_os("PATH").and_then(|paths| {
            std::env::split_paths(&paths)
                .map(|d| d.join(name))
                .find(|p| p.exists())
        })
    })
}

fn spawn_daemon() -> bool {
    let Some(bin) = daemon_binary() else {
        return false;
    };
    let mut c = std::process::Command::new(bin);
    c.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW);
    }
    c.spawn().is_ok()
}

async fn connect() -> Result<Client> {
    if let Ok(c) = Client::connect(ClientKind::Desktop, None).await {
        return Ok(c);
    }
    if spawn_daemon() {
        for _ in 0..50 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if let Ok(c) = Client::connect(ClientKind::Desktop, None).await {
                return Ok(c);
            }
        }
    }
    Client::connect(ClientKind::Desktop, None).await
}

/// Keep a connection to the daemon for the life of the app.
pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            match connect().await {
                Ok(c) => {
                    let c = Arc::new(c);
                    if c.call("subscribe", json!({})).await.is_ok() {
                        let b = app.state::<Bridge>();
                        *b.client.lock().await = Some(c.clone());
                        b.up.send_replace(true);
                        let _ = app.emit("daemon", json!({"event": "daemon.up", "data": {}}));
                        while let Some(n) = c.next_event().await {
                            let _ =
                                app.emit("daemon", json!({"event": n.method, "data": n.params}));
                        }
                        *b.client.lock().await = None;
                        b.up.send_replace(false);
                        let _ = app.emit("daemon", json!({"event": "daemon.down", "data": {}}));
                    }
                }
                Err(e) => {
                    let _ = app.emit(
                        "daemon",
                        json!({"event": "daemon.down", "data": {"message": e.message}}),
                    );
                }
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });
}

pub async fn connected(app: &AppHandle) -> bool {
    *app.state::<Bridge>().up.borrow()
}

/// Call a daemon method, waiting briefly for the connection on startup.
pub async fn call(app: &AppHandle, method: &str, params: Value) -> Result<Value> {
    let b = app.state::<Bridge>();
    let mut up = b.up.subscribe();
    if !*up.borrow() {
        let _ = tokio::time::timeout(Duration::from_secs(8), up.wait_for(|v| *v)).await;
    }
    let c = b.client.lock().await.clone().ok_or_else(|| {
        Error::new(
            Code::DaemonUnavailable,
            "the Workroam background service is not running",
        )
        .with_hint("Workroam starts it automatically; check the logs if this persists")
    })?;
    c.call(method, params).await
}
