//! Jobs (architecture §15): `*.plan` creates a planned job, `job.start`
//! asks for approval when needed and runs it, `job.progress` /
//! `job.finished` / `job.failed` report it.

use crate::approvals::Approval;
use crate::{Ctx, Daemon};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::watch;
use workroam_archive::archive::{ArchiveOptions, Prefer, RestoreArchivePlan};
use workroam_core::{Code, Error, Result};
use workroam_engine::restore::RestorePlan;
use workroam_engine::sync::{PullOptions, PushOptions};
use workroam_engine::Project;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum JobSpec {
    Push {
        project: String,
        release: bool,
        takeover: bool,
        note: String,
    },
    Pull {
        project: String,
        takeover: bool,
        /// Wait this long for the lease to be released.
        wait_s: Option<u64>,
    },
    Restore {
        project: String,
        plan: RestorePlan,
    },
    Archive {
        project: String,
        note: String,
        keep_history: bool,
        takeover: bool,
    },
    ArchiveRestore {
        project: String,
        plan: RestoreArchivePlan,
        prefer: Prefer,
        takeover: bool,
    },
}

impl JobSpec {
    pub fn kind(&self) -> &'static str {
        match self {
            JobSpec::Push { .. } => "push",
            JobSpec::Pull { .. } => "pull",
            JobSpec::Restore { .. } => "restore",
            JobSpec::Archive { .. } => "archive",
            JobSpec::ArchiveRestore { .. } => "archive_restore",
        }
    }
    pub fn project(&self) -> &str {
        match self {
            JobSpec::Push { project, .. }
            | JobSpec::Pull { project, .. }
            | JobSpec::Restore { project, .. }
            | JobSpec::Archive { project, .. }
            | JobSpec::ArchiveRestore { project, .. } => project,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Planned,
    WaitingApproval,
    Running,
    Finished,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JobProgress {
    pub phase: String,
    pub done: u64,
    pub total: Option<u64>,
    pub bytes_per_s: u64,
    pub eta_s: Option<u64>,
    /// Sleep is blocked while the job runs.
    pub sleep_blocked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobInfo {
    pub id: String,
    pub kind: String,
    pub project: String,
    pub state: JobState,
    pub initiator: String,
    /// manual, idle, sleep, wake, arrival, agent, remote.
    pub trigger: String,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub plan: Value,
    pub approval_id: Option<String>,
    pub progress: Option<JobProgress>,
    pub result: Option<Value>,
    pub error: Option<Error>,
}

struct Entry {
    info: JobInfo,
    spec: JobSpec,
    done: watch::Sender<bool>,
    abort: Option<tokio::task::AbortHandle>,
}

#[derive(Default)]
pub struct Jobs {
    map: Mutex<HashMap<String, Entry>>,
}

impl Jobs {
    pub fn create(&self, spec: JobSpec, plan: Value, ctx: &Ctx, trigger: &str) -> JobInfo {
        let id = workroam_core::crypto::random_id("job");
        let info = JobInfo {
            id: id.clone(),
            kind: spec.kind().into(),
            project: spec.project().into(),
            state: JobState::Planned,
            initiator: ctx.initiator(),
            trigger: trigger.into(),
            created_at: Utc::now(),
            started_at: None,
            finished_at: None,
            plan,
            approval_id: None,
            progress: None,
            result: None,
            error: None,
        };
        let (done, _) = watch::channel(false);
        let mut g = self.map.lock().unwrap();
        // Keep the table small: drop old finished jobs.
        if g.len() > 200 {
            let cutoff = Utc::now() - chrono::Duration::hours(6);
            g.retain(|_, e| e.info.finished_at.map(|t| t > cutoff).unwrap_or(true));
        }
        g.insert(
            id,
            Entry {
                info: info.clone(),
                spec,
                done,
                abort: None,
            },
        );
        info
    }

    pub fn get(&self, id: &str) -> Option<JobInfo> {
        let g = self.map.lock().unwrap();
        g.get(id).map(|e| e.info.clone()).or_else(|| {
            g.values()
                .find(|e| e.info.id.starts_with(id))
                .map(|e| e.info.clone())
        })
    }

    pub fn list(&self, all: bool) -> Vec<JobInfo> {
        let mut v: Vec<JobInfo> = self
            .map
            .lock()
            .unwrap()
            .values()
            .map(|e| e.info.clone())
            .filter(|j| {
                all || matches!(
                    j.state,
                    JobState::Planned | JobState::WaitingApproval | JobState::Running
                )
            })
            .collect();
        v.sort_by_key(|j| j.created_at);
        v
    }

    pub fn running_for(&self, project: &str) -> bool {
        self.map.lock().unwrap().values().any(|e| {
            e.info.project == project
                && matches!(e.info.state, JobState::Running | JobState::WaitingApproval)
        })
    }

    fn update(&self, id: &str, f: impl FnOnce(&mut JobInfo)) -> Option<JobInfo> {
        let mut g = self.map.lock().unwrap();
        let e = g.get_mut(id)?;
        f(&mut e.info);
        Some(e.info.clone())
    }

    fn finish(&self, id: &str) {
        if let Some(e) = self.map.lock().unwrap().get(id) {
            let _ = e.done.send(true);
        }
    }

    pub fn cancel(&self, d: &Daemon, id: &str) -> Result<JobInfo> {
        let full = self
            .get(id)
            .ok_or_else(|| Error::invalid(format!("no job {id}")))?
            .id;
        let abort = {
            let mut g = self.map.lock().unwrap();
            let e = g.get_mut(&full).unwrap();
            if matches!(
                e.info.state,
                JobState::Finished | JobState::Failed | JobState::Cancelled
            ) {
                return Ok(e.info.clone());
            }
            e.info.state = JobState::Cancelled;
            e.info.finished_at = Some(Utc::now());
            e.info.error = Some(Error::new(Code::Denied, "cancelled"));
            e.abort.take()
        };
        if let Some(a) = abort {
            a.abort();
        }
        if let Some(a) = self.get(&full).and_then(|j| j.approval_id) {
            let _ = d.approvals.respond(
                &a,
                crate::approvals::Answer {
                    decision: crate::approvals::Decision::Deny,
                    skip_ask: None,
                    access: None,
                    projects: vec![],
                },
            );
        }
        self.finish(&full);
        let info = self.get(&full).unwrap();
        d.emit(
            "job.failed",
            serde_json::to_value(&info).unwrap_or_default(),
        );
        Ok(info)
    }

    /// Wait for a started job and return its result.
    pub async fn wait(&self, id: &str) -> Result<Value> {
        let mut rx = {
            let g = self.map.lock().unwrap();
            let e = g
                .get(id)
                .ok_or_else(|| Error::invalid(format!("no job {id}")))?;
            e.done.subscribe()
        };
        while !*rx.borrow() {
            if rx.changed().await.is_err() {
                break;
            }
        }
        let info = self
            .get(id)
            .ok_or_else(|| Error::internal("job disappeared"))?;
        match (info.state, info.error) {
            (JobState::Finished, _) => Ok(serde_json::to_value(JobResult {
                job: info.id.clone(),
                kind: info.kind.clone(),
                project: info.project.clone(),
                result: info.result.clone().unwrap_or(Value::Null),
            })?),
            (_, Some(e)) => Err(e),
            _ => Err(Error::internal("job ended without a result")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobResult {
    pub job: String,
    pub kind: String,
    pub project: String,
    pub result: Value,
}

/// Does this start need an approval card (PRD SEC-05, UX-01)?
fn needs_approval(d: &Daemon, ctx: &Ctx, spec: &JobSpec, plan: &Value, ask: bool) -> Result<bool> {
    if ask || ctx.is_remote() {
        return Ok(true);
    }
    if !ctx.is_agent() {
        return Ok(false);
    }
    let m = crate::load_machine()?;
    let project = spec.project();
    let mut required = m.sync.agent_requires_approval;
    if let Some(b) = m.projects.get(project) {
        let dir = workroam_core::paths::expand_tilde(&b.path);
        if let Ok(Some(c)) = workroam_core::config::ProjectConfig::load(&dir) {
            if let Some(o) = c.sync.agent_requires_approval {
                required = o;
            }
        }
    }
    // Pulls that would merge or keep local changes always ask.
    if plan
        .get("touches_local")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return Ok(true);
    }
    if !required {
        return Ok(false);
    }
    let skip = d.skip_ask.lock().unwrap().get(project).copied();
    Ok(!matches!(skip, Some(t) if t > Utc::now()))
}

fn approval_title(spec: &JobSpec) -> String {
    match spec {
        JobSpec::Push {
            project, release, ..
        } => {
            if *release {
                format!("Push {project} and release the lease")
            } else {
                format!("Push {project}")
            }
        }
        JobSpec::Pull { project, .. } => format!("Pull {project}"),
        JobSpec::Restore { project, .. } => format!("Restore {project}"),
        JobSpec::Archive { project, .. } => format!("Archive {project}"),
        JobSpec::ArchiveRestore { project, .. } => format!("Restore archived {project}"),
    }
}

/// Start a planned job. With `wait` the call returns the result.
pub async fn start(d: &Arc<Daemon>, ctx: &Ctx, id: &str, ask: bool, wait: bool) -> Result<Value> {
    let (info, spec) = {
        let g = d.jobs.map.lock().unwrap();
        let e = g
            .get(id)
            .or_else(|| g.values().find(|e| e.info.id.starts_with(id)))
            .ok_or_else(|| Error::invalid(format!("no job {id}")))?;
        if e.info.state != JobState::Planned {
            return Err(Error::invalid(format!(
                "job {} was already started",
                e.info.id
            )));
        }
        (e.info.clone(), e.spec.clone())
    };
    let id = info.id.clone();
    let approval = needs_approval(d, ctx, &spec, &info.plan, ask)?;
    d.jobs.update(&id, |j| {
        j.state = if approval {
            JobState::WaitingApproval
        } else {
            JobState::Running
        };
        j.started_at = Some(Utc::now());
    });
    let d2 = d.clone();
    let ctx2 = ctx.clone();
    let id2 = id.clone();
    let handle = tokio::spawn(async move {
        let res = run(&d2, &ctx2, &id2, &spec, approval).await;
        let info = d2.jobs.update(&id2, |j| {
            if j.state == JobState::Cancelled {
                return;
            }
            j.finished_at = Some(Utc::now());
            match &res {
                Ok(v) => {
                    j.state = JobState::Finished;
                    j.result = Some(v.clone());
                }
                Err(e) => {
                    j.state = JobState::Failed;
                    j.error = Some(e.clone());
                }
            }
        });
        if let Some(info) = info {
            let ev = serde_json::to_value(&info).unwrap_or_default();
            match info.state {
                JobState::Finished => {
                    d2.last_failure.lock().unwrap().remove(&info.project);
                    d2.emit("job.finished", ev.clone());
                    crate::notify::job_event(&d2, "finished", &info).await;
                }
                JobState::Failed => {
                    let code = info.error.as_ref().map(|e| e.code);
                    // "Later" and "denied" are answers, not failures.
                    if !matches!(code, Some(Code::ApprovalTimeout) | Some(Code::Denied)) {
                        d2.last_failure
                            .lock()
                            .unwrap()
                            .insert(info.project.clone(), ev.clone());
                        crate::notify::job_event(&d2, "failed", &info).await;
                    }
                    d2.emit("job.failed", ev);
                }
                _ => {}
            }
        }
        d2.jobs.finish(&id2);
    });
    {
        let mut g = d.jobs.map.lock().unwrap();
        if let Some(e) = g.get_mut(&id) {
            e.abort = Some(handle.abort_handle());
        }
    }
    if wait {
        d.jobs.wait(&id).await
    } else {
        Ok(serde_json::to_value(d.jobs.get(&id))?)
    }
}

fn progress_fn(d: &Arc<Daemon>, id: &str) -> workroam_engine::sync::Progress {
    let d = d.clone();
    let id = id.to_string();
    let last = Arc::new(Mutex::new((
        Instant::now(),
        String::new(),
        0u64,
        Instant::now(),
    )));
    Arc::new(move |phase: &str, done: u64, total: Option<u64>| {
        let mut g = last.lock().unwrap();
        let now = Instant::now();
        let phase_changed = g.1 != phase;
        if phase_changed {
            g.1 = phase.to_string();
            g.2 = done;
            g.3 = now;
        }
        if !phase_changed
            && now.duration_since(g.0) < Duration::from_millis(250)
            && Some(done) != total
        {
            return;
        }
        g.0 = now;
        let secs = now.duration_since(g.3).as_secs_f64().max(0.001);
        let rate = ((done.saturating_sub(g.2)) as f64 / secs) as u64;
        let eta = total.and_then(|t| (rate > 0).then(|| t.saturating_sub(done) / rate.max(1)));
        let p = JobProgress {
            phase: phase.to_string(),
            done,
            total,
            bytes_per_s: rate,
            eta_s: eta,
            sleep_blocked: true,
        };
        if let Some(info) = d.jobs.update(&id, |j| j.progress = Some(p.clone())) {
            d.emit(
                "job.progress",
                json!({"job": info.id, "kind": info.kind, "project": info.project, "progress": p}),
            );
        }
    })
}

async fn run(
    d: &Arc<Daemon>,
    ctx: &Ctx,
    id: &str,
    spec: &JobSpec,
    approval: bool,
) -> Result<Value> {
    if approval {
        let plan = d.jobs.get(id).map(|j| j.plan).unwrap_or_default();
        let timeout = crate::load_machine()
            .ok()
            .and_then(|m| workroam_core::duration::parse(&m.sync.approval_timeout).ok())
            .map(|t| t.as_secs())
            .unwrap_or(600);
        let a = Approval {
            id: workroam_core::crypto::random_id("ap"),
            job_id: Some(id.to_string()),
            kind: spec.kind().into(),
            project: Some(spec.project().into()),
            initiator: ctx.initiator(),
            remote: ctx.is_remote(),
            title: approval_title(spec),
            details: plan,
            code: None,
            created_at: Utc::now(),
            timeout_s: timeout,
        };
        d.jobs.update(id, |j| j.approval_id = Some(a.id.clone()));
        let ans = d.approvals.ask(d, a).await?;
        if let Some(s) = &ans.skip_ask {
            if !ctx.is_remote() {
                let dur = workroam_core::duration::parse(s)?;
                d.skip_ask.lock().unwrap().insert(
                    spec.project().to_string(),
                    Utc::now() + chrono::Duration::from_std(dur).unwrap_or_default(),
                );
            }
        }
        d.jobs.update(id, |j| j.state = JobState::Running);
    }
    let ws = d.ws().await?;
    let progress = Some(progress_fn(d, id));
    match spec {
        JobSpec::Push {
            project,
            release,
            takeover,
            note,
        } => {
            let p = Project::open(&ws, project)?;
            let out = workroam_engine::sync::push(
                &p,
                &PushOptions {
                    release: *release,
                    takeover: *takeover,
                    note: note.clone(),
                    progress,
                    ..Default::default()
                },
            )
            .await?;
            if !out.skipped {
                if let Some(s) = &out.snapshot {
                    d.announced
                        .lock()
                        .unwrap()
                        .insert(project.clone(), s.clone());
                }
            }
            crate::to_value(&out)
        }
        JobSpec::Pull {
            project,
            takeover,
            wait_s,
        } => {
            let deadline = wait_s.map(|s| Instant::now() + Duration::from_secs(s));
            loop {
                let mut p = Project::open(&ws, project)?;
                match workroam_engine::sync::pull(
                    &mut p,
                    &PullOptions {
                        takeover: *takeover,
                        progress: progress.clone(),
                        ..Default::default()
                    },
                )
                .await
                {
                    Ok(out) => {
                        if p.binding_state() != workroam_core::config::BindingState::Bound {
                            let dir = p.dir.clone();
                            let name = project.clone();
                            d.update_machine(|m| {
                                workroam_engine::init::bind(
                                    m,
                                    &name,
                                    &dir,
                                    workroam_core::config::BindingState::Bound,
                                )
                            })
                            .await?;
                        }
                        if let Some(s) = &out.snapshot {
                            d.announced
                                .lock()
                                .unwrap()
                                .insert(project.clone(), s.clone());
                        }
                        return crate::to_value(&out);
                    }
                    Err(e) if e.code == Code::LeaseHeld => match deadline {
                        Some(t) if Instant::now() < t => {
                            if let Some(f) = &progress {
                                f("waiting_lease", 0, None);
                            }
                            tokio::time::sleep(Duration::from_secs(15)).await;
                        }
                        _ => return Err(e),
                    },
                    Err(e) => return Err(e),
                }
            }
        }
        JobSpec::Restore { project, plan } => {
            let mut p = Project::open(&ws, project)?;
            crate::to_value(&workroam_engine::restore::restore(&mut p, plan).await?)
        }
        JobSpec::Archive {
            project,
            note,
            keep_history,
            takeover,
        } => {
            let p = Project::open(&ws, project)?;
            let out = workroam_archive::archive::create(
                &p,
                &ArchiveOptions {
                    note: note.clone(),
                    keep_history: *keep_history,
                    takeover: *takeover,
                    progress,
                },
            )
            .await?;
            let name = project.clone();
            d.update_machine(|m| {
                workroam_archive::archive::mark_local(
                    m,
                    &name,
                    workroam_core::config::BindingState::Archived,
                )
            })
            .await?;
            crate::to_value(&out)
        }
        JobSpec::ArchiveRestore {
            project,
            plan,
            prefer,
            takeover,
        } => {
            let out =
                workroam_archive::archive::restore(&ws, plan, prefer, *takeover, progress).await?;
            if out.restored {
                let dir = PathBuf::from(&plan.dir);
                let name = project.clone();
                d.update_machine(|m| {
                    workroam_engine::init::bind(
                        m,
                        &name,
                        &dir,
                        workroam_core::config::BindingState::Bound,
                    )
                })
                .await?;
            }
            crate::to_value(&out)
        }
    }
}
