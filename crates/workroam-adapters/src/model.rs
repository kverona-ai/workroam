//! Adapter file format (ADPT-01).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct Detect {
    pub command: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct GenerateRule {
    pub path: String,
    pub template: String,
    /// `missing` (only when the file does not exist) or `always`.
    #[serde(default = "missing")]
    pub when: String,
}

fn missing() -> String {
    "missing".into()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(default)]
pub struct Rules {
    pub read: Vec<String>,
    pub generate: Vec<GenerateRule>,
    pub import: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(default)]
pub struct Skills {
    pub source: String,
    /// Directory link the agent reads (none when it reads the source natively).
    pub link: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(default)]
pub struct Mcp {
    pub path: String,
    /// `claude-mcp-json`, `codex-toml`, `agents-mcp-json`.
    pub format: String,
    pub import: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(default)]
pub struct Sessions {
    pub dir: String,
    /// `dash-path-key`, `pi-path-key` or `cwd-filter`.
    pub path_key: String,
    pub rewrite_fields: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(default)]
pub struct Exclude {
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(default)]
pub struct Setup {
    pub open_url: Option<String>,
    pub chat_prompt: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(default)]
pub struct Connect {
    /// `local` or `cloud`.
    pub kind: String,
    pub skill: bool,
    /// `stdio` to register `workroam mcp` in the agent's MCP config.
    pub mcp: Option<String>,
    pub transport: Option<String>,
    pub auth: Option<String>,
    pub setup: Setup,
    pub skill_upload: Option<String>,
    pub webhook: Option<String>,
    pub cloud_computer: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Adapter {
    pub id: String,
    pub name: String,
    pub adapter_version: u32,
    #[serde(default)]
    pub agent_versions: String,
    #[serde(default)]
    pub detect: Option<Detect>,
    #[serde(default)]
    pub rules: Rules,
    #[serde(default)]
    pub skills: Option<Skills>,
    #[serde(default)]
    pub mcp: Option<Mcp>,
    #[serde(default)]
    pub sessions: Option<Sessions>,
    #[serde(default)]
    pub exclude: Exclude,
    #[serde(default)]
    pub connect: Connect,
}

impl Adapter {
    pub fn is_cloud(&self) -> bool {
        self.connect.kind == "cloud"
    }
    pub fn is_local(&self) -> bool {
        !self.is_cloud()
    }
    /// Project-relative files this adapter generates (never synced).
    pub fn generated_paths(&self) -> Vec<String> {
        let mut v: Vec<String> = self.rules.generate.iter().map(|g| g.path.clone()).collect();
        if let Some(s) = &self.skills {
            if let Some(l) = &s.link {
                v.push(l.clone());
            }
        }
        if let Some(m) = &self.mcp {
            v.push(m.path.clone());
        }
        v
    }
}
