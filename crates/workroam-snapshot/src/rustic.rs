//! [`SnapshotEngine`] on top of rustic_core.
//!
//! The repository lives at `<prefix>/repo/` (architecture §5). Workroam
//! builds the file list itself (excludes, stripped `AGENTS.md`, virtual
//! files under `.workroam-snapshot/`) and feeds it through a custom
//! `ReadSource`, so the snapshot holds exactly what the manifest describes.

use crate::{
    CheckReport, EntryKind, EntrySource, KeyInfo, KeyRole, NodeKind, ProgressFn, SnapshotEngine,
    SnapshotMeta, SourceEntry, TreeEntry,
};
use chrono::{DateTime, TimeZone, Utc};
use rustic_backend::BackendOptions;
use rustic_core::repofile::{
    KeyFile, KeyId, MasterKey, Metadata, Node, NodeType, SnapshotFile, SnapshotId, StringList,
};
use rustic_core::{
    BackupOptions, CheckOptions, ConfigOptions, Credentials, FileType, Id, KeyOptions, LsOptions,
    OpenStatus, Progress, ProgressBars, ProgressType, PruneOptions, ReadSource, ReadSourceEntry,
    ReadSourceOpen, ReadSubsetOption, Repository, RepositoryBackends, RepositoryOptions,
    RusticError, RusticProgress, RusticResult, SnapshotOptions,
};
use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::io::Read;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use workroam_core::config::WorkspaceLocation;
use workroam_core::{Code, Error, Result};
use workroam_store::Creds;

const USER_PASSWORD: &str = "workroam-password";
const USER_RECOVERY: &str = "workroam-recovery";

fn rerr(e: Box<RusticError>) -> Error {
    let msg = e.to_string();
    if msg.contains("No suitable key found") || msg.contains("seems to be incorrect") {
        return Error::new(
            Code::InvalidArgs,
            "wrong encryption password or recovery key",
        )
        .with_hint("check the password, or use the recovery key");
    }
    if msg.contains("backend")
        || msg.contains("Backend")
        || msg.contains("connection")
        || msg.contains("timed out")
    {
        return Error::network(format!("snapshot storage: {msg}"));
    }
    Error::internal(format!("snapshot engine: {msg}"))
}

fn ts(t: &jiff::Zoned) -> DateTime<Utc> {
    Utc.timestamp_opt(
        t.timestamp().as_second(),
        t.timestamp().subsec_nanosecond() as u32,
    )
    .single()
    .unwrap_or_else(Utc::now)
}

/// Backends for the repository of a workspace.
pub fn backends(loc: &WorkspaceLocation, creds: Option<&Creds>) -> Result<RepositoryBackends> {
    backends_at(loc, creds, "repo")
}

pub fn backends_at(
    loc: &WorkspaceLocation,
    creds: Option<&Creds>,
    sub: &str,
) -> Result<RepositoryBackends> {
    let mut bo = BackendOptions::default();
    if let Some(root) = loc.local_root() {
        let dir = root.join(sub);
        std::fs::create_dir_all(&dir)?;
        bo.repository = Some(format!("local:{}", dir.display()));
    } else {
        let creds = creds.ok_or_else(|| Error::invalid("storage credentials are required"))?;
        bo.repository = Some("opendal:s3".into());
        let mut o = std::collections::BTreeMap::new();
        o.insert("bucket".into(), loc.bucket.clone());
        o.insert("root".into(), workroam_store::object::root_for(loc, sub));
        o.insert(
            "endpoint".into(),
            loc.endpoint.trim_end_matches('/').to_string(),
        );
        o.insert(
            "region".into(),
            if loc.region.is_empty() {
                "us-east-1".into()
            } else {
                loc.region.clone()
            },
        );
        o.insert("access_key_id".into(), creds.access_key.clone());
        o.insert("secret_access_key".into(), creds.secret_key.clone());
        o.insert("disable_config_load".into(), "true".into());
        o.insert("disable_ec2_metadata".into(), "true".into());
        if loc.addressing == workroam_core::config::Addressing::VirtualHost {
            o.insert("enable_virtual_host_style".into(), "true".into());
        }
        bo.options = o;
    }
    bo.to_backends().map_err(rerr)
}

/// Shared progress sink: the engine forwards byte counters to whoever listens.
#[derive(Clone, Default)]
struct ProgressHub {
    sink: Arc<Mutex<Option<ProgressFn>>>,
}

impl std::fmt::Debug for ProgressHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProgressHub")
    }
}

#[derive(Debug)]
struct BytesProgress {
    hub: ProgressHub,
    done: AtomicU64,
    total: AtomicU64,
}

impl RusticProgress for BytesProgress {
    fn is_hidden(&self) -> bool {
        false
    }
    fn set_length(&self, len: u64) {
        self.total.store(len, Ordering::Relaxed);
    }
    fn set_title(&self, _title: &str) {}
    fn inc(&self, inc: u64) {
        let done = self.done.fetch_add(inc, Ordering::Relaxed) + inc;
        let total = self.total.load(Ordering::Relaxed);
        if let Some(f) = self.hub.sink.lock().unwrap().as_ref() {
            f(done, if total > 0 { Some(total) } else { None });
        }
    }
    fn finish(&self) {}
}

impl ProgressBars for ProgressHub {
    fn progress(&self, progress_type: ProgressType, _prefix: &str) -> Progress {
        if matches!(progress_type, ProgressType::Bytes) && self.sink.lock().unwrap().is_some() {
            Progress::new(BytesProgress {
                hub: self.clone(),
                done: AtomicU64::new(0),
                total: AtomicU64::new(0),
            })
        } else {
            Progress::hidden()
        }
    }
}

pub struct RusticEngine {
    backends: RepositoryBackends,
    /// Unlocked master key: later opens skip the password KDF.
    master: MasterKey,
    key_id: Option<String>,
    repo_id: String,
    hub: ProgressHub,
}

fn repo_options() -> RepositoryOptions {
    let mut ro = RepositoryOptions::default();
    let cache = workroam_core::paths::workroam_home()
        .join("cache")
        .join("rustic");
    if std::fs::create_dir_all(&cache).is_ok() {
        ro.cache_dir = Some(cache);
    } else {
        ro.no_cache = true;
    }
    ro
}

fn key_opts(role: KeyRole) -> KeyOptions {
    let mut k = KeyOptions::default();
    k.hostname = Some("workroam".into());
    k.username = Some(match role {
        KeyRole::Password => USER_PASSWORD.into(),
        KeyRole::Recovery => USER_RECOVERY.into(),
        KeyRole::Other => "workroam".into(),
    });
    k.with_created = true;
    k
}

fn full(id: &Id) -> String {
    id.to_hex().as_str().to_string()
}

fn parse_id(s: &str) -> Result<Id> {
    Id::from_str(s).map_err(|_| Error::invalid(format!("not a snapshot id: {s}")))
}

impl RusticEngine {
    /// Create a new repository protected by `password`, plus a second key for `recovery`.
    pub fn init(
        backends: RepositoryBackends,
        password: &str,
        recovery: &str,
    ) -> Result<RusticEngine> {
        let hub = ProgressHub::default();
        let repo =
            Repository::new_with_progress(&repo_options(), &backends, hub.clone()).map_err(rerr)?;
        let repo = repo
            .init(
                &Credentials::password(password),
                &key_opts(KeyRole::Password),
                &ConfigOptions::default(),
            )
            .map_err(rerr)?;
        repo.add_key(recovery, &key_opts(KeyRole::Recovery))
            .map_err(rerr)?;
        Ok(Self::from_repo(repo, backends, hub))
    }

    fn from_repo(
        repo: Repository<OpenStatus>,
        backends: RepositoryBackends,
        hub: ProgressHub,
    ) -> RusticEngine {
        RusticEngine {
            master: repo.key(),
            key_id: repo.key_id().as_ref().map(|k| full(k)),
            repo_id: full(&repo.config().id),
            backends,
            hub,
        }
    }

    /// A freshly opened repository (sees snapshots written by other devices).
    fn repo(&self) -> Result<Repository<OpenStatus>> {
        let repo = Repository::new_with_progress(&repo_options(), &self.backends, self.hub.clone())
            .map_err(rerr)?;
        repo.open(&Credentials::Masterkey(self.master.clone()))
            .map_err(rerr)
    }

    /// Open with a password or a recovery key (both are repository keys).
    pub fn open(backends: RepositoryBackends, secret: &str) -> Result<RusticEngine> {
        let hub = ProgressHub::default();
        let repo =
            Repository::new_with_progress(&repo_options(), &backends, hub.clone()).map_err(rerr)?;
        let repo = repo.open(&Credentials::password(secret)).map_err(rerr)?;
        Ok(Self::from_repo(repo, backends, hub))
    }

    /// Does a repository exist at these backends?
    pub fn exists(backends: &RepositoryBackends) -> Result<bool> {
        let be = backends.repository();
        Ok(!be.list(FileType::Config).map_err(rerr)?.is_empty())
    }

    fn snapshot_file(&self, id: &str) -> Result<SnapshotFile> {
        Self::snapshot_in(&self.repo()?, id)
    }

    fn snapshot_in<S: rustic_core::Open>(repo: &Repository<S>, id: &str) -> Result<SnapshotFile> {
        repo.get_snapshot_from_str(id, |_| true).map_err(|e| {
            let m = e.to_string();
            if m.contains("No suitable") || m.contains("not found") || m.contains("No snapshot") {
                Error::new(Code::InvalidArgs, format!("snapshot {id} not found"))
            } else {
                rerr(e)
            }
        })
    }

    fn meta_of(s: &SnapshotFile) -> SnapshotMeta {
        SnapshotMeta {
            id: full(&s.id),
            time: ts(&s.time),
            hostname: s.hostname.clone(),
            tags: s.tags.iter().map(|t| t.to_string()).collect(),
            label: s.label.clone(),
            description: s.description.clone().unwrap_or_default(),
            parent: s.parent.map(|p| full(&p)),
        }
    }

    pub fn copy_to(&self, other: &RusticEngine, ids: &[String]) -> Result<()> {
        let snaps: Vec<SnapshotFile> = ids
            .iter()
            .map(|i| self.snapshot_file(i))
            .collect::<Result<_>>()?;
        let src = self.repo()?.to_indexed().map_err(rerr)?;
        let dest = other.repo()?.to_indexed_ids().map_err(rerr)?;
        src.copy(&dest, snaps.iter()).map_err(rerr)
    }
}

/// Content for one source entry, opened lazily by rustic's archiver.
#[derive(Debug, Clone)]
pub enum OpenEntry {
    Path(PathBuf),
    Bytes(Arc<Vec<u8>>),
}

impl ReadSourceOpen for OpenEntry {
    type Reader = Box<dyn Read + Send>;
    fn open(self) -> RusticResult<Self::Reader> {
        match self {
            OpenEntry::Path(p) => match std::fs::File::open(&p) {
                Ok(f) => Ok(Box::new(f)),
                Err(e) => Err(RusticError::with_source(
                    rustic_core::ErrorKind::InputOutput,
                    "cannot open `{path}`",
                    e,
                )
                .attach_context("path", p.display().to_string())),
            },
            OpenEntry::Bytes(b) => Ok(Box::new(std::io::Cursor::new(b.as_ref().clone()))),
        }
    }
}

struct VecSource {
    entries: Vec<(PathBuf, Node, Option<OpenEntry>)>,
    size: u64,
}

impl ReadSource for VecSource {
    type Open = OpenEntry;
    type Iter = std::vec::IntoIter<RusticResult<ReadSourceEntry<OpenEntry>>>;
    fn size(&self) -> RusticResult<Option<u64>> {
        Ok(Some(self.size))
    }
    fn entries(&self) -> Self::Iter {
        self.entries
            .iter()
            .map(|(p, n, o)| {
                Ok(ReadSourceEntry {
                    path: p.clone(),
                    node: n.clone(),
                    open: o.clone(),
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
    }
}

const GO_MODE_DIR: u32 = 1 << 31;
const GO_MODE_SYMLINK: u32 = 1 << 27;

fn jiff_ts(t: DateTime<Utc>) -> Option<jiff::Timestamp> {
    jiff::Timestamp::new(t.timestamp(), t.timestamp_subsec_nanos() as i32).ok()
}

fn sort_key(p: &str) -> Vec<&str> {
    p.split('/').collect()
}

/// Build the ordered entry list rustic expects: depth first, sorted by name,
/// every directory before its children.
fn build_source(entries: Vec<SourceEntry>) -> Result<VecSource> {
    let now = Utc::now();
    let mut dirs: BTreeSet<String> = BTreeSet::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for e in &entries {
        if e.path.is_empty()
            || e.path.starts_with('/')
            || e.path
                .split('/')
                .any(|c| c.is_empty() || c == "." || c == "..")
        {
            return Err(Error::internal(format!(
                "invalid snapshot path: {:?}",
                e.path
            )));
        }
        if !seen.insert(e.path.clone()) {
            return Err(Error::internal(format!(
                "duplicate snapshot path: {}",
                e.path
            )));
        }
        let mut acc = String::new();
        let parts: Vec<&str> = e.path.split('/').collect();
        for part in &parts[..parts.len() - 1] {
            if !acc.is_empty() {
                acc.push('/');
            }
            acc.push_str(part);
            dirs.insert(acc.clone());
        }
    }
    let mut items: Vec<(String, Option<SourceEntry>)> =
        dirs.into_iter().map(|d| (d, None)).collect();
    items.extend(entries.into_iter().map(|e| (e.path.clone(), Some(e))));
    items.sort_by(|a, b| sort_key(&a.0).cmp(&sort_key(&b.0)));
    let mut out = Vec::with_capacity(items.len());
    let mut size = 0;
    for (path, entry) in items {
        let name = path.rsplit('/').next().unwrap_or(&path).to_string();
        match entry {
            None => {
                let meta = Metadata {
                    mode: Some(GO_MODE_DIR | 0o755),
                    mtime: jiff_ts(now),
                    ..Default::default()
                };
                out.push((
                    PathBuf::from(&path),
                    Node::new_node(OsStr::new(&name), NodeType::Dir, meta),
                    None,
                ));
            }
            Some(e) => {
                let mtime = jiff_ts(e.mtime.unwrap_or(now));
                match e.kind {
                    EntryKind::File {
                        source,
                        size: s,
                        exec,
                    } => {
                        size += s;
                        let meta = Metadata {
                            mode: Some(if exec { 0o755 } else { 0o644 }),
                            mtime,
                            size: s,
                            ..Default::default()
                        };
                        let open = match source {
                            EntrySource::Path(p) => OpenEntry::Path(p),
                            EntrySource::Bytes(b) => OpenEntry::Bytes(Arc::new(b)),
                        };
                        out.push((
                            PathBuf::from(&path),
                            Node::new_node(OsStr::new(&name), NodeType::File, meta),
                            Some(open),
                        ));
                    }
                    EntryKind::Symlink { target } => {
                        let meta = Metadata {
                            mode: Some(GO_MODE_SYMLINK | 0o777),
                            mtime,
                            size: target.len() as u64,
                            ..Default::default()
                        };
                        let nt = NodeType::Symlink {
                            linktarget: target.clone(),
                            linktarget_raw: None,
                        };
                        out.push((
                            PathBuf::from(&path),
                            Node::new_node(OsStr::new(&name), nt, meta),
                            None,
                        ));
                    }
                }
            }
        }
    }
    Ok(VecSource { entries: out, size })
}

impl SnapshotEngine for RusticEngine {
    fn master_key_material(&self) -> Vec<u8> {
        serde_json::to_vec(&self.master).expect("master key serialises")
    }

    fn repo_id(&self) -> String {
        self.repo_id.clone()
    }

    fn list(&self) -> Result<Vec<SnapshotMeta>> {
        let mut v: Vec<SnapshotMeta> = self
            .repo()?
            .get_all_snapshots()
            .map_err(rerr)?
            .iter()
            .map(Self::meta_of)
            .collect();
        v.sort_by_key(|m| m.time);
        Ok(v)
    }

    fn get(&self, id: &str) -> Result<SnapshotMeta> {
        Ok(Self::meta_of(&self.snapshot_file(id)?))
    }

    fn backup(
        &self,
        entries: Vec<SourceEntry>,
        tags: &[String],
        parent: Option<&str>,
        hostname: &str,
        label: &str,
        description: &str,
        progress: Option<ProgressFn>,
    ) -> Result<String> {
        let expected: BTreeSet<String> = entries.iter().map(|e| e.path.clone()).collect();
        let src = build_source(entries)?;
        let mut so = SnapshotOptions::default();
        so.host = Some(hostname.to_string());
        so.label = Some(label.to_string());
        if !description.is_empty() {
            so.description = Some(description.to_string());
        }
        let mut so = so;
        for t in tags {
            so = so.add_tags(t).map_err(rerr)?;
        }
        let snap = so.to_snapshot().map_err(rerr)?;
        let mut bo = BackupOptions::default();
        bo.parent_opts.ignore_ctime = true;
        bo.parent_opts.ignore_inode = true;
        match parent {
            Some(p) => bo.parent_opts.parents = vec![p.to_string()],
            None => bo.parent_opts.force = true,
        }
        *self.hub.sink.lock().unwrap() = progress;
        let result = (|| {
            let repo = self.repo()?.to_indexed_ids().map_err(rerr)?;
            repo.archive(&bo, &src, snap, &[PathBuf::from("/")])
                .map_err(rerr)
        })();
        *self.hub.sink.lock().unwrap() = None;
        let snap = result?;
        let id = full(&snap.id);
        // rustic skips unreadable files with a warning; Workroam refuses such a snapshot.
        let got: BTreeSet<String> = self
            .tree(&id)?
            .into_iter()
            .filter(|e| !matches!(e.kind, NodeKind::Dir))
            .map(|e| e.path)
            .collect();
        let missing: Vec<&String> = expected.difference(&got).collect();
        if !missing.is_empty() {
            if let Err(e) = self.forget(std::slice::from_ref(&id)) {
                tracing::warn!("could not remove incomplete snapshot {id}: {e}");
            }
            return Err(Error::internal(format!(
                "{} file(s) could not be read while creating the snapshot, e.g. {}",
                missing.len(),
                missing[0]
            ))
            .with_hint("close programs that lock these files and push again"));
        }
        Ok(id)
    }

    fn read_file(&self, snapshot: &str, path: &str) -> Result<Option<Vec<u8>>> {
        let mut out = None;
        self.read_files(snapshot, &[path.to_string()], &mut |_, data| {
            out = Some(data);
            Ok(())
        })?;
        Ok(out)
    }

    fn read_files(
        &self,
        snapshot: &str,
        paths: &[String],
        each: &mut dyn FnMut(&str, Vec<u8>) -> Result<()>,
    ) -> Result<()> {
        let repo = self.repo()?.to_indexed().map_err(rerr)?;
        let snap = Self::snapshot_in(&repo, snapshot)?;
        for p in paths {
            let node = match repo.node_from_snapshot_and_path(&snap, p) {
                Ok(n) => n,
                Err(_) => continue,
            };
            if !node.is_file() {
                continue;
            }
            let f = repo.open_file(&node).map_err(rerr)?;
            let size = node.meta.size as usize;
            let mut data = Vec::with_capacity(size);
            let mut off = 0;
            const STEP: usize = 8 * 1024 * 1024;
            while off < size {
                let len = STEP.min(size - off);
                let chunk = repo.read_file_at(&f, off, len).map_err(rerr)?;
                if chunk.is_empty() {
                    break;
                }
                off += chunk.len();
                data.extend_from_slice(&chunk);
            }
            if data.len() != size {
                return Err(Error::new(
                    Code::Corrupted,
                    format!("{p}: read {} of {size} bytes from snapshot", data.len()),
                ));
            }
            each(p, data)?;
        }
        Ok(())
    }

    fn tree(&self, snapshot: &str) -> Result<Vec<TreeEntry>> {
        let repo = self.repo()?.to_indexed().map_err(rerr)?;
        let snap = Self::snapshot_in(&repo, snapshot)?;
        let root = repo.node_from_snapshot_and_path(&snap, "").map_err(rerr)?;
        let mut lo = LsOptions::default();
        lo.recursive = true;
        let mut out = Vec::new();
        for item in repo.ls(&root, &lo).map_err(rerr)? {
            let (path, node) = item.map_err(rerr)?;
            let p = path
                .components()
                .map(|c| c.as_os_str().to_string_lossy().to_string())
                .collect::<Vec<_>>()
                .join("/");
            if p.is_empty() {
                continue;
            }
            let kind = match &node.node_type {
                NodeType::Dir => NodeKind::Dir,
                NodeType::Symlink { .. } => NodeKind::Symlink {
                    target: node.node_type.to_link().to_string_lossy().to_string(),
                },
                _ => NodeKind::File {
                    size: node.meta.size,
                    exec: node.meta.mode.map(|m| m & 0o111 != 0).unwrap_or(false),
                },
            };
            out.push(TreeEntry { path: p, kind });
        }
        Ok(out)
    }

    fn forget(&self, ids: &[String]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let ids: Vec<SnapshotId> = ids
            .iter()
            .map(|i| parse_id(i).map(SnapshotId::from))
            .collect::<Result<_>>()?;
        self.repo()?.delete_snapshots(&ids).map_err(rerr)
    }

    fn retag(&self, id: &str, add: &[String], remove: &[String]) -> Result<String> {
        let repo = self.repo()?;
        let mut snap = Self::snapshot_in(&repo, id)?;
        let old = snap.id;
        let add_lists: Vec<StringList> = add
            .iter()
            .map(|t| StringList::from_str(t).map_err(|e| Error::internal(e.to_string())))
            .collect::<Result<_>>()?;
        let rm_lists: Vec<StringList> = remove
            .iter()
            .map(|t| StringList::from_str(t).map_err(|e| Error::internal(e.to_string())))
            .collect::<Result<_>>()?;
        snap.add_tags(add_lists);
        snap.remove_tags(&rm_lists);
        snap.original = Some(old);
        repo.save_snapshots(vec![snap]).map_err(rerr)?;
        let new = repo
            .get_all_snapshots()
            .map_err(rerr)?
            .into_iter()
            .filter(|s| {
                s.original == Some(old) && s.id != old && add.iter().all(|t| s.tags.contains(t))
            })
            .max_by_key(|s| s.time.timestamp())
            .ok_or_else(|| Error::internal("retagged snapshot not found"))?;
        repo.delete_snapshots(&[old]).map_err(rerr)?;
        Ok(full(&new.id))
    }

    fn prune(&self) -> Result<()> {
        let opts = PruneOptions::default();
        let repo = self.repo()?;
        let plan = repo.prune_plan(&opts).map_err(rerr)?;
        repo.prune(&opts, plan).map_err(rerr)
    }

    fn check(&self, read_data_percent: Option<f64>) -> Result<CheckReport> {
        let mut co = CheckOptions::default();
        if let Some(p) = read_data_percent {
            co.read_data = true;
            co.read_data_subset = if p >= 100.0 {
                ReadSubsetOption::All
            } else {
                ReadSubsetOption::Percentage(p)
            };
        }
        let results = self.repo()?.check(co).map_err(rerr)?;
        let msgs: Vec<String> = results
            .0
            .iter()
            .map(|(level, e)| format!("{level:?}: {e}"))
            .collect();
        let mut report = CheckReport::default();
        if results.is_ok().is_err() {
            report.errors = msgs;
        } else {
            report.warnings = msgs;
        }
        Ok(report)
    }

    fn keys(&self) -> Result<Vec<KeyInfo>> {
        let be = self.backends.repository();
        let mut out = Vec::new();
        for id in be.list(FileType::Key).map_err(rerr)? {
            let data = be.read_full(FileType::Key, &id).map_err(rerr)?;
            let kf: KeyFile = match serde_json::from_slice(&data) {
                Ok(k) => k,
                Err(_) => continue,
            };
            let role = match kf.username.as_deref() {
                Some(USER_PASSWORD) => KeyRole::Password,
                Some(USER_RECOVERY) => KeyRole::Recovery,
                _ => KeyRole::Other,
            };
            out.push(KeyInfo {
                id: full(&id),
                role,
                created: kf.created.as_ref().map(ts),
            });
        }
        Ok(out)
    }

    fn add_key(&self, secret: &str, role: KeyRole) -> Result<String> {
        let k = self
            .repo()?
            .add_key(secret, &key_opts(role))
            .map_err(rerr)?;
        Ok(full(&k))
    }

    fn delete_key(&self, id: &str) -> Result<()> {
        if self.key_id.as_deref() == Some(id) && self.keys()?.len() <= 1 {
            return Err(Error::invalid("cannot remove the last repository key"));
        }
        self.repo()?
            .delete_key(&KeyId::from(parse_id(id)?))
            .map_err(rerr)
    }

    fn current_key(&self) -> Option<String> {
        self.key_id.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loc(dir: &std::path::Path) -> WorkspaceLocation {
        WorkspaceLocation {
            endpoint: format!("file://{}", dir.display()),
            ..Default::default()
        }
    }

    #[test]
    fn backup_read_retag_keys() {
        let d = tempfile::tempdir().unwrap();
        std::env::set_var("WORKROAM_HOME", d.path().join("home"));
        let work = d.path().join("work");
        std::fs::create_dir_all(work.join("src")).unwrap();
        std::fs::write(work.join("src/main.rs"), b"fn main() {}").unwrap();
        let be = backends(&loc(&d.path().join("store")), None).unwrap();
        assert!(!RusticEngine::exists(&be).unwrap());
        let eng = RusticEngine::init(be.clone(), "pw-123456789", "RECOVERY").unwrap();
        assert!(RusticEngine::exists(&be).unwrap());
        let entries = vec![
            SourceEntry {
                path: "src/main.rs".into(),
                kind: EntryKind::File {
                    source: EntrySource::Path(work.join("src/main.rs")),
                    size: 12,
                    exec: false,
                },
                mtime: None,
            },
            SourceEntry {
                path: "a.txt".into(),
                kind: EntryKind::File {
                    source: EntrySource::Bytes(b"hello".to_vec()),
                    size: 5,
                    exec: true,
                },
                mtime: None,
            },
            SourceEntry {
                path: "link".into(),
                kind: EntryKind::Symlink {
                    target: "a.txt".into(),
                },
                mtime: None,
            },
            SourceEntry::bytes(".workroam-snapshot/manifest.json", b"{}".to_vec()),
        ];
        let tags = vec!["workroam".to_string(), "workroam:project=abc".to_string()];
        let id = eng
            .backup(entries, &tags, None, "dev1", "abc", "", None)
            .unwrap();
        assert_eq!(eng.read_file(&id, "a.txt").unwrap().unwrap(), b"hello");
        assert_eq!(
            eng.read_file(&id, "src/main.rs").unwrap().unwrap(),
            b"fn main() {}"
        );
        assert!(eng.read_file(&id, "nope").unwrap().is_none());
        let tree = eng.tree(&id).unwrap();
        assert!(tree.iter().any(|e| e.path == "link"
            && matches!(&e.kind, NodeKind::Symlink { target } if target == "a.txt")));
        assert!(tree
            .iter()
            .any(|e| e.path == "a.txt" && matches!(e.kind, NodeKind::File { exec: true, .. })));
        // second snapshot with parent
        let id2 = eng
            .backup(
                vec![SourceEntry::bytes("a.txt", b"v2".to_vec())],
                &tags,
                Some(&id),
                "dev1",
                "abc",
                "",
                None,
            )
            .unwrap();
        assert_eq!(eng.get(&id2).unwrap().parent.as_deref(), Some(id.as_str()));
        let id3 = eng.retag(&id2, &["workroam:archived".into()], &[]).unwrap();
        assert!(eng.get(&id3).unwrap().has_tag("workroam:archived"));
        assert_eq!(eng.list().unwrap().len(), 2);
        // keys: password + recovery
        let keys = eng.keys().unwrap();
        assert_eq!(
            keys.iter().filter(|k| k.role == KeyRole::Recovery).count(),
            1
        );
        drop(eng);
        let by_recovery = RusticEngine::open(be.clone(), "RECOVERY").unwrap();
        assert_eq!(
            by_recovery.read_file(&id, "a.txt").unwrap().unwrap(),
            b"hello"
        );
        assert!(RusticEngine::open(be, "wrong").is_err());
        let rep = by_recovery.check(Some(100.0)).unwrap();
        assert!(rep.errors.is_empty(), "{:?}", rep.errors);
    }

    #[test]
    fn missing_source_file_fails() {
        let d = tempfile::tempdir().unwrap();
        std::env::set_var("WORKROAM_HOME", d.path().join("home"));
        let be = backends(&loc(&d.path().join("store")), None).unwrap();
        let eng = RusticEngine::init(be, "pw-123456789", "R").unwrap();
        let entries = vec![SourceEntry {
            path: "gone.txt".into(),
            kind: EntryKind::File {
                source: EntrySource::Path(d.path().join("gone.txt")),
                size: 3,
                exec: false,
            },
            mtime: None,
        }];
        assert!(eng
            .backup(entries, &["workroam".into()], None, "h", "l", "", None)
            .is_err());
        assert!(eng.list().unwrap().is_empty());
    }
}
