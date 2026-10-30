//! Device relay protocol between `workroamd` and `workroam-server`
//! (architecture §16.4, §18.3): JSON text frames over `wss://<host>/relay`.
//! The device signs the server's challenge with its Ed25519 key; the server
//! only forwards calls and never sees storage credentials or keys.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Bytes the device signs to authenticate: domain-separated challenge.
pub fn auth_message(relay_id: &str, device_id: &str, nonce: &str) -> Vec<u8> {
    format!("workroam-relay-auth\n{relay_id}\n{device_id}\n{nonce}").into_bytes()
}

/// Bytes the device signs when it grants a remote connection.
pub fn grant_message(
    relay_id: &str,
    connection_id: &str,
    access: &str,
    projects: &[String],
) -> Vec<u8> {
    format!(
        "workroam-relay-grant\n{relay_id}\n{connection_id}\n{access}\n{}",
        projects.join(",")
    )
    .into_bytes()
}

/// Bytes a device signs for an HTTP request to a full-mode server
/// (`X-Workroam-Device`, `X-Workroam-Time`, `X-Workroam-Signature`).
pub fn http_message(method: &str, path: &str, time: i64, body: &[u8]) -> Vec<u8> {
    format!(
        "workroam-http\n{method}\n{path}\n{time}\n{}",
        workroam_core::crypto::sha256_hex(body)
    )
    .into_bytes()
}

/// Headers of a device-signed HTTP request.
pub const H_DEVICE: &str = "x-workroam-device";
pub const H_TIME: &str = "x-workroam-time";
pub const H_SIGNATURE: &str = "x-workroam-signature";

/// One-time bind code the device gives the bot: `wrb1:` + base32(JSON).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BindCode {
    pub relay_id: String,
    pub device_id: String,
    pub device_name: String,
    /// Ed25519 public key (hex).
    pub pubkey: String,
    /// Unix seconds; 30 minutes after creation.
    pub expires: i64,
}

impl BindCode {
    pub const PREFIX: &'static str = "wrb1:";

    pub fn encode(&self) -> String {
        let json = serde_json::to_vec(self).unwrap_or_default();
        format!(
            "{}{}",
            Self::PREFIX,
            base32_encode(&json).to_ascii_lowercase()
        )
    }

    pub fn decode(s: &str) -> Option<BindCode> {
        let body = s.trim().strip_prefix(Self::PREFIX)?;
        let raw = base32_decode(&body.to_ascii_uppercase())?;
        serde_json::from_slice(&raw).ok()
    }
}

const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

fn base32_encode(data: &[u8]) -> String {
    let mut out = String::new();
    let mut buf: u32 = 0;
    let mut bits = 0;
    for &b in data {
        buf = (buf << 8) | b as u32;
        bits += 8;
        while bits >= 5 {
            out.push(ALPHABET[((buf >> (bits - 5)) & 31) as usize] as char);
            bits -= 5;
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((buf << (5 - bits)) & 31) as usize] as char);
    }
    out
}

fn base32_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut buf: u32 = 0;
    let mut bits = 0;
    for c in s.bytes() {
        let v = ALPHABET.iter().position(|&a| a == c)? as u32;
        buf = (buf << 5) | v;
        bits += 5;
        if bits >= 8 {
            out.push(((buf >> (bits - 8)) & 0xff) as u8);
            bits -= 8;
        }
    }
    Some(out)
}

/// Server → device.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToDevice {
    Challenge {
        relay_id: String,
        nonce: String,
    },
    Welcome {
        relay_id: String,
        public_url: String,
        devices: Vec<String>,
        /// Full-mode features this server offers: `pair`, `notify`, `web`.
        #[serde(default)]
        features: Vec<String>,
    },
    /// A tool call from a remote agent.
    Call {
        call_id: String,
        connection_id: String,
        agent: String,
        tool: String,
        #[serde(default)]
        args: Value,
    },
    /// OAuth confirmation: show a card with this code.
    Authorize {
        request_id: String,
        connection_id: String,
        agent: String,
        agent_name: String,
        code: String,
    },
    Error {
        message: String,
    },
}

/// Device → server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FromDevice {
    Auth {
        device_id: String,
        device_name: String,
        pubkey: String,
        signature: String,
        /// First connection after `workroam-server --bind <code>`.
        #[serde(default)]
        bind_code: Option<String>,
    },
    Result {
        call_id: String,
        ok: bool,
        #[serde(default)]
        result: Value,
        #[serde(default)]
        error: Option<String>,
    },
    Grant {
        request_id: String,
        connection_id: String,
        access: String,
        projects: Vec<String>,
        signature: String,
    },
    Deny {
        request_id: String,
    },
    Revoke {
        connection_id: String,
    },
    /// Register another device of the same workspace on this relay.
    AddDevice {
        device_id: String,
        device_name: String,
        pubkey: String,
    },
    /// Projects whose status the relay may report while this device is offline.
    Presence {
        projects: Vec<String>,
    },
}

/// Remote tools (architecture §16.4) and whether they may change anything.
pub const TOOLS: &[(&str, bool, &str)] = &[
    (
        "list_projects",
        false,
        "Projects this connection may see, with lease holder and last sync",
    ),
    (
        "status",
        false,
        "Status of a project (lease, latest snapshot, unsynced changes)",
    ),
    ("history", false, "Recent snapshots of a project"),
    ("read_handoff", false, "HANDOFF.md of the latest snapshot"),
    (
        "request_push",
        true,
        "Ask the device holding the lease to push (approved on the device)",
    ),
    (
        "request_pull",
        true,
        "Ask a device to pull (approved on the device)",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bind_code_round_trip() {
        let b = BindCode {
            relay_id: "r_1".into(),
            device_id: "d_2".into(),
            device_name: "office".into(),
            pubkey: "ab".repeat(32),
            expires: 1_900_000_000,
        };
        let s = b.encode();
        assert!(s.starts_with("wrb1:"));
        assert_eq!(BindCode::decode(&s), Some(b));
        assert!(BindCode::decode("wrb1:!!").is_none());
    }
}
