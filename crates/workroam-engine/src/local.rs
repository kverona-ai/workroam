//! Per-device project state under `.workroam/local/` (never synced):
//! binding, last synced manifest, hash cache and the operation log.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use workroam_core::model::FileMeta;
use workroam_core::paths::write_atomic;
use workroam_core::Result;

pub const LOCAL_DIR: &str = ".workroam/local";
pub const SYNCING: &str = ".workroam/SYNCING";

pub fn local_dir(project: &Path) -> PathBuf {
    project.join(LOCAL_DIR)
}

fn load<T: for<'de> Deserialize<'de> + Default>(p: &Path) -> T {
    std::fs::read(p)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// `.workroam/local/binding` (architecture §6.7).
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct Binding {
    pub project_id: String,
    pub project: String,
    pub device_id: String,
    pub path: String,
    pub bound_at: Option<DateTime<Utc>>,
}

impl Binding {
    pub fn path(project: &Path) -> PathBuf {
        local_dir(project).join("binding")
    }
    pub fn load(project: &Path) -> Option<Binding> {
        std::fs::read(Self::path(project))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
    }
    pub fn save(&self, project: &Path) -> Result<()> {
        write_atomic(&Self::path(project), &serde_json::to_vec_pretty(self)?)
    }
}

/// Last synchronised state: the common ancestor for the next pull and the
/// parent for the next push.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SyncState {
    pub base_snapshot: Option<String>,
    pub base_files: BTreeMap<String, FileMeta>,
    pub last_push: Option<DateTime<Utc>>,
    pub last_pull: Option<DateTime<Utc>>,
    /// HANDOFF.md hash at the last sync (to report `handoff_updated`).
    pub handoff_sha: Option<String>,
    /// Hash of the repository manifests at the last sync (detects git-only changes).
    #[serde(default)]
    pub repos_sha: Option<String>,
}

impl SyncState {
    pub fn path(project: &Path) -> PathBuf {
        local_dir(project).join("state.json")
    }
    pub fn load(project: &Path) -> SyncState {
        load(&Self::path(project))
    }
    pub fn save(&self, project: &Path) -> Result<()> {
        write_atomic(&Self::path(project), &serde_json::to_vec(self)?)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HashEntry {
    pub size: u64,
    pub mtime_ns: i128,
    pub sha256: String,
}

/// size + mtime → sha256, so unchanged files are not re-read.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HashCache {
    pub entries: BTreeMap<String, HashEntry>,
}

impl HashCache {
    pub fn path(project: &Path) -> PathBuf {
        local_dir(project).join("hashcache.json")
    }
    pub fn load(project: &Path) -> HashCache {
        load(&Self::path(project))
    }
    pub fn save(&self, project: &Path) -> Result<()> {
        write_atomic(&Self::path(project), &serde_json::to_vec(self)?)
    }
}

pub fn mtime_ns(m: &std::fs::Metadata) -> i128 {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as i128)
        .unwrap_or(0)
}

/// Operation log of a pull in progress (architecture §9.3): replayed after a crash.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum OpEntry {
    Begin { snapshot: String, kind: String },
    Done { path: String },
    Finish,
}

pub struct OpLog {
    path: PathBuf,
}

impl OpLog {
    pub fn new(project: &Path) -> OpLog {
        OpLog {
            path: local_dir(project).join("oplog"),
        }
    }
    pub fn begin(&self, snapshot: &str, kind: &str) -> Result<()> {
        if let Some(p) = self.path.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::fs::write(&self.path, "")?;
        self.append(&OpEntry::Begin {
            snapshot: snapshot.into(),
            kind: kind.into(),
        })
    }
    pub fn append(&self, e: &OpEntry) -> Result<()> {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(f, "{}", serde_json::to_string(e)?)?;
        Ok(())
    }
    pub fn done(&self, path: &str) -> Result<()> {
        self.append(&OpEntry::Done { path: path.into() })
    }
    pub fn read(&self) -> Vec<OpEntry> {
        std::fs::read_to_string(&self.path)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }
    /// The unfinished operation, if a previous run was interrupted.
    pub fn pending(&self) -> Option<(String, String)> {
        let entries = self.read();
        if entries.contains(&OpEntry::Finish) {
            return None;
        }
        entries.iter().find_map(|e| match e {
            OpEntry::Begin { snapshot, kind } => Some((snapshot.clone(), kind.clone())),
            _ => None,
        })
    }
    pub fn clear(&self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// `.workroam/SYNCING` marker (agents pause while it exists).
pub struct SyncingGuard {
    path: PathBuf,
}

impl SyncingGuard {
    pub fn new(project: &Path, what: &str) -> Result<SyncingGuard> {
        let path = project.join(SYNCING);
        write_atomic(
            &path,
            format!("{what} {}\n", Utc::now().to_rfc3339()).as_bytes(),
        )?;
        Ok(SyncingGuard { path })
    }
}

impl Drop for SyncingGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
