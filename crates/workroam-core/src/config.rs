//! `machine.toml` (architecture §6.2), `workroam.toml` (§6.3) and `mcp.toml` (§6.4).

use crate::model::{Access, AutoMode, OsKind, RepoMode};
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Addressing {
    #[default]
    Path,
    VirtualHost,
}

impl Addressing {
    pub fn as_str(self) -> &'static str {
        match self {
            Addressing::Path => "path",
            Addressing::VirtualHost => "virtual-host",
        }
    }
    pub fn parse(s: &str) -> Option<Addressing> {
        match s {
            "path" => Some(Addressing::Path),
            "virtual-host" | "virtual" => Some(Addressing::VirtualHost),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Capabilities {
    #[serde(default)]
    pub conditional_create: bool,
    #[serde(default)]
    pub conditional_overwrite: bool,
    #[serde(default)]
    pub multipart: bool,
}

impl Capabilities {
    pub fn conditional(&self) -> bool {
        self.conditional_create && self.conditional_overwrite
    }
}

/// Where the workspace lives. `endpoint` may be `file:///path` for a
/// workspace kept in a local or mounted directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WorkspaceLocation {
    pub endpoint: String,
    pub bucket: String,
    #[serde(default)]
    pub prefix: String,
    #[serde(default)]
    pub addressing: Addressing,
    #[serde(default)]
    pub region: String,
    #[serde(default)]
    pub capabilities: Capabilities,
    #[serde(default)]
    pub workspace_id: String,
    #[serde(default)]
    pub preset: String,
}

impl WorkspaceLocation {
    pub fn is_local(&self) -> bool {
        self.endpoint.starts_with("file://")
    }
    pub fn local_root(&self) -> Option<PathBuf> {
        self.endpoint.strip_prefix("file://").map(|p| {
            let mut root = PathBuf::from(p);
            if !self.bucket.is_empty() {
                root.push(&self.bucket);
            }
            if !self.prefix.is_empty() {
                root.push(self.prefix.trim_matches('/'));
            }
            root
        })
    }
    /// Object key prefix inside the bucket, with a trailing `/` when not empty.
    pub fn key_prefix(&self) -> String {
        let p = self.prefix.trim_matches('/');
        if p.is_empty() {
            String::new()
        } else {
            format!("{p}/")
        }
    }
    pub fn display(&self) -> String {
        if self.is_local() {
            self.endpoint.clone()
        } else if self.prefix.is_empty() {
            format!("{}/{}", self.endpoint.trim_end_matches('/'), self.bucket)
        } else {
            format!(
                "{}/{}/{}",
                self.endpoint.trim_end_matches('/'),
                self.bucket,
                self.prefix.trim_matches('/')
            )
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SyncConfig {
    pub auto: AutoMode,
    pub idle_minutes: u32,
    pub on_sleep: AutoMode,
    pub on_wake: bool,
    pub agent_requires_approval: bool,
    pub approval_timeout: String,
    pub paused_until: String,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            auto: AutoMode::Ask,
            idle_minutes: 15,
            on_sleep: AutoMode::Ask,
            on_wake: true,
            agent_requires_approval: true,
            approval_timeout: "10m".into(),
            paused_until: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NotifyConfig {
    pub ntfy: String,
    pub on_success: bool,
}

impl Default for NotifyConfig {
    fn default() -> Self {
        Self {
            ntfy: String::new(),
            on_success: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    pub banner: bool,
    pub lang: String,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            banner: true,
            lang: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct McpSettings {
    pub enabled: bool,
}

impl Default for McpSettings {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum BindingState {
    #[default]
    Pending,
    Bound,
    Missing,
    Archived,
}

impl BindingState {
    pub fn as_str(self) -> &'static str {
        match self {
            BindingState::Pending => "pending",
            BindingState::Bound => "bound",
            BindingState::Missing => "missing",
            BindingState::Archived => "archived",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectBinding {
    pub path: String,
    #[serde(default)]
    pub state: BindingState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct BackupConfig {
    /// `s3` or `local`.
    pub kind: String,
    pub endpoint: String,
    pub bucket: String,
    pub prefix: String,
    pub region: String,
    pub path: String,
    pub schedule: String,
    pub delete_delay_days: u32,
    pub addressing: Addressing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct RemoteSettings {
    /// Relay URL this device keeps an outbound connection to.
    pub relay_url: String,
    pub relay_id: String,
}

/// `~/.workroam/machine.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineConfig {
    pub name: String,
    pub os: OsKind,
    pub device_id: String,
    #[serde(default)]
    pub access: Access,
    #[serde(default)]
    pub root: String,
    #[serde(default)]
    pub reachable: Vec<String>,
    #[serde(default)]
    pub autostart: bool,
    pub workspace: WorkspaceLocation,
    #[serde(default)]
    pub sync: SyncConfig,
    #[serde(default)]
    pub notify: NotifyConfig,
    #[serde(default)]
    pub ui: UiConfig,
    #[serde(default)]
    pub mcp: McpSettings,
    #[serde(default)]
    pub remote: RemoteSettings,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup: Option<BackupConfig>,
    #[serde(default)]
    pub projects: BTreeMap<String, ProjectBinding>,
}

impl MachineConfig {
    pub fn load(path: &Path) -> Result<Option<MachineConfig>> {
        match std::fs::read_to_string(path) {
            Ok(s) => Ok(Some(toml::from_str(&s)?)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let body = toml::to_string_pretty(self)?;
        let header = "# Workroam 本机配置（machine.toml）。访问密钥与加密密码不在此文件中，只保存在系统密钥库。\n# Workroam machine config. Access keys and the encryption password live in the system keychain only.\n\n";
        crate::paths::write_atomic(path, format!("{header}{body}").as_bytes())
    }

    /// Default directory for a project: `<root>/<name>`.
    pub fn default_project_dir(&self, name: &str) -> PathBuf {
        let root = if self.root.is_empty() {
            crate::paths::home_dir().join("projects")
        } else {
            crate::paths::expand_tilde(&self.root)
        };
        root.join(name)
    }

    pub fn paused(&self) -> bool {
        if self.sync.paused_until.is_empty() {
            return false;
        }
        match chrono::DateTime::parse_from_rfc3339(&self.sync.paused_until) {
            Ok(t) => t > chrono::Utc::now(),
            Err(_) => false,
        }
    }

    pub fn project_for_path(&self, dir: &Path) -> Option<(String, PathBuf)> {
        let dir = crate::paths::normalize(dir);
        let mut best: Option<(String, PathBuf)> = None;
        for (name, b) in &self.projects {
            let p = crate::paths::normalize(&crate::paths::expand_tilde(&b.path));
            if dir.starts_with(&p) {
                let better = best
                    .as_ref()
                    .map(|(_, bp)| p.components().count() > bp.components().count())
                    .unwrap_or(true);
                if better {
                    best = Some((name.clone(), p));
                }
            }
        }
        best
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ProjectSync {
    pub exclude: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto: Option<AutoMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_requires_approval: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AgentToggle {
    pub state: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RepoConfig {
    pub path: String,
    #[serde(default)]
    pub mode: RepoMode,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub history: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub branches: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extras: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sensitive: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub shadow: bool,
}

impl RepoConfig {
    /// `full`, `recent:N` or `snapshot`, parsed.
    pub fn history_depth(&self) -> Option<usize> {
        parse_history(&self.history)
    }
}

/// `None` = full history; `Some(n)` = last n commits (0 = snapshot).
pub fn parse_history(h: &str) -> Option<usize> {
    match h {
        "" | "full" => None,
        "snapshot" => Some(0),
        s => s.strip_prefix("recent:").and_then(|n| n.parse().ok()),
    }
}

pub fn valid_history(h: &str) -> bool {
    matches!(h, "" | "full" | "snapshot")
        || h.strip_prefix("recent:")
            .map(|n| n.parse::<usize>().is_ok())
            .unwrap_or(false)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ProjectRemote {
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ProjectMcp {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

/// `<project>/.workroam/workroam.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ProjectConfig {
    pub name: String,
    pub sync: ProjectSync,
    pub agents: BTreeMap<String, AgentToggle>,
    pub repos: BTreeMap<String, RepoConfig>,
    pub mcp: ProjectMcp,
    pub remote: ProjectRemote,
}

pub const DEFAULT_EXCLUDES: &[&str] = &[
    "node_modules/",
    "target/",
    ".venv/",
    "dist/",
    "__pycache__/",
    ".DS_Store",
    "Thumbs.db",
];

impl ProjectConfig {
    pub fn path(project_dir: &Path) -> PathBuf {
        project_dir.join(".workroam").join("workroam.toml")
    }
    pub fn load(project_dir: &Path) -> Result<Option<ProjectConfig>> {
        match std::fs::read_to_string(Self::path(project_dir)) {
            Ok(s) => Ok(Some(toml::from_str(&s)?)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    pub fn save(&self, project_dir: &Path) -> Result<()> {
        let body = toml::to_string_pretty(self)?;
        let header = "# Workroam 项目同步策略（参与同步）。Project sync policy (synced).\n\n";
        crate::paths::write_atomic(
            &Self::path(project_dir),
            format!("{header}{body}").as_bytes(),
        )
    }
}

/// One MCP server in `.workroam/mcp.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct McpServerDef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    /// Agents that get this server; empty = all.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub targets: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub windows: Option<Box<McpServerDef>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub macos: Option<Box<McpServerDef>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linux: Option<Box<McpServerDef>>,
}

impl McpServerDef {
    /// Apply the per-OS override for `os`.
    pub fn for_os(&self, os: OsKind) -> McpServerDef {
        let ov = match os {
            OsKind::Windows => &self.windows,
            OsKind::Macos => &self.macos,
            OsKind::Linux => &self.linux,
        };
        let mut out = McpServerDef {
            windows: None,
            macos: None,
            linux: None,
            ..self.clone()
        };
        if let Some(o) = ov {
            if o.command.is_some() {
                out.command = o.command.clone();
                out.args = o.args.clone();
            } else if !o.args.is_empty() {
                out.args = o.args.clone();
            }
            if o.url.is_some() {
                out.url = o.url.clone();
            }
            for (k, v) in &o.env {
                out.env.insert(k.clone(), v.clone());
            }
            for (k, v) in &o.headers {
                out.headers.insert(k.clone(), v.clone());
            }
        }
        out
    }
    pub fn targets_agent(&self, agent: &str) -> bool {
        self.targets.is_empty() || self.targets.iter().any(|t| t == agent)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct McpConfig {
    #[serde(default)]
    pub servers: BTreeMap<String, McpServerDef>,
}

impl McpConfig {
    pub fn path(project_dir: &Path) -> PathBuf {
        project_dir.join(".workroam").join("mcp.toml")
    }
    pub fn load(project_dir: &Path) -> Result<McpConfig> {
        match std::fs::read_to_string(Self::path(project_dir)) {
            Ok(s) => Ok(toml::from_str(&s)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(McpConfig::default()),
            Err(e) => Err(e.into()),
        }
    }
    pub fn save(&self, project_dir: &Path) -> Result<()> {
        let body = toml::to_string_pretty(self)?;
        let header = "# MCP 唯一定义：拉取后为每个 Agent 生成原生配置。`${secret:name}` 只在本机解析。\n# Single source of MCP servers. Native configs are generated per agent after each pull.\n\n";
        crate::paths::write_atomic(
            &Self::path(project_dir),
            format!("{header}{body}").as_bytes(),
        )
    }
}

/// Validate a project name: used in paths and on the command line.
pub fn validate_project_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 100 {
        return Err(Error::invalid("project name must be 1-100 characters"));
    }
    if name.starts_with('.') || name.starts_with('-') {
        return Err(Error::invalid("project name cannot start with '.' or '-'"));
    }
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(Error::invalid(
            "project name may only contain letters, digits, '-', '_' and '.'",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_parse() {
        assert_eq!(parse_history("full"), None);
        assert_eq!(parse_history("recent:50"), Some(50));
        assert_eq!(parse_history("snapshot"), Some(0));
        assert!(valid_history("recent:3"));
        assert!(!valid_history("recent:x"));
    }

    #[test]
    fn mcp_override() {
        let src = r#"
[servers.filesystem]
command = "npx"
args = ["-y", "server", "{project}/docs"]
targets = ["claude", "codex"]

[servers.filesystem.windows]
command = "cmd"
args = ["/c", "npx", "-y", "server", "{project}/docs"]
"#;
        let cfg: McpConfig = toml::from_str(src).unwrap();
        let fs = &cfg.servers["filesystem"];
        assert_eq!(fs.for_os(OsKind::Windows).command.as_deref(), Some("cmd"));
        assert_eq!(fs.for_os(OsKind::Linux).command.as_deref(), Some("npx"));
        assert!(fs.targets_agent("claude"));
        assert!(!fs.targets_agent("grok"));
    }

    #[test]
    fn machine_roundtrip() {
        let m = MachineConfig {
            name: "office-win".into(),
            os: OsKind::Windows,
            device_id: "d_1".into(),
            access: Access::ReadWrite,
            root: "D:\\projects".into(),
            reachable: vec!["git.corp.internal".into()],
            autostart: true,
            workspace: WorkspaceLocation {
                endpoint: "https://s3.example.com".into(),
                bucket: "b".into(),
                ..Default::default()
            },
            sync: SyncConfig::default(),
            notify: NotifyConfig::default(),
            ui: UiConfig::default(),
            mcp: McpSettings::default(),
            remote: RemoteSettings::default(),
            backup: None,
            projects: BTreeMap::from([(
                "proj-a".into(),
                ProjectBinding {
                    path: "D:\\projects\\proj-a".into(),
                    state: BindingState::Bound,
                },
            )]),
        };
        let s = toml::to_string_pretty(&m).unwrap();
        let back: MachineConfig = toml::from_str(&s).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn names() {
        assert!(validate_project_name("proj-a").is_ok());
        assert!(validate_project_name("../x").is_err());
        assert!(validate_project_name("").is_err());
    }
}
