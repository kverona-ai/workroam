//! Approval cards (architecture §15, PRD UX-01..UX-05). A job that needs
//! approval waits here until someone answers on a card, in a terminal, with
//! `workroam approve|later|deny`, or the timeout answers "later".

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::oneshot;
use workroam_core::{Code, Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Approve,
    Later,
    Deny,
}

impl Decision {
    pub fn parse(s: &str) -> Result<Decision> {
        match s {
            "approve" | "yes" | "ok" => Ok(Decision::Approve),
            "later" => Ok(Decision::Later),
            "deny" | "no" | "reject" => Ok(Decision::Deny),
            _ => Err(Error::invalid(format!(
                "unknown decision {s} (approve, later or deny)"
            ))),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Answer {
    pub decision: Decision,
    /// "Don't ask again for this project" duration (e.g. 1h).
    #[serde(default)]
    pub skip_ask: Option<String>,
    /// Remote connections: granted access and projects.
    #[serde(default)]
    pub access: Option<String>,
    #[serde(default)]
    pub projects: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Approval {
    pub id: String,
    pub job_id: Option<String>,
    /// push, pull, restore, archive, archive_restore, connect.
    pub kind: String,
    pub project: Option<String>,
    pub initiator: String,
    pub remote: bool,
    pub title: String,
    /// Plan shown by "view details" (files, commits, lease).
    pub details: Value,
    /// Confirmation code for remote connections.
    #[serde(default)]
    pub code: Option<String>,
    pub created_at: DateTime<Utc>,
    pub timeout_s: u64,
}

#[derive(Default)]
pub struct Approvals {
    open: Mutex<HashMap<String, (Approval, Option<oneshot::Sender<Answer>>)>>,
}

impl Approvals {
    pub fn list(&self) -> Vec<Approval> {
        let mut v: Vec<Approval> = self
            .open
            .lock()
            .unwrap()
            .values()
            .map(|(a, _)| a.clone())
            .collect();
        v.sort_by_key(|a| a.created_at);
        v
    }

    pub fn get(&self, id: &str) -> Option<Approval> {
        let g = self.open.lock().unwrap();
        g.get(id).map(|(a, _)| a.clone()).or_else(|| {
            g.values()
                .find(|(a, _)| a.id.starts_with(id))
                .map(|(a, _)| a.clone())
        })
    }

    /// Register and wait. Timeout counts as "later" (exit code 11).
    pub async fn ask(&self, d: &crate::Daemon, a: Approval) -> Result<Answer> {
        let (tx, rx) = oneshot::channel();
        let id = a.id.clone();
        let timeout = Duration::from_secs(a.timeout_s);
        d.emit("approval.requested", serde_json::to_value(&a)?);
        self.open.lock().unwrap().insert(id.clone(), (a, Some(tx)));
        let res = tokio::time::timeout(timeout, rx).await;
        self.open.lock().unwrap().remove(&id);
        let timed_out = res.is_err();
        let ans = match res {
            Ok(Ok(ans)) => ans,
            Ok(Err(_)) => {
                return Err(Error::new(
                    Code::ApprovalTimeout,
                    "the approval was withdrawn",
                ))
            }
            Err(_) => Answer {
                decision: Decision::Later,
                skip_ask: None,
                access: None,
                projects: vec![],
            },
        };
        d.emit(
            "approval.resolved",
            serde_json::json!({"id": id, "decision": ans.decision}),
        );
        match ans.decision {
            Decision::Approve => Ok(ans),
            Decision::Later => Err(Error::new(
                Code::ApprovalTimeout,
                if timed_out {
                    "no answer in time; postponed"
                } else {
                    "postponed"
                },
            )
            .with_hint("run the command again when you are ready")),
            Decision::Deny => Err(Error::new(Code::Denied, "the request was denied")),
        }
    }

    pub fn respond(&self, id: &str, ans: Answer) -> Result<Approval> {
        let mut g = self.open.lock().unwrap();
        let key = if g.contains_key(id) {
            id.to_string()
        } else {
            g.keys()
                .find(|k| k.starts_with(id))
                .cloned()
                .ok_or_else(|| Error::invalid(format!("no pending approval {id}")))?
        };
        let (a, tx) = g.get_mut(&key).unwrap();
        let a = a.clone();
        if let Some(tx) = tx.take() {
            let _ = tx.send(ans);
        }
        Ok(a)
    }
}
