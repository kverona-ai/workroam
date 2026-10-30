//! Per-machine generation after every pull (PULL-05, architecture §12.3):
//! rule reference files, Skills links, native MCP configs and the machine block.

use crate::envblock;
use crate::model::Adapter;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use workroam_core::config::{McpConfig, McpServerDef};
use workroam_core::model::OsKind;
use workroam_core::{Error, Result};

/// Resolves `${secret:name}` on this machine.
pub type SecretFn<'a> = &'a dyn Fn(&str) -> Option<String>;

pub struct GenerateOptions<'a> {
    pub project_dir: &'a Path,
    pub project_name: &'a str,
    pub os: OsKind,
    /// Agents enabled for this project.
    pub agents: &'a [Adapter],
    pub mcp: &'a McpConfig,
    pub secret: SecretFn<'a>,
    /// Register `workroam mcp` (stdio) for agents that support it.
    pub workroam_mcp: bool,
    pub env: Option<&'a envblock::EnvInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Generated {
    /// Project-relative paths written by Workroam on this machine.
    pub paths: Vec<String>,
    pub warnings: Vec<String>,
}

impl Generated {
    pub fn state_path(project_dir: &Path) -> PathBuf {
        project_dir
            .join(".workroam")
            .join("local")
            .join("generated.json")
    }
    pub fn load(project_dir: &Path) -> Generated {
        std::fs::read(Self::state_path(project_dir))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }
    fn save(&self, project_dir: &Path) -> Result<()> {
        workroam_core::paths::write_atomic(
            &Self::state_path(project_dir),
            &serde_json::to_vec_pretty(self)?,
        )
    }
}

/// Replace `{project}` and `${secret:x}` in a value.
pub fn resolve(
    value: &str,
    project_dir: &Path,
    project_name: &str,
    secret: SecretFn,
) -> Result<String> {
    let mut out = value
        .replace("{project_dir}", &project_dir.display().to_string())
        .replace("{project}", project_name);
    while let Some(a) = out.find("${secret:") {
        let Some(rel) = out[a..].find('}') else { break };
        let name = out[a + 9..a + rel].to_string();
        let v = secret(&name).ok_or_else(|| {
            Error::new(
                workroam_core::Code::InvalidArgs,
                format!("secret `{name}` is not set on this machine"),
            )
            .with_hint(format!("workroam secret set {name}"))
        })?;
        out.replace_range(a..a + rel + 1, &v);
    }
    Ok(out)
}

/// Servers for one agent, with per-OS overrides applied and values resolved.
pub fn servers_for(
    opts: &GenerateOptions,
    agent: &Adapter,
) -> (BTreeMap<String, McpServerDef>, Vec<String>) {
    let mut out = BTreeMap::new();
    let mut warnings = Vec::new();
    for (name, def) in &opts.mcp.servers {
        if !def.targets_agent(&agent.id) {
            continue;
        }
        let d = def.for_os(opts.os);
        let r = |s: &str| resolve(s, opts.project_dir, opts.project_name, opts.secret);
        let resolved = (|| -> Result<McpServerDef> {
            Ok(McpServerDef {
                command: d.command.as_deref().map(r).transpose()?,
                args: d.args.iter().map(|a| r(a)).collect::<Result<_>>()?,
                env: d
                    .env
                    .iter()
                    .map(|(k, v)| Ok((k.clone(), r(v)?)))
                    .collect::<Result<_>>()?,
                url: d.url.as_deref().map(r).transpose()?,
                headers: d
                    .headers
                    .iter()
                    .map(|(k, v)| Ok((k.clone(), r(v)?)))
                    .collect::<Result<_>>()?,
                targets: vec![],
                windows: None,
                macos: None,
                linux: None,
            })
        })();
        match resolved {
            Ok(x) => {
                out.insert(name.clone(), x);
            }
            Err(e) => warnings.push(format!("{}: {name}: {}", agent.name, e.message)),
        }
    }
    if opts.workroam_mcp
        && agent.connect.mcp.as_deref() == Some("stdio")
        && !out.contains_key("workroam")
    {
        out.insert(
            "workroam".into(),
            McpServerDef {
                command: Some("workroam".into()),
                args: vec!["mcp".into()],
                ..Default::default()
            },
        );
    }
    (out, warnings)
}

fn json_server(d: &McpServerDef, format: &str) -> Value {
    let mut m = Map::new();
    if let Some(url) = &d.url {
        if format == "claude-mcp-json" {
            m.insert("type".into(), json!("http"));
            m.insert("url".into(), json!(url));
        } else {
            m.insert("serverUrl".into(), json!(url));
        }
        if !d.headers.is_empty() {
            m.insert("headers".into(), json!(d.headers));
        }
    } else {
        m.insert(
            "command".into(),
            json!(d.command.clone().unwrap_or_default()),
        );
        m.insert("args".into(), json!(d.args));
        if !d.env.is_empty() {
            m.insert("env".into(), json!(d.env));
        }
    }
    Value::Object(m)
}

/// Render a native MCP config, merging with whatever else the file holds.
pub fn render_mcp(
    format: &str,
    servers: &BTreeMap<String, McpServerDef>,
    existing: Option<&str>,
) -> Result<String> {
    match format {
        "claude-mcp-json" | "agents-mcp-json" => {
            let mut root = existing
                .and_then(|s| serde_json::from_str::<Value>(s).ok())
                .filter(|v| v.is_object())
                .unwrap_or_else(|| json!({}));
            let map: Map<String, Value> = servers
                .iter()
                .map(|(k, d)| (k.clone(), json_server(d, format)))
                .collect();
            root["mcpServers"] = Value::Object(map);
            Ok(serde_json::to_string_pretty(&root)? + "\n")
        }
        "codex-toml" => {
            let mut root: toml::Table = existing.and_then(|s| s.parse().ok()).unwrap_or_default();
            let mut tbl = toml::Table::new();
            for (k, d) in servers {
                let mut t = toml::Table::new();
                if let Some(url) = &d.url {
                    t.insert("url".into(), url.clone().into());
                    if !d.headers.is_empty() {
                        t.insert(
                            "http_headers".into(),
                            toml::Value::try_from(&d.headers)
                                .map_err(|e| Error::internal(e.to_string()))?,
                        );
                    }
                } else {
                    t.insert(
                        "command".into(),
                        d.command.clone().unwrap_or_default().into(),
                    );
                    t.insert(
                        "args".into(),
                        toml::Value::try_from(&d.args)
                            .map_err(|e| Error::internal(e.to_string()))?,
                    );
                    if !d.env.is_empty() {
                        t.insert(
                            "env".into(),
                            toml::Value::try_from(&d.env)
                                .map_err(|e| Error::internal(e.to_string()))?,
                        );
                    }
                }
                tbl.insert(k.clone(), toml::Value::Table(t));
            }
            root.insert("mcp_servers".into(), toml::Value::Table(tbl));
            Ok(toml::to_string_pretty(&root)?)
        }
        other => Err(Error::invalid(format!("unknown MCP format `{other}`"))),
    }
}

/// Parse a native MCP config back into server definitions (import, §12.3).
pub fn parse_mcp(format: &str, content: &str) -> Result<BTreeMap<String, McpServerDef>> {
    let mut out = BTreeMap::new();
    match format {
        "claude-mcp-json" | "agents-mcp-json" => {
            let v: Value = serde_json::from_str(content)?;
            if let Some(m) = v.get("mcpServers").and_then(|m| m.as_object()) {
                for (k, s) in m {
                    let strmap = |key: &str| -> BTreeMap<String, String> {
                        s.get(key)
                            .and_then(|e| e.as_object())
                            .map(|o| {
                                o.iter()
                                    .map(|(a, b)| {
                                        (a.clone(), b.as_str().unwrap_or_default().to_string())
                                    })
                                    .collect()
                            })
                            .unwrap_or_default()
                    };
                    out.insert(
                        k.clone(),
                        McpServerDef {
                            command: s.get("command").and_then(|c| c.as_str()).map(String::from),
                            args: s
                                .get("args")
                                .and_then(|a| a.as_array())
                                .map(|a| {
                                    a.iter()
                                        .filter_map(|x| x.as_str().map(String::from))
                                        .collect()
                                })
                                .unwrap_or_default(),
                            env: strmap("env"),
                            url: s
                                .get("url")
                                .or_else(|| s.get("serverUrl"))
                                .and_then(|c| c.as_str())
                                .map(String::from),
                            headers: strmap("headers"),
                            ..Default::default()
                        },
                    );
                }
            }
        }
        "codex-toml" => {
            let v: toml::Table = content
                .parse()
                .map_err(|e: toml::de::Error| Error::invalid(e.to_string()))?;
            if let Some(m) = v.get("mcp_servers").and_then(|m| m.as_table()) {
                for (k, s) in m {
                    let strmap = |key: &str| -> BTreeMap<String, String> {
                        s.get(key)
                            .and_then(|e| e.as_table())
                            .map(|o| {
                                o.iter()
                                    .map(|(a, b)| {
                                        (a.clone(), b.as_str().unwrap_or_default().to_string())
                                    })
                                    .collect()
                            })
                            .unwrap_or_default()
                    };
                    out.insert(
                        k.clone(),
                        McpServerDef {
                            command: s.get("command").and_then(|c| c.as_str()).map(String::from),
                            args: s
                                .get("args")
                                .and_then(|a| a.as_array())
                                .map(|a| {
                                    a.iter()
                                        .filter_map(|x| x.as_str().map(String::from))
                                        .collect()
                                })
                                .unwrap_or_default(),
                            env: strmap("env"),
                            url: s.get("url").and_then(|c| c.as_str()).map(String::from),
                            headers: strmap("http_headers"),
                            ..Default::default()
                        },
                    );
                }
            }
        }
        other => return Err(Error::invalid(format!("unknown MCP format `{other}`"))),
    }
    Ok(out)
}

fn write_if_changed(path: &Path, content: &str) -> Result<bool> {
    if std::fs::read_to_string(path).ok().as_deref() == Some(content) {
        return Ok(false);
    }
    workroam_core::paths::write_atomic(path, content.as_bytes())?;
    Ok(true)
}

/// Generate every per-machine file and record them in `.workroam/local/generated.json`.
pub fn generate(opts: &GenerateOptions) -> Result<Generated> {
    let dir = opts.project_dir;
    let previous = Generated::load(dir);
    let was_ours = |p: &str| previous.paths.iter().any(|x| x == p);
    let mut gen = Generated::default();
    let add = |gen: &mut Generated, p: &str| {
        if !gen.paths.iter().any(|x| x == p) {
            gen.paths.push(p.to_string());
        }
    };

    for agent in opts.agents.iter().filter(|a| a.is_local()) {
        // Rule reference files.
        for g in &agent.rules.generate {
            let path = dir.join(&g.path);
            let exists = path.exists();
            if g.when == "always" || !exists || was_ours(&g.path) {
                if exists && !was_ours(&g.path) && g.when != "always" {
                    continue;
                }
                write_if_changed(&path, &g.template)?;
                add(&mut gen, &g.path);
            }
        }
        // Skills links.
        if let Some(sk) = &agent.skills {
            if let Some(link) = &sk.link {
                let src = dir.join(&sk.source);
                std::fs::create_dir_all(&src)?;
                let lp = dir.join(link);
                let is_link = std::fs::symlink_metadata(&lp)
                    .map(|m| m.file_type().is_symlink())
                    .unwrap_or(false)
                    || workroam_platform::fsutil::is_junction(&lp);
                if lp.exists() && !is_link && !was_ours(link) {
                    gen.warnings.push(format!(
                        "{}: {link} exists and is not a link; left as is",
                        agent.name
                    ));
                } else {
                    if is_link || lp.exists() {
                        let _ = workroam_platform::fsutil::remove_any(&lp);
                    }
                    if let Some(parent) = lp.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    workroam_platform::fsutil::link_dir(&lp, &src)?;
                    add(&mut gen, link);
                }
            }
        }
        // Native MCP config.
        if let Some(m) = &agent.mcp {
            let (servers, warns) = servers_for(opts, agent);
            gen.warnings.extend(warns);
            let path = dir.join(&m.path);
            if servers.is_empty() && !path.exists() {
                continue;
            }
            let existing = std::fs::read_to_string(&path).ok();
            let body = render_mcp(&m.format, &servers, existing.as_deref())?;
            write_if_changed(&path, &body)?;
            add(&mut gen, &m.path);
        }
    }

    // Files we generated before but no longer do (agent removed).
    for p in &previous.paths {
        if !gen.paths.contains(p) {
            let path = dir.join(p);
            let _ = workroam_platform::fsutil::remove_any(&path);
        }
    }

    // The machine block.
    if let Some(env) = opts.env {
        let agents_md = dir.join("AGENTS.md");
        let cur = std::fs::read_to_string(&agents_md).unwrap_or_default();
        let next = envblock::apply(&cur, &envblock::render(env));
        write_if_changed(&agents_md, &next)?;
    }

    gen.save(dir)?;
    Ok(gen)
}

/// Before push: the machine block is stripped from the synced copy of AGENTS.md.
pub fn strip_for_push(agents_md: &str) -> String {
    envblock::strip(agents_md)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_claude_and_codex() {
        let td = tempfile::tempdir().unwrap();
        let dir = td.path();
        std::fs::create_dir_all(dir.join(".agents/skills/demo")).unwrap();
        std::fs::write(dir.join("AGENTS.md"), "# Rules\n").unwrap();
        let mut mcp = McpConfig::default();
        mcp.servers.insert(
            "db".into(),
            McpServerDef {
                command: Some("db-mcp".into()),
                args: vec!["--token".into(), "${secret:db}".into(), "{project}".into()],
                windows: Some(Box::new(McpServerDef {
                    command: Some("db-mcp.exe".into()),
                    ..Default::default()
                })),
                ..Default::default()
            },
        );
        let agents = vec![
            crate::by_id("claude").unwrap(),
            crate::by_id("codex").unwrap(),
        ];
        let secret = |n: &str| (n == "db").then(|| "s3cr3t".to_string());
        let env = envblock::EnvInfo {
            device: "home".into(),
            ..Default::default()
        };
        let opts = GenerateOptions {
            project_dir: dir,
            project_name: "proj-a",
            os: OsKind::Linux,
            agents: &agents,
            mcp: &mcp,
            secret: &secret,
            workroam_mcp: true,
            env: Some(&env),
        };
        let g = generate(&opts).unwrap();
        assert!(g.paths.contains(&"CLAUDE.md".to_string()));
        assert!(g.paths.contains(&".mcp.json".to_string()));
        assert!(g.paths.contains(&".codex/config.toml".to_string()));
        let mj: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join(".mcp.json")).unwrap()).unwrap();
        assert_eq!(mj["mcpServers"]["db"]["args"][1], "s3cr3t");
        assert_eq!(mj["mcpServers"]["db"]["args"][2], "proj-a");
        assert_eq!(mj["mcpServers"]["workroam"]["command"], "workroam");
        let codex = std::fs::read_to_string(dir.join(".codex/config.toml")).unwrap();
        let parsed = parse_mcp("codex-toml", &codex).unwrap();
        assert_eq!(parsed["db"].command.as_deref(), Some("db-mcp"));
        assert!(dir.join(".claude/skills/demo").exists());
        assert!(std::fs::read_to_string(dir.join("AGENTS.md"))
            .unwrap()
            .contains(envblock::ENV_BEGIN));
        // Second run is stable.
        let g2 = generate(&opts).unwrap();
        assert_eq!(g.paths, g2.paths);
        // Windows override.
        let (s, _) = servers_for(
            &GenerateOptions {
                os: OsKind::Windows,
                ..opts
            },
            &agents[0],
        );
        assert_eq!(s["db"].command.as_deref(), Some("db-mcp.exe"));
    }
}
