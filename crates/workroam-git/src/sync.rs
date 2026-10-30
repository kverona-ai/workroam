//! Transport of code repositories between devices (§11.1 – §11.5).
//!
//! `bundle` mode keeps a self-contained *base* bundle that is reused byte
//! for byte in every snapshot (deduplicated by the snapshot engine) and an
//! *incremental* bundle with everything newer, including the WIP commit.
//! `recent:N` / `snapshot` histories are rebuilt with identical trees; the
//! rebuild map lets the intranet device translate the other side's commits
//! back onto the real history.

use crate::cmd::Git;
use globset::{Glob, GlobSetBuilder};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use workroam_core::config::RepoConfig;
use workroam_core::model::{RepoManifest, RepoMode, WipInfo};
use workroam_core::{Code, Error, Result};

const OUT: &str = "refs/workroam/out/";
const WIPOUT: &str = "refs/workroam/wipout/";
const IN: &str = "refs/workroam/in/";
const INWIP: &str = "refs/workroam/inwip/";
/// Roll a new base bundle after this many incremental commits.
const ROLL_AFTER: u64 = 300;

/// Real commit <-> rebuilt commit pairs (architecture §11.2).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RebuildMap {
    pub pairs: Vec<(String, String)>,
}

impl RebuildMap {
    pub fn rebuilt_of(&self, real: &str) -> Option<&str> {
        self.pairs
            .iter()
            .rev()
            .find(|(r, _)| r == real)
            .map(|(_, b)| b.as_str())
    }
    pub fn real_of(&self, rebuilt: &str) -> Option<&str> {
        self.pairs
            .iter()
            .rev()
            .find(|(_, b)| b == rebuilt)
            .map(|(r, _)| r.as_str())
    }
    pub fn push(&mut self, real: &str, rebuilt: &str) {
        if self.rebuilt_of(real) != Some(rebuilt) {
            self.pairs.push((real.to_string(), rebuilt.to_string()));
        }
    }
}

/// Per repository state kept in `.workroam/local/git/<name>/` (never synced).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RepoLocal {
    /// Branch tips as last transported (pushed or pulled), transported history.
    pub transported: BTreeMap<String, String>,
    /// Tips contained in `base.bundle`.
    pub base_tips: Vec<String>,
    /// Tree of the working state last transported (WIP tree or HEAD tree).
    pub last_tree: Option<String>,
}

pub struct RepoCache {
    pub dir: PathBuf,
}

impl RepoCache {
    pub fn new(project: &Path, name: &str) -> RepoCache {
        RepoCache {
            dir: project.join(".workroam/local/git").join(name),
        }
    }
    pub fn base_bundle(&self) -> PathBuf {
        self.dir.join("base.bundle")
    }
    pub fn load_map(&self) -> RebuildMap {
        std::fs::read(self.dir.join("map.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }
    pub fn save_map(&self, m: &RebuildMap) -> Result<()> {
        workroam_core::paths::write_atomic(&self.dir.join("map.json"), &serde_json::to_vec(m)?)
    }
    pub fn load_local(&self) -> RepoLocal {
        std::fs::read(self.dir.join("state.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }
    pub fn save_local(&self, s: &RepoLocal) -> Result<()> {
        workroam_core::paths::write_atomic(
            &self.dir.join("state.json"),
            &serde_json::to_vec_pretty(s)?,
        )
    }
}

/// Files a repository contributes to a snapshot: (snapshot path, local file).
#[derive(Debug, Clone, Default)]
pub struct Export {
    pub manifest: RepoManifest,
    pub files: Vec<(String, PathBuf)>,
}

pub fn snapshot_dir(name: &str) -> String {
    format!(".workroam-snapshot/git/{name}")
}

/// Tree of the current working state including untracked, non-ignored
/// files, computed with a temporary index (never touches the user's index).
pub fn worktree_tree(git: &Git) -> Result<Option<String>> {
    let head = match git.head() {
        Some(h) => h,
        None => return Ok(None),
    };
    let idx = git.git_dir().join("workroam-index");
    let idx_s = idx.to_string_lossy().to_string();
    let env = [("GIT_INDEX_FILE", idx_s.as_str())];
    git.run_env(&["read-tree", &head], &env)?;
    git.run_env(&["add", "-A"], &env)?;
    let tree = git.run_env(&["write-tree"], &env)?;
    let _ = std::fs::remove_file(&idx);
    Ok(Some(tree))
}

/// Make a WIP commit for the working state on top of `parent` (GIT-05).
/// Returns `None` when the working tree equals HEAD.
pub fn make_wip(git: &Git, parent: &str) -> Result<Option<(String, String)>> {
    let Some(tree) = worktree_tree(git)? else {
        return Ok(None);
    };
    let head_tree = git.tree_of("HEAD").unwrap_or_default();
    if tree == head_tree {
        return Ok(None);
    }
    let c = git.run_env(
        &["commit-tree", &tree, "-p", parent, "-m", "workroam wip"],
        &[
            ("GIT_AUTHOR_NAME", "Workroam"),
            ("GIT_AUTHOR_EMAIL", "wip@workroam.invalid"),
            ("GIT_COMMITTER_NAME", "Workroam"),
            ("GIT_COMMITTER_EMAIL", "wip@workroam.invalid"),
        ],
    )?;
    Ok(Some((c, tree)))
}

/// Restore a WIP commit as uncommitted changes on top of HEAD.
pub fn restore_wip(git: &Git, wip: &str) -> Result<()> {
    let idx = git.git_dir().join("workroam-restore-index");
    let idx_s = idx.to_string_lossy().to_string();
    let env = [("GIT_INDEX_FILE", idx_s.as_str())];
    git.run_env(&["read-tree", wip], &env)?;
    git.run_env(&["checkout-index", "-a", "-f"], &env)?;
    let _ = std::fs::remove_file(&idx);
    // Files deleted in the WIP.
    let deleted = git.run(&["diff", "--name-only", "-z", "--diff-filter=D", "HEAD", wip])?;
    for p in deleted.split('\0').filter(|p| !p.is_empty()) {
        let _ = std::fs::remove_file(git.dir.join(p));
    }
    Ok(())
}

fn select_branches(git: &Git, cfg: &RepoConfig) -> Result<BTreeMap<String, String>> {
    let all = git.branches();
    if cfg.branches.is_empty() {
        if let Some(cur) = git.current_branch() {
            if let Some(c) = all.get(&cur) {
                return Ok(BTreeMap::from([(cur, c.clone())]));
            }
        }
        return Ok(all);
    }
    let mut b = GlobSetBuilder::new();
    for p in &cfg.branches {
        b.add(Glob::new(p).map_err(|e| Error::invalid(format!("branch pattern {p}: {e}")))?);
    }
    let set = b.build().map_err(|e| Error::invalid(e.to_string()))?;
    let mut out: BTreeMap<String, String> = all
        .iter()
        .filter(|(n, _)| set.is_match(n.as_str()))
        .map(|(a, b)| (a.clone(), b.clone()))
        .collect();
    // The checked-out branch always travels.
    if let Some(cur) = git.current_branch() {
        if let Some(c) = all.get(&cur) {
            out.insert(cur, c.clone());
        }
    }
    Ok(out)
}

/// Does this device own the real history (vs. working on rebuilt commits)?
fn is_real_side(git: &Git, map: &RebuildMap) -> bool {
    map.pairs.is_empty() || map.pairs.iter().rev().take(50).any(|(r, _)| git.exists(r))
}

/// Rebuild the last `depth` first-parent commits of `tip` onto the rebuilt
/// history, extending `map`. Returns the rebuilt tip.
fn rebuild(git: &Git, tip: &str, depth: usize, map: &mut RebuildMap) -> Result<String> {
    if let Some(b) = map.rebuilt_of(tip) {
        return Ok(b.to_string());
    }
    let chain = git.first_parent_chain(tip, depth.max(1))?;
    // Newest commit of the chain that is already mapped.
    let mut start = chain.iter().rposition(|c| map.rebuilt_of(c).is_some());
    let mut parent: Option<String> = start.map(|i| map.rebuilt_of(&chain[i]).unwrap().to_string());
    if start.is_none() {
        // Keep continuity with the previous rebuilt history (§11.2 step 5).
        for (real, rebuilt) in map.pairs.iter().rev() {
            if git.exists(real) && git.is_ancestor(real, tip) {
                parent = Some(rebuilt.clone());
                start = chain
                    .iter()
                    .position(|c| !git.is_ancestor(c, real))
                    .map(|i| i.saturating_sub(1))
                    .or(Some(chain.len() - 1));
                // `start` points at the last commit not to rebuild.
                if let Some(i) = start {
                    if !git.is_ancestor(&chain[i], real) {
                        start = if i == 0 { None } else { Some(i - 1) };
                    }
                }
                break;
            }
        }
    }
    let rest: Vec<String> = match start {
        Some(i) => chain[i + 1..].to_vec(),
        None => chain.clone(),
    };
    let rest = if depth == 0 {
        vec![tip.to_string()]
    } else {
        rest
    };
    let mut cur = parent;
    for c in &rest {
        let info = git.commit_info(c)?;
        let parents: Vec<String> = cur.iter().cloned().collect();
        let n = git.recommit(&info, &parents)?;
        map.push(c, &n);
        cur = Some(n);
    }
    cur.ok_or_else(|| Error::internal("history rebuild produced no commit"))
}

pub struct ExportOptions<'a> {
    pub name: &'a str,
    pub project: &'a Path,
    pub cfg: &'a RepoConfig,
    /// Write bundles here (inside `.workroam/local/`).
    pub stage: &'a Path,
    /// Archive: a fresh, self-contained base bundle for every mode.
    pub archive: bool,
}

/// Prepare a repository for a push (bundle, remote) or an archive.
pub fn export(o: &ExportOptions) -> Result<Export> {
    let dir = workroam_core::paths::from_snapshot_path(o.project, &o.cfg.path);
    let git = Git::new(&dir);
    if !crate::cmd::is_repo(&dir) {
        return Ok(Export {
            manifest: RepoManifest {
                path: o.cfg.path.clone(),
                mode: o.cfg.mode,
                ..Default::default()
            },
            files: vec![],
        });
    }
    git.ensure_not_in_progress(o.name)?;
    let mode = o.cfg.mode;
    let cache = RepoCache::new(o.project, o.name);
    let mut local = cache.load_local();
    let mut man = RepoManifest {
        path: o.cfg.path.clone(),
        mode,
        history: o.cfg.history.clone(),
        head_branch: git.current_branch(),
        origin_url: git
            .remote_url(o.cfg.via.as_deref().unwrap_or("origin"))
            .or_else(|| git.remote_url("origin")),
        via: o.cfg.via.clone(),
        user_email: git.config_get("user.email"),
        ..Default::default()
    };
    if mode == RepoMode::Files && !o.archive {
        man.baseline = git.head();
        return Ok(Export {
            manifest: man,
            files: vec![],
        });
    }
    let branches = select_branches(&git, o.cfg)?;
    if mode == RepoMode::Remote && !o.archive {
        return export_remote(&git, o, man, branches, &cache, &mut local);
    }
    // bundle (or archive of any mode)
    let depth = if mode == RepoMode::Bundle {
        o.cfg.history_depth()
    } else {
        None
    };
    let mut map = cache.load_map();
    let real_side = depth.is_some() && is_real_side(&git, &map);
    let mut transported = BTreeMap::new();
    for (b, tip) in &branches {
        let t = if real_side {
            rebuild(&git, tip, depth.unwrap(), &mut map)?
        } else {
            tip.clone()
        };
        transported.insert(b.clone(), t);
    }
    // WIP on the checked-out branch.
    let mut wip_ref = None;
    if let (Some(cur), Some(head)) = (git.current_branch(), git.head()) {
        let base = transported.get(&cur).cloned().unwrap_or(head);
        if let Some((w, tree)) = make_wip(&git, &base)? {
            man.wip = Some(WipInfo {
                branch: cur.clone(),
                commit: w.clone(),
                base: base.clone(),
            });
            wip_ref = Some((format!("{WIPOUT}{cur}"), w));
            local.last_tree = Some(tree);
        } else {
            local.last_tree = git.tree_of("HEAD");
        }
    }
    // Temporary refs for bundling.
    for (b, t) in &transported {
        git.update_ref(&format!("{OUT}{b}"), t)?;
    }
    if let Some((r, w)) = &wip_ref {
        git.update_ref(r, w)?;
    }
    std::fs::create_dir_all(o.stage)?;
    let snap_dir = snapshot_dir(o.name);
    let result = (|| -> Result<Vec<(String, PathBuf)>> {
        let mut files = Vec::new();
        let tips: Vec<String> = transported.values().cloned().collect();
        let base_ok = !o.archive
            && cache.base_bundle().exists()
            && !local.base_tips.is_empty()
            && local
                .base_tips
                .iter()
                .all(|bt| git.exists(bt) && tips.iter().any(|t| git.is_ancestor(bt, t)))
            && {
                let mut args: Vec<&str> = tips.iter().map(String::as_str).collect();
                args.push("--not");
                args.extend(local.base_tips.iter().map(String::as_str));
                git.count(&args) < ROLL_AFTER
            };
        let base_path = if base_ok {
            cache.base_bundle()
        } else {
            // New self-contained base bundle with the branches only.
            if transported.is_empty() {
                return Ok(files);
            }
            let p = o.stage.join(format!("{}-base.bundle", o.name));
            let mut args = vec![
                "bundle".to_string(),
                "create".to_string(),
                p.to_string_lossy().to_string(),
            ];
            args.extend(transported.keys().map(|b| format!("{OUT}{b}")));
            let a: Vec<&str> = args.iter().map(String::as_str).collect();
            git.run(&a)?;
            if !o.archive {
                std::fs::create_dir_all(&cache.dir)?;
                std::fs::copy(&p, cache.base_bundle())?;
                local.base_tips = tips.clone();
            }
            p
        };
        files.push((format!("{snap_dir}/base.bundle"), base_path));
        // Incremental bundle: everything not in the base (new commits + WIP).
        let base_tips = if base_ok {
            local.base_tips.clone()
        } else {
            tips.clone()
        };
        let mut refs: Vec<String> = transported.keys().map(|b| format!("{OUT}{b}")).collect();
        if let Some((r, _)) = &wip_ref {
            refs.push(r.clone());
        }
        let p = o.stage.join(format!("{}-incr.bundle", o.name));
        let mut args = vec![
            "bundle".to_string(),
            "create".to_string(),
            p.to_string_lossy().to_string(),
        ];
        args.extend(refs);
        args.push("--not".into());
        args.extend(base_tips);
        let a: Vec<&str> = args.iter().map(String::as_str).collect();
        match git.run(&a) {
            Ok(_) => files.push((format!("{snap_dir}/incr.bundle"), p)),
            Err(e) if e.message.contains("empty bundle") => {}
            Err(e) => return Err(e),
        }
        Ok(files)
    })();
    for b in transported.keys() {
        git.delete_ref(&format!("{OUT}{b}"));
    }
    if let Some((r, _)) = &wip_ref {
        git.delete_ref(r);
    }
    let mut files = result?;
    man.commits = {
        let mut n = 0;
        for (b, t) in &transported {
            n += match local.transported.get(b) {
                Some(prev) if git.exists(prev) => git.count(&[t, "--not", prev]),
                Some(_) => 0,
                None => git.count(&[t]),
            };
        }
        n
    };
    man.branches = transported.clone();
    man.base_bundle = files
        .iter()
        .find(|(p, _)| p.ends_with("base.bundle"))
        .map(|(p, _)| p.clone());
    man.bundle = files
        .iter()
        .find(|(p, _)| p.ends_with("incr.bundle"))
        .map(|(p, _)| p.clone());
    if depth.is_some() {
        let mp = o.stage.join(format!("{}-map.json", o.name));
        std::fs::write(&mp, serde_json::to_vec(&map)?)?;
        let sp = format!("{snap_dir}/map.json");
        files.push((sp.clone(), mp));
        man.rebuild_map = Some(sp);
        if !o.archive {
            cache.save_map(&map)?;
        }
    }
    if !o.archive {
        local.transported = transported;
        cache.save_local(&local)?;
    }
    Ok(Export {
        manifest: man,
        files,
    })
}

fn export_remote(
    git: &Git,
    o: &ExportOptions,
    mut man: RepoManifest,
    branches: BTreeMap<String, String>,
    cache: &RepoCache,
    local: &mut RepoLocal,
) -> Result<Export> {
    let via = o.cfg.via.clone().unwrap_or_else(|| "origin".into());
    for b in branches.keys() {
        git.run(&[
            "push",
            "--quiet",
            &via,
            &format!("refs/heads/{b}:refs/heads/{b}"),
        ])
        .map_err(|e| {
            Error::new(Code::Network, format!("git push {via} {b}: {}", e.message))
                .with_hint("pull and merge first, or check that the remote is reachable")
        })?;
    }
    if let (Some(cur), Some(head)) = (git.current_branch(), git.head()) {
        if let Some((w, tree)) = make_wip(git, &head)? {
            git.run(&[
                "push",
                "--quiet",
                "--force",
                &via,
                &format!("{w}:refs/heads/workroam/wip/{cur}"),
            ])
            .map_err(|e| Error::new(Code::Network, format!("git push WIP: {}", e.message)))?;
            man.wip = Some(WipInfo {
                branch: cur.clone(),
                commit: w,
                base: head,
            });
            local.last_tree = Some(tree);
        } else {
            local.last_tree = git.tree_of("HEAD");
            let _ = git.run(&[
                "push",
                "--quiet",
                &via,
                "--delete",
                &format!("refs/heads/workroam/wip/{cur}"),
            ]);
        }
    }
    man.commits = branches
        .iter()
        .map(|(b, t)| match local.transported.get(b) {
            Some(prev) if git.exists(prev) => git.count(&[t, "--not", prev]),
            _ => 0,
        })
        .sum();
    man.branches = branches.clone();
    local.transported = branches;
    cache.save_local(local)?;
    Ok(Export {
        manifest: man,
        files: vec![],
    })
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImportOutcome {
    pub notes: Vec<String>,
    pub conflicts: Vec<String>,
    pub cloned: bool,
}

pub struct ImportOptions<'a> {
    pub name: &'a str,
    pub project: &'a Path,
    pub man: &'a RepoManifest,
    pub base_bundle: Option<&'a Path>,
    pub incr_bundle: Option<&'a Path>,
    pub map: Option<RebuildMap>,
    /// Archive restore: never move existing branches, park divergence on `workroam/archive/*`.
    pub archive: bool,
}

/// Bring a repository up to the state described by the manifest (PULL-04).
pub fn import(o: &ImportOptions) -> Result<ImportOutcome> {
    let dir = workroam_core::paths::from_snapshot_path(o.project, &o.man.path);
    let mut out = ImportOutcome::default();
    if o.man.mode == RepoMode::Files && o.base_bundle.is_none() {
        return Ok(out);
    }
    std::fs::create_dir_all(&dir)?;
    let git = Git::new(&dir);
    let fresh = !crate::cmd::is_repo(&dir);
    if fresh {
        let head = o
            .man
            .head_branch
            .clone()
            .or_else(|| o.man.branches.keys().next().cloned())
            .unwrap_or_else(|| "main".into());
        git.run(&["init", "--quiet", "-b", &head])?;
        let _ = git.run(&["config", "core.autocrlf", "false"]);
        if let Some(email) = &o.man.user_email {
            let _ = git.run(&["config", "user.email", email]);
        }
        out.cloned = true;
    } else {
        git.ensure_not_in_progress(o.name)?;
    }
    let cache = RepoCache::new(o.project, o.name);
    let mut local = cache.load_local();
    // Fetch the incoming refs into refs/workroam/in/*.
    for r in git
        .refs_under("refs/workroam/in")
        .keys()
        .chain(git.refs_under("refs/workroam/inwip").keys())
    {
        git.delete_ref(r);
    }
    let use_remote = o.man.mode == RepoMode::Remote && o.base_bundle.is_none();
    if use_remote {
        let via = o.man.via.clone().unwrap_or_else(|| "origin".into());
        if git.remote_url(&via).is_none() {
            let url = o.man.origin_url.clone().ok_or_else(|| {
                Error::invalid(format!("repository {} has no remote URL", o.name))
            })?;
            git.run(&["remote", "add", &via, &url])?;
        }
        let mut specs: Vec<String> = o
            .man
            .branches
            .keys()
            .map(|b| format!("+refs/heads/{b}:{IN}{b}"))
            .collect();
        if let Some(w) = &o.man.wip {
            specs.push(format!(
                "+refs/heads/workroam/wip/{}:{INWIP}{}",
                w.branch, w.branch
            ));
        }
        let mut args = vec![
            "fetch".to_string(),
            "--quiet".into(),
            "--no-tags".into(),
            via.clone(),
        ];
        args.extend(specs);
        let a: Vec<&str> = args.iter().map(String::as_str).collect();
        git.run(&a)
            .map_err(|e| Error::new(Code::Network, format!("git fetch {via}: {}", e.message)))?;
    } else {
        for bundle in [o.base_bundle, o.incr_bundle].into_iter().flatten() {
            let b = bundle.to_string_lossy().to_string();
            git.run(&[
                "fetch",
                "--quiet",
                "--no-tags",
                &b,
                &format!("+{OUT}*:{IN}*"),
                &format!("+{WIPOUT}*:{INWIP}*"),
            ])?;
        }
        if let (Some(base), false) = (o.base_bundle, o.archive) {
            std::fs::create_dir_all(&cache.dir)?;
            if base != cache.base_bundle() {
                std::fs::copy(base, cache.base_bundle())?;
            }
            local.base_tips = bundle_heads(&git, base)?;
        }
        if fresh {
            if let Some(url) = &o.man.origin_url {
                if !url.ends_with(".bundle") {
                    let _ = git.run(&[
                        "remote",
                        "add",
                        o.man.via.as_deref().unwrap_or("origin"),
                        url,
                    ]);
                }
            }
        }
    }
    // Translate rebuilt history back onto the real one where we own it.
    let mut map = o.map.clone().unwrap_or_else(|| cache.load_map());
    if let Some(m) = &o.map {
        // Merge the snapshot's map with ours (ours may know newer local pairs).
        let mut merged = cache.load_map();
        for (r, b) in &m.pairs {
            merged.push(r, b);
        }
        map = merged;
    }
    let depth = workroam_core::config::parse_history(&o.man.history);
    let real_side =
        depth.is_some() && !fresh && map.pairs.iter().rev().take(50).any(|(r, _)| git.exists(r));
    let safe_worktree = fresh || {
        let cur = worktree_tree(&git)?;
        cur.is_none() || cur == local.last_tree || cur == git.tree_of("HEAD")
    };
    let current = git.current_branch();
    let mut new_transported = BTreeMap::new();
    for b in o.man.branches.keys() {
        let Some(incoming) = git.rev_parse(&format!("{IN}{b}")) else {
            continue;
        };
        new_transported.insert(b.clone(), incoming.clone());
        let target = if real_side {
            translate(&git, &incoming, &mut map)?
        } else {
            incoming.clone()
        };
        let local_tip = git.rev_parse(&format!("refs/heads/{b}"));
        let action = match &local_tip {
            None => Some(target.clone()),
            Some(l) if *l == target => None,
            Some(l) if git.is_ancestor(l, &target) => Some(target.clone()),
            Some(l) if git.is_ancestor(&target, l) => None,
            Some(l) if !o.archive && local.transported.get(b).map(|t| t == l).unwrap_or(false) => {
                Some(target.clone())
            }
            Some(_) => {
                let park = if o.archive {
                    format!("workroam/archive/{b}")
                } else {
                    format!("workroam/incoming/{b}")
                };
                git.update_ref(&format!("refs/heads/{park}"), &target)?;
                out.conflicts.push(format!("{}: {b} 与本地分叉，已放在分支 {park}，请用 git 合并 / diverged; incoming commits are on {park}", o.name));
                None
            }
        };
        if let Some(t) = action {
            if Some(b) == current.as_ref() && !fresh {
                if safe_worktree {
                    git.run(&["reset", "--quiet", "--hard", &t])?;
                } else if git.run(&["merge", "--quiet", "--ff-only", &t]).is_err() {
                    let park = format!("workroam/incoming/{b}");
                    git.update_ref(&format!("refs/heads/{park}"), &t)?;
                    out.conflicts.push(format!("{}: 本地有未同步的改动，新提交放在分支 {park} / local changes; incoming commits are on {park}", o.name));
                    continue;
                }
            } else {
                git.update_ref(&format!("refs/heads/{b}"), &t)?;
            }
            out.notes
                .push(format!("{}: {b} → {}", o.name, &t[..12.min(t.len())]));
        }
    }
    // Check out the pusher's branch and restore the WIP.
    let head_branch = o
        .man
        .wip
        .as_ref()
        .map(|w| w.branch.clone())
        .or_else(|| o.man.head_branch.clone());
    if let Some(hb) = &head_branch {
        if (fresh || safe_worktree) && git.rev_parse(&format!("refs/heads/{hb}")).is_some() {
            if git.current_branch().as_deref() != Some(hb.as_str()) {
                git.run(&["checkout", "--quiet", "-f", hb])?;
            } else if fresh {
                git.run(&["reset", "--quiet", "--hard", "HEAD"])?;
            }
        }
    }
    if let Some(w) = &o.man.wip {
        if let Some(wip) = git.rev_parse(&format!("{INWIP}{}", w.branch)) {
            let on_branch = git.current_branch().as_deref() == Some(w.branch.as_str());
            let tree_now = worktree_tree(&git)?;
            let clean_now = tree_now == git.tree_of("HEAD") || tree_now == local.last_tree || fresh;
            if on_branch && clean_now && !o.archive {
                git.run(&["reset", "--quiet", "--hard", "HEAD"])?;
                restore_wip(&git, &wip)?;
                local.last_tree = git.tree_of(&wip);
            } else if on_branch && clean_now && o.archive && tree_now == git.tree_of("HEAD") {
                restore_wip(&git, &wip)?;
            } else {
                let park = if o.archive {
                    "workroam/archive-wip".to_string()
                } else {
                    format!("workroam/incoming-wip/{}", w.branch)
                };
                git.update_ref(&format!("refs/heads/{park}"), &wip)?;
                out.conflicts.push(format!(
                    "{}: 未提交的改动无法直接还原，已保存为分支 {park} / WIP saved on {park}",
                    o.name
                ));
            }
        }
    } else if safe_worktree && !o.archive {
        local.last_tree = git.tree_of("HEAD");
    }
    if !o.archive {
        local.transported = new_transported;
        cache.save_local(&local)?;
        if depth.is_some() {
            cache.save_map(&map)?;
        }
    }
    Ok(out)
}

/// Translate an incoming rebuilt tip onto the real history (§11.2 step 4).
fn translate(git: &Git, incoming: &str, map: &mut RebuildMap) -> Result<String> {
    if let Some(r) = map.real_of(incoming) {
        return Ok(r.to_string());
    }
    let walk = git.run(&["rev-list", "--first-parent", incoming])?;
    let mut new_commits = Vec::new();
    let mut anchor = None;
    for c in walk.lines() {
        if let Some(r) = map.real_of(c) {
            anchor = Some(r.to_string());
            break;
        }
        new_commits.push(c.to_string());
    }
    let Some(mut cur) = anchor else {
        // Unrelated history: keep it as is.
        return Ok(incoming.to_string());
    };
    new_commits.reverse();
    for h in new_commits {
        let info = git.commit_info(&h)?;
        let real = git.recommit(&info, &[cur.clone()])?;
        map.push(&real, &h);
        cur = real;
    }
    Ok(cur)
}

fn bundle_heads(git: &Git, bundle: &Path) -> Result<Vec<String>> {
    let s = git.run(&["bundle", "list-heads", &bundle.to_string_lossy()])?;
    Ok(s.lines()
        .filter_map(|l| l.split_whitespace().next())
        .map(str::to_string)
        .collect())
}

/// Read-only shadow repository holding the files-mode baseline (GIT-07).
pub fn write_shadow(project: &Path, name: &str, repo_rel: &str) -> Result<PathBuf> {
    let shadow = project.join(".workroam/local/shadow").join(name);
    let work = workroam_core::paths::from_snapshot_path(project, repo_rel);
    std::fs::create_dir_all(&shadow)?;
    let gd = shadow.join(".git");
    let gds = gd.to_string_lossy().to_string();
    let wts = work.to_string_lossy().to_string();
    let g = Git::new(&shadow);
    if !gd.exists() {
        g.run(&["init", "--quiet", "--bare", &gds])?;
    }
    let env = [("GIT_DIR", gds.as_str()), ("GIT_WORK_TREE", wts.as_str())];
    g.run_env(&["add", "-A"], &env)?;
    let _ = g.run_env(
        &[
            "-c",
            "user.name=Workroam",
            "-c",
            "user.email=shadow@workroam.invalid",
            "commit",
            "--quiet",
            "--allow-empty",
            "-m",
            "workroam baseline (read-only shadow, do not commit)",
        ],
        &env,
    );
    Ok(shadow)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git_in(dir: &Path, args: &[&str]) -> String {
        Git::new(dir)
            .run_env(
                args,
                &[
                    ("GIT_AUTHOR_NAME", "T"),
                    ("GIT_AUTHOR_EMAIL", "t@example.com"),
                    ("GIT_COMMITTER_NAME", "T"),
                    ("GIT_COMMITTER_EMAIL", "t@example.com"),
                ],
            )
            .unwrap()
    }

    fn commit(dir: &Path, file: &str, content: &str, msg: &str) -> String {
        std::fs::write(dir.join(file), content).unwrap();
        git_in(dir, &["add", "-A"]);
        git_in(dir, &["commit", "-q", "-m", msg]);
        Git::new(dir).head().unwrap()
    }

    fn setup_repo(project: &Path) -> PathBuf {
        let r = project.join("repo-a");
        std::fs::create_dir_all(&r).unwrap();
        git_in(&r, &["init", "-q", "-b", "main"]);
        r
    }

    /// Simulate a push from `from` and a pull into `to` (copying bundle files).
    fn transfer(from: &Path, to: &Path, cfg: &RepoConfig) -> ImportOutcome {
        let stage = from.join(".workroam/local/stage");
        let ex = export(&ExportOptions {
            name: "repo-a",
            project: from,
            cfg,
            stage: &stage,
            archive: false,
        })
        .unwrap();
        let inbox = to.join(".workroam/local/inbox");
        std::fs::create_dir_all(&inbox).unwrap();
        let mut base = None;
        let mut incr = None;
        let mut map = None;
        for (sp, lp) in &ex.files {
            let dest = inbox.join(sp.rsplit('/').next().unwrap());
            std::fs::copy(lp, &dest).unwrap();
            if sp.ends_with("base.bundle") {
                base = Some(dest);
            } else if sp.ends_with("incr.bundle") {
                incr = Some(dest);
            } else {
                map = Some(serde_json::from_slice(&std::fs::read(&dest).unwrap()).unwrap());
            }
        }
        import(&ImportOptions {
            name: "repo-a",
            project: to,
            man: &ex.manifest,
            base_bundle: base.as_deref(),
            incr_bundle: incr.as_deref(),
            map,
            archive: false,
        })
        .unwrap()
    }

    #[test]
    fn full_history_roundtrip_with_wip() {
        let d = tempfile::tempdir().unwrap();
        let office = d.path().join("office");
        let home = d.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let r = setup_repo(&office);
        commit(&r, "a.txt", "1", "c1");
        let c2 = commit(&r, "a.txt", "2", "c2");
        std::fs::write(r.join("a.txt"), "2-wip").unwrap();
        std::fs::write(r.join("new.txt"), "untracked").unwrap();
        let cfg = RepoConfig {
            path: "repo-a".into(),
            mode: RepoMode::Bundle,
            history: "full".into(),
            ..Default::default()
        };
        let out = transfer(&office, &home, &cfg);
        assert!(out.conflicts.is_empty(), "{:?}", out.conflicts);
        let h = home.join("repo-a");
        assert_eq!(Git::new(&h).head().unwrap(), c2);
        assert_eq!(std::fs::read_to_string(h.join("a.txt")).unwrap(), "2-wip");
        assert_eq!(
            std::fs::read_to_string(h.join("new.txt")).unwrap(),
            "untracked"
        );
        // The user's staging area at office was not touched.
        assert!(Git::new(&r)
            .run(&["diff", "--cached", "--name-only"])
            .unwrap()
            .is_empty());
        // Home commits, pushes back; office fast-forwards (its WIP was already transported).
        git_in(&h, &["add", "-A"]);
        git_in(&h, &["commit", "-q", "-m", "home work"]);
        let c3 = Git::new(&h).head().unwrap();
        let out = transfer(&home, &office, &cfg);
        assert!(out.conflicts.is_empty(), "{:?}", out.conflicts);
        assert_eq!(Git::new(&r).head().unwrap(), c3);
        assert!(!Git::new(&r).is_dirty());
    }

    #[test]
    fn recent_history_rebuild_and_translate_back() {
        let d = tempfile::tempdir().unwrap();
        let office = d.path().join("office");
        let home = d.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let r = setup_repo(&office);
        for i in 0..5 {
            commit(&r, "f.txt", &i.to_string(), &format!("c{i}"));
        }
        let real_tip = Git::new(&r).head().unwrap();
        let cfg = RepoConfig {
            path: "repo-a".into(),
            mode: RepoMode::Bundle,
            history: "recent:2".into(),
            ..Default::default()
        };
        transfer(&office, &home, &cfg);
        let h = home.join("repo-a");
        let hg = Git::new(&h);
        assert_eq!(hg.count(&["HEAD"]), 2, "home sees only the last 2 commits");
        assert_eq!(hg.tree_of("HEAD"), Git::new(&r).tree_of("HEAD"));
        assert_ne!(hg.head().unwrap(), real_tip);
        // home commits twice and pushes back
        commit(&h, "f.txt", "home1", "h1");
        commit(&h, "g.txt", "home2", "h2");
        let home_tree = hg.tree_of("HEAD");
        let out = transfer(&home, &office, &cfg);
        assert!(out.conflicts.is_empty(), "{:?}", out.conflicts);
        let og = Git::new(&r);
        assert_eq!(og.tree_of("HEAD"), home_tree);
        assert!(
            og.is_ancestor(&real_tip, "HEAD"),
            "home commits were rebased onto the real history"
        );
        assert_eq!(og.count(&["HEAD"]), 7);
        // office commits again; home gets an incremental rebuilt commit on top of h2
        commit(&r, "f.txt", "office-again", "o6");
        transfer(&office, &home, &cfg);
        assert_eq!(hg.count(&["HEAD"]), 5);
        assert_eq!(
            std::fs::read_to_string(h.join("f.txt")).unwrap(),
            "office-again"
        );
    }

    #[test]
    fn divergence_is_parked_on_a_branch() {
        let d = tempfile::tempdir().unwrap();
        let office = d.path().join("office");
        let home = d.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let r = setup_repo(&office);
        commit(&r, "a.txt", "1", "c1");
        let cfg = RepoConfig {
            path: "repo-a".into(),
            mode: RepoMode::Bundle,
            history: "full".into(),
            ..Default::default()
        };
        transfer(&office, &home, &cfg);
        let h = home.join("repo-a");
        commit(&h, "a.txt", "home", "home");
        commit(&r, "a.txt", "office", "office");
        let out = transfer(&home, &office, &cfg);
        assert_eq!(out.conflicts.len(), 1);
        assert!(Git::new(&r)
            .rev_parse("refs/heads/workroam/incoming/main")
            .is_some());
        assert_eq!(std::fs::read_to_string(r.join("a.txt")).unwrap(), "office");
    }

    #[test]
    fn rebase_in_progress_blocks_export() {
        let d = tempfile::tempdir().unwrap();
        let r = setup_repo(d.path());
        commit(&r, "a.txt", "1", "c1");
        std::fs::create_dir_all(r.join(".git/rebase-merge")).unwrap();
        let cfg = RepoConfig {
            path: "repo-a".into(),
            mode: RepoMode::Bundle,
            ..Default::default()
        };
        let stage = d.path().join("stage");
        let e = export(&ExportOptions {
            name: "repo-a",
            project: d.path(),
            cfg: &cfg,
            stage: &stage,
            archive: false,
        })
        .err()
        .unwrap();
        assert_eq!(e.code, Code::GitInProgress);
    }
}
