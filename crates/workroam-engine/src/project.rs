//! A project on this device: directory, configuration, agents and the
//! synced view of its files.

use crate::local::{Binding, SyncState};
use crate::scan::{self, LocalFile, ScanSpec};
use crate::workspace::Workspace;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use workroam_adapters::{envblock, generate, Adapter};
use workroam_core::config::{BindingState, McpConfig, ProjectConfig};
use workroam_core::model::{FileMeta, RepoMode};
use workroam_core::{Code, Error, Result};
use workroam_snapshot::{EntryKind, EntrySource, SourceEntry};

pub struct Project<'a> {
    pub ws: &'a Workspace,
    pub name: String,
    pub pid: String,
    pub dir: PathBuf,
    pub cfg: ProjectConfig,
}

/// One synced file: its metadata and where its content comes from.
#[derive(Debug, Clone)]
pub struct SyncFile {
    pub meta: FileMeta,
    pub abs: PathBuf,
    /// Content differs from disk (AGENTS.md without the machine block).
    pub bytes: Option<Vec<u8>>,
}

pub fn default_config(name: &str) -> ProjectConfig {
    ProjectConfig {
        name: name.to_string(),
        sync: workroam_core::config::ProjectSync {
            exclude: workroam_core::config::DEFAULT_EXCLUDES
                .iter()
                .map(|s| s.to_string())
                .collect(),
            ..Default::default()
        },
        ..Default::default()
    }
}

impl<'a> Project<'a> {
    /// The project as bound on this device (or its default directory).
    pub fn open(ws: &'a Workspace, name: &str) -> Result<Project<'a>> {
        workroam_core::config::validate_project_name(name)?;
        let dir = match ws.machine.projects.get(name) {
            Some(b) => workroam_core::paths::expand_tilde(&b.path),
            None => ws.machine.default_project_dir(name),
        };
        Self::at(ws, name, dir)
    }

    pub fn at(ws: &'a Workspace, name: &str, dir: PathBuf) -> Result<Project<'a>> {
        let cfg = ProjectConfig::load(&dir)?.unwrap_or_else(|| default_config(name));
        Ok(Project {
            pid: ws.project_id(name),
            ws,
            name: name.to_string(),
            dir,
            cfg,
        })
    }

    pub fn reload(&mut self) -> Result<()> {
        self.cfg = ProjectConfig::load(&self.dir)?.unwrap_or_else(|| default_config(&self.name));
        Ok(())
    }

    pub fn binding_state(&self) -> BindingState {
        self.ws
            .machine
            .projects
            .get(&self.name)
            .map(|b| b.state)
            .unwrap_or(BindingState::Pending)
    }

    /// Refuse to sync a project whose directory is gone or belongs to another project.
    pub fn ensure_bound_dir(&self) -> Result<()> {
        if self.binding_state() == BindingState::Bound || Binding::load(&self.dir).is_some() {
            if !self.dir.exists() {
                return Err(Error::new(
                    Code::InvalidArgs,
                    format!("project directory {} is missing", self.dir.display()),
                )
                .with_hint(
                    "put the folder back, or remove the project on this device and add it again",
                ));
            }
            if let Some(b) = Binding::load(&self.dir) {
                if b.project_id != self.pid {
                    return Err(Error::new(
                        Code::InvalidArgs,
                        format!(
                            "{} belongs to another project ({})",
                            self.dir.display(),
                            b.project
                        ),
                    )
                    .with_hint("choose another directory"));
                }
            }
        }
        Ok(())
    }

    pub fn state(&self) -> SyncState {
        SyncState::load(&self.dir)
    }

    pub fn bind(&self) -> Result<()> {
        let b = Binding {
            project_id: self.pid.clone(),
            project: self.name.clone(),
            device_id: self.ws.machine.device_id.clone(),
            path: self.dir.display().to_string(),
            bound_at: Some(chrono::Utc::now()),
        };
        b.save(&self.dir)
    }

    /// Enabled local agents (all detected when none are configured).
    pub fn agents(&self) -> Vec<Adapter> {
        let all = workroam_adapters::registry::local();
        if self.cfg.agents.is_empty() {
            return all;
        }
        all.into_iter()
            .filter(|a| self.cfg.agents.get(&a.id).map(|t| t.state).unwrap_or(false))
            .collect()
    }

    /// Files generated on this machine (never synced).
    pub fn generated(&self) -> Vec<String> {
        let mut v = generate::Generated::load(&self.dir).paths;
        for a in workroam_adapters::registry::local() {
            for p in a.generated_paths() {
                // Rule files the user wrote themselves stay synced.
                let ours = v.contains(&p);
                let is_rule = a.rules.generate.iter().any(|g| g.path == p);
                if (!is_rule || ours) && !v.contains(&p) {
                    v.push(p);
                }
            }
        }
        v
    }

    pub fn scan(&self) -> Result<BTreeMap<String, LocalFile>> {
        scan::scan(&ScanSpec {
            project: &self.dir,
            cfg: &self.cfg,
            generated: self.generated(),
        })
    }

    /// Local files as they would be synced (machine block stripped).
    pub fn sync_view(&self) -> Result<BTreeMap<String, SyncFile>> {
        let files = crate::workspace::blocking(|| self.scan())?;
        let mut out = BTreeMap::new();
        for (rel, f) in files {
            if rel == "AGENTS.md" {
                let raw = std::fs::read(&f.abs)?;
                let text = String::from_utf8_lossy(&raw).into_owned();
                let stripped = envblock::strip(&text).into_bytes();
                let meta = FileMeta {
                    sha256: workroam_core::crypto::sha256_hex(&stripped),
                    size: stripped.len() as u64,
                    ..f.meta.clone()
                };
                out.insert(
                    rel,
                    SyncFile {
                        meta,
                        abs: f.abs,
                        bytes: Some(stripped),
                    },
                );
            } else {
                out.insert(
                    rel,
                    SyncFile {
                        meta: f.meta,
                        abs: f.abs,
                        bytes: None,
                    },
                );
            }
        }
        Ok(out)
    }

    pub fn source_entries(files: &BTreeMap<String, SyncFile>) -> Vec<SourceEntry> {
        files
            .iter()
            .map(|(rel, f)| {
                let kind = match (&f.meta.symlink, &f.bytes) {
                    (Some(t), _) => EntryKind::Symlink { target: t.clone() },
                    (None, Some(b)) => EntryKind::File {
                        source: EntrySource::Bytes(b.clone()),
                        size: b.len() as u64,
                        exec: f.meta.exec,
                    },
                    (None, None) => EntryKind::File {
                        source: EntrySource::Path(f.abs.clone()),
                        size: f.meta.size,
                        exec: f.meta.exec,
                    },
                };
                SourceEntry {
                    path: rel.clone(),
                    kind,
                    mtime: None,
                }
            })
            .collect()
    }

    /// The machine block for AGENTS.md.
    pub fn env_info(&self) -> envblock::EnvInfo {
        let m = &self.ws.machine;
        let internal = m
            .reachable
            .iter()
            .any(|h| workroam_git::scan::looks_internal(h));
        let repos = self
            .cfg
            .repos
            .iter()
            .map(|(name, r)| {
                let dir = workroam_core::paths::from_snapshot_path(&self.dir, &r.path);
                let git = workroam_git::cmd::Git::new(&dir);
                let host = git
                    .remote_url(r.via.as_deref().unwrap_or("origin"))
                    .and_then(|u| workroam_git::cmd::remote_host(&u));
                let can_push = match (&host, r.mode) {
                    (_, RepoMode::Files) => false,
                    (Some(h), _) => m.reachable.iter().any(|x| x == h),
                    (None, _) => false,
                };
                envblock::RepoEnv {
                    name: name.clone(),
                    mode: r.mode.as_str().to_string(),
                    can_push,
                }
            })
            .collect();
        envblock::EnvInfo {
            device: m.name.clone(),
            os: m.os.display().to_string(),
            os_version: workroam_platform::sysinfo::os_version(),
            shell: workroam_platform::sysinfo::shell(),
            project_dir: self.dir.display().to_string(),
            internal_network: internal,
            read_only: self.ws.is_read_only(),
            repos,
        }
    }

    /// Regenerate machine files: machine block, MCP configs, Skills links, rule files.
    pub fn generate(&self) -> Result<generate::Generated> {
        let mcp = McpConfig::load(&self.dir)?;
        let agents = self.agents();
        let env = self.env_info();
        let secret = |name: &str| {
            workroam_platform::secrets::get(&workroam_platform::secrets::keys::secret(name))
                .ok()
                .flatten()
        };
        let enabled = self.cfg.mcp.enabled.unwrap_or(self.ws.machine.mcp.enabled);
        generate::generate(&generate::GenerateOptions {
            project_dir: &self.dir,
            project_name: &self.name,
            os: self.ws.machine.os,
            agents: &agents,
            mcp: &mcp,
            secret: &secret,
            workroam_mcp: enabled,
            env: Some(&env),
        })
    }

    pub fn is_empty_dir(&self) -> bool {
        workroam_platform::dircheck::is_empty_or_missing(&self.dir)
    }
}

pub fn rel_display(p: &Path) -> String {
    p.display().to_string()
}
