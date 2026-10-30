//! OAuth 2.1 for the remote MCP connector (architecture §16.4): dynamic
//! client registration, authorization code + PKCE (S256), refresh tokens.
//! There are no user accounts: the authorization page shows a 6-digit
//! code and the user approves it on their device's card.

use crate::app::{AuthRequest, AuthStatus, Shared};
use crate::state::{ClientRec, TokenKind, TokenRec};
use axum::extract::{Form, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::Json;
use base64::Engine;
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use workroam_ipc::relay::ToDevice;

pub const ACCESS_TTL: i64 = 3600;
pub const REFRESH_TTL_DAYS: i64 = 30;
const AUTH_TTL: Duration = Duration::from_secs(10 * 60);

pub fn new_secret(prefix: &str) -> String {
    let b: [u8; 32] = workroam_core::crypto::random_bytes();
    format!(
        "{prefix}{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
    )
}

pub fn hash(token: &str) -> String {
    workroam_core::crypto::sha256_hex(token.as_bytes())
}

fn six_digits() -> String {
    let b: [u8; 4] = workroam_core::crypto::random_bytes();
    format!("{:06}", u32::from_le_bytes(b) % 1_000_000)
}

fn err(status: StatusCode, error: &str, desc: &str) -> Response {
    (
        status,
        Json(json!({"error": error, "error_description": desc})),
    )
        .into_response()
}

pub async fn protected_resource(State(app): State<Shared>) -> Json<Value> {
    let b = app.base();
    Json(json!({
        "resource": format!("{b}/mcp"),
        "authorization_servers": [b],
        "bearer_methods_supported": ["header"],
        "scopes_supported": ["workroam"],
        "resource_name": "Workroam",
    }))
}

pub async fn metadata(State(app): State<Shared>) -> Json<Value> {
    let b = app.base();
    Json(json!({
        "issuer": b,
        "authorization_endpoint": format!("{b}/oauth/authorize"),
        "token_endpoint": format!("{b}/oauth/token"),
        "registration_endpoint": format!("{b}/oauth/register"),
        "revocation_endpoint": format!("{b}/oauth/revoke"),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none"],
        "scopes_supported": ["workroam"],
    }))
}

/// RFC 7591 dynamic client registration (public clients only).
pub async fn register(State(app): State<Shared>, Json(body): Json<Value>) -> Response {
    let uris: Vec<String> = body["redirect_uris"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|u| u.as_str().map(String::from))
        .collect();
    if uris.is_empty() || uris.len() > 10 {
        return err(
            StatusCode::BAD_REQUEST,
            "invalid_redirect_uri",
            "redirect_uris is required",
        );
    }
    for u in &uris {
        let ok = match url::Url::parse(u) {
            Ok(p) => match p.scheme() {
                "https" => true,
                "http" => p
                    .host_str()
                    .is_some_and(|h| h == "127.0.0.1" || h == "localhost" || h == "[::1]"),
                // Native apps may use their own scheme.
                "javascript" | "data" | "file" | "vbscript" | "blob" => false,
                _ => true,
            },
            Err(_) => false,
        };
        if !ok {
            return err(
                StatusCode::BAD_REQUEST,
                "invalid_redirect_uri",
                "redirect URIs must be https (or loopback)",
            );
        }
    }
    let name: String = body["client_name"]
        .as_str()
        .unwrap_or("Cloud agent")
        .chars()
        .filter(|c| !c.is_control())
        .take(60)
        .collect();
    let rec = ClientRec {
        client_id: workroam_core::crypto::random_id("cl"),
        client_name: name,
        redirect_uris: uris,
        created_at: Utc::now(),
    };
    let out = json!({
        "client_id": rec.client_id,
        "client_name": rec.client_name,
        "redirect_uris": rec.redirect_uris,
        "client_id_issued_at": rec.created_at.timestamp(),
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none",
    });
    app.update(|p| {
        // Keep the list small: drop the oldest clients that never connected.
        if p.clients.len() >= 200 {
            let used: Vec<String> = p.connections.iter().map(|c| c.client_id.clone()).collect();
            if let Some(i) = p.clients.iter().position(|c| !used.contains(&c.client_id)) {
                p.clients.remove(i);
            }
        }
        p.clients.push(rec);
    });
    (StatusCode::CREATED, Json(out)).into_response()
}

/// The agent's stable id from its client name: `grok`, `muse`, … .
pub fn agent_id(client_name: &str) -> String {
    let n = client_name.to_lowercase();
    // Adapter ids (adapters/*.toml) so `workroam agent disconnect <id>` matches.
    for (word, id) in [("grok", "grok-bot"), ("muse", "muse")] {
        if n.contains(word) {
            return id.into();
        }
    }
    let s: String = n
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() {
        "remote".into()
    } else {
        s
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn page(title: &str, body: &str) -> Html<String> {
    Html(format!(
        r#"<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>{t}</title>
<style>:root{{--bg:#f7f9fc;--fg:#0f1b2d;--mut:#5b6b82;--card:#fff;--line:#e3e9f2;--blue:#2f6bff;--mint:#2ed3b7}}
@media (prefers-color-scheme:dark){{:root{{--bg:#0b1220;--fg:#e8eef8;--mut:#93a3bb;--card:#121b2d;--line:#22304a}}}}
body{{margin:0;min-height:100vh;display:grid;place-items:center;background:var(--bg);color:var(--fg);font:15px/1.5 system-ui,-apple-system,"Segoe UI",sans-serif}}
.c{{background:var(--card);border:1px solid var(--line);border-radius:16px;padding:32px;max-width:420px;margin:16px;box-shadow:0 10px 30px rgba(15,27,45,.08)}}
h1{{font-size:20px;margin:0 0 8px}}p{{color:var(--mut);margin:8px 0}}.code{{font:600 40px/1.2 ui-monospace,Menlo,monospace;letter-spacing:.2em;text-align:center;margin:24px 0;color:var(--blue)}}
.bar{{height:4px;border-radius:2px;background:linear-gradient(90deg,var(--blue),#1fb6e8,var(--mint));margin-bottom:24px}}</style></head>
<body><div class="c"><div class="bar"></div>{body}</div></body></html>"#,
        t = esc(title),
    ))
}

#[derive(Deserialize)]
pub struct AuthorizeQuery {
    response_type: Option<String>,
    client_id: Option<String>,
    redirect_uri: Option<String>,
    state: Option<String>,
    code_challenge: Option<String>,
    code_challenge_method: Option<String>,
}

pub async fn authorize(State(app): State<Shared>, Query(q): Query<AuthorizeQuery>) -> Response {
    if !app.https_ok() {
        return page("Workroam", "<h1>Not available over plain HTTP</h1><p>This relay has no HTTPS address yet. Start it with <code>--public-url https://…</code> or <code>--tunnel</code>.</p>").into_response();
    }
    let client = {
        let p = app.persist.lock().unwrap();
        q.client_id
            .as_deref()
            .and_then(|id| p.clients.iter().find(|c| c.client_id == id).cloned())
    };
    let Some(client) = client else {
        return (
            StatusCode::BAD_REQUEST,
            page(
                "Workroam",
                "<h1>Unknown client</h1><p>Register the client first.</p>",
            ),
        )
            .into_response();
    };
    let redirect = q
        .redirect_uri
        .clone()
        .unwrap_or_else(|| client.redirect_uris[0].clone());
    if !client.redirect_uris.contains(&redirect) {
        return (
            StatusCode::BAD_REQUEST,
            page("Workroam", "<h1>Wrong redirect address</h1>"),
        )
            .into_response();
    }
    if q.response_type.as_deref() != Some("code")
        || q.code_challenge.as_deref().unwrap_or("").len() < 43
        || q.code_challenge_method.as_deref() != Some("S256")
    {
        return (StatusCode::BAD_REQUEST, page("Workroam", "<h1>Unsupported request</h1><p>Use the authorization code flow with PKCE (S256).</p>")).into_response();
    }
    let req = AuthRequest {
        poll_secret: new_secret("wrq_"),
        client_id: client.client_id.clone(),
        client_name: client.client_name.clone(),
        agent: agent_id(&client.client_name),
        redirect_uri: redirect,
        state: q.state.clone(),
        code_challenge: q.code_challenge.clone().unwrap_or_default(),
        connection_id: workroam_core::crypto::random_id("c"),
        code: six_digits(),
        created: Instant::now(),
        status: AuthStatus::Pending,
    };
    let online = {
        let mut live = app.live.lock().unwrap();
        live.auth.retain(|_, r| r.created.elapsed() < AUTH_TTL);
        let msg = ToDevice::Authorize {
            request_id: req.poll_secret.clone(),
            connection_id: req.connection_id.clone(),
            agent: req.agent.clone(),
            agent_name: req.client_name.clone(),
            code: req.code.clone(),
        };
        let mut n = 0;
        for s in live.sessions.values() {
            if s.tx.send(msg.clone()).is_ok() {
                n += 1;
            }
        }
        live.auth.insert(req.poll_secret.clone(), req.clone());
        n
    };
    let waiting = if online > 0 {
        "<p id=s>Waiting for you to approve on your computer…</p>".to_string()
    } else {
        "<p id=s>None of your devices is online. Open Workroam on your computer; the card appears as soon as it connects.</p>".to_string()
    };
    let body = format!(
        r#"<h1>Connect {name} to Workroam</h1><p>Check that your computer shows this code, then approve there.</p><div class="code">{code}</div>{waiting}
<script>const id={id};async function poll(){{try{{const r=await fetch('/oauth/authorize/status?id='+encodeURIComponent(id));const j=await r.json();
if(j.status==='granted'){{document.getElementById('s').textContent='Approved. Returning to {name_js}…';location.href=j.redirect;return}}
if(j.status==='denied'){{document.getElementById('s').textContent='The connection was declined on your computer.';return}}
if(j.status==='expired'){{document.getElementById('s').textContent='This request expired. Start again from {name_js}.';return}}}}catch(e){{}}setTimeout(poll,1500)}}poll()</script>"#,
        name = esc(&client.client_name),
        name_js = esc(&client.client_name).replace('\'', ""),
        code = req.code,
        id = serde_json::to_string(&req.poll_secret).unwrap_or_default(),
    );
    page("Connect to Workroam", &body).into_response()
}

pub async fn status(
    State(app): State<Shared>,
    Query(q): Query<HashMap<String, String>>,
) -> Json<Value> {
    let id = q.get("id").cloned().unwrap_or_default();
    let live = app.live.lock().unwrap();
    match live.auth.get(&id) {
        Some(r) if r.created.elapsed() < AUTH_TTL => match &r.status {
            AuthStatus::Pending => Json(json!({"status": "pending"})),
            AuthStatus::Granted { redirect } => {
                Json(json!({"status": "granted", "redirect": redirect}))
            }
            AuthStatus::Denied => Json(json!({"status": "denied"})),
        },
        _ => Json(json!({"status": "expired"})),
    }
}

fn issue(app: &Shared, connection_id: &str) -> Value {
    let access = new_secret("wra_");
    let refresh = new_secret("wrr_");
    let now = Utc::now();
    app.update(|p| {
        p.tokens.push(TokenRec {
            hash: hash(&access),
            kind: TokenKind::Access,
            connection_id: connection_id.to_string(),
            expires: now + chrono::Duration::seconds(ACCESS_TTL),
        });
        p.tokens.push(TokenRec {
            hash: hash(&refresh),
            kind: TokenKind::Refresh,
            connection_id: connection_id.to_string(),
            expires: now + chrono::Duration::days(REFRESH_TTL_DAYS),
        });
    });
    json!({
        "access_token": access,
        "token_type": "Bearer",
        "expires_in": ACCESS_TTL,
        "refresh_token": refresh,
        "scope": "workroam",
    })
}

pub async fn token(State(app): State<Shared>, Form(f): Form<HashMap<String, String>>) -> Response {
    if !app.https_ok() {
        return err(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "this relay has no HTTPS address",
        );
    }
    match f.get("grant_type").map(String::as_str) {
        Some("authorization_code") => {
            let code = f.get("code").cloned().unwrap_or_default();
            let Some(c) = app.live.lock().unwrap().codes.remove(&code) else {
                return err(
                    StatusCode::BAD_REQUEST,
                    "invalid_grant",
                    "unknown or used code",
                );
            };
            if c.created.elapsed() > AUTH_TTL {
                return err(StatusCode::BAD_REQUEST, "invalid_grant", "the code expired");
            }
            if f.get("client_id").is_some_and(|id| id != &c.client_id) {
                return err(StatusCode::BAD_REQUEST, "invalid_grant", "client mismatch");
            }
            if f.get("redirect_uri").is_some_and(|u| u != &c.redirect_uri) {
                return err(
                    StatusCode::BAD_REQUEST,
                    "invalid_grant",
                    "redirect_uri mismatch",
                );
            }
            let verifier = f.get("code_verifier").cloned().unwrap_or_default();
            let want = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(workroam_core::crypto::sha256(verifier.as_bytes()));
            if verifier.len() < 43 || want != c.code_challenge {
                return err(
                    StatusCode::BAD_REQUEST,
                    "invalid_grant",
                    "PKCE check failed",
                );
            }
            if app
                .persist
                .lock()
                .unwrap()
                .connection(&c.connection_id)
                .is_none()
            {
                return err(
                    StatusCode::BAD_REQUEST,
                    "invalid_grant",
                    "the connection was revoked",
                );
            }
            Json(issue(&app, &c.connection_id)).into_response()
        }
        Some("refresh_token") => {
            let h = hash(f.get("refresh_token").map(String::as_str).unwrap_or(""));
            let conn = app.update(|p| {
                let i = p.tokens.iter().position(|t| {
                    t.hash == h && t.kind == TokenKind::Refresh && t.expires > Utc::now()
                })?;
                let t = p.tokens.remove(i);
                p.connection(&t.connection_id)
                    .map(|c| c.connection_id.clone())
            });
            match conn {
                Some(c) => Json(issue(&app, &c)).into_response(),
                None => err(
                    StatusCode::BAD_REQUEST,
                    "invalid_grant",
                    "unknown, expired or revoked refresh token",
                ),
            }
        }
        _ => err(
            StatusCode::BAD_REQUEST,
            "unsupported_grant_type",
            "authorization_code or refresh_token",
        ),
    }
}

pub async fn revoke(
    State(app): State<Shared>,
    Form(f): Form<HashMap<String, String>>,
) -> StatusCode {
    let h = hash(f.get("token").map(String::as_str).unwrap_or(""));
    app.update(|p| p.tokens.retain(|t| t.hash != h));
    StatusCode::OK
}

/// The connection behind a Bearer token, or `None`.
pub fn bearer(app: &Shared, headers: &HeaderMap) -> Option<crate::state::ConnRec> {
    let v = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    let tok = v
        .strip_prefix("Bearer ")
        .or_else(|| v.strip_prefix("bearer "))?;
    let h = hash(tok.trim());
    let p = app.persist.lock().unwrap();
    let t = p
        .tokens
        .iter()
        .find(|t| t.hash == h && t.kind == TokenKind::Access && t.expires > Utc::now())?;
    p.connection(&t.connection_id).cloned()
}
