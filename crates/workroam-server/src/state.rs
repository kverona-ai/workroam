//! The one state file (architecture §18.1): bound device keys, connection
//! grants and token hashes. No storage credentials, no encryption keys, no
//! project data. Queued requests and live sessions are memory only.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use workroam_ipc::relay::BindCode;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Persist {
    #[serde(default)]
    pub relay_id: Option<String>,
    /// Waiting for its device's first connection.
    #[serde(default)]
    pub bind: Option<BindCode>,
    #[serde(default)]
    pub devices: Vec<DeviceRec>,
    #[serde(default)]
    pub clients: Vec<ClientRec>,
    #[serde(default)]
    pub connections: Vec<ConnRec>,
    #[serde(default)]
    pub tokens: Vec<TokenRec>,
    /// Full mode: per-device storage keys (no secrets, only what revoking needs).
    #[serde(default)]
    pub device_keys: Vec<DeviceKeyRec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceRec {
    pub id: String,
    pub name: String,
    /// Ed25519 public key (hex).
    pub pubkey: String,
    pub bound_at: DateTime<Utc>,
    #[serde(default)]
    pub last_seen: Option<DateTime<Utc>>,
}

/// An OAuth client from dynamic client registration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientRec {
    pub client_id: String,
    pub client_name: String,
    pub redirect_uris: Vec<String>,
    pub created_at: DateTime<Utc>,
}

/// A connection a device granted (signed by that device).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnRec {
    pub connection_id: String,
    pub client_id: String,
    pub agent: String,
    pub agent_name: String,
    /// `read-only` or `sync`.
    pub access: String,
    pub projects: Vec<String>,
    pub device_id: String,
    pub signature: String,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub revoked: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TokenKind {
    Access,
    Refresh,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenRec {
    /// SHA-256 of the token (hex); the token itself is never stored.
    pub hash: String,
    pub kind: TokenKind,
    pub connection_id: String,
    pub expires: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceKeyRec {
    pub device_ref: String,
    pub device_name: String,
    pub access_key: String,
    pub provider: String,
    pub provider_ref: String,
    pub scope: String,
    pub created_at: DateTime<Utc>,
}

impl Persist {
    pub fn default_path() -> PathBuf {
        if let Ok(p) = std::env::var("WORKROAM_SERVER_HOME") {
            return PathBuf::from(p).join("state.json");
        }
        workroam_core::paths::home_dir()
            .join(".workroam-server")
            .join("state.json")
    }

    pub fn load(path: &Path) -> std::io::Result<Persist> {
        match std::fs::read(path) {
            Ok(b) => serde_json::from_slice(&b)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Persist::default()),
            Err(e) => Err(e),
        }
    }

    /// Atomic write, owner-only.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(d, std::fs::Permissions::from_mode(0o700));
            }
        }
        let tmp = path.with_extension("json.tmp");
        let data = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&tmp)?;
            f.write_all(&data)?;
            f.sync_all()?;
        }
        #[cfg(not(unix))]
        std::fs::write(&tmp, &data)?;
        std::fs::rename(&tmp, path)
    }

    /// Drop expired tokens and the tokens of revoked connections.
    pub fn gc(&mut self) {
        let now = Utc::now();
        let live: Vec<String> = self
            .connections
            .iter()
            .filter(|c| !c.revoked)
            .map(|c| c.connection_id.clone())
            .collect();
        self.tokens
            .retain(|t| t.expires > now && live.contains(&t.connection_id));
        if let Some(b) = &self.bind {
            if b.expires < now.timestamp() {
                self.bind = None;
            }
        }
    }

    pub fn device(&self, id: &str) -> Option<&DeviceRec> {
        self.devices.iter().find(|d| d.id == id)
    }

    pub fn connection(&self, id: &str) -> Option<&ConnRec> {
        self.connections
            .iter()
            .find(|c| c.connection_id == id && !c.revoked)
    }
}
