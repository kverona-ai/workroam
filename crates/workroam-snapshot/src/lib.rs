//! Snapshot engine (ADR-0003). Business code depends on the
//! [`SnapshotEngine`] trait only; [`rustic::RusticEngine`] implements it on
//! top of rustic_core, so a workspace stays readable with restic tools.

pub mod rustic;

use chrono::{DateTime, Utc};
use std::path::PathBuf;
use std::sync::Arc;
use workroam_core::Result;

/// Where file content comes from when building a snapshot.
#[derive(Debug, Clone)]
pub enum EntrySource {
    Path(PathBuf),
    Bytes(Vec<u8>),
}

#[derive(Debug, Clone)]
pub enum EntryKind {
    File {
        source: EntrySource,
        size: u64,
        exec: bool,
    },
    Symlink {
        target: String,
    },
}

/// One file in a snapshot, addressed by its `/` separated relative path.
/// Parent directories are created implicitly.
#[derive(Debug, Clone)]
pub struct SourceEntry {
    pub path: String,
    pub kind: EntryKind,
    pub mtime: Option<DateTime<Utc>>,
}

impl SourceEntry {
    pub fn bytes(path: impl Into<String>, data: Vec<u8>) -> SourceEntry {
        let size = data.len() as u64;
        SourceEntry {
            path: path.into(),
            kind: EntryKind::File {
                source: EntrySource::Bytes(data),
                size,
                exec: false,
            },
            mtime: Some(Utc::now()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SnapshotMeta {
    pub id: String,
    pub time: DateTime<Utc>,
    pub hostname: String,
    pub tags: Vec<String>,
    pub label: String,
    pub description: String,
    pub parent: Option<String>,
}

impl SnapshotMeta {
    pub fn has_tag(&self, t: &str) -> bool {
        self.tags.iter().any(|x| x == t)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeKind {
    File { size: u64, exec: bool },
    Dir,
    Symlink { target: String },
}

#[derive(Debug, Clone)]
pub struct TreeEntry {
    pub path: String,
    pub kind: NodeKind,
}

#[derive(Debug, Clone, Default)]
pub struct CheckReport {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

/// Progress callback: (bytes done, bytes total if known).
pub type ProgressFn = Arc<dyn Fn(u64, Option<u64>) + Send + Sync>;

/// Which kind of repository key a key file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyRole {
    Password,
    Recovery,
    Other,
}

#[derive(Debug, Clone)]
pub struct KeyInfo {
    pub id: String,
    pub role: KeyRole,
    pub created: Option<DateTime<Utc>>,
}

pub trait SnapshotEngine: Send + Sync {
    /// Secret material of the master key (input for the control sub key).
    fn master_key_material(&self) -> Vec<u8>;
    fn repo_id(&self) -> String;

    fn list(&self) -> Result<Vec<SnapshotMeta>>;
    fn get(&self, id: &str) -> Result<SnapshotMeta>;
    /// Create a snapshot from `entries`. Returns the snapshot id.
    fn backup(
        &self,
        entries: Vec<SourceEntry>,
        tags: &[String],
        parent: Option<&str>,
        hostname: &str,
        label: &str,
        description: &str,
        progress: Option<ProgressFn>,
    ) -> Result<String>;
    fn read_file(&self, snapshot: &str, path: &str) -> Result<Option<Vec<u8>>>;
    fn tree(&self, snapshot: &str) -> Result<Vec<TreeEntry>>;
    /// Read many files in one pass (avoids reopening the index per file).
    fn read_files(
        &self,
        snapshot: &str,
        paths: &[String],
        each: &mut dyn FnMut(&str, Vec<u8>) -> Result<()>,
    ) -> Result<()>;
    fn forget(&self, ids: &[String]) -> Result<()>;
    /// Replace the tags of a snapshot; returns the new snapshot id.
    fn retag(&self, id: &str, add: &[String], remove: &[String]) -> Result<String>;
    fn prune(&self) -> Result<()>;
    fn check(&self, read_data_percent: Option<f64>) -> Result<CheckReport>;

    /// Copy snapshots into a new standalone repository at `dir` (archive packs).
    fn export_to(&self, dir: &std::path::Path, password: &str, ids: &[String]) -> Result<()>;
    /// Copy every snapshot of the standalone repository at `dir` into this one; returns their ids.
    fn import_from(&self, dir: &std::path::Path, password: &str) -> Result<Vec<String>>;

    fn keys(&self) -> Result<Vec<KeyInfo>>;
    fn add_key(&self, secret: &str, role: KeyRole) -> Result<String>;
    fn delete_key(&self, id: &str) -> Result<()>;
    fn current_key(&self) -> Option<String>;
}
