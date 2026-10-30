//! `/mcp`: the remote MCP connector (Streamable HTTP, JSON responses).
//! Every tool call is checked against the connection's grant here and
//! again on the device, then forwarded over the device's relay session.

use crate::app::{CallResult, Queued, Shared};
use crate::state::ConnRec;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tokio::sync::oneshot;
use workroam_ipc::relay::{ToDevice, TOOLS};

const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];
/// A push or pull waits for the user's card (10 minutes) and the transfer.
const CALL_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const READ_TIMEOUT: Duration = Duration::from_secs(60);

fn unauthorized(app: &Shared) -> Response {
    let mut r = (
        StatusCode::UNAUTHORIZED,
        Json(json!({"error": "invalid_token", "error_description": "sign in to Workroam first"})),
    )
        .into_response();
    let v = format!(
        "Bearer resource_metadata=\"{}/.well-known/oauth-protected-resource\"",
        app.base()
    );
    if let Ok(h) = HeaderValue::from_str(&v) {
        r.headers_mut()
            .insert(axum::http::header::WWW_AUTHENTICATE, h);
    }
    r
}

pub async fn get() -> StatusCode {
    StatusCode::METHOD_NOT_ALLOWED
}

pub async fn delete() -> StatusCode {
    StatusCode::NO_CONTENT
}

pub fn tool_list(access: &str) -> Value {
    let tools: Vec<Value> = TOOLS
        .iter()
        .filter(|(_, write, _)| !*write || access == "sync")
        .map(|(name, write, desc)| {
            let mut props = json!({"project": {"type": "string", "description": "Project name (see list_projects)"}});
            let mut required = vec!["project"];
            match *name {
                "list_projects" => {
                    props = json!({});
                    required.clear();
                }
                "history" => props["limit"] = json!({"type": "integer", "minimum": 1, "maximum": 50}),
                "request_push" => props["release"] = json!({"type": "boolean", "description": "Release the lease after pushing"}),
                _ => {}
            }
            json!({
                "name": name,
                "description": desc,
                "inputSchema": {"type": "object", "properties": props, "required": required, "additionalProperties": false},
                "annotations": {"readOnlyHint": !write},
            })
        })
        .collect();
    json!(tools)
}

pub async fn post(
    State(app): State<Shared>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let Some(conn) = crate::oauth::bearer(&app, &headers) else {
        return unauthorized(&app);
    };
    let msg: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return Json(json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": e.to_string()}})).into_response()
        }
    };
    let reply = match msg {
        Value::Array(batch) => {
            let mut out = Vec::new();
            for m in &batch {
                if let Some(r) = handle(&app, &conn, m).await {
                    out.push(r);
                }
            }
            (!out.is_empty()).then(|| json!(out))
        }
        m => handle(&app, &conn, &m).await,
    };
    match reply {
        Some(r) => {
            let mut resp = Json(r).into_response();
            if let Ok(h) = HeaderValue::from_str(&conn.connection_id) {
                resp.headers_mut().insert("mcp-session-id", h);
            }
            resp
        }
        None => StatusCode::ACCEPTED.into_response(),
    }
}

async fn handle(app: &Shared, conn: &ConnRec, m: &Value) -> Option<Value> {
    let id = m.get("id").cloned()?;
    let method = m["method"].as_str().unwrap_or("");
    let params = m.get("params").cloned().unwrap_or(json!({}));
    let r: Result<Value, (i64, String)> = match method {
        "initialize" => {
            let asked = params["protocolVersion"].as_str().unwrap_or("");
            let v = if PROTOCOL_VERSIONS.contains(&asked) {
                asked
            } else {
                PROTOCOL_VERSIONS[0]
            };
            Ok(json!({
                "protocolVersion": v,
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "workroam", "title": "Workroam", "version": crate::VERSION},
                "instructions": "Workroam syncs the user's AI agent workspaces between their computers. Use list_projects first. request_push and request_pull ask the user on their computer every time; tell the user to check their computer.",
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": tool_list(&conn.access)})),
        "tools/call" => {
            let name = params["name"].as_str().unwrap_or("").to_string();
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            Ok(call(app, conn, &name, args).await)
        }
        "resources/list" => Ok(json!({"resources": []})),
        "prompts/list" => Ok(json!({"prompts": []})),
        _ => Err((-32601, format!("method not found: {method}"))),
    };
    Some(match r {
        Ok(v) => json!({"jsonrpc": "2.0", "id": id, "result": v}),
        Err((c, msg)) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": c, "message": msg}}),
    })
}

fn tool_error(text: impl Into<String>) -> Value {
    json!({"content": [{"type": "text", "text": text.into()}], "isError": true})
}

fn tool_ok(v: Value) -> Value {
    let text = serde_json::to_string_pretty(&v).unwrap_or_default();
    let structured = if v.is_object() {
        v
    } else {
        json!({"result": v})
    };
    json!({"content": [{"type": "text", "text": text}], "structuredContent": structured, "isError": false})
}

/// The device a call goes to: the one that granted the connection, else
/// any online device that offers the project.
fn target(app: &Shared, conn: &ConnRec, project: Option<&str>) -> Option<String> {
    let live = app.live.lock().unwrap();
    if live.sessions.contains_key(&conn.device_id) {
        return Some(conn.device_id.clone());
    }
    live.sessions
        .iter()
        .find(|(_, s)| project.is_none_or(|p| s.projects.iter().any(|x| x == p)))
        .or_else(|| live.sessions.iter().next())
        .map(|(id, _)| id.clone())
}

pub async fn call(app: &Shared, conn: &ConnRec, tool: &str, args: Value) -> Value {
    let Some((_, write, _)) = TOOLS.iter().find(|(n, _, _)| *n == tool) else {
        return tool_error(format!("unknown tool {tool}"));
    };
    if *write && conn.access != "sync" {
        return tool_error(
            "this connection is read-only; reconnect with sync access to request pushes and pulls",
        );
    }
    let project = args["project"].as_str().map(String::from);
    if tool != "list_projects" {
        let Some(p) = &project else {
            return tool_error("project is required (see list_projects)");
        };
        if !conn.projects.contains(p) {
            return tool_error(format!("this connection may not access {p}"));
        }
    }
    let call_id = workroam_core::crypto::random_id("call");
    let msg = ToDevice::Call {
        call_id: call_id.clone(),
        connection_id: conn.connection_id.clone(),
        agent: conn.agent.clone(),
        tool: tool.to_string(),
        args,
    };
    let Some(device) = target(app, conn, project.as_deref()) else {
        let last = {
            let p = app.persist.lock().unwrap();
            p.device(&conn.device_id)
                .and_then(|d| d.last_seen)
                .map(|t| t.to_rfc3339())
        };
        if *write {
            let mut live = app.live.lock().unwrap();
            live.queue
                .entry(conn.device_id.clone())
                .or_default()
                .push(Queued {
                    at: Instant::now(),
                    msg,
                });
            live.detached.push(call_id);
            return tool_ok(json!({
                "queued": true,
                "device_online": false,
                "message": "The device is offline. The request waits up to 24 hours; the user is asked on the device when it comes online.",
            }));
        }
        return tool_ok(json!({
            "device_online": false,
            "last_seen": last,
            "message": "The device is offline.",
        }));
    };
    let (tx, rx) = oneshot::channel::<CallResult>();
    {
        let mut live = app.live.lock().unwrap();
        live.calls.insert(call_id.clone(), tx);
        if let Some(s) = live.sessions.get(&device) {
            let _ = s.tx.send(msg);
        }
    }
    let limit = if *write { CALL_TIMEOUT } else { READ_TIMEOUT };
    let res = tokio::time::timeout(limit, rx).await;
    app.live.lock().unwrap().calls.remove(&call_id);
    tracing::info!(connection = %conn.connection_id, tool, device = %device, "call");
    match res {
        Ok(Ok(r)) if r.ok => tool_ok(r.result),
        Ok(Ok(r)) => {
            let code = r.result["code"].as_i64().unwrap_or(1);
            let what = match code {
                10 => "declined on the device",
                11 => "the user chose Later or did not answer in time",
                12 => "another device holds the lease",
                _ => "failed",
            };
            tool_error(format!(
                "{what}: {} (code {code})",
                r.error.unwrap_or_default()
            ))
        }
        Ok(Err(_)) => tool_error("the device disconnected before answering"),
        Err(_) => tool_error(format!(
            "no answer from the device within {} minutes",
            limit.as_secs() / 60
        )),
    }
}

/// Unused results of queued calls are dropped after a day.
pub fn sweep(app: &Shared) {
    let mut live = app.live.lock().unwrap();
    for q in live.queue.values_mut() {
        q.retain(|x| x.at.elapsed() < crate::relay::QUEUE_TTL);
    }
    live.queue.retain(|_, q| !q.is_empty());
}
