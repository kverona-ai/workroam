//! Archive a project, list, restore, extract and delete archives
//! (architecture §10.2, §10.3).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use workroam_core::config::{BindingState, MachineConfig, ProjectConfig};
use workroam_core::model::{
    project_tag, ArchiveRecord, FileMeta, Head, HeadState, Manifest, TAG_ARCHIVED,
};
use workroam_core::{Code, Error, Result};
use workroam_engine::local::{Binding, SyncState};
use workroam_engine::restore::{self as engine_restore, RestoreOutcome, RestoreTarget};
use workroam_engine::sync::{
    self, apply_snapshot, collect, diff_local, push, read_manifest, take_lease, write_snapshot,
    LeaseView, Progress, PullOptions, PullOutcome, PushOptions,
};
use workroam_engine::{blocking, Project, Workspace};
use workroam_store::control::keys;

fn meta_key(m: &FileMeta) -> (&str, bool, Option<&str>) {
    (m.sha256.as_str(), m.exec, m.symlink.as_deref())
}

fn same_files(a: &BTreeMap<String, FileMeta>, b: &BTreeMap<String, FileMeta>) -> bool {
    a.len() == b.len()
        && a.iter()
            .all(|(k, m)| b.get(k).map(|o| meta_key(o) == meta_key(m)) == Some(true))
}

/// Paths whose content differs between two file sets (either side missing counts).
fn differing(a: &BTreeMap<String, FileMeta>, b: &BTreeMap<String, FileMeta>) -> Vec<String> {
    let keys: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
    keys.into_iter()
        .filter(|k| a.get(*k).map(meta_key) != b.get(*k).map(meta_key))
        .cloned()
        .collect()
}

async fn record(ws: &Workspace, pid: &str) -> Result<Option<ArchiveRecord>> {
    Ok(ws
        .control
        .get::<ArchiveRecord>(&keys::archive(pid))
        .await?
        .map(|(a, _)| a))
}

async fn require_record(ws: &Workspace, name: &str) -> Result<(String, ArchiveRecord)> {
    let pid = ws.project_id(name);
    match record(ws, &pid).await? {
        Some(r) => Ok((pid, r)),
        None => Err(Error::invalid(format!("project {name} is not archived"))
            .with_hint("workroam archive list shows the archived projects")),
    }
}

/// Update this device's binding after an archive operation.
pub fn mark_local(machine: &mut MachineConfig, name: &str, state: BindingState) {
    if let Some(b) = machine.projects.get_mut(name) {
        b.state = state;
    }
}

// ---------------------------------------------------------------- create --

#[derive(Clone, Default)]
pub struct ArchiveOptions {
    pub note: String,
    pub keep_history: bool,
    pub takeover: bool,
    pub progress: Option<Progress>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchivePlan {
    pub project: String,
    pub pid: String,
    pub dir: String,
    pub files: usize,
    pub size: u64,
    pub repos: Vec<String>,
    /// Local changes that are pushed first.
    pub unsynced: u64,
    pub lease: LeaseView,
    /// Other registered devices; they stop syncing the project.
    pub devices: Vec<String>,
    pub keep_history: bool,
    pub note: String,
    /// Why the archive cannot be created now, if anything.
    pub blocker: Option<String>,
    pub blocker_code: Option<Code>,
}

pub async fn plan(p: &Project<'_>, o: &ArchiveOptions) -> Result<ArchivePlan> {
    p.ensure_bound_dir()?;
    if !p.dir.exists() {
        return Err(
            Error::invalid(format!("project {} is not on this device", p.name))
                .with_hint("pull it first, or archive it on the device that has it"),
        );
    }
    let push = sync::plan_push(p).await?;
    let local = p.sync_view()?;
    let mut blocker = push.blocker();
    if let Some(h) = &push.head {
        if h.state == HeadState::Archived {
            blocker = Some(Error::new(
                Code::Archived,
                format!("project {} is already archived", p.name),
            ));
        }
    }
    let devices = workroam_device::registry::list(&p.ws.control)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|d| d.id != p.ws.machine.device_id)
        .map(|d| d.name)
        .collect();
    Ok(ArchivePlan {
        project: p.name.clone(),
        pid: p.pid.clone(),
        dir: p.dir.display().to_string(),
        files: local.len(),
        size: local.values().map(|f| f.meta.size).sum(),
        repos: p.cfg.repos.keys().cloned().collect(),
        unsynced: push.summary.files(),
        lease: push.lease.clone(),
        devices,
        keep_history: o.keep_history,
        note: o.note.clone(),
        blocker_code: blocker.as_ref().map(|e| e.code),
        blocker: blocker.map(|e| e.message),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ArchiveOutcome {
    pub project: String,
    pub snapshot: String,
    /// Snapshot pushed first for unsynced changes.
    pub pushed: Option<String>,
    pub size: u64,
    pub repos: Vec<String>,
    pub archived_at: Option<DateTime<Utc>>,
}

/// Push unsynced changes, write a self-contained archive snapshot, the
/// archive record and an archived head, then release the lease.
pub async fn create(p: &Project<'_>, o: &ArchiveOptions) -> Result<ArchiveOutcome> {
    p.ws.ensure_writable()?;
    let pl = plan(p, o).await?;
    if let (Some(msg), Some(code)) = (&pl.blocker, pl.blocker_code) {
        return Err(Error::new(code, msg.clone()));
    }
    let pushed = push(
        p,
        &PushOptions {
            takeover: o.takeover,
            note: "before archive".into(),
            progress: o.progress.clone(),
            ..Default::default()
        },
    )
    .await?;
    let (head, etag) =
        p.ws.head(&p.name)
            .await?
            .ok_or_else(|| Error::internal("the project has no head after pushing"))?;
    let local = p.sync_view()?;
    let collected = collect(p, &local, true, head.snapshot.clone())?;
    let size: u64 = collected
        .entries
        .iter()
        .map(|e| match &e.kind {
            workroam_snapshot::EntryKind::File { size, .. } => *size,
            _ => 0,
        })
        .sum();
    let repos: Vec<String> = collected.manifest.repos.keys().cloned().collect();
    let note = if o.note.is_empty() {
        "archive".to_string()
    } else {
        o.note.clone()
    };
    let (id, manifest) = write_snapshot(
        p,
        collected,
        &note,
        None,
        &[TAG_ARCHIVED.to_string()],
        o.progress.clone(),
    )?;
    let now = Utc::now();
    let rec = ArchiveRecord {
        name: p.name.clone(),
        snapshot: id.clone(),
        archived_at: now,
        device: p.ws.device(),
        note: o.note.clone(),
        size,
        keep_history: o.keep_history,
        repos: repos.clone(),
    };
    p.ws.control.put(&keys::archive(&p.pid), &rec).await?;
    let new_head = Head {
        project: p.name.clone(),
        snapshot: Some(id.clone()),
        parent: head.snapshot.clone(),
        device_id: p.ws.machine.device_id.clone(),
        device_name: p.ws.machine.name.clone(),
        created_at: now,
        state: HeadState::Archived,
        summary: Some(manifest.summary.clone()),
    };
    if !workroam_lease::write_head(&p.ws.control, &p.pid, &new_head, Some(&etag)).await? {
        let _ = p.ws.control.delete(&keys::archive(&p.pid)).await;
        return Err(
            Error::new(Code::Conflict, "another device pushed while archiving")
                .with_hint("pull, then archive again"),
        );
    }
    // This copy now equals the archive (scenario ② when restored here).
    let mut st = p.state();
    st.base_snapshot = Some(id.clone());
    st.base_files = manifest.files.clone();
    st.save(&p.dir)?;
    let _ = workroam_lease::release(&p.ws.control, &p.pid, &p.ws.device()).await;
    Ok(ArchiveOutcome {
        project: p.name.clone(),
        snapshot: id,
        pushed: if pushed.skipped {
            None
        } else {
            pushed.snapshot
        },
        size,
        repos,
        archived_at: Some(now),
    })
}

// ------------------------------------------------------------------ list --

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveEntry {
    pub name: String,
    pub pid: String,
    pub record: ArchiveRecord,
    /// Local copy on this device, if one is still bound.
    pub local_dir: Option<String>,
}

/// Archived projects, optionally filtered by a case-insensitive search on
/// name and note.
pub async fn list(ws: &Workspace, query: Option<&str>) -> Result<Vec<ArchiveEntry>> {
    let q = query.map(|s| s.to_lowercase());
    let mut out = Vec::new();
    for p in ws.projects().await? {
        let Some(rec) = p.archive else { continue };
        if let Some(q) = &q {
            if !p.name.to_lowercase().contains(q) && !rec.note.to_lowercase().contains(q) {
                continue;
            }
        }
        let local_dir = ws.machine.projects.get(&p.name).and_then(|b| {
            let d = workroam_core::paths::expand_tilde(&b.path);
            d.exists().then(|| d.display().to_string())
        });
        out.push(ArchiveEntry {
            name: p.name,
            pid: p.pid,
            record: rec,
            local_dir,
        });
    }
    out.sort_by_key(|e| std::cmp::Reverse(e.record.archived_at));
    Ok(out)
}

// --------------------------------------------------------------- restore --

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Scenario {
    /// ① Directory missing or empty.
    Fresh,
    /// ② Local equals the archive.
    InSync,
    /// ③ Local has changes on top of the archive.
    LocalNewer,
    /// ④ Local is an older copy.
    LocalOlder,
    /// ⑤ Both sides changed.
    Diverged,
    /// ⑥ The directory belongs to something else.
    Occupied,
}

impl Scenario {
    pub fn number(self) -> u8 {
        match self {
            Scenario::Fresh => 1,
            Scenario::InSync => 2,
            Scenario::LocalNewer => 3,
            Scenario::LocalOlder => 4,
            Scenario::Diverged => 5,
            Scenario::Occupied => 6,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoreArchivePlan {
    pub project: String,
    pub pid: String,
    pub record: ArchiveRecord,
    pub dir: String,
    pub scenario: Scenario,
    pub scenario_number: u8,
    /// `.workroam/local/` was missing: compared file by file with the archive.
    pub base_unknown: bool,
    /// Files that differ between the local copy and the archive.
    pub differing: Vec<String>,
    pub local_changes: u64,
    pub files: usize,
    pub repos: Vec<String>,
    pub reason: Option<String>,
}

fn default_dir(ws: &Workspace, name: &str) -> PathBuf {
    match ws.machine.projects.get(name) {
        Some(b) => workroam_core::paths::expand_tilde(&b.path),
        None => ws.machine.default_project_dir(name),
    }
}

pub async fn restore_plan(
    ws: &Workspace,
    name: &str,
    dir: Option<PathBuf>,
) -> Result<RestoreArchivePlan> {
    let (pid, rec) = require_record(ws, name).await?;
    let dir = dir.unwrap_or_else(|| default_dir(ws, name));
    let p = Project::at(ws, name, dir.clone())?;
    let man = read_manifest(&p, &rec.snapshot)?;
    let mut plan = RestoreArchivePlan {
        project: name.to_string(),
        pid: pid.clone(),
        dir: dir.display().to_string(),
        scenario: Scenario::Fresh,
        scenario_number: 1,
        base_unknown: false,
        differing: vec![],
        local_changes: 0,
        files: man.files.len(),
        repos: man.repos.keys().cloned().collect(),
        reason: None,
        record: rec.clone(),
    };
    if workroam_platform::dircheck::is_empty_or_missing(&dir) {
        return Ok(plan);
    }
    let own_config = ProjectConfig::load(&dir)
        .ok()
        .flatten()
        .map(|c| c.name == name)
        .unwrap_or(false);
    let scenario = match Binding::load(&dir) {
        Some(b) if b.project_id != pid => {
            plan.reason = Some(format!(
                "{} belongs to project {}",
                dir.display(),
                b.project
            ));
            Scenario::Occupied
        }
        None if !own_config => {
            plan.reason = Some(format!(
                "{} holds other content, not this project",
                dir.display()
            ));
            Scenario::Occupied
        }
        binding => {
            plan.base_unknown = binding.is_none();
            let state = if plan.base_unknown {
                SyncState::default()
            } else {
                p.state()
            };
            let local: BTreeMap<String, FileMeta> = p
                .sync_view()?
                .into_iter()
                .map(|(k, f)| (k, f.meta))
                .collect();
            plan.differing = differing(&local, &man.files);
            plan.local_changes = differing(&local, &state.base_files).len() as u64;
            let base_known = !plan.base_unknown && state.base_snapshot.is_some();
            if same_files(&local, &man.files) {
                Scenario::InSync
            } else if base_known && same_files(&state.base_files, &man.files) {
                Scenario::LocalNewer
            } else if base_known
                && same_files(&local, &state.base_files)
                && is_ancestor(ws, &pid, state.base_snapshot.as_deref().unwrap_or(""))
            {
                Scenario::LocalOlder
            } else {
                Scenario::Diverged
            }
        }
    };
    plan.scenario = scenario;
    plan.scenario_number = scenario.number();
    Ok(plan)
}

/// Snapshot series are linear per project: any earlier snapshot of the
/// same project is an ancestor of the archive.
fn is_ancestor(ws: &Workspace, pid: &str, id: &str) -> bool {
    blocking(|| ws.snap.get(id))
        .map(|m| m.has_tag(&project_tag(pid)))
        .unwrap_or(false)
}

/// How to resolve ⑤ (and ③ when the user picks the archive).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case", tag = "kind", content = "dir")]
pub enum Prefer {
    /// Three-way merge with the last sync as the common ancestor.
    #[default]
    Merge,
    /// Overwrite with the archive after moving local files aside.
    Archive,
    /// Keep the local files; they go up with the next push.
    Local,
    /// Extract into another directory; the project stays archived.
    Compare(PathBuf),
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RestoreArchiveOutcome {
    pub project: String,
    pub dir: String,
    pub scenario: Option<Scenario>,
    /// New snapshot pushed on top of the archive.
    pub snapshot: Option<String>,
    /// `<name>.before-restore-<time>/` when the archive won.
    pub moved_to: Option<String>,
    pub conflicts: Vec<String>,
    pub merged: Vec<String>,
    pub notes: Vec<String>,
    pub warnings: Vec<String>,
    /// Compare mode: where the archive was extracted.
    pub extracted_to: Option<String>,
    /// The project is active again.
    pub restored: bool,
}

/// Move local files that differ from the archive into a sibling folder.
fn move_aside(dir: &Path, name: &str, paths: &[String]) -> Result<Option<PathBuf>> {
    let present: Vec<&String> = paths
        .iter()
        .filter(|k| workroam_core::paths::from_snapshot_path(dir, k).exists())
        .collect();
    if present.is_empty() {
        return Ok(None);
    }
    let stamp = Utc::now().format("%Y%m%d-%H%M%S");
    let parent = dir.parent().unwrap_or(dir);
    let dest = parent.join(format!("{name}.before-restore-{stamp}"));
    for k in present {
        let src = workroam_core::paths::from_snapshot_path(dir, k);
        let dst = workroam_core::paths::from_snapshot_path(&dest, k);
        if let Some(d) = dst.parent() {
            std::fs::create_dir_all(d)?;
        }
        if std::fs::rename(&src, &dst).is_err() {
            std::fs::copy(&src, &dst)?;
            workroam_platform::fsutil::remove_retry(&src)?;
        }
        if let Some(d) = src.parent() {
            workroam_platform::fsutil::prune_empty_dirs(d, dir);
        }
    }
    Ok(Some(dest))
}

/// Restore an archived project into `plan.dir` and make it active again.
pub async fn restore(
    ws: &Workspace,
    plan: &RestoreArchivePlan,
    prefer: &Prefer,
    takeover: bool,
    progress: Option<Progress>,
) -> Result<RestoreArchiveOutcome> {
    let dir = PathBuf::from(&plan.dir);
    let mut out = RestoreArchiveOutcome {
        project: plan.project.clone(),
        dir: plan.dir.clone(),
        scenario: Some(plan.scenario),
        ..Default::default()
    };
    if plan.scenario == Scenario::Occupied {
        return Err(Error::invalid(
            plan.reason
                .clone()
                .unwrap_or_else(|| format!("{} is in use", plan.dir)),
        )
        .with_hint("choose another directory with --dir"));
    }
    if let Prefer::Compare(to) = prefer {
        let r = extract(ws, &plan.project, to.clone()).await?;
        out.extracted_to = Some(r.dir);
        out.notes = r.notes;
        out.warnings = r.warnings;
        return Ok(out);
    }
    ws.ensure_writable()?;
    let (pid, rec) = require_record(ws, &plan.project).await?;
    let snapshot = rec.snapshot.clone();
    let mut p = Project::at(ws, &plan.project, dir.clone())?;
    for name in &plan.repos {
        if let Some(r) = p.cfg.repos.get(name) {
            let rd = workroam_core::paths::from_snapshot_path(&dir, &r.path);
            if workroam_git::cmd::is_repo(&rd) {
                workroam_git::cmd::Git::new(&rd).ensure_not_in_progress(name)?;
            }
        }
    }
    take_lease(&p, takeover).await?;
    let man: Manifest = read_manifest(&p, &snapshot)?;
    let opts = PullOptions {
        takeover,
        target: Some(snapshot.clone()),
        progress,
        archive: true,
    };
    let mut po = PullOutcome::default();
    let keep_local = |p: &Project<'_>, man: &Manifest| -> Result<Vec<String>> {
        let mut st = p.state();
        st.base_snapshot = Some(snapshot.clone());
        st.base_files = man.files.clone();
        st.save(&p.dir)?;
        p.bind()?;
        Ok(p.generate().map(|g| g.warnings).unwrap_or_default())
    };
    match (plan.scenario, prefer) {
        (Scenario::Fresh, _) | (Scenario::LocalOlder, _) => {
            apply_snapshot(&mut p, &snapshot, &opts, &mut po).await?;
        }
        (Scenario::InSync, _)
        | (Scenario::LocalNewer, Prefer::Merge | Prefer::Local)
        | (Scenario::Diverged, Prefer::Local) => {
            out.warnings.extend(keep_local(&p, &man)?);
        }
        (Scenario::Diverged, Prefer::Merge) => {
            if plan.base_unknown {
                // Never overwrite: differing files become conflict copies.
                let mut st = p.state();
                st.base_snapshot = None;
                st.base_files = BTreeMap::new();
                st.save(&p.dir)?;
            }
            apply_snapshot(&mut p, &snapshot, &opts, &mut po).await?;
        }
        (_, Prefer::Archive) => {
            let local: BTreeMap<String, FileMeta> = p
                .sync_view()?
                .into_iter()
                .map(|(k, f)| (k, f.meta))
                .collect();
            let moved = move_aside(&dir, &plan.project, &differing(&local, &man.files))?;
            out.moved_to = moved.map(|d| d.display().to_string());
            let st = SyncState {
                base_snapshot: None,
                base_files: BTreeMap::new(),
                ..p.state()
            };
            st.save(&p.dir)?;
            apply_snapshot(&mut p, &snapshot, &opts, &mut po).await?;
        }
        (Scenario::Occupied, _) | (_, Prefer::Compare(_)) => unreachable!(),
    }
    out.conflicts = po.conflicts;
    out.merged = po.merged;
    out.notes = po.notes;
    out.warnings.extend(po.warnings);
    p.reload()?;
    // Finish: head active again on top of the archive, record removed, push.
    let (head, etag) = workroam_lease::read_head(&ws.control, &pid)
        .await?
        .ok_or_else(|| Error::new(Code::Corrupted, "archived project has no head"))?;
    let active = Head {
        state: HeadState::Active,
        snapshot: Some(snapshot.clone()),
        created_at: Utc::now(),
        device_id: ws.machine.device_id.clone(),
        device_name: ws.machine.name.clone(),
        ..head
    };
    if !workroam_lease::write_head(&ws.control, &pid, &active, Some(&etag)).await? {
        return Err(Error::new(
            Code::Conflict,
            "another device changed the project while restoring",
        ));
    }
    ws.control.delete(&keys::archive(&pid)).await?;
    out.restored = true;
    if out.conflicts.is_empty() {
        let r = push(
            &p,
            &PushOptions {
                restored_from: Some(snapshot.clone()),
                force: true,
                note: "restore archive".into(),
                ..Default::default()
            },
        )
        .await?;
        out.snapshot = r.snapshot;
    } else {
        out.notes.push(format!(
            "{} conflict copies to review; push after resolving them",
            out.conflicts.len()
        ));
        out.snapshot = Some(snapshot);
    }
    Ok(out)
}

// --------------------------------------------------------------- extract --

/// Copy the archive into another directory without changing its state
/// (also works on read-only devices).
pub async fn extract(ws: &Workspace, name: &str, dir: PathBuf) -> Result<RestoreOutcome> {
    let (_, rec) = require_record(ws, name).await?;
    let mut p = Project::at(ws, name, dir.clone())?;
    let plan =
        engine_restore::plan_restore(&p, &rec.snapshot, vec![], RestoreTarget::Dir(dir)).await?;
    engine_restore::restore(&mut p, &plan).await
}

// ---------------------------------------------------------------- delete --

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DeleteOutcome {
    pub project: String,
    pub forgotten: usize,
}

/// Delete an archive for good: its snapshots, record, head and lease. The
/// next `prune` frees the data.
pub async fn delete(ws: &Workspace, name: &str) -> Result<DeleteOutcome> {
    ws.ensure_writable()?;
    let (pid, _) = require_record(ws, name).await?;
    let tag = project_tag(&pid);
    let ids: Vec<String> = blocking(|| ws.snap.list())?
        .into_iter()
        .filter(|s| s.has_tag(&tag))
        .map(|s| s.id)
        .collect();
    if !ids.is_empty() {
        blocking(|| ws.snap.forget(&ids))?;
    }
    ws.control.delete(&keys::archive(&pid)).await?;
    ws.control.delete(&keys::head(&pid)).await?;
    ws.control.delete(&keys::lease(&pid)).await?;
    Ok(DeleteOutcome {
        project: name.to_string(),
        forgotten: ids.len(),
    })
}

/// Local changes since the archive in a bound copy (for status lines).
pub fn local_changes(p: &Project<'_>) -> Result<u64> {
    let st = p.state();
    Ok(diff_local(&st.base_files, &p.sync_view()?).0.files())
}
