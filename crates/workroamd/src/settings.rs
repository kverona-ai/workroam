//! `config.list / get / set` (architecture §17.2): one set of keys for the
//! desktop settings page and `workroam config`.

use crate::Daemon;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use workroam_core::config::{MachineConfig, ProjectConfig};
use workroam_core::model::{AutoMode, RelayRecord};
use workroam_core::{Error, Result};
use workroam_store::control::keys;

#[derive(Debug, Clone, Copy, Serialize)]
pub struct KeyDef {
    pub key: &'static str,
    /// machine, project (machine with a project override), project-only, workspace.
    pub scope: &'static str,
    pub kind: &'static str,
}

pub const KEYS: &[KeyDef] = &[
    KeyDef {
        key: "device.name",
        scope: "machine",
        kind: "string",
    },
    KeyDef {
        key: "root",
        scope: "machine",
        kind: "path",
    },
    KeyDef {
        key: "reachable",
        scope: "machine",
        kind: "list",
    },
    KeyDef {
        key: "autostart",
        scope: "machine",
        kind: "bool",
    },
    KeyDef {
        key: "sync.auto",
        scope: "project",
        kind: "off|ask|auto",
    },
    KeyDef {
        key: "sync.agent_requires_approval",
        scope: "project",
        kind: "bool",
    },
    KeyDef {
        key: "sync.idle_minutes",
        scope: "machine",
        kind: "number",
    },
    KeyDef {
        key: "sync.on_sleep",
        scope: "machine",
        kind: "off|ask|auto",
    },
    KeyDef {
        key: "sync.on_wake",
        scope: "machine",
        kind: "bool",
    },
    KeyDef {
        key: "sync.approval_timeout",
        scope: "machine",
        kind: "duration",
    },
    KeyDef {
        key: "notify.ntfy",
        scope: "machine",
        kind: "url",
    },
    KeyDef {
        key: "notify.on_success",
        scope: "machine",
        kind: "bool",
    },
    KeyDef {
        key: "ui.banner",
        scope: "machine",
        kind: "bool",
    },
    KeyDef {
        key: "ui.lang",
        scope: "machine",
        kind: "zh|en|",
    },
    KeyDef {
        key: "mcp.enabled",
        scope: "project",
        kind: "bool",
    },
    KeyDef {
        key: "remote.enabled",
        scope: "project-only",
        kind: "bool",
    },
    KeyDef {
        key: "remote.default_device",
        scope: "workspace",
        kind: "device",
    },
    KeyDef {
        key: "retention.days",
        scope: "workspace",
        kind: "number",
    },
    KeyDef {
        key: "retention.weekly",
        scope: "workspace",
        kind: "number",
    },
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Setting {
    pub key: String,
    pub value: Value,
    /// machine, project, workspace or default.
    pub source: String,
    pub scope: String,
}

fn def(key: &str) -> Result<&'static KeyDef> {
    KEYS.iter().find(|k| k.key == key).ok_or_else(|| {
        Error::invalid(format!("unknown setting {key}"))
            .with_hint("workroam config list shows every key")
    })
}

fn parse_bool(v: &str) -> Result<bool> {
    match v {
        "true" | "on" | "yes" | "1" => Ok(true),
        "false" | "off" | "no" | "0" => Ok(false),
        _ => Err(Error::invalid(format!("{v} is not true or false"))),
    }
}

fn parse_auto(v: &str) -> Result<AutoMode> {
    match v {
        "off" => Ok(AutoMode::Off),
        "ask" => Ok(AutoMode::Ask),
        "auto" => Ok(AutoMode::Auto),
        _ => Err(Error::invalid(format!("{v} is not off, ask or auto"))),
    }
}

fn parse_num(v: &str) -> Result<u32> {
    v.parse()
        .map_err(|_| Error::invalid(format!("{v} is not a number")))
}

fn machine_value(m: &MachineConfig, key: &str) -> Value {
    match key {
        "device.name" => json!(m.name),
        "root" => json!(m.root),
        "reachable" => json!(m.reachable),
        "autostart" => json!(m.autostart),
        "sync.auto" => json!(m.sync.auto.as_str()),
        "sync.agent_requires_approval" => json!(m.sync.agent_requires_approval),
        "sync.idle_minutes" => json!(m.sync.idle_minutes),
        "sync.on_sleep" => json!(m.sync.on_sleep.as_str()),
        "sync.on_wake" => json!(m.sync.on_wake),
        "sync.approval_timeout" => json!(m.sync.approval_timeout),
        "notify.ntfy" => json!(m.notify.ntfy),
        "notify.on_success" => json!(m.notify.on_success),
        "ui.banner" => json!(m.ui.banner),
        "ui.lang" => json!(m.ui.lang),
        "mcp.enabled" => json!(m.mcp.enabled),
        _ => Value::Null,
    }
}

fn project_value(c: &ProjectConfig, key: &str) -> Option<Value> {
    match key {
        "sync.auto" => c.sync.auto.map(|a| json!(a.as_str())),
        "sync.agent_requires_approval" => c.sync.agent_requires_approval.map(|b| json!(b)),
        "mcp.enabled" => c.mcp.enabled.map(|b| json!(b)),
        "remote.enabled" => Some(json!(c.remote.enabled)),
        _ => None,
    }
}

fn project_dir(m: &MachineConfig, project: &str) -> Result<std::path::PathBuf> {
    m.projects
        .get(project)
        .map(|b| workroam_core::paths::expand_tilde(&b.path))
        .ok_or_else(|| Error::invalid(format!("project {project} is not on this device")))
}

pub async fn get(d: &Daemon, key: &str, project: Option<&str>) -> Result<Setting> {
    let kd = def(key)?;
    let m = crate::load_machine()?;
    if kd.scope == "workspace" {
        let ws = d.ws().await?;
        let value = match key {
            "retention.days" => json!(ws.settings().await?.retention_days),
            "retention.weekly" => json!(ws.settings().await?.retention_weekly),
            _ => json!(ws
                .control
                .get_all::<RelayRecord>(keys::RELAYS)
                .await?
                .into_iter()
                .find_map(|(_, r)| r.default_device)),
        };
        return Ok(Setting {
            key: key.into(),
            value,
            source: "workspace".into(),
            scope: kd.scope.into(),
        });
    }
    if let Some(p) = project {
        if kd.scope.starts_with("project") {
            let cfg = ProjectConfig::load(&project_dir(&m, p)?)?.unwrap_or_default();
            if let Some(v) = project_value(&cfg, key) {
                return Ok(Setting {
                    key: key.into(),
                    value: v,
                    source: "project".into(),
                    scope: kd.scope.into(),
                });
            }
        }
    }
    if kd.scope == "project-only" {
        return Ok(Setting {
            key: key.into(),
            value: json!(false),
            source: "default".into(),
            scope: kd.scope.into(),
        });
    }
    Ok(Setting {
        key: key.into(),
        value: machine_value(&m, key),
        source: "machine".into(),
        scope: kd.scope.into(),
    })
}

pub async fn list(d: &Daemon, project: Option<&str>) -> Result<Vec<Setting>> {
    let mut out = Vec::new();
    for k in KEYS {
        match get(d, k.key, project).await {
            Ok(s) => out.push(s),
            Err(e) if k.scope == "workspace" => out.push(Setting {
                key: k.key.into(),
                value: Value::Null,
                source: format!("unavailable: {}", e.message),
                scope: k.scope.into(),
            }),
            Err(e) => return Err(e),
        }
    }
    Ok(out)
}

pub async fn set(d: &Daemon, key: &str, value: &str, project: Option<&str>) -> Result<Setting> {
    let kd = def(key)?;
    let value = value.trim();
    match kd.scope {
        "workspace" => {
            let ws = d.ws().await?;
            ws.ensure_writable()?;
            match key {
                "retention.days" | "retention.weekly" => {
                    let mut s = ws.settings().await?;
                    let n = parse_num(value)?;
                    if key == "retention.days" {
                        if n == 0 {
                            return Err(Error::invalid("retention.days must be at least 1"));
                        }
                        s.retention_days = n;
                    } else {
                        s.retention_weekly = n;
                    }
                    ws.save_settings(&s).await?;
                }
                _ => {
                    let dev = workroam_device::registry::find(&ws.control, value)
                        .await?
                        .ok_or_else(|| Error::invalid(format!("no device named {value}")))?;
                    let relays = ws.control.get_all::<RelayRecord>(keys::RELAYS).await?;
                    if relays.is_empty() {
                        return Err(Error::invalid("no relay is connected yet")
                            .with_hint("workroam agent connect <agent> sets one up"));
                    }
                    for (k, mut r) in relays {
                        r.default_device = Some(dev.id.clone());
                        ws.control.put(&k, &r).await?;
                    }
                }
            }
        }
        "project-only" => {
            let p = project.ok_or_else(|| {
                Error::invalid(format!("{key} is set per project")).with_hint("add -p <project>")
            })?;
            let m = crate::load_machine()?;
            let dir = project_dir(&m, p)?;
            let mut c = ProjectConfig::load(&dir)?.unwrap_or_default();
            c.remote.enabled = parse_bool(value)?;
            c.save(&dir)?;
        }
        _ if project.is_some() && kd.scope == "project" => {
            let m = crate::load_machine()?;
            let dir = project_dir(&m, project.unwrap())?;
            let mut c = ProjectConfig::load(&dir)?.unwrap_or_default();
            match key {
                "sync.auto" => c.sync.auto = Some(parse_auto(value)?),
                "sync.agent_requires_approval" => {
                    c.sync.agent_requires_approval = Some(parse_bool(value)?)
                }
                _ => c.mcp.enabled = Some(parse_bool(value)?),
            }
            c.save(&dir)?;
        }
        _ => {
            // Validate first so a bad value never reaches machine.toml.
            let mut m = crate::load_machine()?;
            apply_machine(&mut m, key, value)?;
            let rename = key == "device.name";
            let autostart = key == "autostart";
            let m2 = m.clone();
            d.update_machine(move |x| *x = m2).await?;
            if rename {
                if let Ok(ws) = d.ws().await {
                    if let Ok(Some(mut rec)) =
                        workroam_device::registry::get(&ws.control, &m.device_id).await
                    {
                        rec.name = m.name.clone();
                        let _ = workroam_device::registry::register(&ws.control, &rec).await;
                    }
                }
            }
            if autostart {
                if m.autostart {
                    workroam_platform::service::install(
                        &workroam_platform::service::daemon_binary(),
                    )?;
                } else {
                    workroam_platform::service::uninstall()?;
                }
            }
        }
    }
    get(d, key, project).await
}

fn apply_machine(m: &mut MachineConfig, key: &str, v: &str) -> Result<()> {
    match key {
        "device.name" => {
            if v.is_empty() {
                return Err(Error::invalid("the device name cannot be empty"));
            }
            m.name = v.to_string()
        }
        "root" => m.root = v.to_string(),
        "reachable" => {
            m.reachable = v
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        }
        "autostart" => m.autostart = parse_bool(v)?,
        "sync.auto" => m.sync.auto = parse_auto(v)?,
        "sync.agent_requires_approval" => m.sync.agent_requires_approval = parse_bool(v)?,
        "sync.idle_minutes" => m.sync.idle_minutes = parse_num(v)?,
        "sync.on_sleep" => m.sync.on_sleep = parse_auto(v)?,
        "sync.on_wake" => m.sync.on_wake = parse_bool(v)?,
        "sync.approval_timeout" => {
            workroam_core::duration::parse(v)?;
            m.sync.approval_timeout = v.to_string()
        }
        "notify.ntfy" => {
            if !v.is_empty() && url::Url::parse(v).is_err() {
                return Err(Error::invalid(format!("{v} is not a URL")));
            }
            m.notify.ntfy = v.to_string()
        }
        "notify.on_success" => m.notify.on_success = parse_bool(v)?,
        "ui.banner" => m.ui.banner = parse_bool(v)?,
        "ui.lang" => {
            if !matches!(v, "" | "zh" | "en") {
                return Err(Error::invalid(
                    "ui.lang is zh, en or empty (follow the system)",
                ));
            }
            m.ui.lang = v.to_string()
        }
        "mcp.enabled" => m.mcp.enabled = parse_bool(v)?,
        _ => return Err(Error::invalid(format!("{key} cannot be set here"))),
    }
    Ok(())
}
