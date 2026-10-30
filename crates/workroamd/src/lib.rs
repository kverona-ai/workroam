//! `workroamd`: the one place where Workroam's business logic runs
//! (architecture §3.1, §15, §17.1). The desktop app, the CLI, `workroam
//! mcp` and the device relay are all IPC clients of this daemon. The CLI can
//! also run it in-process when no background service is running.

pub mod approvals;
pub mod handlers;
pub mod jobs;
pub mod notify;
pub mod relay;
pub mod server;
pub mod settings;
pub mod triggers;

use chrono::{DateTime, Utc};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;
use workroam_core::config::MachineConfig;
use workroam_core::{Code, Error, Result};
use workroam_engine::Workspace;
use workroam_ipc::{ClientKind, Notification, Origin};
use workroam_platform::secrets;
use workroam_store::Creds;

/// Who is calling (from the `hello` handshake and the relay envelope).
#[derive(Debug, Clone)]
pub struct Ctx {
    pub client: ClientKind,
    /// An agent drives this client (MCP, relay, or the CLI run by an agent).
    pub agent: Option<String>,
    pub origin: Option<Origin>,
}

impl Ctx {
    pub fn human(client: ClientKind) -> Ctx {
        Ctx {
            client,
            agent: None,
            origin: None,
        }
    }
    pub fn is_agent(&self) -> bool {
        self.client.is_agent() || self.agent.is_some() || self.origin.is_some()
    }
    pub fn is_remote(&self) -> bool {
        self.origin.is_some() || self.client == ClientKind::Relay
    }
    /// Shown on approval cards as the initiator.
    pub fn initiator(&self) -> String {
        if let Some(o) = &self.origin {
            return o.agent.clone();
        }
        if let Some(a) = &self.agent {
            return a.clone();
        }
        self.client.as_str().to_string()
    }
}

pub struct Daemon {
    /// Running as the background service (auto-sync, relay) rather than
    /// in-process for one CLI command.
    pub service: bool,
    ws: tokio::sync::Mutex<Option<Arc<Workspace>>>,
    pub events: broadcast::Sender<Notification>,
    pub jobs: jobs::Jobs,
    pub approvals: approvals::Approvals,
    pub setup: tokio::sync::Mutex<HashMap<String, workroam_setup::session::SetupSession>>,
    pub paused_until: Mutex<Option<DateTime<Utc>>>,
    /// "Don't ask again for 1 hour" per project.
    pub skip_ask: Mutex<HashMap<String, DateTime<Utc>>>,
    /// Last failed job per project (shown by `status`).
    pub last_failure: Mutex<HashMap<String, Value>>,
    /// Snapshots already announced as arrivals.
    pub announced: Mutex<HashMap<String, String>>,
    pub started_at: DateTime<Utc>,
    pub relay: relay::RelayState,
}

impl Daemon {
    pub fn new(service: bool) -> Arc<Daemon> {
        let (events, _) = broadcast::channel(512);
        let paused = load_machine()
            .ok()
            .and_then(|m| DateTime::parse_from_rfc3339(&m.sync.paused_until).ok())
            .map(|t| t.with_timezone(&Utc))
            .filter(|t| *t > Utc::now());
        Arc::new(Daemon {
            service,
            ws: tokio::sync::Mutex::new(None),
            events,
            jobs: jobs::Jobs::default(),
            approvals: approvals::Approvals::default(),
            setup: tokio::sync::Mutex::new(HashMap::new()),
            paused_until: Mutex::new(paused),
            skip_ask: Mutex::new(HashMap::new()),
            last_failure: Mutex::new(HashMap::new()),
            announced: Mutex::new(HashMap::new()),
            started_at: Utc::now(),
            relay: relay::RelayState::default(),
        })
    }

    pub fn emit(&self, method: &str, params: Value) {
        let _ = self.events.send(Notification::new(method, params));
    }

    /// The open workspace, reopened when `machine.toml` changed.
    pub async fn ws(&self) -> Result<Arc<Workspace>> {
        let machine = load_machine()?;
        let mut g = self.ws.lock().await;
        if let Some(w) = g.as_ref() {
            if w.machine.workspace == machine.workspace {
                if w.machine != machine {
                    let w2 = Arc::new(Workspace {
                        machine,
                        ..(**w).clone()
                    });
                    *g = Some(w2.clone());
                    return Ok(w2);
                }
                return Ok(w.clone());
            }
        }
        let creds = storage_creds(&machine)?;
        let password = secrets::get(secrets::keys::PASSWORD)?.ok_or_else(|| {
            Error::new(
                Code::Denied,
                "the workspace password is not saved on this device",
            )
            .with_hint("run workroam password reset --recovery-key-stdin, or set it up again")
        })?;
        let w = Arc::new(Workspace::open(machine, creds, &password).await?);
        *g = Some(w.clone());
        Ok(w)
    }

    /// Forget the open workspace (after setup, password or key changes).
    pub async fn reset_ws(&self) {
        *self.ws.lock().await = None;
    }

    /// Change `machine.toml` and the open workspace together.
    pub async fn update_machine(
        &self,
        f: impl FnOnce(&mut MachineConfig),
    ) -> Result<MachineConfig> {
        let mut m = load_machine()?;
        f(&mut m);
        m.save(&workroam_core::paths::machine_toml())?;
        let mut g = self.ws.lock().await;
        if let Some(w) = g.as_ref() {
            *g = Some(Arc::new(Workspace {
                machine: m.clone(),
                ..(**w).clone()
            }));
        }
        Ok(m)
    }

    pub fn is_paused(&self) -> bool {
        let mut p = self.paused_until.lock().unwrap();
        match *p {
            Some(t) if t > Utc::now() => true,
            Some(_) => {
                *p = None;
                false
            }
            None => false,
        }
    }
}

pub fn load_machine() -> Result<MachineConfig> {
    MachineConfig::load(&workroam_core::paths::machine_toml())?.ok_or_else(|| {
        Error::invalid("this computer has not joined a Workroam workspace yet")
            .with_hint("run workroam setup, or open the Workroam app")
    })
}

pub fn is_configured() -> bool {
    workroam_core::paths::machine_toml().exists()
}

pub fn storage_creds(m: &MachineConfig) -> Result<Option<Creds>> {
    if m.workspace.is_local() {
        return Ok(None);
    }
    Ok(Some(Creds {
        access_key: secrets::require(secrets::keys::ACCESS_KEY)?,
        secret_key: secrets::require(secrets::keys::SECRET_KEY)?,
    }))
}

/// Parameters helpers shared by the handlers.
pub(crate) fn p_str(v: &Value, k: &str) -> Option<String> {
    v.get(k)
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
}

pub(crate) fn p_bool(v: &Value, k: &str) -> bool {
    v.get(k).and_then(|x| x.as_bool()).unwrap_or(false)
}

pub(crate) fn p_req(v: &Value, k: &str) -> Result<String> {
    p_str(v, k).ok_or_else(|| Error::invalid(format!("missing parameter `{k}`")))
}

pub(crate) fn to_value<T: serde::Serialize>(t: &T) -> Result<Value> {
    Ok(serde_json::to_value(t)?)
}
