//! `workroam verify` (architecture §10.5): the repository is consistent,
//! every control object decrypts, every head and archive points at an
//! existing snapshot. Problems end with exit code 22.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use workroam_core::model::{ArchiveRecord, Head};
use workroam_core::{Code, Error, Result};
use workroam_engine::{blocking, Workspace};
use workroam_store::control::keys;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VerifyReport {
    pub ok: bool,
    pub snapshots: usize,
    pub projects: usize,
    pub control_objects: usize,
    pub problems: Vec<String>,
    pub warnings: Vec<String>,
    pub read_data_percent: Option<f64>,
}

impl VerifyReport {
    /// `Err(Corrupted)` carrying the report when anything is wrong.
    pub fn into_result(self) -> Result<VerifyReport> {
        if self.ok {
            Ok(self)
        } else {
            Err(Error::new(
                Code::Corrupted,
                format!("{} problem(s) found", self.problems.len()),
            )
            .with_data(serde_json::to_value(&self).unwrap_or_default())
            .with_hint(
                "restore the affected projects from the off-site mirror or an earlier snapshot",
            ))
        }
    }
}

pub async fn verify(ws: &Workspace, read_data_percent: Option<f64>) -> Result<VerifyReport> {
    let mut r = VerifyReport {
        read_data_percent,
        ..Default::default()
    };
    let check = blocking(|| ws.snap.check(read_data_percent))?;
    r.problems.extend(check.errors);
    r.warnings.extend(check.warnings);
    let snaps = blocking(|| ws.snap.list())?;
    r.snapshots = snaps.len();
    let ids: BTreeSet<String> = snaps.into_iter().map(|s| s.id).collect();
    for k in ws.control.list("").await? {
        if k == keys::WORKSPACE {
            continue;
        }
        r.control_objects += 1;
        // Decrypt as raw JSON: any shape is fine, the seal must hold.
        match ws.control.get::<serde_json::Value>(&k).await {
            Ok(_) => {}
            Err(e) => {
                r.problems.push(format!("control/{k}: {}", e.message));
                continue;
            }
        }
        if k.ends_with("/head") {
            r.projects += 1;
            if let Ok(Some((h, _))) = ws.control.get::<Head>(&k).await {
                if let Some(s) = &h.snapshot {
                    if !ids.contains(s) {
                        r.problems.push(format!(
                            "project {}: head points at missing snapshot {s}",
                            h.project
                        ));
                    }
                }
            }
        } else if k.ends_with("/archive") {
            if let Ok(Some((a, _))) = ws.control.get::<ArchiveRecord>(&k).await {
                if !ids.contains(&a.snapshot) {
                    r.problems.push(format!(
                        "archive {}: snapshot {} is missing",
                        a.name, a.snapshot
                    ));
                }
            }
        }
    }
    r.ok = r.problems.is_empty();
    Ok(r)
}
