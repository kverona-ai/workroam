//! Shared server state and the HTTP router (one port for everything).

use crate::state::Persist;
use axum::routing::{get, post};
use axum::Router;
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::{mpsc, oneshot};
use workroam_ipc::relay::ToDevice;

/// An online device's relay session.
pub struct Session {
    pub tx: mpsc::UnboundedSender<ToDevice>,
    pub name: String,
    pub since: DateTime<Utc>,
    /// Projects with remote access on (from `presence`).
    pub projects: Vec<String>,
}

/// A device's answer to a forwarded call.
pub struct CallResult {
    pub ok: bool,
    pub result: Value,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub enum AuthStatus {
    Pending,
    Granted { redirect: String },
    Denied,
}

/// An OAuth authorization waiting for the device's card.
#[derive(Clone, Debug)]
pub struct AuthRequest {
    pub poll_secret: String,
    pub client_id: String,
    pub client_name: String,
    pub agent: String,
    pub redirect_uri: String,
    pub state: Option<String>,
    pub code_challenge: String,
    pub connection_id: String,
    pub code: String,
    pub created: Instant,
    pub status: AuthStatus,
}

/// An issued authorization code (10 minutes, single use).
#[derive(Clone, Debug)]
pub struct AuthCode {
    pub connection_id: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub code_challenge: String,
    pub created: Instant,
}

/// A remote push or pull waiting for its device (24 hours at most).
pub struct Queued {
    pub at: Instant,
    pub msg: ToDevice,
}

#[derive(Default)]
pub struct Live {
    pub sessions: HashMap<String, Session>,
    pub calls: HashMap<String, oneshot::Sender<CallResult>>,
    pub auth: HashMap<String, AuthRequest>,
    pub codes: HashMap<String, AuthCode>,
    pub queue: HashMap<String, Vec<Queued>>,
    /// Call ids of queued calls (their result goes nowhere: the agent was told it is queued).
    pub detached: Vec<String>,
}

pub struct Config {
    pub listen: String,
    pub public_url: Mutex<Option<String>>,
    pub state_path: PathBuf,
    #[cfg(feature = "full")]
    pub full: Option<crate::full::FullConfig>,
}

pub struct App {
    pub cfg: Config,
    pub persist: Mutex<Persist>,
    pub live: Mutex<Live>,
}

pub type Shared = Arc<App>;

impl App {
    pub fn new(cfg: Config, persist: Persist) -> Shared {
        Arc::new(App {
            cfg,
            persist: Mutex::new(persist),
            live: Mutex::new(Live::default()),
        })
    }

    /// Change the persisted state and write it.
    pub fn update<R>(&self, f: impl FnOnce(&mut Persist) -> R) -> R {
        let mut p = self.persist.lock().unwrap();
        let r = f(&mut p);
        p.gc();
        if let Err(e) = p.save(&self.cfg.state_path) {
            tracing::error!("cannot write {}: {e}", self.cfg.state_path.display());
        }
        r
    }

    /// The public base URL (no trailing slash).
    pub fn base(&self) -> String {
        self.cfg
            .public_url
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| format!("http://{}", self.cfg.listen))
            .trim_end_matches('/')
            .to_string()
    }

    /// OAuth tokens only over HTTPS (loopback is allowed for local testing).
    pub fn https_ok(&self) -> bool {
        let b = self.base();
        if b.starts_with("https://") {
            return true;
        }
        url::Url::parse(&b)
            .ok()
            .and_then(|u| {
                u.host_str()
                    .map(|h| h == "127.0.0.1" || h == "localhost" || h == "[::1]")
            })
            .unwrap_or(false)
    }

    pub fn features(&self) -> Vec<String> {
        #[cfg(feature = "full")]
        if let Some(f) = &self.cfg.full {
            return f.features();
        }
        vec![]
    }
}

pub fn router(app: Shared) -> Router {
    let r = Router::new()
        .route("/healthz", get(health))
        .route("/relay", get(crate::relay::ws))
        .route(
            "/mcp",
            post(crate::mcp::post)
                .get(crate::mcp::get)
                .delete(crate::mcp::delete),
        )
        .route(
            "/.well-known/oauth-protected-resource",
            get(crate::oauth::protected_resource),
        )
        .route(
            "/.well-known/oauth-protected-resource/mcp",
            get(crate::oauth::protected_resource),
        )
        .route(
            "/.well-known/oauth-authorization-server",
            get(crate::oauth::metadata),
        )
        .route(
            "/.well-known/oauth-authorization-server/mcp",
            get(crate::oauth::metadata),
        )
        .route("/oauth/register", post(crate::oauth::register))
        .route("/oauth/authorize", get(crate::oauth::authorize))
        .route("/oauth/authorize/status", get(crate::oauth::status))
        .route("/oauth/token", post(crate::oauth::token))
        .route("/oauth/revoke", post(crate::oauth::revoke));
    #[cfg(feature = "full")]
    let r = crate::full::routes(r);
    #[cfg(not(feature = "full"))]
    let r = r.route("/", get(index));
    r.with_state(app)
}

async fn health(axum::extract::State(app): axum::extract::State<Shared>) -> axum::Json<Value> {
    let p = app.persist.lock().unwrap();
    // Counts only: this endpoint is public, device names are not.
    let online = app.live.lock().unwrap().sessions.len();
    axum::Json(serde_json::json!({
        "ok": true,
        "version": crate::VERSION,
        "mode": if app.features().is_empty() { "relay" } else { "full" },
        "bound": p.relay_id.is_some() && !p.devices.is_empty(),
        "devices": p.devices.len(),
        "online": online,
        "public_url": app.base(),
    }))
}

#[cfg(not(feature = "full"))]
async fn index() -> axum::response::Html<&'static str> {
    axum::response::Html(
        "<!doctype html><meta charset=utf-8><title>Workroam relay</title><body style=\"font:15px system-ui;margin:3rem\"><h1>Workroam relay</h1><p>This server connects cloud agents to your Workroam devices. Add <code>/mcp</code> to this address as a remote MCP connector.</p>",
    )
}
