//! Data model: devices, control objects (architecture §6.6) and the snapshot
//! manifest (architecture §6.5).

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Access {
    #[default]
    ReadWrite,
    ReadOnly,
}

impl Access {
    pub fn as_str(self) -> &'static str {
        match self {
            Access::ReadWrite => "read-write",
            Access::ReadOnly => "read-only",
        }
    }
    pub fn parse(s: &str) -> Option<Access> {
        match s {
            "read-write" | "rw" => Some(Access::ReadWrite),
            "read-only" | "ro" => Some(Access::ReadOnly),
            _ => None,
        }
    }
}

/// Auto sync level (AUTO-01).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "lowercase")]
pub enum AutoMode {
    Off,
    #[default]
    Ask,
    Auto,
}

impl AutoMode {
    pub fn as_str(self) -> &'static str {
        match self {
            AutoMode::Off => "off",
            AutoMode::Ask => "ask",
            AutoMode::Auto => "auto",
        }
    }
    pub fn parse(s: &str) -> Option<AutoMode> {
        match s {
            "off" => Some(AutoMode::Off),
            "ask" => Some(AutoMode::Ask),
            "auto" => Some(AutoMode::Auto),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum OsKind {
    Windows,
    Macos,
    Linux,
}

impl OsKind {
    pub fn current() -> OsKind {
        if cfg!(windows) {
            OsKind::Windows
        } else if cfg!(target_os = "macos") {
            OsKind::Macos
        } else {
            OsKind::Linux
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            OsKind::Windows => "windows",
            OsKind::Macos => "macos",
            OsKind::Linux => "linux",
        }
    }
    pub fn display(self) -> &'static str {
        match self {
            OsKind::Windows => "Windows",
            OsKind::Macos => "macOS",
            OsKind::Linux => "Linux",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeviceRef {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os: Option<OsKind>,
}

/// `control/workspace.json` (plaintext, no secrets).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkspaceInfo {
    pub layout: u32,
    pub workspace_id: String,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub repo_id: String,
    #[serde(default)]
    pub product: String,
}

/// `control/settings` (encrypted).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkspaceSettings {
    /// Keep every snapshot of the last N days.
    pub retention_days: u32,
    /// Before that, keep one per week for N weeks (0 = forever).
    pub retention_weekly: u32,
}

impl Default for WorkspaceSettings {
    fn default() -> Self {
        Self {
            retention_days: 30,
            retention_weekly: 52,
        }
    }
}

/// Access key pair as stored in `control/credentials` and the pairing code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AccessKey {
    pub access_key: String,
    pub secret_key: String,
    #[serde(default)]
    pub scope: Access,
}

/// `control/credentials` (encrypted, architecture §8.3).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct CredentialsObject {
    pub current: Option<AccessKey>,
    pub next: Option<AccessKey>,
    pub rotated_at: Option<DateTime<Utc>>,
    /// Read-only key for read-only devices (key mode `create` only).
    #[serde(default)]
    pub read_only: Option<AccessKey>,
    /// Devices that already switched to `next`.
    #[serde(default)]
    pub switched: Vec<String>,
}

/// `control/devices/<id>` (encrypted).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DeviceRecord {
    pub id: String,
    pub name: String,
    pub os: OsKind,
    pub access: Access,
    pub joined_at: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
    #[serde(default)]
    pub last_sync: Option<DateTime<Utc>>,
    #[serde(default)]
    pub reachable: Vec<String>,
    /// Ed25519 public key used by the device relay (hex).
    #[serde(default)]
    pub relay_pubkey: Option<String>,
    #[serde(default)]
    pub agents: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "lowercase")]
pub enum LeaseState {
    #[default]
    Held,
    Released,
}

/// `control/projects/<pid>/lease` (encrypted, conditional writes).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Lease {
    pub holder: DeviceRef,
    pub acquired_at: DateTime<Utc>,
    pub renewed_at: DateTime<Utc>,
    pub state: LeaseState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "lowercase")]
pub enum HeadState {
    #[default]
    Active,
    Archived,
}

/// `control/projects/<pid>/head` (encrypted, conditional writes).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Head {
    /// Project name. Only readable after decryption.
    pub project: String,
    pub snapshot: Option<String>,
    pub parent: Option<String>,
    pub device_id: String,
    #[serde(default)]
    pub device_name: String,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub state: HeadState,
    #[serde(default)]
    pub summary: Option<ChangeSummary>,
}

/// `control/projects/<pid>/archive` (encrypted).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ArchiveRecord {
    pub name: String,
    pub snapshot: String,
    pub archived_at: DateTime<Utc>,
    pub device: DeviceRef,
    #[serde(default)]
    pub note: String,
    pub size: u64,
    #[serde(default)]
    pub keep_history: bool,
    #[serde(default)]
    pub repos: Vec<String>,
}

/// A remote connection grant (architecture §16.4), stored as
/// `control/grants/<connection-id>` (encrypted, visible to every device).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Grant {
    pub connection_id: String,
    pub agent: String,
    pub agent_name: String,
    pub relay_url: String,
    pub relay_id: String,
    /// `read-only` or `sync`.
    pub access: String,
    pub projects: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub created_by: DeviceRef,
    #[serde(default)]
    pub revoked: bool,
    #[serde(default)]
    pub webhook_url: Option<String>,
}

/// `control/relays/<relay-id>`: relays bound to this workspace.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RelayRecord {
    pub relay_id: String,
    pub url: String,
    pub bound_at: DateTime<Utc>,
    pub devices: Vec<String>,
    #[serde(default)]
    pub default_device: Option<String>,
    #[serde(default)]
    pub remote_projects: Vec<String>,
}

/// Change summary shown on approval, completion and arrival cards.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ChangeSummary {
    pub added: u64,
    pub modified: u64,
    pub deleted: u64,
    pub bytes: u64,
    pub commits: u64,
    #[serde(default)]
    pub repos: Vec<String>,
    #[serde(default)]
    pub conflicts: u64,
}

impl ChangeSummary {
    pub fn files(&self) -> u64 {
        self.added + self.modified + self.deleted
    }
    pub fn is_empty(&self) -> bool {
        self.files() == 0 && self.commits == 0
    }
}

/// Per file metadata recorded in the manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FileMeta {
    /// SHA-256 of the content (for symlinks, of the link target).
    pub sha256: String,
    pub size: u64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub exec: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symlink: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "lowercase")]
pub enum RepoMode {
    #[default]
    Bundle,
    Remote,
    Files,
}

impl RepoMode {
    pub fn as_str(self) -> &'static str {
        match self {
            RepoMode::Bundle => "bundle",
            RepoMode::Remote => "remote",
            RepoMode::Files => "files",
        }
    }
    pub fn parse(s: &str) -> Option<RepoMode> {
        match s {
            "bundle" => Some(RepoMode::Bundle),
            "remote" => Some(RepoMode::Remote),
            "files" => Some(RepoMode::Files),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
pub struct WipInfo {
    pub branch: String,
    pub commit: String,
    /// Commit the WIP is based on (in the transported history).
    pub base: String,
}

/// Per repository state recorded in the manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
pub struct RepoManifest {
    pub path: String,
    pub mode: RepoMode,
    #[serde(default)]
    pub history: String,
    /// Branch tips as transported (rebuilt commits for `recent`/`snapshot`).
    #[serde(default)]
    pub branches: BTreeMap<String, String>,
    /// Branch checked out on the pushing device.
    #[serde(default)]
    pub head_branch: Option<String>,
    #[serde(default)]
    pub origin_url: Option<String>,
    #[serde(default)]
    pub via: Option<String>,
    /// Full base bundle (self-contained) and incremental bundle in the snapshot.
    #[serde(default)]
    pub base_bundle: Option<String>,
    #[serde(default)]
    pub bundle: Option<String>,
    #[serde(default)]
    pub rebuild_map: Option<String>,
    #[serde(default)]
    pub wip: Option<WipInfo>,
    /// `files` mode baseline.
    #[serde(default)]
    pub baseline: Option<String>,
    #[serde(default)]
    pub commits: u64,
    #[serde(default)]
    pub user_email: Option<String>,
}

/// `.workroam-snapshot/manifest.json` inside each snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Manifest {
    pub format: u32,
    pub project: String,
    pub name: String,
    pub parent: Option<String>,
    #[serde(default)]
    pub restored_from: Option<String>,
    pub device: DeviceRef,
    /// Absolute project path on the pushing device (for session migration).
    #[serde(default)]
    pub device_path: String,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub handoff_updated: bool,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub note: String,
    pub files: BTreeMap<String, FileMeta>,
    #[serde(default)]
    pub repos: BTreeMap<String, RepoManifest>,
    /// agent id -> directory inside `.workroam-snapshot/sessions/`.
    #[serde(default)]
    pub sessions: BTreeMap<String, String>,
    #[serde(default)]
    pub summary: ChangeSummary,
}

/// Directory that holds virtual files inside a snapshot.
pub const SNAPSHOT_META_DIR: &str = ".workroam-snapshot";
pub const MANIFEST_PATH: &str = ".workroam-snapshot/manifest.json";

/// Snapshot tags (architecture §5).
pub const TAG_WORKROAM: &str = "workroam";
pub const TAG_ARCHIVED: &str = "workroam:archived";
pub fn project_tag(pid: &str) -> String {
    format!("workroam:project={pid}")
}

/// Summary of a snapshot for history lists.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SnapshotInfo {
    pub id: String,
    pub short_id: String,
    pub time: DateTime<Utc>,
    pub device: String,
    pub archived: bool,
    #[serde(default)]
    pub summary: ChangeSummary,
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub restored_from: Option<String>,
    #[serde(default)]
    pub note: String,
}
