//! Object access for control objects and mirrors, with conditional writes
//! (LEASE-01). S3 goes through OpenDAL; `file://` workspaces use a local
//! directory with a lock file so the same semantics hold.

use crate::Creds;
use futures_util::TryStreamExt;
use std::path::{Path, PathBuf};
use workroam_core::config::{Addressing, WorkspaceLocation};
use workroam_core::{Code, Error, Result};

#[derive(Debug, Clone)]
pub struct StoredObject {
    pub data: Vec<u8>,
    pub etag: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PutOutcome {
    Written {
        etag: String,
    },
    /// `If-None-Match: *` failed because the object exists, or `If-Match` failed.
    PreconditionFailed,
}

#[derive(Clone)]
enum Kind {
    S3(opendal::Operator),
    Local(PathBuf),
}

/// A store rooted at a key prefix (for example `<prefix>/control/`).
#[derive(Clone)]
pub struct Store {
    kind: Kind,
    /// Whether the backend honours conditional writes.
    pub conditional: bool,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.kind {
            Kind::S3(op) => write!(f, "Store(s3 {})", op.info().root()),
            Kind::Local(p) => write!(f, "Store(local {})", p.display()),
        }
    }
}

fn map_err(e: opendal::Error) -> Error {
    match e.kind() {
        opendal::ErrorKind::PermissionDenied => {
            Error::new(Code::Network, format!("storage denied access: {e}"))
                .with_hint("check the access key permissions")
        }
        _ => Error::network(format!("storage error: {e}")),
    }
}

pub fn s3_operator(
    loc: &WorkspaceLocation,
    creds: &Creds,
    root: &str,
) -> Result<opendal::Operator> {
    let mut b = opendal::services::S3::default()
        .bucket(&loc.bucket)
        .endpoint(loc.endpoint.trim_end_matches('/'))
        .region(if loc.region.is_empty() {
            "us-east-1"
        } else {
            &loc.region
        })
        .access_key_id(&creds.access_key)
        .secret_access_key(&creds.secret_key)
        .root(root)
        .disable_config_load()
        .disable_ec2_metadata();
    if loc.addressing == Addressing::VirtualHost {
        b = b.enable_virtual_host_style();
    }
    let op = opendal::Operator::new(b).map_err(map_err)?;
    Ok(op)
}

/// `/<prefix>/<sub>` for OpenDAL roots.
pub fn root_for(loc: &WorkspaceLocation, sub: &str) -> String {
    let prefix = loc.prefix.trim_matches('/');
    if prefix.is_empty() {
        format!("/{sub}")
    } else {
        format!("/{prefix}/{sub}")
    }
}

impl Store {
    /// Open the store for `sub` (`control`, `repo`, …) of a workspace.
    pub fn open(loc: &WorkspaceLocation, creds: Option<&Creds>, sub: &str) -> Result<Store> {
        if let Some(root) = loc.local_root() {
            let dir = if sub.is_empty() { root } else { root.join(sub) };
            return Ok(Store {
                kind: Kind::Local(dir),
                conditional: true,
            });
        }
        let creds = creds.ok_or_else(|| Error::invalid("storage credentials are required"))?;
        let op = s3_operator(loc, creds, &root_for(loc, sub))?;
        Ok(Store {
            kind: Kind::S3(op),
            conditional: loc.capabilities.conditional(),
        })
    }

    pub fn local(dir: impl Into<PathBuf>) -> Store {
        Store {
            kind: Kind::Local(dir.into()),
            conditional: true,
        }
    }

    pub fn from_operator(op: opendal::Operator, conditional: bool) -> Store {
        Store {
            kind: Kind::S3(op),
            conditional,
        }
    }

    pub fn is_local(&self) -> bool {
        matches!(self.kind, Kind::Local(_))
    }

    pub fn local_dir(&self) -> Option<&Path> {
        match &self.kind {
            Kind::Local(p) => Some(p),
            _ => None,
        }
    }

    fn local_path(dir: &Path, key: &str) -> PathBuf {
        workroam_core::paths::from_snapshot_path(dir, key)
    }

    fn local_lock(dir: &Path) -> Result<std::fs::File> {
        use fs4::fs_std::FileExt;
        std::fs::create_dir_all(dir)?;
        let f = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.join(".workroam-lock"))?;
        f.lock_exclusive()?;
        Ok(f)
    }

    fn local_etag(data: &[u8]) -> String {
        workroam_core::crypto::sha256_hex(data)[..32].to_string()
    }

    pub async fn get(&self, key: &str) -> Result<Option<StoredObject>> {
        match &self.kind {
            Kind::S3(op) => {
                let meta = match op.stat(key).await {
                    Ok(m) => m,
                    Err(e) if e.kind() == opendal::ErrorKind::NotFound => return Ok(None),
                    Err(e) => return Err(map_err(e)),
                };
                let etag = meta.etag().unwrap_or("").to_string();
                let data = match if etag.is_empty() {
                    op.read(key).await
                } else {
                    op.read_with(key).if_match(&etag).await
                } {
                    Ok(d) => d.to_vec(),
                    Err(e) if e.kind() == opendal::ErrorKind::NotFound => return Ok(None),
                    Err(e) if e.kind() == opendal::ErrorKind::ConditionNotMatch => {
                        // Changed between stat and read: read again without a condition.
                        return Box::pin(self.get(key)).await;
                    }
                    Err(e) => return Err(map_err(e)),
                };
                Ok(Some(StoredObject { data, etag }))
            }
            Kind::Local(dir) => {
                let p = Self::local_path(dir, key);
                match std::fs::read(&p) {
                    Ok(data) => {
                        let etag = Self::local_etag(&data);
                        Ok(Some(StoredObject { data, etag }))
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                    Err(e) => Err(e.into()),
                }
            }
        }
    }

    pub async fn exists(&self, key: &str) -> Result<bool> {
        match &self.kind {
            Kind::S3(op) => match op.stat(key).await {
                Ok(_) => Ok(true),
                Err(e) if e.kind() == opendal::ErrorKind::NotFound => Ok(false),
                Err(e) => Err(map_err(e)),
            },
            Kind::Local(dir) => Ok(Self::local_path(dir, key).exists()),
        }
    }

    pub async fn put(&self, key: &str, data: Vec<u8>) -> Result<String> {
        match &self.kind {
            Kind::S3(op) => {
                let meta = op.write(key, data).await.map_err(map_err)?;
                Ok(meta.etag().unwrap_or("").to_string())
            }
            Kind::Local(dir) => {
                let _lock = Self::local_lock(dir)?;
                let etag = Self::local_etag(&data);
                workroam_core::paths::write_atomic(&Self::local_path(dir, key), &data)?;
                Ok(etag)
            }
        }
    }

    /// Create only if absent (`If-None-Match: *`).
    pub async fn put_if_absent(&self, key: &str, data: Vec<u8>) -> Result<PutOutcome> {
        match &self.kind {
            Kind::S3(op) => {
                if !self.conditional {
                    if op.exists(key).await.map_err(map_err)? {
                        return Ok(PutOutcome::PreconditionFailed);
                    }
                    let meta = op.write(key, data).await.map_err(map_err)?;
                    return Ok(PutOutcome::Written {
                        etag: meta.etag().unwrap_or("").to_string(),
                    });
                }
                match op.write_with(key, data).if_not_exists(true).await {
                    Ok(meta) => {
                        let etag = match meta.etag() {
                            Some(e) => e.to_string(),
                            None => op
                                .stat(key)
                                .await
                                .map_err(map_err)?
                                .etag()
                                .unwrap_or("")
                                .to_string(),
                        };
                        Ok(PutOutcome::Written { etag })
                    }
                    Err(e)
                        if matches!(
                            e.kind(),
                            opendal::ErrorKind::ConditionNotMatch
                                | opendal::ErrorKind::AlreadyExists
                        ) =>
                    {
                        Ok(PutOutcome::PreconditionFailed)
                    }
                    Err(e) => Err(map_err(e)),
                }
            }
            Kind::Local(dir) => {
                let _lock = Self::local_lock(dir)?;
                let p = Self::local_path(dir, key);
                if p.exists() {
                    return Ok(PutOutcome::PreconditionFailed);
                }
                let etag = Self::local_etag(&data);
                workroam_core::paths::write_atomic(&p, &data)?;
                Ok(PutOutcome::Written { etag })
            }
        }
    }

    /// Overwrite only if the current ETag matches (`If-Match`).
    pub async fn put_if_match(&self, key: &str, data: Vec<u8>, etag: &str) -> Result<PutOutcome> {
        match &self.kind {
            Kind::S3(op) => {
                if !self.conditional {
                    let cur = op.stat(key).await.map_err(map_err)?;
                    if cur.etag().unwrap_or("") != etag {
                        return Ok(PutOutcome::PreconditionFailed);
                    }
                    let meta = op.write(key, data).await.map_err(map_err)?;
                    return Ok(PutOutcome::Written {
                        etag: meta.etag().unwrap_or("").to_string(),
                    });
                }
                match op.write_with(key, data).if_match(etag).await {
                    Ok(meta) => {
                        let etag = match meta.etag() {
                            Some(e) => e.to_string(),
                            None => op
                                .stat(key)
                                .await
                                .map_err(map_err)?
                                .etag()
                                .unwrap_or("")
                                .to_string(),
                        };
                        Ok(PutOutcome::Written { etag })
                    }
                    Err(e)
                        if matches!(
                            e.kind(),
                            opendal::ErrorKind::ConditionNotMatch | opendal::ErrorKind::NotFound
                        ) =>
                    {
                        Ok(PutOutcome::PreconditionFailed)
                    }
                    Err(e) => Err(map_err(e)),
                }
            }
            Kind::Local(dir) => {
                let _lock = Self::local_lock(dir)?;
                let p = Self::local_path(dir, key);
                let cur = match std::fs::read(&p) {
                    Ok(d) => d,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        return Ok(PutOutcome::PreconditionFailed)
                    }
                    Err(e) => return Err(e.into()),
                };
                if Self::local_etag(&cur) != etag {
                    return Ok(PutOutcome::PreconditionFailed);
                }
                let new = Self::local_etag(&data);
                workroam_core::paths::write_atomic(&p, &data)?;
                Ok(PutOutcome::Written { etag: new })
            }
        }
    }

    pub async fn delete(&self, key: &str) -> Result<()> {
        match &self.kind {
            Kind::S3(op) => op.delete(key).await.map_err(map_err),
            Kind::Local(dir) => {
                let _lock = Self::local_lock(dir)?;
                match std::fs::remove_file(Self::local_path(dir, key)) {
                    Ok(()) => Ok(()),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(e) => Err(e.into()),
                }
            }
        }
    }

    /// All object keys under `prefix` (recursive), relative to the store root.
    pub async fn list(&self, prefix: &str) -> Result<Vec<String>> {
        match &self.kind {
            Kind::S3(op) => {
                let mut lister = op
                    .lister_with(prefix)
                    .recursive(true)
                    .await
                    .map_err(map_err)?;
                let mut out = Vec::new();
                while let Some(entry) = lister.try_next().await.map_err(map_err)? {
                    if entry.metadata().mode().is_file() {
                        out.push(entry.path().trim_start_matches('/').to_string());
                    }
                }
                out.sort();
                Ok(out)
            }
            Kind::Local(dir) => {
                let base = Self::local_path(dir, prefix);
                let mut out = Vec::new();
                let walk_root = if base.is_dir() {
                    base
                } else {
                    base.parent()
                        .map(Path::to_path_buf)
                        .unwrap_or_else(|| dir.clone())
                };
                if !walk_root.exists() {
                    return Ok(out);
                }
                for e in walkdir::WalkDir::new(&walk_root)
                    .into_iter()
                    .filter_map(|e| e.ok())
                {
                    if e.file_type().is_file() {
                        if let Some(rel) = workroam_core::paths::to_snapshot_path(dir, e.path()) {
                            if rel.starts_with(prefix)
                                && !rel.ends_with(".workroam-lock")
                                && !rel.contains(".wr-tmp-")
                            {
                                out.push(rel);
                            }
                        }
                    }
                }
                out.sort();
                Ok(out)
            }
        }
    }

    /// Size and ETag of every object under `prefix` (for mirroring).
    pub async fn list_meta(&self, prefix: &str) -> Result<Vec<(String, u64, String)>> {
        match &self.kind {
            Kind::S3(op) => {
                let mut lister = op
                    .lister_with(prefix)
                    .recursive(true)
                    .await
                    .map_err(map_err)?;
                let mut out = Vec::new();
                while let Some(entry) = lister.try_next().await.map_err(map_err)? {
                    if entry.metadata().mode().is_file() {
                        let path = entry.path().trim_start_matches('/').to_string();
                        let m = entry.metadata();
                        let (len, etag) = if m.content_length() > 0 || m.etag().is_some() {
                            (m.content_length(), m.etag().unwrap_or("").to_string())
                        } else {
                            let s = op.stat(&path).await.map_err(map_err)?;
                            (s.content_length(), s.etag().unwrap_or("").to_string())
                        };
                        out.push((path, len, etag));
                    }
                }
                out.sort();
                Ok(out)
            }
            Kind::Local(dir) => {
                let mut out = Vec::new();
                for k in self.list(prefix).await? {
                    let p = Self::local_path(dir, &k);
                    let len = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
                    out.push((k, len, String::new()));
                }
                Ok(out)
            }
        }
    }

    pub async fn read_raw(&self, key: &str) -> Result<Vec<u8>> {
        self.get(key)
            .await?
            .map(|o| o.data)
            .ok_or_else(|| Error::new(Code::Corrupted, format!("object {key} is missing")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn local_conditional_semantics() {
        let d = tempfile::tempdir().unwrap();
        let s = Store::local(d.path());
        let PutOutcome::Written { etag } = s
            .put_if_absent("projects/p/lease", b"one".to_vec())
            .await
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(
            s.put_if_absent("projects/p/lease", b"two".to_vec())
                .await
                .unwrap(),
            PutOutcome::PreconditionFailed
        );
        assert_eq!(
            s.put_if_match("projects/p/lease", b"two".to_vec(), "stale")
                .await
                .unwrap(),
            PutOutcome::PreconditionFailed
        );
        let PutOutcome::Written { etag: e2 } = s
            .put_if_match("projects/p/lease", b"two".to_vec(), &etag)
            .await
            .unwrap()
        else {
            panic!()
        };
        assert_ne!(etag, e2);
        assert_eq!(
            s.get("projects/p/lease").await.unwrap().unwrap().data,
            b"two"
        );
        assert_eq!(s.list("projects/").await.unwrap(), vec!["projects/p/lease"]);
        s.delete("projects/p/lease").await.unwrap();
        assert!(s.get("projects/p/lease").await.unwrap().is_none());
    }
}
