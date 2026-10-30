//! Full mode (architecture §8.4, §18.4): 6-digit pairing codes with a key
//! created for each device, revoking a device's key, ntfy forwarding and
//! the static web reader. Needs the storage administrator key, so it only
//! runs on the user's own server.

// Handlers return a ready response as the error.
#![allow(clippy::result_large_err)]

use crate::app::Shared;
use crate::relay::verify;
use crate::state::{DeviceKeyRec, DeviceRec};
use axum::extract::{Path as UrlPath, State};
use axum::http::{header, HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use chrono::Utc;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use workroam_core::config::WorkspaceLocation;
use workroam_core::model::Access;
use workroam_device::{provisioner_for, AdminCreds, ProvisionedKey};
use workroam_ipc::relay::{http_message, H_DEVICE, H_SIGNATURE, H_TIME};

const PAIR_TTL: Duration = Duration::from_secs(10 * 60);
const MAX_ATTEMPTS: u32 = 5;

#[derive(Debug, clap::Args)]
pub struct FullArgs {
    /// Admin API endpoint when it differs from the storage endpoint the devices use.
    #[arg(long, env = "WORKROAM_ADMIN_ENDPOINT")]
    pub admin_endpoint: Option<String>,
    /// ntfy topic URL for phone notifications (e.g. https://ntfy.sh/my-topic).
    #[arg(long, env = "WORKROAM_NTFY")]
    pub ntfy: Option<String>,
    /// Serve the web reader from this directory instead of the built-in copy.
    #[arg(long)]
    pub web_dir: Option<std::path::PathBuf>,
}

pub struct FullConfig {
    admin: Option<AdminCreds>,
    ntfy: Option<String>,
    web_dir: Option<std::path::PathBuf>,
    pairs: Mutex<HashMap<String, Pending>>,
    failures: Mutex<Vec<Instant>>,
}

struct Pending {
    blob: String,
    scope: Access,
    loc: WorkspaceLocation,
    created: Instant,
    attempts: u32,
}

impl FullConfig {
    /// The admin key comes from the environment only (never a flag, never the state file).
    pub fn from_args(a: &FullArgs) -> Result<FullConfig, String> {
        let admin = match (
            std::env::var("WORKROAM_ADMIN_ACCESS_KEY"),
            std::env::var("WORKROAM_ADMIN_SECRET_KEY"),
        ) {
            (Ok(ak), Ok(sk)) if !ak.is_empty() => Some(AdminCreds {
                access_key: ak,
                secret_key: sk,
                admin_endpoint: a.admin_endpoint.clone(),
                admin_token: std::env::var("WORKROAM_ADMIN_TOKEN").ok(),
            }),
            _ => {
                eprintln!("note: WORKROAM_ADMIN_ACCESS_KEY / WORKROAM_ADMIN_SECRET_KEY are not set; pairing codes are off");
                None
            }
        };
        Ok(FullConfig {
            admin,
            ntfy: a.ntfy.clone().filter(|s| !s.is_empty()),
            web_dir: a.web_dir.clone(),
            pairs: Mutex::new(HashMap::new()),
            failures: Mutex::new(Vec::new()),
        })
    }

    pub fn features(&self) -> Vec<String> {
        let mut f = vec!["web".to_string()];
        if self.admin.is_some() {
            f.push("pair".into());
        }
        if self.ntfy.is_some() {
            f.push("notify".into());
        }
        f
    }
}

pub fn routes(r: Router<Shared>) -> Router<Shared> {
    r.route("/pair", post(pair))
        .route("/pair/redeem", post(redeem))
        .route("/devices/{id}", delete(remove_device))
        .route("/notify", post(notify))
        .route("/", get(web))
        .fallback(get(web))
}

fn full(app: &Shared) -> Result<&FullConfig, Response> {
    app.cfg
        .full
        .as_ref()
        .ok_or_else(|| (StatusCode::NOT_FOUND, "full mode is off").into_response())
}

fn bad(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({"error": msg}))).into_response()
}

/// A request signed by a device bound to this server.
fn signed(
    app: &Shared,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<DeviceRec, Response> {
    let get = |h: &str| {
        headers
            .get(h)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string()
    };
    let (id, time, sig) = (get(H_DEVICE), get(H_TIME), get(H_SIGNATURE));
    let t: i64 = time
        .parse()
        .map_err(|_| bad(StatusCode::UNAUTHORIZED, "missing signature"))?;
    if (Utc::now().timestamp() - t).abs() > 300 {
        return Err(bad(
            StatusCode::UNAUTHORIZED,
            "the request time is off by more than 5 minutes",
        ));
    }
    let dev = app
        .persist
        .lock()
        .unwrap()
        .device(&id)
        .cloned()
        .ok_or_else(|| {
            bad(
                StatusCode::UNAUTHORIZED,
                "this device is not bound to this server",
            )
        })?;
    if !verify(
        &dev.pubkey,
        &http_message(method.as_str(), uri.path(), t, body),
        &sig,
    ) {
        return Err(bad(StatusCode::UNAUTHORIZED, "signature check failed"));
    }
    Ok(dev)
}

fn six_digits() -> String {
    let b: [u8; 4] = workroam_core::crypto::random_bytes();
    format!("{:06}", u32::from_le_bytes(b) % 1_000_000)
}

/// `POST /pair` (a bound device): a 6-digit code for a pairing blob that
/// is still encrypted with the workspace password.
async fn pair(
    State(app): State<Shared>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let f = match full(&app) {
        Ok(f) => f,
        Err(r) => return r,
    };
    if let Err(r) = signed(&app, &method, &uri, &headers, &body) {
        return r;
    }
    if f.admin.is_none() {
        return bad(
            StatusCode::SERVICE_UNAVAILABLE,
            "the server has no administrator key, so it cannot create device keys",
        );
    }
    let v: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let Some(blob) = v["pairing_code"].as_str().filter(|b| b.starts_with("wr1:")) else {
        return bad(StatusCode::BAD_REQUEST, "pairing_code is required");
    };
    let Ok(loc) = serde_json::from_value::<WorkspaceLocation>(v["location"].clone()) else {
        return bad(StatusCode::BAD_REQUEST, "location is required");
    };
    let scope = if v["scope"].as_str() == Some("read-only") {
        Access::ReadOnly
    } else {
        Access::ReadWrite
    };
    let mut pairs = f.pairs.lock().unwrap();
    pairs.retain(|_, p| p.created.elapsed() < PAIR_TTL);
    let mut code = six_digits();
    while pairs.contains_key(&code) {
        code = six_digits();
    }
    pairs.insert(
        code.clone(),
        Pending {
            blob: blob.to_string(),
            scope,
            loc,
            created: Instant::now(),
            attempts: 0,
        },
    );
    Json(json!({"code": code, "expires_in": PAIR_TTL.as_secs(), "server": app.base()}))
        .into_response()
}

/// `POST /pair/redeem` (a new device): the pairing blob plus a key made for it.
async fn redeem(State(app): State<Shared>, Json(v): Json<Value>) -> Response {
    let f = match full(&app) {
        Ok(f) => f,
        Err(r) => return r,
    };
    {
        let mut fails = f.failures.lock().unwrap();
        fails.retain(|t| t.elapsed() < Duration::from_secs(600));
        if fails.len() >= 20 {
            return bad(
                StatusCode::TOO_MANY_REQUESTS,
                "too many wrong codes; wait 10 minutes",
            );
        }
    }
    let code: String = v["code"]
        .as_str()
        .unwrap_or("")
        .chars()
        .filter(|c| c.is_ascii_digit())
        .collect();
    let pending = {
        let mut pairs = f.pairs.lock().unwrap();
        pairs.retain(|_, p| p.created.elapsed() < PAIR_TTL && p.attempts < MAX_ATTEMPTS);
        pairs.remove(&code)
    };
    let Some(p) = pending else {
        f.failures.lock().unwrap().push(Instant::now());
        return bad(StatusCode::NOT_FOUND, "wrong or expired pairing code");
    };
    let Some(admin) = &f.admin else {
        return bad(StatusCode::SERVICE_UNAVAILABLE, "no administrator key");
    };
    let mut admin = admin.clone();
    if admin.admin_endpoint.is_none() {
        admin.admin_endpoint = Some(p.loc.endpoint.clone());
    }
    let Some(prov) = provisioner_for(&p.loc.preset, &p.loc.endpoint) else {
        return bad(
            StatusCode::BAD_REQUEST,
            "this storage has no admin interface Workroam can use to create keys",
        );
    };
    let key = match prov.create_key(&p.loc, &admin, p.scope).await {
        Ok(k) => k,
        Err(e) => {
            return bad(
                StatusCode::BAD_GATEWAY,
                &format!("cannot create the device key: {}", e.message),
            )
        }
    };
    let name: String = v["device_name"]
        .as_str()
        .unwrap_or("device")
        .chars()
        .take(64)
        .collect();
    let device_ref = v["device_id"]
        .as_str()
        .map(String::from)
        .unwrap_or_else(|| workroam_core::crypto::random_id("dk"));
    let pubkey = v["pubkey"]
        .as_str()
        .filter(|k| hex::decode(k).map(|b| b.len()) == Ok(32))
        .map(String::from);
    let relay_id = app.update(|s| {
        s.device_keys.retain(|k| k.device_ref != device_ref);
        s.device_keys.push(DeviceKeyRec {
            device_ref: device_ref.clone(),
            device_name: name.clone(),
            access_key: key.access_key.clone(),
            provider: key.provider.clone(),
            provider_ref: key.provider_ref.clone(),
            scope: key.scope.as_str().to_string(),
            created_at: Utc::now(),
        });
        if let Some(pk) = &pubkey {
            s.devices.retain(|d| d.id != device_ref);
            s.devices.push(DeviceRec {
                id: device_ref.clone(),
                name: name.clone(),
                pubkey: pk.clone(),
                bound_at: Utc::now(),
                last_seen: None,
            });
        }
        s.relay_id.clone()
    });
    tracing::info!(device = %name, "paired");
    Json(json!({
        "pairing_code": p.blob,
        "access_key": key.access_key,
        "secret_key": key.secret_key,
        "scope": key.scope,
        "device_ref": device_ref,
        "relay": {"url": app.base(), "relay_id": relay_id},
    }))
    .into_response()
}

/// `DELETE /devices/<id>` (a bound device): revoke that device's key.
async fn remove_device(
    State(app): State<Shared>,
    UrlPath(id): UrlPath<String>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let f = match full(&app) {
        Ok(f) => f,
        Err(r) => return r,
    };
    if let Err(r) = signed(&app, &method, &uri, &headers, &body) {
        return r;
    }
    let v: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let rec = app
        .persist
        .lock()
        .unwrap()
        .device_keys
        .iter()
        .find(|k| k.device_ref == id)
        .cloned();
    let mut revoked = false;
    if let (Some(rec), Some(admin)) = (&rec, &f.admin) {
        let loc: WorkspaceLocation =
            serde_json::from_value(v["location"].clone()).unwrap_or_default();
        let prov = workroam_device::provisioner_for(&loc.preset, &loc.endpoint);
        let key = ProvisionedKey {
            access_key: rec.access_key.clone(),
            secret_key: String::new(),
            scope: if rec.scope == "read-only" {
                Access::ReadOnly
            } else {
                Access::ReadWrite
            },
            provider_ref: rec.provider_ref.clone(),
            provider: rec.provider.clone(),
        };
        if let Some(p) = prov {
            let mut admin = admin.clone();
            if admin.admin_endpoint.is_none() {
                admin.admin_endpoint = Some(loc.endpoint.clone());
            }
            if let Err(e) = p.revoke_key(&loc, &admin, &key).await {
                return bad(
                    StatusCode::BAD_GATEWAY,
                    &format!("cannot revoke the key: {}", e.message),
                );
            }
            revoked = true;
        }
    }
    app.update(|s| {
        s.device_keys.retain(|k| k.device_ref != id);
        s.devices.retain(|d| d.id != id);
    });
    if let Some(s) = app.live.lock().unwrap().sessions.remove(&id) {
        drop(s);
    }
    Json(json!({"removed": id, "key_revoked": revoked})).into_response()
}

/// `POST /notify` (a bound device): forward to the server's ntfy topic.
async fn notify(
    State(app): State<Shared>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let f = match full(&app) {
        Ok(f) => f,
        Err(r) => return r,
    };
    let dev = match signed(&app, &method, &uri, &headers, &body) {
        Ok(d) => d,
        Err(r) => return r,
    };
    let Some(topic) = &f.ntfy else {
        return bad(StatusCode::SERVICE_UNAVAILABLE, "no ntfy topic configured");
    };
    let v: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let title = v["title"].as_str().unwrap_or("Workroam");
    let message = v["message"].as_str().unwrap_or("");
    let r = reqwest::Client::new()
        .post(topic)
        .header("Title", title)
        .header("Tags", v["tags"].as_str().unwrap_or("workroam"))
        .header("Priority", v["priority"].as_str().unwrap_or("default"))
        .timeout(Duration::from_secs(15))
        .body(format!("{message}\n— {}", dev.name))
        .send()
        .await;
    match r.and_then(|r| r.error_for_status()) {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => bad(StatusCode::BAD_GATEWAY, &format!("ntfy: {e}")),
    }
}

/// The web reader (apps/web), built in or from `--web-dir`.
const WEB: &[(&str, &str, &[u8])] = &[
    (
        "index.html",
        "text/html; charset=utf-8",
        include_bytes!("../../../apps/web/index.html"),
    ),
    (
        "app.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../../../apps/web/app.js"),
    ),
    (
        "reader.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../../../apps/web/reader.js"),
    ),
    (
        "style.css",
        "text/css; charset=utf-8",
        include_bytes!("../../../apps/web/style.css"),
    ),
    (
        "workroam_web.wasm",
        "application/wasm",
        include_bytes!("../../../apps/web/workroam_web.wasm"),
    ),
];

fn content_type(name: &str) -> &'static str {
    match name.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "wasm" => "application/wasm",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "json" => "application/json",
        _ => "application/octet-stream",
    }
}

async fn web(State(app): State<Shared>, uri: Uri) -> Response {
    let Ok(f) = full(&app) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut name = uri.path().trim_start_matches('/').to_string();
    if name.is_empty() {
        name = "index.html".into();
    }
    if name.contains("..") || name.contains('\\') {
        return StatusCode::NOT_FOUND.into_response();
    }
    let headers = |ct: &'static str| {
        [
            (header::CONTENT_TYPE, ct),
            (header::CACHE_CONTROL, "no-cache"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::X_FRAME_OPTIONS, "DENY"),
            (header::REFERRER_POLICY, "no-referrer"),
        ]
    };
    if let Some(dir) = &f.web_dir {
        if let Ok(b) = std::fs::read(dir.join(&name)) {
            return (headers(content_type(&name)), b).into_response();
        }
        return StatusCode::NOT_FOUND.into_response();
    }
    match WEB.iter().find(|(n, _, _)| *n == name) {
        Some((_, ct, b)) => (headers(ct), *b).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
