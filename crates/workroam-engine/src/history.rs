//! Snapshot history, lookup, comparison and retention (HIST-*, architecture §10.1).

use crate::project::Project;
use crate::sync::{ChangeKind, Description, FileChange};
use crate::workspace::{blocking, Workspace};
use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, TimeZone, Utc};
use std::collections::{BTreeMap, BTreeSet};
use workroam_core::model::{project_tag, ArchiveRecord, SnapshotInfo, TAG_ARCHIVED};
use workroam_core::{Code, Error, Result};
use workroam_snapshot::SnapshotMeta;

pub fn info(m: &SnapshotMeta) -> SnapshotInfo {
    let d: Description = serde_json::from_str(&m.description).unwrap_or_default();
    SnapshotInfo {
        id: m.id.clone(),
        short_id: m.id.chars().take(8).collect(),
        time: m.time,
        device: if d.device.name.is_empty() {
            m.hostname.clone()
        } else {
            d.device.name
        },
        archived: m.has_tag(TAG_ARCHIVED),
        summary: d.summary,
        parent: m.parent.clone(),
        restored_from: d.restored_from,
        note: d.note,
    }
}

/// Snapshots of one project, newest first.
pub fn snapshots(ws: &Workspace, pid: &str) -> Result<Vec<SnapshotMeta>> {
    let tag = project_tag(pid);
    let mut v: Vec<SnapshotMeta> = blocking(|| ws.snap.list())?
        .into_iter()
        .filter(|s| s.has_tag(&tag))
        .collect();
    v.sort_by_key(|s| std::cmp::Reverse(s.time));
    Ok(v)
}

pub fn list(p: &Project<'_>, limit: Option<usize>) -> Result<Vec<SnapshotInfo>> {
    let mut v: Vec<SnapshotInfo> = snapshots(p.ws, &p.pid)?.iter().map(info).collect();
    if let Some(l) = limit {
        v.truncate(l);
    }
    Ok(v)
}

/// Parse `2026-10-29`, `2026-10-29 18:00`, RFC 3339 (local time when no zone).
pub fn parse_time(s: &str) -> Option<DateTime<Utc>> {
    let s = s.trim();
    if let Ok(t) = DateTime::parse_from_rfc3339(s) {
        return Some(t.with_timezone(&Utc));
    }
    for f in [
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%dT%H:%M",
    ] {
        if let Ok(n) = NaiveDateTime::parse_from_str(s, f) {
            return chrono::Local
                .from_local_datetime(&n)
                .single()
                .map(|t| t.with_timezone(&Utc));
        }
    }
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        let n = d.and_hms_opt(23, 59, 59)?;
        return chrono::Local
            .from_local_datetime(&n)
            .single()
            .map(|t| t.with_timezone(&Utc));
    }
    None
}

/// Resolve `latest`, an id or id prefix, `@<time>` (latest before that time) or `~N` (N back).
pub async fn resolve(p: &Project<'_>, spec: &str) -> Result<String> {
    let snaps = snapshots(p.ws, &p.pid)?;
    let spec = spec.trim();
    let not_found = || {
        Error::invalid(format!("no snapshot of {} matches `{spec}`", p.name))
            .with_hint("see workroam history")
    };
    if spec.is_empty() || spec == "latest" || spec == "head" {
        if let Some((h, _)) = p.ws.head(&p.name).await? {
            if let Some(s) = h.snapshot {
                return Ok(s);
            }
        }
        return snaps.first().map(|s| s.id.clone()).ok_or_else(not_found);
    }
    if let Some(n) = spec.strip_prefix('~').and_then(|n| n.parse::<usize>().ok()) {
        return snaps.get(n).map(|s| s.id.clone()).ok_or_else(not_found);
    }
    if let Some(t) = spec.strip_prefix('@').and_then(parse_time) {
        return snaps
            .iter()
            .find(|s| s.time <= t)
            .map(|s| s.id.clone())
            .ok_or_else(not_found);
    }
    let m: Vec<&SnapshotMeta> = snaps.iter().filter(|s| s.id.starts_with(spec)).collect();
    match m.len() {
        1 => Ok(m[0].id.clone()),
        0 => {
            if let Some(t) = parse_time(spec) {
                return snaps
                    .iter()
                    .find(|s| s.time <= t)
                    .map(|s| s.id.clone())
                    .ok_or_else(not_found);
            }
            Err(not_found())
        }
        _ => Err(Error::invalid(format!(
            "`{spec}` matches several snapshots; use more characters"
        ))),
    }
}

/// File differences between two snapshots (or a snapshot and its parent).
pub fn diff(p: &Project<'_>, from: Option<&str>, to: &str) -> Result<Vec<FileChange>> {
    let b = crate::sync::read_manifest(p, to)?;
    let a = match from.map(String::from).or(b.parent.clone()) {
        Some(f) => crate::sync::read_manifest(p, &f)?.files,
        None => BTreeMap::new(),
    };
    let mut out = Vec::new();
    let keys: BTreeSet<&String> = a.keys().chain(b.files.keys()).collect();
    for k in keys {
        match (a.get(k), b.files.get(k)) {
            (None, Some(m)) => out.push(FileChange {
                path: k.clone(),
                kind: ChangeKind::Added,
                size: m.size,
            }),
            (Some(_), None) => out.push(FileChange {
                path: k.clone(),
                kind: ChangeKind::Deleted,
                size: 0,
            }),
            (Some(x), Some(y)) if x.sha256 != y.sha256 || x.exec != y.exec => {
                out.push(FileChange {
                    path: k.clone(),
                    kind: ChangeKind::Modified,
                    size: y.size,
                })
            }
            _ => {}
        }
    }
    Ok(out)
}

/// Snapshots to forget under the retention policy (§HIST: daily window, then weekly).
pub fn retention_candidates(
    snaps: &[SnapshotMeta],
    keep: &BTreeSet<String>,
    days: u32,
    weeks: u32,
    keep_all: bool,
    now: DateTime<Utc>,
) -> Vec<String> {
    if keep_all {
        return vec![];
    }
    let recent = now - Duration::days(days as i64);
    let weekly_until = if weeks == 0 {
        None
    } else {
        Some(recent - Duration::weeks(weeks as i64))
    };
    let mut seen_weeks = BTreeSet::new();
    let mut out = Vec::new();
    // Newest first: the first snapshot of each week is the one kept.
    let mut sorted: Vec<&SnapshotMeta> = snaps.iter().collect();
    sorted.sort_by_key(|s| std::cmp::Reverse(s.time));
    for s in sorted {
        if keep.contains(&s.id) || s.has_tag(TAG_ARCHIVED) || s.time >= recent {
            continue;
        }
        let wk = (s.time.iso_week().year(), s.time.iso_week().week());
        let within = weekly_until.map(|u| s.time >= u).unwrap_or(true);
        if within && seen_weeks.insert(wk) {
            continue;
        }
        out.push(s.id.clone());
    }
    out
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct PruneReport {
    pub forgotten: usize,
    pub projects: usize,
}

/// Apply the workspace retention policy to every project and prune unused data.
pub async fn prune(ws: &Workspace, dry_run: bool) -> Result<PruneReport> {
    ws.ensure_writable()?;
    let settings = ws.settings().await?;
    let mut report = PruneReport::default();
    let all = blocking(|| ws.snap.list())?;
    let mut forget = Vec::new();
    for proj in ws.projects().await? {
        report.projects += 1;
        let tag = project_tag(&proj.pid);
        let snaps: Vec<SnapshotMeta> = all.iter().filter(|s| s.has_tag(&tag)).cloned().collect();
        let mut keep = BTreeSet::new();
        if let Some(h) = proj.head.as_ref().and_then(|h| h.snapshot.clone()) {
            keep.insert(h);
        }
        let keep_all = match ws
            .control
            .get::<ArchiveRecord>(&workroam_store::control::keys::archive(&proj.pid))
            .await?
        {
            Some((a, _)) => {
                keep.insert(a.snapshot.clone());
                a.keep_history
            }
            None => false,
        };
        forget.extend(retention_candidates(
            &snaps,
            &keep,
            settings.retention_days,
            settings.retention_weekly,
            keep_all,
            Utc::now(),
        ));
    }
    report.forgotten = forget.len();
    if !dry_run {
        if !forget.is_empty() {
            blocking(|| ws.snap.forget(&forget))?;
        }
        blocking(|| ws.snap.prune())?;
    }
    Ok(report)
}

pub fn ensure_exists(ws: &Workspace, id: &str) -> Result<SnapshotMeta> {
    blocking(|| ws.snap.get(id))
        .map_err(|_| Error::new(Code::Corrupted, format!("snapshot {id} is missing")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(id: &str, days_ago: i64, now: DateTime<Utc>) -> SnapshotMeta {
        SnapshotMeta {
            id: id.into(),
            time: now - Duration::days(days_ago),
            hostname: "h".into(),
            tags: vec![],
            label: String::new(),
            description: String::new(),
            parent: None,
        }
    }

    #[test]
    fn retention() {
        let now = Utc.with_ymd_and_hms(2026, 10, 30, 12, 0, 0).unwrap();
        let snaps: Vec<SnapshotMeta> = (0..120).map(|d| meta(&format!("s{d}"), d, now)).collect();
        let forget = retention_candidates(&snaps, &BTreeSet::new(), 30, 4, false, now);
        // 30 recent days stay, then about one per week for 4 weeks, the rest goes.
        let kept = 120 - forget.len();
        assert!((34..=36).contains(&kept), "kept {kept}");
        assert!(!forget.contains(&"s0".to_string()));
        assert!(forget.contains(&"s119".to_string()));
        assert!(retention_candidates(&snaps, &BTreeSet::new(), 30, 4, true, now).is_empty());
        assert!(parse_time("2026-10-29 18:00").is_some());
        assert!(parse_time("2026-10-29").is_some());
    }
}
