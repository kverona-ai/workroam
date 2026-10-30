//! `workroam mcp`: a stdio MCP server for local agents (AGT-04). Each tool
//! is one IPC call made as `ClientKind::Mcp`, so a push or pull asks for
//! approval exactly as the CLI does when an agent runs it.

use crate::conn;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use workroam_core::{Error, Result};
use workroam_ipc::{Client, ClientKind};

const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

fn scope_props() -> Value {
    json!({
        "project": {"type": "string", "description": "Project name. Default: the project that contains cwd."},
        "cwd": {"type": "string", "description": "A directory inside the project. Default: the server's working directory."}
    })
}

fn with_scope(extra: Value) -> Value {
    let mut p = scope_props();
    if let (Some(o), Some(e)) = (p.as_object_mut(), extra.as_object()) {
        for (k, v) in e {
            o.insert(k.clone(), v.clone());
        }
    }
    json!({"type": "object", "properties": p, "additionalProperties": false})
}

pub fn tools() -> Value {
    json!([
        {
            "name": "status",
            "title": "Workroam status",
            "description": "Sync state of the project: lease holder, unsynced changes, last snapshot, other devices.",
            "inputSchema": with_scope(json!({"all": {"type": "boolean", "description": "Every project on this computer."}})),
            "annotations": {"readOnlyHint": true}
        },
        {
            "name": "push",
            "title": "Push to Workroam",
            "description": "Save the project to the workspace as a new snapshot. The user approves it on their computer; the call waits for the answer.",
            "inputSchema": with_scope(json!({
                "release": {"type": "boolean", "description": "Release the lease so another computer can continue."},
                "note": {"type": "string", "description": "A short note stored with the snapshot."}
            })),
            "annotations": {"readOnlyHint": false, "destructiveHint": false}
        },
        {
            "name": "pull",
            "title": "Pull from Workroam",
            "description": "Bring the latest snapshot of the project to this computer and take the lease. The user approves it.",
            "inputSchema": with_scope(json!({})),
            "annotations": {"readOnlyHint": false, "destructiveHint": false}
        },
        {
            "name": "history",
            "title": "Workroam history",
            "description": "Recent snapshots of the project, newest first.",
            "inputSchema": with_scope(json!({"limit": {"type": "integer", "minimum": 1, "maximum": 200, "description": "Default 20."}})),
            "annotations": {"readOnlyHint": true}
        },
        {
            "name": "handoff_template",
            "title": "HANDOFF.md template",
            "description": "The HANDOFF.md template and the current file, so you can write what the next computer needs to know before a push.",
            "inputSchema": with_scope(json!({})),
            "annotations": {"readOnlyHint": true}
        }
    ])
}

fn scope(args: &Value) -> Value {
    match args.get("project").and_then(|p| p.as_str()) {
        Some(p) => json!({"project": p}),
        None => {
            json!({"cwd": args.get("cwd").and_then(|c| c.as_str()).map(String::from).unwrap_or_else(|| {
                std::env::current_dir().map(|d| d.display().to_string()).unwrap_or_default()
            })})
        }
    }
}

fn merge(mut a: Value, b: Value) -> Value {
    if let (Some(x), Some(y)) = (a.as_object_mut(), b.as_object()) {
        for (k, v) in y {
            if !v.is_null() {
                x.insert(k.clone(), v.clone());
            }
        }
    }
    a
}

async fn run_plans(c: &Client, plans: Value) -> Result<Value> {
    let mut out = Vec::new();
    for pl in plans.as_array().into_iter().flatten() {
        if let Some(b) = pl.get("blocker").filter(|b| !b.is_null()) {
            return Err(
                serde_json::from_value(b.clone()).unwrap_or_else(|_| Error::internal("blocked"))
            );
        }
        let Some(job) = pl["job"].as_str() else {
            continue;
        };
        out.push(
            c.call("job.start", json!({"job": job, "wait": true}))
                .await?,
        );
    }
    Ok(json!(out))
}

pub async fn call_tool(c: &Client, name: &str, args: &Value) -> Result<Value> {
    let sc = scope(args);
    match name {
        "status" => {
            let all = args.get("all").and_then(|a| a.as_bool()).unwrap_or(false);
            let p = if all { json!({"all": true}) } else { sc };
            c.call("status", p).await
        }
        "push" => {
            let p = merge(
                sc,
                json!({"release": args.get("release"), "note": args.get("note")}),
            );
            let plans = c.call("push.plan", p).await?;
            run_plans(c, plans).await
        }
        "pull" => {
            let plans = c.call("pull.plan", sc).await?;
            run_plans(c, plans).await
        }
        "history" => {
            let limit = args.get("limit").and_then(|l| l.as_u64()).unwrap_or(20);
            c.call("history.list", merge(sc, json!({"limit": limit})))
                .await
        }
        "handoff_template" => {
            c.call("handoff.read", merge(sc, json!({"template": true})))
                .await
        }
        other => Err(Error::invalid(format!("unknown tool {other}"))),
    }
}

fn tool_result(r: Result<Value>) -> Value {
    match r {
        Ok(v) => {
            let text = if let Some(t) = v.get("template").and_then(|t| t.as_str()) {
                t.to_string()
            } else {
                serde_json::to_string_pretty(&v).unwrap_or_default()
            };
            let structured = if v.is_object() {
                v
            } else {
                json!({"result": v})
            };
            json!({"content": [{"type": "text", "text": text}], "structuredContent": structured, "isError": false})
        }
        Err(e) => {
            let mut text = format!("{} (exit code {})", e.message, e.code.exit_code());
            if let Some(h) = &e.hint {
                text.push_str(&format!("\nhint: {h}"));
            }
            json!({"content": [{"type": "text", "text": text}], "structuredContent": {"error": e}, "isError": true})
        }
    }
}

/// Answer one JSON-RPC message; `None` for notifications.
pub async fn handle(c: &Client, msg: &Value) -> Option<Value> {
    let id = msg.get("id").cloned()?;
    let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let params = msg.get("params").cloned().unwrap_or(json!({}));
    let result = match method {
        "initialize" => {
            let asked = params
                .get("protocolVersion")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let version = if PROTOCOL_VERSIONS.contains(&asked) {
                asked
            } else {
                PROTOCOL_VERSIONS[0]
            };
            Ok(json!({
                "protocolVersion": version,
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "workroam", "title": "Workroam", "version": crate::VERSION},
                "instructions": "Workroam keeps this project in sync across the user's computers. Call status before you start, write HANDOFF.md (see handoff_template), then push with release=true when you stop. Push and pull wait for the user's approval."
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": tools()})),
        "tools/call" => {
            let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            if tools()
                .as_array()
                .is_some_and(|t| t.iter().any(|x| x["name"] == name))
            {
                Ok(tool_result(call_tool(c, name, &args).await))
            } else {
                Err((-32602, format!("unknown tool {name}")))
            }
        }
        "resources/list" => Ok(json!({"resources": []})),
        "prompts/list" => Ok(json!({"prompts": []})),
        _ => Err((-32601, format!("method not found: {method}"))),
    };
    Some(match result {
        Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
        Err((code, m)) => {
            json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": m}})
        }
    })
}

pub async fn serve() -> Result<()> {
    let c = conn::connect(ClientKind::Mcp).await?;
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut stdout = tokio::io::stdout();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(Value::Array(batch)) => {
                let mut out = Vec::new();
                for m in &batch {
                    if let Some(r) = handle(&c.client, m).await {
                        out.push(r);
                    }
                }
                (!out.is_empty()).then(|| json!(out))
            }
            Ok(m) => handle(&c.client, &m).await,
            Err(e) => Some(
                json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": e.to_string()}}),
            ),
        };
        if let Some(r) = reply {
            let mut s = serde_json::to_string(&r).unwrap_or_default();
            s.push('\n');
            stdout.write_all(s.as_bytes()).await?;
            stdout.flush().await?;
        }
    }
    Ok(())
}
