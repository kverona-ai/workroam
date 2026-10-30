//! Off-site mirror (architecture §10.5): an object-level copy of the whole
//! workspace prefix. Content-addressed repository objects are copied once;
//! control objects when they change. `credentials` and leases are never
//! copied. Objects deleted from the primary are removed from the mirror
//! only after `delete_delay_days`.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use workroam_core::config::{BackupConfig, WorkspaceLocation};
use workroam_core::Result;
use workroam_store::object::Store;
use workroam_store::Creds;

/// `~/.workroam/state/backup.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MirrorState {
    pub last_ok: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    /// Content hash of each control object as last copied.
    #[serde(default)]
    pub control: BTreeMap<String, String>,
    /// Objects gone from the primary, with the time that was first seen.
    #[serde(default)]
    pub pending_delete: BTreeMap<String, DateTime<Utc>>,
}

impl MirrorState {
    pub fn path() -> PathBuf {
        workroam_core::paths::state_dir().join("backup.json")
    }
    pub fn load_from(p: &Path) -> MirrorState {
        std::fs::read(p)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }
    pub fn save_to(&self, p: &Path) -> Result<()> {
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d)?;
        }
        workroam_core::paths::write_atomic(p, &serde_json::to_vec_pretty(self)?)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MirrorReport {
    pub copied: usize,
    pub bytes: u64,
    pub unchanged: usize,
    pub pending_delete: usize,
    pub deleted: usize,
    pub duration_ms: u64,
}

/// The mirror as a workspace location (for `verify --backup` and the
/// "restore from backup" setup path).
pub fn location(cfg: &BackupConfig, workspace_id: &str) -> WorkspaceLocation {
    if cfg.kind == "local" {
        WorkspaceLocation {
            endpoint: format!("file://{}", cfg.path),
            workspace_id: workspace_id.to_string(),
            ..Default::default()
        }
    } else {
        WorkspaceLocation {
            endpoint: cfg.endpoint.clone(),
            bucket: cfg.bucket.clone(),
            prefix: cfg.prefix.clone(),
            region: cfg.region.clone(),
            addressing: cfg.addressing,
            workspace_id: workspace_id.to_string(),
            ..Default::default()
        }
    }
}

fn skipped(key: &str) -> bool {
    key == "control/credentials"
        || (key.starts_with("control/projects/") && key.ends_with("/lease"))
        || key.starts_with("repo/locks/")
}

/// Copy order: packs and indexes, then keys/config, then snapshots, then
/// control objects, so every head in the mirror points at a snapshot that
/// is already there.
fn rank(key: &str) -> u8 {
    if key.starts_with("repo/data/") || key.starts_with("repo/index/") {
        0
    } else if key.starts_with("repo/snapshots/") {
        2
    } else if key.starts_with("repo/") {
        1
    } else {
        3
    }
}

pub async fn mirror(
    primary: &WorkspaceLocation,
    primary_creds: Option<&Creds>,
    cfg: &BackupConfig,
    mirror_creds: Option<&Creds>,
    state_path: &Path,
) -> Result<MirrorReport> {
    let started = std::time::Instant::now();
    let src = Store::open(primary, primary_creds, "")?;
    let dst = Store::open(&location(cfg, &primary.workspace_id), mirror_creds, "")?;
    let mut state = MirrorState::load_from(state_path);
    let res = run(&src, &dst, cfg, &mut state).await;
    match &res {
        Ok(_) => {
            state.last_ok = Some(Utc::now());
            state.last_error = None;
        }
        Err(e) => state.last_error = Some(e.message.clone()),
    }
    state.save_to(state_path)?;
    let mut r = res?;
    r.duration_ms = started.elapsed().as_millis() as u64;
    Ok(r)
}

async fn run(
    src: &Store,
    dst: &Store,
    cfg: &BackupConfig,
    state: &mut MirrorState,
) -> Result<MirrorReport> {
    let mut report = MirrorReport::default();
    let mut objects: Vec<(String, u64)> = Vec::new();
    for prefix in ["repo/", "control/"] {
        for (k, size, _) in src.list_meta(prefix).await? {
            if !skipped(&k) {
                objects.push((k, size));
            }
        }
    }
    objects.sort_by_key(|(k, _)| (rank(k), k.clone()));
    let mut have: BTreeMap<String, u64> = BTreeMap::new();
    for prefix in ["repo/", "control/"] {
        for (k, size, _) in dst.list_meta(prefix).await? {
            have.insert(k, size);
        }
    }
    let present: BTreeSet<&String> = objects.iter().map(|(k, _)| k).collect();
    for (key, size) in &objects {
        let is_control = key.starts_with("control/") || key == "repo/config";
        if !is_control && have.get(key) == Some(size) {
            report.unchanged += 1;
            continue;
        }
        let Some(obj) = src.get(key).await? else {
            continue;
        };
        if is_control {
            let sha = workroam_core::crypto::sha256_hex(&obj.data);
            if have.contains_key(key) && state.control.get(key) == Some(&sha) {
                report.unchanged += 1;
                continue;
            }
            state.control.insert(key.clone(), sha);
        }
        report.bytes += obj.data.len() as u64;
        dst.put(key, obj.data).await?;
        report.copied += 1;
    }
    // Deletions, delayed.
    let now = Utc::now();
    let delay = Duration::days(cfg.delete_delay_days.max(1) as i64);
    state.pending_delete.retain(|k, _| !present.contains(k));
    for k in have.keys() {
        if !present.contains(k) && !skipped(k) {
            state.pending_delete.entry(k.clone()).or_insert(now);
        }
    }
    let due: Vec<String> = state
        .pending_delete
        .iter()
        .filter(|(_, t)| now - **t >= delay)
        .map(|(k, _)| k.clone())
        .collect();
    for k in due {
        dst.delete(&k).await?;
        state.pending_delete.remove(&k);
        state.control.remove(&k);
        report.deleted += 1;
    }
    report.pending_delete = state.pending_delete.len();
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn copies_in_order_and_delays_deletes() {
        let td = tempfile::tempdir().unwrap();
        let src = Store::local(td.path().join("a"));
        let dst = Store::local(td.path().join("b"));
        src.put("repo/data/00/p1", b"pack".to_vec()).await.unwrap();
        src.put("repo/snapshots/s1", b"snap".to_vec())
            .await
            .unwrap();
        src.put("control/projects/x/head", b"h".to_vec())
            .await
            .unwrap();
        src.put("control/projects/x/lease", b"l".to_vec())
            .await
            .unwrap();
        src.put("control/credentials", b"c".to_vec()).await.unwrap();
        let cfg = BackupConfig {
            kind: "local".into(),
            delete_delay_days: 30,
            ..Default::default()
        };
        let mut st = MirrorState::default();
        let r = run(&src, &dst, &cfg, &mut st).await.unwrap();
        assert_eq!(r.copied, 3);
        assert!(dst.get("control/credentials").await.unwrap().is_none());
        assert!(dst.get("control/projects/x/lease").await.unwrap().is_none());
        let r = run(&src, &dst, &cfg, &mut st).await.unwrap();
        assert_eq!(r.copied, 0);
        src.delete("repo/data/00/p1").await.unwrap();
        let r = run(&src, &dst, &cfg, &mut st).await.unwrap();
        assert_eq!((r.pending_delete, r.deleted), (1, 0));
        assert!(dst.get("repo/data/00/p1").await.unwrap().is_some());
        let t = st.pending_delete.values_mut().next().unwrap();
        *t -= Duration::days(31);
        let r = run(&src, &dst, &cfg, &mut st).await.unwrap();
        assert_eq!(r.deleted, 1);
        assert!(dst.get("repo/data/00/p1").await.unwrap().is_none());
    }
}
