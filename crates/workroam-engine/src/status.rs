//! `workroam status`: lease, latest snapshot, unsynced changes, repositories.

use crate::project::Project;
use crate::sync::{diff_local, lease_view, LeaseView};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use workroam_core::model::{ChangeSummary, Head, HeadState};
use workroam_core::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoStatus {
    pub name: String,
    pub path: String,
    pub mode: String,
    pub history: String,
    pub branch: Option<String>,
    pub dirty: bool,
    pub can_push: bool,
    pub exists: bool,
    pub in_progress: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectStatus {
    pub project: String,
    pub pid: String,
    pub dir: String,
    pub binding: String,
    pub dir_exists: bool,
    pub lease: LeaseView,
    pub head: Option<Head>,
    pub base: Option<String>,
    /// A newer snapshot is waiting to be pulled.
    pub incoming: bool,
    pub local_changes: ChangeSummary,
    pub last_push: Option<DateTime<Utc>>,
    pub last_pull: Option<DateTime<Utc>>,
    pub archived: bool,
    pub syncing: bool,
    pub handoff: workroam_adapters::handoff::Handoff,
    pub repos: Vec<RepoStatus>,
    pub agents: Vec<String>,
    pub auto: String,
}

pub async fn project_status(p: &Project<'_>) -> Result<ProjectStatus> {
    let head = p.ws.head(&p.name).await?.map(|(h, _)| h);
    let state = p.state();
    let dir_exists = p.dir.exists();
    let local_changes = if dir_exists && !p.is_empty_dir() {
        diff_local(&state.base_files, &p.sync_view()?).0
    } else {
        ChangeSummary::default()
    };
    let env = p.env_info();
    let repos = p
        .cfg
        .repos
        .iter()
        .map(|(name, r)| {
            let dir = workroam_core::paths::from_snapshot_path(&p.dir, &r.path);
            let exists = workroam_git::cmd::is_repo(&dir);
            let g = workroam_git::cmd::Git::new(&dir);
            RepoStatus {
                name: name.clone(),
                path: r.path.clone(),
                mode: r.mode.as_str().into(),
                history: r.history.clone(),
                branch: if exists { g.current_branch() } else { None },
                dirty: exists && g.is_dirty(),
                can_push: env.repos.iter().any(|e| &e.name == name && e.can_push),
                exists,
                in_progress: if exists {
                    g.in_progress().map(|s| s.to_string())
                } else {
                    None
                },
            }
        })
        .collect();
    let auto = p.cfg.sync.auto.unwrap_or(p.ws.machine.sync.auto);
    Ok(ProjectStatus {
        project: p.name.clone(),
        pid: p.pid.clone(),
        dir: p.dir.display().to_string(),
        binding: p.binding_state().as_str().into(),
        dir_exists,
        lease: lease_view(p).await?,
        incoming: head.as_ref().and_then(|h| h.snapshot.clone()).is_some()
            && head.as_ref().and_then(|h| h.snapshot.clone()) != state.base_snapshot,
        archived: head
            .as_ref()
            .map(|h| h.state == HeadState::Archived)
            .unwrap_or(false),
        head,
        base: state.base_snapshot.clone(),
        local_changes,
        last_push: state.last_push,
        last_pull: state.last_pull,
        syncing: p.dir.join(crate::local::SYNCING).exists(),
        handoff: workroam_adapters::handoff::read(&p.dir),
        repos,
        agents: p.agents().into_iter().map(|a| a.id).collect(),
        auto: auto.as_str().into(),
    })
}
