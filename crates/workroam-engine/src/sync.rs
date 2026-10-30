//! Push and pull (architecture §9.2 – §9.4).

use crate::local::{OpEntry, OpLog, SyncState, SyncingGuard};
use crate::merge::{self, MergeOutcome};
use crate::project::{Project, SyncFile};
use crate::workspace::blocking;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use workroam_core::model::{
    project_tag, ChangeSummary, DeviceRef, FileMeta, Head, HeadState, Manifest, RepoManifest,
    MANIFEST_PATH, TAG_WORKROAM,
};
use workroam_core::{Code, Error, Result};
use workroam_lease::{Acquire, LeaseStatus};
use workroam_platform::xp::XpProblem;
use workroam_snapshot::SourceEntry;

/// Progress callback: (phase, done, total).
pub type Progress = Arc<dyn Fn(&str, u64, Option<u64>) + Send + Sync>;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Merge,
    Conflict,
    KeepLocal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    pub kind: ChangeKind,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LeaseView {
    /// `free`, `mine`, `other`, `unsupported`.
    pub state: String,
    pub holder: Option<DeviceRef>,
    pub acquired_at: Option<DateTime<Utc>>,
    pub renewed_at: Option<DateTime<Utc>>,
    pub stale: bool,
}

pub async fn lease_view(p: &Project<'_>) -> Result<LeaseView> {
    if !p.ws.control.store.conditional {
        return Ok(LeaseView {
            state: "unsupported".into(),
            ..Default::default()
        });
    }
    Ok(
        match workroam_lease::status(&p.ws.control, &p.pid, &p.ws.machine.device_id).await? {
            LeaseStatus::Free => LeaseView {
                state: "free".into(),
                ..Default::default()
            },
            LeaseStatus::Mine(l) => LeaseView {
                state: "mine".into(),
                holder: Some(l.holder),
                acquired_at: Some(l.acquired_at),
                renewed_at: Some(l.renewed_at),
                stale: false,
            },
            LeaseStatus::Other { lease, stale } => LeaseView {
                state: "other".into(),
                holder: Some(lease.holder),
                acquired_at: Some(lease.acquired_at),
                renewed_at: Some(lease.renewed_at),
                stale,
            },
        },
    )
}

fn lease_error(view: &LeaseView) -> Error {
    let who = view
        .holder
        .as_ref()
        .map(|h| h.name.clone())
        .unwrap_or_default();
    let mut e = Error::new(Code::LeaseHeld, format!("{who} is working on this project")).with_hint(
        "ask that device to push with --release, wait with --wait, or take over with --takeover",
    );
    e.data = Some(serde_json::to_value(view).unwrap_or_default());
    e
}

/// Acquire (or confirm) the lease for a write. `takeover` forces it.
pub async fn take_lease(p: &Project<'_>, takeover: bool) -> Result<Option<DeviceRef>> {
    if p.ws.is_read_only() {
        return Ok(None);
    }
    match workroam_lease::acquire(&p.ws.control, &p.pid, &p.ws.device()).await? {
        Acquire::Acquired | Acquire::AlreadyMine | Acquire::Unsupported => Ok(None),
        Acquire::HeldByOther { lease, stale } => {
            if takeover {
                let prev = workroam_lease::takeover(&p.ws.control, &p.pid, &p.ws.device()).await?;
                Ok(prev.map(|l| l.holder))
            } else {
                Err(lease_error(&LeaseView {
                    state: "other".into(),
                    holder: Some(lease.holder),
                    acquired_at: Some(lease.acquired_at),
                    renewed_at: Some(lease.renewed_at),
                    stale,
                }))
            }
        }
    }
}

fn key(m: &FileMeta) -> String {
    format!(
        "{}:{}:{}",
        m.sha256,
        m.exec,
        m.symlink.as_deref().unwrap_or("")
    )
}

/// Local changes relative to the last synced state.
pub fn diff_local(
    base: &BTreeMap<String, FileMeta>,
    local: &BTreeMap<String, SyncFile>,
) -> (ChangeSummary, Vec<FileChange>) {
    let mut s = ChangeSummary::default();
    let mut v = Vec::new();
    for (path, f) in local {
        match base.get(path) {
            None => {
                s.added += 1;
                s.bytes += f.meta.size;
                v.push(FileChange {
                    path: path.clone(),
                    kind: ChangeKind::Added,
                    size: f.meta.size,
                });
            }
            Some(b) if key(b) != key(&f.meta) => {
                s.modified += 1;
                s.bytes += f.meta.size;
                v.push(FileChange {
                    path: path.clone(),
                    kind: ChangeKind::Modified,
                    size: f.meta.size,
                });
            }
            _ => {}
        }
    }
    for (path, b) in base {
        if !local.contains_key(path) {
            s.deleted += 1;
            v.push(FileChange {
                path: path.clone(),
                kind: ChangeKind::Deleted,
                size: b.size,
            });
        }
    }
    (s, v)
}

const LIST_LIMIT: usize = 500;

// ---------------------------------------------------------------- push --

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushPlan {
    pub project: String,
    pub pid: String,
    pub dir: String,
    pub base: Option<String>,
    pub head: Option<Head>,
    /// Another device pushed since this device's last sync.
    pub behind: bool,
    pub lease: LeaseView,
    pub summary: ChangeSummary,
    pub changes: Vec<FileChange>,
    pub problems: Vec<XpProblem>,
    pub conflict_copies: Vec<String>,
    pub git_in_progress: Vec<String>,
    pub repos: Vec<String>,
    pub handoff_updated: bool,
    pub nothing_to_push: bool,
    pub read_only: bool,
    pub archived: bool,
}

impl PushPlan {
    /// The reason this push cannot run, as the error it would end with.
    pub fn blocker(&self) -> Option<Error> {
        if self.read_only {
            return Some(Error::new(
                Code::ReadOnlyDevice,
                "this device is read-only and cannot push",
            ));
        }
        if self.archived {
            return Some(
                Error::new(
                    Code::Archived,
                    format!("project {} is archived", self.project),
                )
                .with_hint("restore it with workroam archive restore"),
            );
        }
        if let Some(r) = self.git_in_progress.first() {
            return Some(
                Error::new(
                    Code::GitInProgress,
                    format!("repository {r} has a rebase or merge in progress"),
                )
                .with_hint("finish or abort it first"),
            );
        }
        let blocking: Vec<&XpProblem> = self.problems.iter().filter(|p| p.blocking).collect();
        if !blocking.is_empty() {
            return Some(
                Error::new(
                    Code::CrossPlatform,
                    format!(
                        "{} file name(s) cannot be used on every system",
                        blocking.len()
                    ),
                )
                .with_hint("rename the listed files, then push again")
                .with_data(serde_json::to_value(&blocking).unwrap_or_default()),
            );
        }
        if !self.conflict_copies.is_empty() {
            return Some(
                Error::new(Code::Conflict, format!("{} conflict copies are waiting to be merged", self.conflict_copies.len()))
                    .with_hint("merge the *.conflict-* files into the originals, delete the copies, push again")
                    .with_data(serde_json::json!(self.conflict_copies)),
            );
        }
        if self.behind {
            let who = self
                .head
                .as_ref()
                .map(|h| h.device_name.clone())
                .unwrap_or_default();
            return Some(
                Error::new(Code::Conflict, format!("{who} pushed a newer snapshot"))
                    .with_hint("pull first, then push"),
            );
        }
        None
    }
}

pub async fn plan_push(p: &Project<'_>) -> Result<PushPlan> {
    p.ensure_bound_dir()?;
    let head = p.ws.head(&p.name).await?.map(|(h, _)| h);
    let state = p.state();
    let local = p.sync_view()?;
    let (summary, mut changes) = diff_local(&state.base_files, &local);
    let windows_root = if p.ws.machine.os == workroam_core::model::OsKind::Windows {
        p.dir.display().to_string().len()
    } else {
        0
    };
    let problems =
        workroam_platform::xp::check_paths(local.keys().map(|s| s.as_str()), windows_root);
    let conflict_copies: Vec<String> = local
        .keys()
        .filter(|k| merge::is_conflict_copy(k))
        .cloned()
        .collect();
    let mut git_in_progress = Vec::new();
    for (name, r) in &p.cfg.repos {
        let g =
            workroam_git::cmd::Git::new(workroam_core::paths::from_snapshot_path(&p.dir, &r.path));
        if workroam_git::cmd::is_repo(&g.dir) && g.in_progress().is_some() {
            git_in_progress.push(name.clone());
        }
    }
    let behind = match (&head, &state.base_snapshot) {
        (Some(h), base) => h.snapshot.is_some() && h.snapshot != *base,
        (None, _) => false,
    };
    let handoff = std::fs::read(workroam_adapters::handoff::path(&p.dir))
        .ok()
        .map(|b| workroam_core::crypto::sha256_hex(&b));
    let handoff_updated = handoff.is_some() && handoff != state.handoff_sha;
    let nothing = summary.is_empty() && p.cfg.repos.is_empty() && state.base_snapshot.is_some();
    changes.truncate(LIST_LIMIT);
    Ok(PushPlan {
        project: p.name.clone(),
        pid: p.pid.clone(),
        dir: p.dir.display().to_string(),
        base: state.base_snapshot.clone(),
        archived: head
            .as_ref()
            .map(|h| h.state == HeadState::Archived)
            .unwrap_or(false),
        head,
        behind,
        lease: lease_view(p).await?,
        summary,
        changes,
        problems,
        conflict_copies,
        git_in_progress,
        repos: p.cfg.repos.keys().cloned().collect(),
        handoff_updated,
        nothing_to_push: nothing,
        read_only: p.ws.is_read_only(),
    })
}

#[derive(Clone, Default)]
pub struct PushOptions {
    pub release: bool,
    pub takeover: bool,
    pub note: String,
    pub progress: Option<Progress>,
    /// Snapshot written by a restore (`restored_from`).
    pub restored_from: Option<String>,
    /// Write even when nothing changed.
    pub force: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PushOutcome {
    pub project: String,
    pub snapshot: Option<String>,
    pub summary: ChangeSummary,
    pub skipped: bool,
    pub released: bool,
    pub took_over_from: Option<DeviceRef>,
    pub notes: Vec<String>,
    pub duration_ms: u64,
}

pub struct Collected {
    pub entries: Vec<SourceEntry>,
    pub manifest: Manifest,
    pub repos_sha: String,
    pub handoff_sha: Option<String>,
}

/// Everything that goes into a snapshot (files, git, sessions, manifest).
pub fn collect(
    p: &Project<'_>,
    local: &BTreeMap<String, SyncFile>,
    archive: bool,
    parent: Option<String>,
) -> Result<Collected> {
    let mut entries = Project::source_entries(local);
    let stage = crate::local::local_dir(&p.dir).join("stage");
    let mut repos = BTreeMap::new();
    let mut commits = 0;
    for (name, cfg) in &p.cfg.repos {
        let ex = blocking(|| {
            workroam_git::sync::export(&workroam_git::sync::ExportOptions {
                name,
                project: &p.dir,
                cfg,
                stage: &stage,
                archive,
            })
        })?;
        commits += ex.manifest.commits;
        for (snap_path, file) in ex.files {
            let size = std::fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
            entries.push(SourceEntry {
                path: snap_path,
                kind: workroam_snapshot::EntryKind::File {
                    source: workroam_snapshot::EntrySource::Path(file),
                    size,
                    exec: false,
                },
                mtime: None,
            });
        }
        repos.insert(name.clone(), ex.manifest);
    }
    let mut sessions = BTreeMap::new();
    if p.cfg.sync.sessions.unwrap_or(true) {
        for a in p.agents() {
            let files = workroam_adapters::sessions::collect(&a, &p.dir);
            if files.is_empty() {
                continue;
            }
            let dir = format!("{}/{}", workroam_adapters::sessions::SNAPSHOT_DIR, a.id);
            for (rel, abs) in files {
                let size = std::fs::metadata(&abs).map(|m| m.len()).unwrap_or(0);
                entries.push(SourceEntry {
                    path: format!("{dir}/{rel}"),
                    kind: workroam_snapshot::EntryKind::File {
                        source: workroam_snapshot::EntrySource::Path(abs),
                        size,
                        exec: false,
                    },
                    mtime: None,
                });
            }
            sessions.insert(a.id.clone(), dir);
        }
    }
    let state = p.state();
    let (mut summary, _) = diff_local(&state.base_files, local);
    summary.commits = commits;
    summary.repos = repos.keys().cloned().collect();
    let handoff_sha = local
        .get(workroam_adapters::handoff::PATH)
        .map(|f| f.meta.sha256.clone());
    let manifest = Manifest {
        format: workroam_core::MANIFEST_FORMAT,
        project: p.pid.clone(),
        name: p.name.clone(),
        parent,
        restored_from: None,
        device: p.ws.device(),
        device_path: p.dir.display().to_string(),
        created_at: Utc::now(),
        handoff_updated: handoff_sha.is_some() && handoff_sha != state.handoff_sha,
        archived: archive,
        note: String::new(),
        files: local
            .iter()
            .map(|(k, f)| (k.clone(), f.meta.clone()))
            .collect(),
        repos: repos.clone(),
        sessions,
        summary,
    };
    let repos_sha =
        workroam_core::crypto::sha256_hex(&serde_json::to_vec(&strip_volatile(&repos))?);
    Ok(Collected {
        entries,
        manifest,
        repos_sha,
        handoff_sha,
    })
}

/// Repository manifests without per-push counters, to detect git-only changes.
fn strip_volatile(repos: &BTreeMap<String, RepoManifest>) -> BTreeMap<String, RepoManifest> {
    repos
        .iter()
        .map(|(k, r)| {
            (
                k.clone(),
                RepoManifest {
                    commits: 0,
                    ..r.clone()
                },
            )
        })
        .collect()
}

/// Snapshot description: a small JSON summary for fast history listing.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Description {
    pub name: String,
    pub device: DeviceRef,
    pub summary: ChangeSummary,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub restored_from: Option<String>,
    #[serde(default)]
    pub handoff_updated: bool,
}

pub fn write_snapshot(
    p: &Project<'_>,
    mut c: Collected,
    note: &str,
    restored_from: Option<String>,
    extra_tags: &[String],
    progress: Option<Progress>,
) -> Result<(String, Manifest)> {
    c.manifest.note = note.to_string();
    c.manifest.restored_from = restored_from.clone();
    let manifest_bytes = serde_json::to_vec_pretty(&c.manifest)?;
    c.entries
        .push(SourceEntry::bytes(MANIFEST_PATH, manifest_bytes));
    let mut tags = vec![TAG_WORKROAM.to_string(), project_tag(&p.pid)];
    tags.extend(extra_tags.iter().cloned());
    let desc = serde_json::to_string(&Description {
        name: p.name.clone(),
        device: p.ws.device(),
        summary: c.manifest.summary.clone(),
        note: note.to_string(),
        restored_from,
        handoff_updated: c.manifest.handoff_updated,
    })?;
    let prog = progress
        .map(|f| -> workroam_snapshot::ProgressFn { Arc::new(move |d, t| f("upload", d, t)) });
    let parent = c.manifest.parent.clone();
    let id = blocking(|| {
        p.ws.snap.backup(
            c.entries,
            &tags,
            parent.as_deref(),
            &p.ws.machine.name,
            &p.pid,
            &desc,
            prog,
        )
    })?;
    Ok((id, c.manifest))
}

pub async fn push(p: &Project<'_>, o: &PushOptions) -> Result<PushOutcome> {
    let started = std::time::Instant::now();
    p.ws.ensure_writable()?;
    let plan = plan_push(p).await?;
    if let Some(e) = plan.blocker() {
        return Err(e);
    }
    let took_over_from = take_lease(p, o.takeover).await?;
    let _sleep = workroam_platform::inhibit::SleepGuard::acquire("Workroam is pushing");
    let head = p.ws.head(&p.name).await?;
    let state = p.state();
    if let Some((h, _)) = &head {
        workroam_lease::ensure_active(h)?;
        if h.snapshot.is_some() && h.snapshot != state.base_snapshot {
            return Err(Error::new(
                Code::Conflict,
                format!("{} pushed a newer snapshot", h.device_name),
            )
            .with_hint("pull first, then push"));
        }
    }
    let local = p.sync_view()?;
    if let Some(f) = &o.progress {
        f("prepare", 0, None);
    }
    let collected = collect(p, &local, false, state.base_snapshot.clone())?;
    let mut out = PushOutcome {
        project: p.name.clone(),
        took_over_from,
        ..Default::default()
    };
    let unchanged = collected.manifest.summary.is_empty()
        && state.base_snapshot.is_some()
        && state.repos_sha.as_deref() == Some(collected.repos_sha.as_str())
        && !o.force;
    if !unchanged {
        let repos_sha = collected.repos_sha.clone();
        let handoff_sha = collected.handoff_sha.clone();
        let (id, manifest) = write_snapshot(
            p,
            collected,
            &o.note,
            o.restored_from.clone(),
            &[],
            o.progress.clone(),
        )?;
        let new_head = Head {
            project: p.name.clone(),
            snapshot: Some(id.clone()),
            parent: state.base_snapshot.clone(),
            device_id: p.ws.machine.device_id.clone(),
            device_name: p.ws.machine.name.clone(),
            created_at: Utc::now(),
            state: HeadState::Active,
            summary: Some(manifest.summary.clone()),
        };
        let expected = head.as_ref().map(|(_, e)| e.as_str());
        if !workroam_lease::write_head(&p.ws.control, &p.pid, &new_head, expected).await? {
            return Err(
                Error::new(Code::Conflict, "another device pushed at the same time")
                    .with_hint("pull first, then push"),
            );
        }
        let mut st = state.clone();
        st.base_snapshot = Some(id.clone());
        st.base_files = manifest.files.clone();
        st.last_push = Some(Utc::now());
        st.handoff_sha = handoff_sha;
        st.repos_sha = Some(repos_sha);
        st.save(&p.dir)?;
        p.bind()?;
        out.snapshot = Some(id);
        out.summary = manifest.summary;
    } else {
        out.skipped = true;
        out.snapshot = state.base_snapshot.clone();
    }
    if o.release {
        out.released = workroam_lease::release(&p.ws.control, &p.pid, &p.ws.device()).await?;
    }
    let _ = workroam_device::registry::touch(&p.ws.control, &p.ws.machine.device_id, true).await;
    out.duration_ms = started.elapsed().as_millis() as u64;
    Ok(out)
}

// ---------------------------------------------------------------- pull --

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullPlan {
    pub project: String,
    pub pid: String,
    pub dir: String,
    pub head: Option<Head>,
    pub target: Option<String>,
    pub base: Option<String>,
    pub up_to_date: bool,
    pub lease: LeaseView,
    pub summary: ChangeSummary,
    pub changes: Vec<FileChange>,
    /// Unsynced local changes that stay or are merged.
    pub local_changes: u64,
    /// Some local change would be merged or kept as a conflict copy.
    pub touches_local: bool,
    pub occupied: bool,
    pub archived: bool,
    pub repos: Vec<String>,
}

impl PullPlan {
    pub fn blocker(&self) -> Option<Error> {
        if self.archived {
            return Some(
                Error::new(
                    Code::Archived,
                    format!("project {} is archived", self.project),
                )
                .with_hint("restore it with workroam archive restore"),
            );
        }
        if self.occupied {
            return Some(
                Error::new(
                    Code::InvalidArgs,
                    format!("{} already holds other content", self.dir),
                )
                .with_hint("choose an empty directory or the project's own folder"),
            );
        }
        None
    }
}

#[derive(Debug, Clone)]
enum Action {
    Write {
        path: String,
        meta: FileMeta,
        expect: Option<String>,
    },
    Delete {
        path: String,
        expect: String,
    },
    Merge {
        path: String,
        meta: FileMeta,
        expect: String,
    },
    Conflict {
        path: String,
        meta: FileMeta,
    },
    KeepLocal {
        path: String,
    },
}

fn actions(
    local: &BTreeMap<String, SyncFile>,
    base: &BTreeMap<String, FileMeta>,
    target: &BTreeMap<String, FileMeta>,
) -> Vec<Action> {
    let mut paths: BTreeSet<&String> = local.keys().collect();
    paths.extend(base.keys());
    paths.extend(target.keys());
    let mut out = Vec::new();
    for path in paths {
        let l = local.get(path).map(|f| key(&f.meta));
        let b = base.get(path).map(key);
        let t = target.get(path).map(key);
        if l == t {
            continue;
        }
        let local_sha = local.get(path).map(|f| {
            if f.bytes.is_some() {
                String::new()
            } else {
                f.meta.sha256.clone()
            }
        });
        if t == b {
            if l.is_some() || b.is_some() {
                out.push(Action::KeepLocal { path: path.clone() });
            }
            continue;
        }
        if l == b {
            match target.get(path) {
                Some(m) => out.push(Action::Write {
                    path: path.clone(),
                    meta: m.clone(),
                    expect: Some(local_sha.unwrap_or_default()),
                }),
                None => out.push(Action::Delete {
                    path: path.clone(),
                    expect: local_sha.unwrap_or_default(),
                }),
            }
            continue;
        }
        match (local.get(path), target.get(path)) {
            (Some(_), None) => out.push(Action::KeepLocal { path: path.clone() }),
            (None, Some(m)) => out.push(Action::Write {
                path: path.clone(),
                meta: m.clone(),
                expect: Some(String::new()),
            }),
            (Some(_), Some(m)) => {
                if b.is_some() && m.symlink.is_none() {
                    out.push(Action::Merge {
                        path: path.clone(),
                        meta: m.clone(),
                        expect: local_sha.unwrap_or_default(),
                    })
                } else {
                    out.push(Action::Conflict {
                        path: path.clone(),
                        meta: m.clone(),
                    })
                }
            }
            (None, None) => {}
        }
    }
    out
}

fn summarize(
    acts: &[Action],
    local: &BTreeMap<String, SyncFile>,
) -> (ChangeSummary, Vec<FileChange>, u64, bool) {
    let mut s = ChangeSummary::default();
    let mut v = Vec::new();
    let mut local_changes = 0;
    let mut touches = false;
    for a in acts {
        match a {
            Action::Write { path, meta, .. } => {
                if local.contains_key(path) {
                    s.modified += 1;
                    v.push(FileChange {
                        path: path.clone(),
                        kind: ChangeKind::Modified,
                        size: meta.size,
                    });
                } else {
                    s.added += 1;
                    v.push(FileChange {
                        path: path.clone(),
                        kind: ChangeKind::Added,
                        size: meta.size,
                    });
                }
                s.bytes += meta.size;
            }
            Action::Delete { path, .. } => {
                s.deleted += 1;
                v.push(FileChange {
                    path: path.clone(),
                    kind: ChangeKind::Deleted,
                    size: 0,
                });
            }
            Action::Merge { path, meta, .. } => {
                s.modified += 1;
                s.bytes += meta.size;
                touches = true;
                local_changes += 1;
                v.push(FileChange {
                    path: path.clone(),
                    kind: ChangeKind::Merge,
                    size: meta.size,
                });
            }
            Action::Conflict { path, meta } => {
                s.conflicts += 1;
                touches = true;
                local_changes += 1;
                v.push(FileChange {
                    path: path.clone(),
                    kind: ChangeKind::Conflict,
                    size: meta.size,
                });
            }
            Action::KeepLocal { path } => {
                local_changes += 1;
                v.push(FileChange {
                    path: path.clone(),
                    kind: ChangeKind::KeepLocal,
                    size: 0,
                });
            }
        }
    }
    (s, v, local_changes, touches)
}

pub fn read_manifest(p: &Project<'_>, snapshot: &str) -> Result<Manifest> {
    let raw = blocking(|| p.ws.snap.read_file(snapshot, MANIFEST_PATH))?.ok_or_else(|| {
        Error::new(
            Code::Corrupted,
            format!("snapshot {snapshot} has no Workroam manifest"),
        )
    })?;
    Ok(serde_json::from_slice(&raw)?)
}

fn occupied(p: &Project<'_>) -> bool {
    if p.is_empty_dir() {
        return false;
    }
    match crate::local::Binding::load(&p.dir) {
        Some(b) => b.project_id != p.pid,
        None => {
            // A folder with this project's own configuration can be re-attached.
            let own = workroam_core::config::ProjectConfig::load(&p.dir)
                .ok()
                .flatten()
                .map(|c| c.name == p.name)
                .unwrap_or(false);
            !own
        }
    }
}

pub async fn plan_pull(p: &Project<'_>, target: Option<&str>) -> Result<PullPlan> {
    p.ensure_bound_dir()?;
    let head = p.ws.head(&p.name).await?.map(|(h, _)| h);
    let state = p.state();
    let target = target
        .map(String::from)
        .or_else(|| head.as_ref().and_then(|h| h.snapshot.clone()));
    let archived = head
        .as_ref()
        .map(|h| h.state == HeadState::Archived)
        .unwrap_or(false);
    let mut plan = PullPlan {
        project: p.name.clone(),
        pid: p.pid.clone(),
        dir: p.dir.display().to_string(),
        head,
        target: target.clone(),
        base: state.base_snapshot.clone(),
        up_to_date: target.is_none() || target == state.base_snapshot,
        lease: lease_view(p).await?,
        summary: ChangeSummary::default(),
        changes: vec![],
        local_changes: 0,
        touches_local: false,
        occupied: occupied(p),
        archived,
        repos: vec![],
    };
    if plan.up_to_date || plan.occupied {
        if plan.up_to_date && !plan.occupied && p.dir.exists() {
            let local = p.sync_view()?;
            plan.local_changes = diff_local(&state.base_files, &local).0.files();
        }
        return Ok(plan);
    }
    let t = target.unwrap();
    let man = read_manifest(p, &t)?;
    let local = if p.dir.exists() {
        p.sync_view()?
    } else {
        BTreeMap::new()
    };
    let acts = actions(&local, &state.base_files, &man.files);
    let (s, mut v, lc, touches) = summarize(&acts, &local);
    plan.summary = s;
    plan.summary.commits = man.summary.commits;
    plan.summary.repos = man.repos.keys().cloned().collect();
    v.truncate(LIST_LIMIT);
    plan.changes = v;
    plan.local_changes = lc;
    plan.touches_local = touches;
    plan.repos = man.repos.keys().cloned().collect();
    Ok(plan)
}

#[derive(Clone, Default)]
pub struct PullOptions {
    pub takeover: bool,
    /// Pull this snapshot instead of the head (archive restore, restore).
    pub target: Option<String>,
    pub progress: Option<Progress>,
    /// Restore of an archive: never move existing branches.
    pub archive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PullOutcome {
    pub project: String,
    pub snapshot: Option<String>,
    pub summary: ChangeSummary,
    pub up_to_date: bool,
    pub conflicts: Vec<String>,
    pub merged: Vec<String>,
    pub kept_local: Vec<String>,
    pub notes: Vec<String>,
    pub warnings: Vec<String>,
    pub took_over_from: Option<DeviceRef>,
    pub from_device: Option<DeviceRef>,
    pub handoff_updated: bool,
    pub duration_ms: u64,
}

pub async fn pull(p: &mut Project<'_>, o: &PullOptions) -> Result<PullOutcome> {
    let started = std::time::Instant::now();
    let plan = plan_pull(p, o.target.as_deref()).await?;
    if let Some(e) = plan.blocker() {
        return Err(e);
    }
    let mut out = PullOutcome {
        project: p.name.clone(),
        ..Default::default()
    };
    out.took_over_from = take_lease(p, o.takeover).await?;
    if plan.up_to_date {
        out.up_to_date = true;
        out.snapshot = plan.target.clone();
        if plan.target.is_some() && !p.dir.join(".workroam/local/binding").exists() {
            p.bind()?;
        }
        return Ok(out);
    }
    let target = plan.target.clone().unwrap();
    apply_snapshot(p, &target, o, &mut out).await?;
    out.duration_ms = started.elapsed().as_millis() as u64;
    let _ = workroam_device::registry::touch(&p.ws.control, &p.ws.machine.device_id, true).await;
    Ok(out)
}

/// Bring the directory to `target` (merging local changes), then regenerate machine files.
pub async fn apply_snapshot(
    p: &mut Project<'_>,
    target: &str,
    o: &PullOptions,
    out: &mut PullOutcome,
) -> Result<()> {
    std::fs::create_dir_all(&p.dir)?;
    let _syncing = SyncingGuard::new(&p.dir, "pull")?;
    let _sleep = workroam_platform::inhibit::SleepGuard::acquire("Workroam is pulling");
    let oplog = OpLog::new(&p.dir);
    oplog.begin(target, "pull")?;
    let state = p.state();
    let man = read_manifest(p, target)?;
    out.from_device = Some(man.device.clone());
    out.handoff_updated = man.handoff_updated;
    let local = p.sync_view()?;
    let acts = actions(&local, &state.base_files, &man.files);
    let (summary, _, _, _) = summarize(&acts, &local);
    out.summary = summary;
    out.summary.commits = man.summary.commits;
    out.summary.repos = man.repos.keys().cloned().collect();

    let mut wanted: BTreeMap<String, Action> = BTreeMap::new();
    for a in &acts {
        match a {
            Action::Write { path, .. }
            | Action::Merge { path, .. }
            | Action::Conflict { path, .. } => {
                wanted.insert(path.clone(), a.clone());
            }
            Action::KeepLocal { path } => out.kept_local.push(path.clone()),
            Action::Delete { .. } => {}
        }
    }
    let dir = p.dir.clone();
    let device_name = man.device.name.clone();
    let base_snapshot = state.base_snapshot.clone();
    let total = wanted.len() as u64;
    let mut done = 0u64;
    let mut errors: Vec<String> = Vec::new();
    let paths: Vec<String> = wanted.keys().cloned().collect();
    {
        let snap = p.ws.snap.clone();
        let mut each = |path: &str, data: Vec<u8>| -> Result<()> {
            done += 1;
            if let Some(f) = &o.progress {
                f("download", done, Some(total));
            }
            let Some(a) = wanted.get(path) else {
                return Ok(());
            };
            let abs = workroam_core::paths::from_snapshot_path(&dir, path);
            let r: Result<()> = (|| {
                match a {
                    Action::Write { meta, expect, .. } => {
                        write_entry(
                            &abs,
                            &dir,
                            meta,
                            &data,
                            expect.as_deref().filter(|e| !e.is_empty() || !abs.exists()),
                        )?;
                    }
                    Action::Merge { meta, expect, .. } => {
                        let base = match &base_snapshot {
                            Some(b) => snap.read_file(b, path)?.unwrap_or_default(),
                            None => Vec::new(),
                        };
                        let ours = std::fs::read(&abs)?;
                        let ours_view = if path == "AGENTS.md" {
                            workroam_adapters::envblock::strip(&String::from_utf8_lossy(&ours))
                                .into_bytes()
                        } else {
                            ours
                        };
                        match merge::merge3(&base, &ours_view, &data) {
                            MergeOutcome::Clean(m) => {
                                workroam_platform::fsutil::replace_file(
                                    &abs,
                                    &m,
                                    meta.exec,
                                    Some(expect).filter(|e| !e.is_empty()).map(|s| s.as_str()),
                                )?;
                                out.merged.push(path.to_string());
                            }
                            MergeOutcome::Conflict => {
                                let cp = merge::conflict_path(path, &device_name, &|c| {
                                    dir.join(c).exists()
                                });
                                write_entry(
                                    &workroam_core::paths::from_snapshot_path(&dir, &cp),
                                    &dir,
                                    meta,
                                    &data,
                                    None,
                                )?;
                                out.conflicts.push(cp);
                            }
                        }
                    }
                    Action::Conflict { meta, .. } => {
                        let cp =
                            merge::conflict_path(path, &device_name, &|c| dir.join(c).exists());
                        write_entry(
                            &workroam_core::paths::from_snapshot_path(&dir, &cp),
                            &dir,
                            meta,
                            &data,
                            None,
                        )?;
                        out.conflicts.push(cp);
                    }
                    _ => {}
                }
                Ok(())
            })();
            match r {
                Ok(()) => oplog.done(path)?,
                Err(e) => errors.push(format!("{path}: {}", e.message)),
            }
            Ok(())
        };
        blocking(|| snap.read_files(target, &paths, &mut each))?;
    }
    // Symlinks have no content to read.
    for (path, a) in &wanted {
        if let Action::Write { meta, .. } = a {
            if meta.symlink.is_some()
                && !oplog
                    .read()
                    .iter()
                    .any(|e| matches!(e, OpEntry::Done { path: x } if x == path))
            {
                let abs = workroam_core::paths::from_snapshot_path(&dir, path);
                if let Err(e) = write_entry(&abs, &dir, meta, &[], None) {
                    errors.push(format!("{path}: {}", e.message));
                }
            }
        }
    }
    for a in &acts {
        if let Action::Delete { path, expect } = a {
            let abs = workroam_core::paths::from_snapshot_path(&dir, path);
            let current = workroam_platform::fsutil::file_sha256(&abs).ok().flatten();
            let ok = expect.is_empty()
                || current.as_deref() == Some(expect.as_str())
                || path == "AGENTS.md";
            if ok {
                if let Err(e) = workroam_platform::fsutil::remove_retry(&abs) {
                    if e.kind() != std::io::ErrorKind::NotFound {
                        errors.push(format!("{path}: {e}"));
                    }
                }
                if let Some(parent) = abs.parent() {
                    workroam_platform::fsutil::prune_empty_dirs(parent, &dir);
                }
            }
            oplog.done(path)?;
        }
    }
    if !errors.is_empty() {
        return Err(Error::new(
            Code::Network,
            format!("{} file(s) could not be written", errors.len()),
        )
        .with_hint("close programs that hold these files and pull again")
        .with_data(serde_json::json!(errors)));
    }

    // Configuration may have changed with the pull.
    p.reload()?;
    import_repos(p, target, &man, o.archive, out)?;
    restore_sessions(p, target, &man, out);
    match p.generate() {
        Ok(g) => out.warnings.extend(g.warnings),
        Err(e) => out.warnings.push(format!("generate: {}", e.message)),
    }

    let mut st = SyncState::load(&p.dir);
    st.base_snapshot = Some(target.to_string());
    st.base_files = man.files.clone();
    st.last_pull = Some(Utc::now());
    st.handoff_sha = man
        .files
        .get(workroam_adapters::handoff::PATH)
        .map(|m| m.sha256.clone());
    st.repos_sha = Some(workroam_core::crypto::sha256_hex(&serde_json::to_vec(
        &strip_volatile(&man.repos),
    )?));
    st.save(&p.dir)?;
    p.bind()?;
    oplog.append(&OpEntry::Finish)?;
    oplog.clear();
    out.snapshot = Some(target.to_string());
    Ok(())
}

pub(crate) fn write_entry(
    abs: &std::path::Path,
    root: &std::path::Path,
    meta: &FileMeta,
    data: &[u8],
    expect: Option<&str>,
) -> Result<()> {
    if let Some(t) = &meta.symlink {
        if let Ok(m) = std::fs::symlink_metadata(abs) {
            if m.file_type().is_symlink() || m.is_file() {
                workroam_platform::fsutil::remove_retry(abs)?;
            }
        }
        workroam_platform::fsutil::create_link(abs, t, root)?;
        return Ok(());
    }
    workroam_platform::fsutil::replace_file(abs, data, meta.exec, expect)?;
    Ok(())
}

pub(crate) fn import_repos(
    p: &Project<'_>,
    target: &str,
    man: &Manifest,
    archive: bool,
    out: &mut PullOutcome,
) -> Result<()> {
    if man.repos.is_empty() {
        return Ok(());
    }
    let stage = crate::local::local_dir(&p.dir).join("stage").join("in");
    for (name, rm) in &man.repos {
        let rdir = stage.join(name);
        std::fs::create_dir_all(&rdir)?;
        let fetch = |sp: &Option<String>, file: &str| -> Result<Option<std::path::PathBuf>> {
            let Some(sp) = sp else { return Ok(None) };
            let Some(data) = blocking(|| p.ws.snap.read_file(target, sp))? else {
                return Ok(None);
            };
            let dst = rdir.join(file);
            std::fs::write(&dst, data)?;
            Ok(Some(dst))
        };
        let base = fetch(&rm.base_bundle, "base.bundle")?;
        let incr = fetch(&rm.bundle, "incr.bundle")?;
        let map = fetch(&rm.rebuild_map, "map.json")?
            .and_then(|f| std::fs::read(f).ok())
            .and_then(|b| serde_json::from_slice::<workroam_git::sync::RebuildMap>(&b).ok());
        let res = blocking(|| {
            workroam_git::sync::import(&workroam_git::sync::ImportOptions {
                name,
                project: &p.dir,
                man: rm,
                base_bundle: base.as_deref(),
                incr_bundle: incr.as_deref(),
                map,
                archive,
            })
        });
        match res {
            Ok(r) => {
                out.notes
                    .extend(r.notes.into_iter().map(|n| format!("{name}: {n}")));
                out.conflicts
                    .extend(r.conflicts.into_iter().map(|c| format!("{name}: {c}")));
            }
            Err(e) if e.code == Code::GitInProgress => return Err(e),
            Err(e) => out.warnings.push(format!("{name}: {}", e.message)),
        }
        let _ = std::fs::remove_dir_all(&rdir);
    }
    Ok(())
}

fn restore_sessions(p: &Project<'_>, target: &str, man: &Manifest, out: &mut PullOutcome) {
    if man.sessions.is_empty() || !p.cfg.sync.sessions.unwrap_or(true) {
        return;
    }
    let tree = match blocking(|| p.ws.snap.tree(target)) {
        Ok(t) => t,
        Err(e) => {
            out.warnings.push(format!("sessions: {}", e.message));
            return;
        }
    };
    for (agent, dir) in &man.sessions {
        let Some(a) = workroam_adapters::by_id(agent) else {
            continue;
        };
        let prefix = format!("{dir}/");
        let paths: Vec<String> = tree
            .iter()
            .filter(|e| {
                e.path.starts_with(&prefix) && !matches!(e.kind, workroam_snapshot::NodeKind::Dir)
            })
            .map(|e| e.path.clone())
            .collect();
        let mut files = Vec::new();
        let r = blocking(|| {
            p.ws.snap.read_files(target, &paths, &mut |path, data| {
                files.push((path[prefix.len()..].to_string(), data));
                Ok(())
            })
        });
        if let Err(e) = r {
            out.warnings
                .push(format!("sessions {agent}: {}", e.message));
            continue;
        }
        out.warnings.extend(workroam_adapters::sessions::restore(
            &a,
            &files,
            &man.device_path,
            &p.dir,
        ));
    }
}
