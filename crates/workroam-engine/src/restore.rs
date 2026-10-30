//! Restore from a snapshot (architecture §10.1): in place (new snapshot with
//! `restored_from`) or into another directory (no lease, no control writes).

use crate::merge;
use crate::project::Project;
use crate::sync::{self, PullOutcome, PushOptions};
use crate::workspace::blocking;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use workroam_core::model::{FileMeta, Manifest};
use workroam_core::{Code, Error, Result};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "kind", content = "dir")]
pub enum RestoreTarget {
    InPlace,
    Dir(PathBuf),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestorePlan {
    pub project: String,
    pub snapshot: workroam_core::model::SnapshotInfo,
    pub target: RestoreTarget,
    pub dir: String,
    pub paths: Vec<String>,
    /// Files that will be written or removed.
    pub files: usize,
    /// Local unsynced changes saved as `*.conflict-before-restore*` first.
    pub saved_local: Vec<String>,
    pub repos: Vec<String>,
}

fn selected(paths: &[String], rel: &str) -> bool {
    paths.is_empty()
        || paths.iter().any(|p| {
            let p = p.trim_matches('/');
            rel == p || rel.starts_with(&format!("{p}/"))
        })
}

struct Work {
    write: BTreeMap<String, FileMeta>,
    delete: Vec<String>,
    save: Vec<String>,
}

fn work(p: &Project<'_>, man: &Manifest, paths: &[String], in_place: bool) -> Result<Work> {
    let mut w = Work {
        write: BTreeMap::new(),
        delete: vec![],
        save: vec![],
    };
    if !in_place {
        w.write = man
            .files
            .iter()
            .filter(|(k, _)| selected(paths, k))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        return Ok(w);
    }
    let state = p.state();
    let local = p.sync_view()?;
    let keys: BTreeSet<&String> = local
        .keys()
        .chain(man.files.keys())
        .filter(|k| selected(paths, k))
        .collect();
    for k in keys {
        let l = local.get(k).map(|f| &f.meta);
        let t = man.files.get(k);
        if l.map(|x| (&x.sha256, x.exec)) == t.map(|x| (&x.sha256, x.exec)) {
            continue;
        }
        let unsynced =
            l.map(|x| x.sha256.clone()) != state.base_files.get(k).map(|b| b.sha256.clone());
        if l.is_some() && unsynced {
            w.save.push(k.clone());
        }
        match t {
            Some(m) => {
                w.write.insert(k.clone(), m.clone());
            }
            None => w.delete.push(k.clone()),
        }
    }
    Ok(w)
}

pub async fn plan_restore(
    p: &Project<'_>,
    spec: &str,
    paths: Vec<String>,
    target: RestoreTarget,
) -> Result<RestorePlan> {
    let id = crate::history::resolve(p, spec).await?;
    let meta = crate::history::ensure_exists(p.ws, &id)?;
    let man = sync::read_manifest(p, &id)?;
    let in_place = target == RestoreTarget::InPlace;
    let dir = match &target {
        RestoreTarget::InPlace => p.dir.clone(),
        RestoreTarget::Dir(d) => d.clone(),
    };
    if !in_place && !workroam_platform::dircheck::is_empty_or_missing(&dir) {
        return Err(Error::invalid(format!("{} is not empty", dir.display()))
            .with_hint("restore into an empty or new directory"));
    }
    let w = if in_place {
        work(p, &man, &paths, true)?
    } else {
        work(p, &man, &paths, false)?
    };
    Ok(RestorePlan {
        project: p.name.clone(),
        snapshot: crate::history::info(&meta),
        dir: dir.display().to_string(),
        target,
        files: w.write.len() + w.delete.len(),
        saved_local: w.save,
        repos: man
            .repos
            .iter()
            .filter(|(_, r)| selected(&paths, &r.path) || paths.is_empty())
            .map(|(k, _)| k.clone())
            .collect(),
        paths,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RestoreOutcome {
    pub project: String,
    pub restored_from: String,
    pub dir: String,
    pub files: usize,
    pub saved_local: Vec<String>,
    pub before_restore_branches: Vec<String>,
    /// New snapshot (in place only).
    pub snapshot: Option<String>,
    pub notes: Vec<String>,
    pub warnings: Vec<String>,
}

/// Write files of `snapshot` into `dir` (used by restore and archive extract).
pub fn write_files(
    p: &Project<'_>,
    snapshot: &str,
    dir: &std::path::Path,
    files: &BTreeMap<String, FileMeta>,
) -> Result<()> {
    let paths: Vec<String> = files
        .iter()
        .filter(|(_, m)| m.symlink.is_none())
        .map(|(k, _)| k.clone())
        .collect();
    let mut errors = Vec::new();
    blocking(|| {
        p.ws.snap.read_files(snapshot, &paths, &mut |path, data| {
            if let Some(m) = files.get(path) {
                let abs = workroam_core::paths::from_snapshot_path(dir, path);
                if let Err(e) = sync::write_entry(&abs, dir, m, &data, None) {
                    errors.push(format!("{path}: {}", e.message));
                }
            }
            Ok(())
        })
    })?;
    for (path, m) in files.iter().filter(|(_, m)| m.symlink.is_some()) {
        let abs = workroam_core::paths::from_snapshot_path(dir, path);
        if let Err(e) = sync::write_entry(&abs, dir, m, &[], None) {
            errors.push(format!("{path}: {}", e.message));
        }
    }
    if !errors.is_empty() {
        return Err(Error::new(
            Code::Network,
            format!("{} file(s) could not be written", errors.len()),
        )
        .with_data(serde_json::json!(errors)));
    }
    Ok(())
}

pub async fn restore(p: &mut Project<'_>, plan: &RestorePlan) -> Result<RestoreOutcome> {
    let id = plan.snapshot.id.clone();
    let man = sync::read_manifest(p, &id)?;
    let mut out = RestoreOutcome {
        project: p.name.clone(),
        restored_from: id.clone(),
        dir: plan.dir.clone(),
        ..Default::default()
    };
    match &plan.target {
        RestoreTarget::Dir(dir) => {
            std::fs::create_dir_all(dir)?;
            let w = work(p, &man, &plan.paths, false)?;
            write_files(p, &id, dir, &w.write)?;
            out.files = w.write.len();
            let mut copy = Project::at(p.ws, &p.name, dir.clone())?;
            let mut po = PullOutcome::default();
            let man_sel = Manifest {
                repos: man
                    .repos
                    .iter()
                    .filter(|(k, _)| plan.repos.contains(k))
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
                ..man.clone()
            };
            sync::import_repos(&copy, &id, &man_sel, false, &mut po)?;
            copy.reload()?;
            if let Ok(g) = copy.generate() {
                out.warnings.extend(g.warnings);
            }
            out.notes = po.notes;
            out.warnings.extend(po.warnings);
            Ok(out)
        }
        RestoreTarget::InPlace => {
            p.ws.ensure_writable()?;
            sync::take_lease(p, false).await?;
            let head = p.ws.head(&p.name).await?.map(|(h, _)| h);
            let state = p.state();
            if let Some(h) = &head {
                workroam_lease::ensure_active(h)?;
                if h.snapshot.is_some() && h.snapshot != state.base_snapshot {
                    return Err(Error::new(
                        Code::Conflict,
                        format!("{} pushed a newer snapshot", h.device_name),
                    )
                    .with_hint("pull first, then restore"));
                }
            }
            let _syncing = crate::local::SyncingGuard::new(&p.dir, "restore")?;
            let w = work(p, &man, &plan.paths, true)?;
            for k in &w.save {
                let src = workroam_core::paths::from_snapshot_path(&p.dir, k);
                let cp = merge::conflict_path(k, "before-restore", &|c| p.dir.join(c).exists());
                let dst = workroam_core::paths::from_snapshot_path(&p.dir, &cp);
                std::fs::copy(&src, &dst)?;
                out.saved_local.push(cp);
            }
            write_files(p, &id, &p.dir.clone(), &w.write)?;
            for k in &w.delete {
                let abs = workroam_core::paths::from_snapshot_path(&p.dir, k);
                let _ = workroam_platform::fsutil::remove_retry(&abs);
                if let Some(parent) = abs.parent() {
                    workroam_platform::fsutil::prune_empty_dirs(parent, &p.dir);
                }
            }
            out.files = w.write.len() + w.delete.len();
            // Repositories: keep the current position, then move to the snapshot's.
            let stamp = Utc::now().format("%Y%m%d-%H%M%S").to_string();
            for name in &plan.repos {
                let Some(rm) = man.repos.get(name) else {
                    continue;
                };
                let dir = workroam_core::paths::from_snapshot_path(&p.dir, &rm.path);
                let git = workroam_git::cmd::Git::new(&dir);
                if !workroam_git::cmd::is_repo(&dir) {
                    continue;
                }
                git.ensure_not_in_progress(name)?;
                let keep = format!("workroam/before-restore/{stamp}");
                let tip = match git.head() {
                    Some(h) => blocking(|| workroam_git::sync::make_wip(&git, &h))
                        .ok()
                        .flatten()
                        .map(|(c, _)| c)
                        .unwrap_or(h),
                    None => continue,
                };
                git.run(&["branch", "-f", &keep, &tip])?;
                out.before_restore_branches.push(format!("{name}: {keep}"));
                let mut po = PullOutcome::default();
                let one = Manifest {
                    repos: [(name.clone(), rm.clone())].into_iter().collect(),
                    ..man.clone()
                };
                sync::import_repos(p, &id, &one, false, &mut po)?;
                let cache = workroam_git::sync::RepoCache::new(&p.dir, name);
                let map = cache.load_map();
                if let Some(b) = &rm.head_branch {
                    if let Some(c) = rm.branches.get(b) {
                        let real = map.real_of(c).unwrap_or(c).to_string();
                        if git.exists(&real) {
                            git.run(&["checkout", "-q", "-B", b, &real])?;
                            git.run(&["reset", "-q", "--hard", &real])?;
                        }
                    }
                }
                if let Some(wip) = &rm.wip {
                    if git.exists(&wip.commit) {
                        blocking(|| workroam_git::sync::restore_wip(&git, &wip.commit))?;
                    }
                }
                out.notes.extend(po.notes);
            }
            let res = sync::push(
                p,
                &PushOptions {
                    restored_from: Some(id.clone()),
                    force: true,
                    note: format!("restore {}", &id[..8.min(id.len())]),
                    ..Default::default()
                },
            )
            .await?;
            out.snapshot = res.snapshot;
            Ok(out)
        }
    }
}
