//! Message types. Requests carry an `id`; server events are notifications
//! without one. Errors use the stable Workroam codes: `error.code` is the
//! CLI exit code, `error.data` holds the code name, hint and details.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use workroam_core::{Code, Error};

/// Major protocol version; a client with another major version is refused.
pub const PROTOCOL: u32 = 1;
/// Version of the JSON printed by `workroam --json`.
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientKind {
    Desktop,
    Cli,
    Mcp,
    Relay,
}

impl ClientKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ClientKind::Desktop => "desktop",
            ClientKind::Cli => "cli",
            ClientKind::Mcp => "mcp",
            ClientKind::Relay => "relay",
        }
    }
    /// Calls from agents (MCP, relay) need approval unless settings relax it.
    pub fn is_agent(self) -> bool {
        matches!(self, ClientKind::Mcp | ClientKind::Relay)
    }
}

/// Who started a relayed request (shown on the approval card).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Origin {
    pub connection_id: String,
    pub agent: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hello {
    pub protocol: u32,
    pub client: ClientKind,
    pub version: String,
    /// The calling process is an agent (set by the CLI when stdout is not a
    /// terminal and an agent environment is detected).
    #[serde(default)]
    pub agent: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelloReply {
    pub protocol: u32,
    pub version: String,
    pub device: String,
    pub configured: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub jsonrpc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
    /// Relayed requests only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
}

impl Request {
    pub fn new(id: u64, method: &str, params: Value) -> Request {
        Request {
            jsonrpc: "2.0".into(),
            id: Some(Value::from(id)),
            method: method.into(),
            params,
            origin: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ErrorData {
    #[serde(default)]
    name: Option<Code>,
    #[serde(default)]
    hint: Option<String>,
    #[serde(default)]
    data: Option<Value>,
}

impl From<&Error> for RpcError {
    fn from(e: &Error) -> RpcError {
        RpcError {
            code: e.code.exit_code(),
            message: e.message.clone(),
            data: Some(
                serde_json::to_value(ErrorData {
                    name: Some(e.code),
                    hint: e.hint.clone(),
                    data: e.data.clone(),
                })
                .unwrap_or(Value::Null),
            ),
        }
    }
}

impl From<RpcError> for Error {
    fn from(r: RpcError) -> Error {
        let d: ErrorData = r
            .data
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        Error {
            code: d.name.unwrap_or_else(|| Code::from_exit_code(r.code)),
            message: r.message,
            hint: d.hint,
            data: d.data,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl Response {
    pub fn ok(id: Value, result: Value) -> Response {
        Response {
            jsonrpc: "2.0".into(),
            id,
            result: Some(result),
            error: None,
        }
    }
    pub fn err(id: Value, e: &Error) -> Response {
        Response {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(e.into()),
        }
    }
    pub fn into_result(self) -> Result<Value, Error> {
        match self.error {
            Some(e) => Err(e.into()),
            None => Ok(self.result.unwrap_or(Value::Null)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

impl Notification {
    pub fn new(method: &str, params: Value) -> Notification {
        Notification {
            jsonrpc: "2.0".into(),
            method: method.into(),
            params,
        }
    }
}

/// Any line on the wire.
#[derive(Debug, Clone)]
pub enum Message {
    Request(Request),
    Response(Response),
    Notification(Notification),
}

impl Message {
    pub fn parse(line: &str) -> Result<Message, Error> {
        let v: Value = serde_json::from_str(line)
            .map_err(|e| Error::invalid(format!("invalid JSON-RPC message: {e}")))?;
        let has = |k: &str| v.get(k).is_some();
        if has("method") {
            if has("id") && !v["id"].is_null() {
                Ok(Message::Request(serde_json::from_value(v)?))
            } else {
                Ok(Message::Notification(serde_json::from_value(v)?))
            }
        } else if has("result") || has("error") {
            Ok(Message::Response(serde_json::from_value(v)?))
        } else {
            Err(Error::invalid("not a JSON-RPC message"))
        }
    }

    pub fn to_line(&self) -> String {
        let mut s = match self {
            Message::Request(r) => serde_json::to_string(r),
            Message::Response(r) => serde_json::to_string(r),
            Message::Notification(n) => serde_json::to_string(n),
        }
        .unwrap_or_default();
        s.push('\n');
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_error() {
        let e = Error::new(Code::LeaseHeld, "held by office").with_hint("pull --takeover");
        let r = Response::err(Value::from(3), &e);
        let line = Message::Response(r).to_line();
        let Message::Response(back) = Message::parse(&line).unwrap() else {
            panic!()
        };
        let e2 = back.into_result().unwrap_err();
        assert_eq!(e2.code, Code::LeaseHeld);
        assert_eq!(e2.hint.as_deref(), Some("pull --takeover"));
        assert!(line.contains("\"code\":12"));
    }

    #[test]
    fn classify() {
        assert!(matches!(
            Message::parse(r#"{"jsonrpc":"2.0","method":"job.progress","params":{}}"#).unwrap(),
            Message::Notification(_)
        ));
        assert!(matches!(
            Message::parse(r#"{"jsonrpc":"2.0","id":1,"method":"status"}"#).unwrap(),
            Message::Request(_)
        ));
    }
}
