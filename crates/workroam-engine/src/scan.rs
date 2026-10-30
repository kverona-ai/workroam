//! Collecting the synced file list of a project (PUSH-03): exclusions,
//! generated files, repositories and the hash cache.

use crate::local::{mtime_ns, HashCache, HashEntry};
use globset::{Glob, GlobSet, GlobSetBuilder};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use workroam_core::config::{ProjectConfig, DEFAULT_EXCLUDES};
use workroam_core::model::{FileMeta, RepoMode};
use workroam_core::Result;

/// Paths that never leave the machine.
pub const ALWAYS_EXCLUDED: &[&str] = &[
    ".workroam/local/",
    ".workroam/SYNCING",
    ".workroam-snapshot/",
    ".git/",
];

pub struct Excludes {
    dirs: GlobSet,
    anchored: GlobSet,
    names: GlobSet,
    exact: BTreeSet<String>,
    prefixes: Vec<String>,
}

fn build(globs: &[String]) -> GlobSet {
    let mut b = GlobSetBuilder::new();
    for g in globs {
        if let Ok(x) = Glob::new(g) {
            b.add(x);
        }
    }
    b.build().unwrap_or_else(|_| GlobSet::empty())
}

impl Excludes {
    /// `patterns` use a gitignore-like subset: `dir/` matches a directory at any
    /// depth, `a/b*` is anchored at the project root, `*.log` matches names.
    pub fn new(
        patterns: &[String],
        exact: impl IntoIterator<Item = String>,
        prefixes: Vec<String>,
    ) -> Excludes {
        let (mut dirs, mut anchored, mut names) = (vec![], vec![], vec![]);
        for p in patterns {
            let p = p.trim();
            if p.is_empty() || p.starts_with('#') {
                continue;
            }
            if let Some(d) = p.strip_suffix('/') {
                if d.contains('/') {
                    anchored.push(format!("{}/**", d.trim_start_matches('/')));
                } else {
                    dirs.push(d.to_string());
                }
            } else if p.contains('/') {
                let a = p.trim_start_matches('/');
                anchored.push(a.to_string());
                anchored.push(format!("{a}/**"));
            } else {
                names.push(p.to_string());
            }
        }
        Excludes {
            dirs: build(&dirs),
            anchored: build(&anchored),
            names: build(&names),
            exact: exact.into_iter().collect(),
            prefixes,
        }
    }

    pub fn is_excluded(&self, rel: &str, is_dir: bool) -> bool {
        if self.exact.contains(rel) {
            return true;
        }
        let with_slash = if is_dir {
            format!("{rel}/")
        } else {
            rel.to_string()
        };
        if self
            .prefixes
            .iter()
            .any(|p| with_slash.starts_with(p.as_str()) || rel == p.trim_end_matches('/'))
        {
            return true;
        }
        if self.anchored.is_match(rel) {
            return true;
        }
        let comps: Vec<&str> = rel.split('/').collect();
        let last = comps.last().copied().unwrap_or_default();
        if self.names.is_match(last) {
            return true;
        }
        let dir_comps = if is_dir {
            &comps[..]
        } else {
            &comps[..comps.len().saturating_sub(1)]
        };
        dir_comps.iter().any(|c| self.dirs.is_match(c))
    }
}

/// Everything the scanner needs to know about a project.
pub struct ScanSpec<'a> {
    pub project: &'a Path,
    pub cfg: &'a ProjectConfig,
    /// Generated per-machine files (adapters) and other exact exclusions.
    pub generated: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct LocalFile {
    pub meta: FileMeta,
    pub abs: PathBuf,
    /// Modification time on disk; snapshots keep it so unchanged files are
    /// recognised by the previous snapshot and not re-read.
    pub mtime: Option<chrono::DateTime<chrono::Utc>>,
}

/// Repository working trees carried by git (bundle / remote) rather than as files.
pub fn repo_prefixes(project: &Path, cfg: &ProjectConfig) -> (Vec<String>, Vec<String>) {
    let mut skip = Vec::new();
    let mut extras = Vec::new();
    for r in cfg.repos.values() {
        let p = r.path.trim_matches('/').to_string();
        if r.mode == RepoMode::Files {
            skip.push(format!("{p}/.git/"));
            continue;
        }
        if !workroam_git::cmd::is_repo(&workroam_core::paths::from_snapshot_path(project, &p)) {
            continue;
        }
        skip.push(format!("{p}/"));
        for e in &r.extras {
            extras.push(format!("{p}/{}", e.trim_start_matches('/')));
        }
    }
    (skip, extras)
}

/// Walk the project and hash every synced file.
pub fn scan(spec: &ScanSpec) -> Result<BTreeMap<String, LocalFile>> {
    let project = spec.project;
    let mut patterns: Vec<String> = DEFAULT_EXCLUDES.iter().map(|s| s.to_string()).collect();
    patterns.extend(spec.cfg.sync.exclude.iter().cloned());
    let (repo_skip, extras) = repo_prefixes(project, spec.cfg);
    let mut prefixes: Vec<String> = ALWAYS_EXCLUDED.iter().map(|s| s.to_string()).collect();
    prefixes.extend(repo_skip.iter().cloned());
    let ex = Excludes::new(&patterns, spec.generated.iter().cloned(), prefixes);
    let extras_ex = Excludes::new(&patterns, spec.generated.iter().cloned(), vec![]);
    let mut cache = HashCache::load(project);
    let mut seen = BTreeMap::new();
    let mut out = BTreeMap::new();
    let mut walk = |root: &Path, use_extras: bool| -> Result<()> {
        if !root.exists() {
            return Ok(());
        }
        let it = walkdir::WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                let Some(rel) = workroam_core::paths::to_snapshot_path(project, e.path()) else {
                    return true;
                };
                if rel.is_empty() {
                    return true;
                }
                let is_dir =
                    e.file_type().is_dir() && !workroam_platform::fsutil::is_junction(e.path());
                if rel.ends_with("/.git") || rel == ".git" {
                    return false;
                }
                if use_extras {
                    !extras_ex.is_excluded(&rel, is_dir) && !rel.split('/').any(|c| c == ".git")
                } else {
                    !ex.is_excluded(&rel, is_dir)
                }
            });
        for e in it {
            let e = match e {
                Ok(e) => e,
                Err(err) => {
                    tracing::warn!("scan: {err}");
                    continue;
                }
            };
            let ft = e.file_type();
            if ft.is_dir() && !workroam_platform::fsutil::is_junction(e.path()) {
                continue;
            }
            let Some(rel) = workroam_core::paths::to_snapshot_path(project, e.path()) else {
                continue;
            };
            if rel.is_empty() {
                continue;
            }
            let lmeta = std::fs::symlink_metadata(e.path())?;
            let meta = if lmeta.file_type().is_symlink()
                || workroam_platform::fsutil::is_junction(e.path())
            {
                let target = std::fs::read_link(e.path())
                    .map(|t| t.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_default();
                FileMeta {
                    sha256: workroam_core::crypto::sha256_hex(target.as_bytes()),
                    size: 0,
                    exec: false,
                    symlink: Some(target),
                }
            } else {
                let size = lmeta.len();
                let mt = mtime_ns(&lmeta);
                let sha = match cache.entries.get(&rel) {
                    Some(c) if c.size == size && c.mtime_ns == mt && mt != 0 => c.sha256.clone(),
                    _ => {
                        let Some(h) = workroam_platform::fsutil::file_sha256(e.path())? else {
                            continue;
                        };
                        cache.entries.insert(
                            rel.clone(),
                            HashEntry {
                                size,
                                mtime_ns: mt,
                                sha256: h.clone(),
                            },
                        );
                        h
                    }
                };
                FileMeta {
                    sha256: sha,
                    size,
                    exec: workroam_platform::fsutil::is_exec(&lmeta),
                    symlink: None,
                }
            };
            seen.insert(rel.clone(), ());
            out.insert(
                rel,
                LocalFile {
                    meta,
                    abs: e.path().to_path_buf(),
                    mtime: lmeta.modified().ok().map(chrono::DateTime::from),
                },
            );
        }
        Ok(())
    };
    walk(project, false)?;
    for x in &extras {
        walk(&workroam_core::paths::from_snapshot_path(project, x), true)?;
    }
    cache.entries.retain(|k, _| seen.contains_key(k));
    cache.save(project)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exclude_rules() {
        let ex = Excludes::new(
            &[
                "node_modules/".into(),
                "*.log".into(),
                "docs/build/".into(),
                "/secret.txt".into(),
            ],
            ["CLAUDE.md".to_string()],
            vec![".workroam/local/".into(), "repo-a/".into()],
        );
        assert!(ex.is_excluded("a/node_modules", true));
        assert!(ex.is_excluded("a/node_modules/x.js", false));
        assert!(!ex.is_excluded("node_modules.txt", false));
        assert!(ex.is_excluded("x/y.log", false));
        assert!(ex.is_excluded("docs/build/a.html", false));
        assert!(!ex.is_excluded("x/docs/build/a.html", false));
        assert!(ex.is_excluded("secret.txt", false));
        assert!(ex.is_excluded("CLAUDE.md", false));
        assert!(ex.is_excluded(".workroam/local/state.json", false));
        assert!(ex.is_excluded("repo-a", true));
        assert!(!ex.is_excluded(".workroam/workroam.toml", false));
    }
}
