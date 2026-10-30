//! Automatic sync (architecture §13): idle pushes from the file watcher,
//! pushes before sleep or shutdown, arrival checks every 5 minutes and
//! after wake, lease renewal every 10 minutes, credential rotation adoption,
//! the scheduled mirror, and finishing a pull the last run left half done
//! (§9.3).

use crate::{Ctx, Daemon};
use chrono::Utc;
use notify::{RecursiveMode, Watcher};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use workroam_core::config::{BindingState, MachineConfig, ProjectConfig};
use workroam_core::model::{AutoMode, HeadState};
use workroam_engine::Project;
use workroam_ipc::ClientKind;
use workroam_platform::power::PowerEvent;

const TICK: Duration = Duration::from_secs(30);
const ARRIVAL_EVERY: Duration = Duration::from_secs(5 * 60);
const RENEW_EVERY: Duration = Duration::from_secs(10 * 60);

fn auto_mode(m: &MachineConfig, dir: &Path) -> AutoMode {
    ProjectConfig::load(dir)
        .ok()
        .flatten()
        .and_then(|c| c.sync.auto)
        .unwrap_or(m.sync.auto)
}

fn ignored(rel: &Path) -> bool {
    let s = rel.to_string_lossy().replace('\\', "/");
    s.starts_with(".workroam/local")
        || s.contains("/.git/")
        || s.starts_with(".git/")
        || s.contains(".workroam/SYNCING")
        || s.contains("node_modules/")
        || s.contains("/target/")
}

/// Watch every bound project directory; remember the last write per project.
struct Watch {
    last_write: Arc<Mutex<HashMap<String, Instant>>>,
    watcher: Option<notify::RecommendedWatcher>,
    watched: HashSet<PathBuf>,
}

impl Watch {
    fn new() -> Watch {
        Watch {
            last_write: Arc::new(Mutex::new(HashMap::new())),
            watcher: None,
            watched: HashSet::new(),
        }
    }

    fn refresh(&mut self, m: &MachineConfig) {
        let dirs: HashMap<PathBuf, String> = m
            .projects
            .iter()
            .filter(|(_, b)| b.state == BindingState::Bound)
            .map(|(n, b)| (workroam_core::paths::expand_tilde(&b.path), n.clone()))
            .collect();
        let want: HashSet<PathBuf> = dirs.keys().cloned().collect();
        if want == self.watched && self.watcher.is_some() {
            return;
        }
        let last = self.last_write.clone();
        let map = dirs.clone();
        let w = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let Ok(ev) = res else { return };
            if matches!(ev.kind, notify::EventKind::Access(_)) {
                return;
            }
            for p in ev.paths {
                for (root, name) in &map {
                    if let Ok(rel) = p.strip_prefix(root) {
                        if !ignored(rel) {
                            last.lock().unwrap().insert(name.clone(), Instant::now());
                        }
                    }
                }
            }
        });
        let Ok(mut w) = w else {
            tracing::warn!("file watcher unavailable; idle pushes are off");
            return;
        };
        for d in &want {
            if let Err(e) = w.watch(d, RecursiveMode::Recursive) {
                tracing::warn!("cannot watch {}: {e}", d.display());
            }
        }
        self.watcher = Some(w);
        self.watched = want;
    }
}

pub fn spawn(d: Arc<Daemon>) {
    watch_power(&d);
    tokio::spawn(async move {
        recover(&d).await;
        let mut watch = Watch::new();
        let mut last_tick = Instant::now();
        let mut last_arrival = Instant::now() - ARRIVAL_EVERY;
        let mut last_renew = Instant::now();
        loop {
            tokio::select! {
                _ = tokio::time::sleep(TICK) => {}
                _ = d.kick.notified() => {}
            }
            let now = Instant::now();
            // The system said so, or a long gap between ticks means the computer slept.
            let woke =
                d.woke.swap(false, Ordering::SeqCst) || now.duration_since(last_tick) > TICK * 6;
            last_tick = now;
            let Ok(m) = crate::load_machine() else {
                continue;
            };
            watch.refresh(&m);
            crate::relay::ensure_running(&d);
            if woke && m.sync.on_wake {
                tracing::info!("wake detected; checking for new progress");
                // Give the network a moment to come back.
                tokio::time::sleep(Duration::from_secs(5)).await;
                last_arrival = now - ARRIVAL_EVERY;
            }
            if now.duration_since(last_arrival) >= ARRIVAL_EVERY {
                last_arrival = now;
                clear_stale_markers(&d, &m);
                if let Err(e) = arrivals(&d, &m).await {
                    tracing::debug!("arrival check: {}", e.message);
                }
            }
            if now.duration_since(last_renew) >= RENEW_EVERY {
                last_renew = now;
                renew(&d).await;
                adopt_rotation(&d).await;
                scheduled_backup(&d, &m).await;
            }
            if !d.is_paused() && m.sync.idle_minutes > 0 {
                idle_pushes(&d, &m, &watch).await;
            }
        }
    });
}

/// Hand power events to the trigger loop. A `WillSleep` with a grace
/// period (macOS) is handled before the callback returns, so the system
/// waits for the push; elsewhere the work is started and sleep proceeds.
fn watch_power(d: &Arc<Daemon>) {
    let handle = tokio::runtime::Handle::current();
    let d = d.clone();
    let ok = workroam_platform::power::watch(move |ev| match ev {
        PowerEvent::WillSleep { grace } if grace > Duration::ZERO => {
            let d = d.clone();
            handle.block_on(async move { before_sleep(&d, grace, "sleep").await });
        }
        PowerEvent::WillSleep { .. } => {
            let d = d.clone();
            handle.spawn(async move { before_sleep(&d, Duration::ZERO, "sleep").await });
        }
        PowerEvent::WillShutdown => {
            let d = d.clone();
            handle.spawn(async move { before_sleep(&d, Duration::ZERO, "shutdown").await });
        }
        PowerEvent::DidWake => {
            d.woke.store(true, Ordering::SeqCst);
            d.kick.notify_one();
        }
    });
    if !ok {
        tracing::info!("no power event source on this system; sync.on_sleep is inactive");
    }
}

/// `sync.on_sleep`: push every bound project (and release its lease)
/// before the computer sleeps or shuts down. `ask` posts the usual
/// approval card, which waits until someone is back; before a shutdown
/// nobody can answer, so only `auto` acts then. With a grace period the
/// call waits for the jobs that long.
pub async fn before_sleep(d: &Arc<Daemon>, grace: Duration, trigger: &str) {
    let Ok(m) = crate::load_machine() else { return };
    let mode = m.sync.on_sleep;
    if mode == AutoMode::Off || d.is_paused() || (trigger == "shutdown" && mode != AutoMode::Auto) {
        return;
    }
    let mut jobs = Vec::new();
    for (name, b) in &m.projects {
        if b.state != BindingState::Bound || d.jobs.running_for(name) {
            continue;
        }
        let tier = auto_mode(&m, &workroam_core::paths::expand_tilde(&b.path));
        if tier == AutoMode::Off {
            continue;
        }
        let ask = mode == AutoMode::Ask || tier == AutoMode::Ask;
        match release_push(d, name, ask, trigger).await {
            Ok(Some(job)) => jobs.push(job),
            Ok(None) => {}
            Err(e) => tracing::debug!("{trigger} push of {name}: {}", e.message),
        }
    }
    if grace > Duration::ZERO && !jobs.is_empty() {
        tracing::info!(
            "waiting up to {grace:?} for {} job(s) before {trigger}",
            jobs.len()
        );
        let waits = jobs.iter().map(|id| d.jobs.wait(id));
        let _ = tokio::time::timeout(grace, futures_util::future::join_all(waits)).await;
    }
}

/// Plan a push that releases the lease and start it. Nothing to push and
/// no lease to release: no job. In `ask` mode a lease alone is not worth a
/// card.
async fn release_push(
    d: &Arc<Daemon>,
    project: &str,
    ask: bool,
    trigger: &str,
) -> workroam_core::Result<Option<String>> {
    let ctx = Ctx::human(ClientKind::Desktop);
    let plans = crate::handlers::handle(
        d,
        &ctx,
        "push.plan",
        json!({"project": project, "release": true, "trigger": trigger}),
    )
    .await?;
    let first = plans.get(0).cloned().unwrap_or_default();
    let summary = &first["plan"]["summary"];
    let files = ["added", "modified", "deleted"]
        .iter()
        .map(|k| summary[k].as_u64().unwrap_or(0))
        .sum::<u64>();
    let lease_mine = first["plan"]["lease"]["state"] == "mine";
    if files == 0 && (ask || !lease_mine) {
        return Ok(None);
    }
    let Some(job) = first["job"].as_str() else {
        return Ok(None);
    };
    crate::jobs::start(d, &ctx, job, ask, false).await?;
    Ok(Some(job.to_string()))
}

/// Finish what the last run left behind (architecture §9.3): a pull whose
/// operation log has no end is run again (completed steps are skipped),
/// and a SYNCING marker of a dead process is removed.
async fn recover(d: &Arc<Daemon>) {
    let Ok(m) = crate::load_machine() else { return };
    clear_stale_markers(d, &m);
    let Ok(ws) = d.ws().await else { return };
    for (name, b) in &m.projects {
        if b.state != BindingState::Bound {
            continue;
        }
        let Ok(pr) = Project::open(&ws, name) else {
            continue;
        };
        let Some(snap) = workroam_engine::sync::interrupted_pull(&pr) else {
            continue;
        };
        tracing::warn!(
            "{name}: finishing the interrupted pull of {}",
            &snap[..snap.len().min(8)]
        );
        let ctx = Ctx::human(ClientKind::Desktop);
        let res = async {
            let plans = crate::handlers::handle(
                d,
                &ctx,
                "pull.plan",
                json!({"project": name, "trigger": "recovery"}),
            )
            .await?;
            let first = plans.get(0).cloned().unwrap_or_default();
            match first["job"].as_str() {
                // The person approved this pull once already.
                Some(job) => crate::jobs::start(d, &ctx, job, false, true)
                    .await
                    .map(|_| ()),
                None => Ok(()),
            }
        }
        .await;
        if let Err(e) = res {
            tracing::warn!(
                "{name}: the interrupted pull could not be finished: {}",
                e.message
            );
        }
    }
}

fn clear_stale_markers(d: &Daemon, m: &MachineConfig) {
    for (name, b) in &m.projects {
        if b.state != BindingState::Bound || d.jobs.running_for(name) {
            continue;
        }
        let dir = workroam_core::paths::expand_tilde(&b.path);
        if workroam_engine::local::clear_stale_syncing(&dir) {
            tracing::warn!("{name}: removed the SYNCING marker of a sync that did not finish");
        }
    }
}

async fn idle_pushes(d: &Arc<Daemon>, m: &MachineConfig, w: &Watch) {
    let idle = Duration::from_secs(m.sync.idle_minutes as u64 * 60);
    let due: Vec<String> = {
        let g = w.last_write.lock().unwrap();
        g.iter()
            .filter(|(_, t)| t.elapsed() >= idle)
            .map(|(n, _)| n.clone())
            .collect()
    };
    for name in due {
        w.last_write.lock().unwrap().remove(&name);
        let Some(b) = m.projects.get(&name) else {
            continue;
        };
        let mode = auto_mode(m, &workroam_core::paths::expand_tilde(&b.path));
        if mode == AutoMode::Off || d.jobs.running_for(&name) {
            continue;
        }
        let _ = auto_job(d, "push.plan", &name, mode == AutoMode::Ask, "idle").await;
    }
}

/// Plan and start a job like a person would, with `ask` for "ask" mode.
async fn auto_job(
    d: &Arc<Daemon>,
    method: &str,
    project: &str,
    ask: bool,
    trigger: &str,
) -> workroam_core::Result<()> {
    let ctx = Ctx::human(ClientKind::Desktop);
    let plans = crate::handlers::handle(
        d,
        &ctx,
        method,
        json!({"project": project, "trigger": trigger}),
    )
    .await?;
    let first = plans.get(0).cloned().unwrap_or_default();
    if method == "push.plan" {
        let summary = &first["plan"]["summary"];
        let files = ["added", "modified", "deleted"]
            .iter()
            .map(|k| summary[k].as_u64().unwrap_or(0))
            .sum::<u64>();
        if files == 0 {
            return Ok(());
        }
    }
    if let Some(job) = first["job"].as_str() {
        let touches = first["plan"]["touches_local"].as_bool().unwrap_or(false);
        crate::jobs::start(d, &ctx, job, ask || touches, false).await?;
    }
    Ok(())
}

async fn arrivals(d: &Arc<Daemon>, m: &MachineConfig) -> workroam_core::Result<()> {
    let ws = d.ws().await?;
    for (name, b) in &m.projects {
        if b.state != BindingState::Bound {
            continue;
        }
        let Some((head, _)) = ws.head(name).await? else {
            continue;
        };
        if head.state == HeadState::Archived {
            if b.state != BindingState::Archived {
                let n = name.clone();
                d.update_machine(|m| {
                    if let Some(x) = m.projects.get_mut(&n) {
                        x.state = BindingState::Archived;
                    }
                })
                .await?;
                d.emit("status.changed", json!({"project": name, "archived": true}));
            }
            continue;
        }
        let pr = Project::open(&ws, name)?;
        let base = pr.state().base_snapshot;
        let Some(snap) = head.snapshot.clone() else {
            continue;
        };
        if Some(&snap) == base.as_ref() || head.device_id == m.device_id {
            continue;
        }
        if d.announced.lock().unwrap().get(name) == Some(&snap) {
            continue;
        }
        d.announced
            .lock()
            .unwrap()
            .insert(name.clone(), snap.clone());
        let plan = workroam_engine::sync::plan_pull(&pr, None).await?;
        let ev = json!({
            "project": name,
            "snapshot": snap,
            "from_device": head.device_name,
            "time": head.created_at,
            "summary": head.summary,
            "touches_local": plan.touches_local,
            "local_changes": plan.local_changes,
        });
        d.emit("arrival", ev.clone());
        crate::notify::send(d, "arrival", name, json!({"message": format!("{} pushed {name}", head.device_name), "changes": head.summary})).await;
        let mode = auto_mode(m, &pr.dir);
        if mode == AutoMode::Auto && !d.is_paused() && !d.jobs.running_for(name) {
            let _ = auto_job(d, "pull.plan", name, plan.touches_local, "arrival").await;
        }
    }
    Ok(())
}

async fn renew(d: &Arc<Daemon>) {
    let Ok(ws) = d.ws().await else { return };
    for name in ws.machine.projects.keys() {
        let pid = ws.project_id(name);
        let _ = workroam_lease::renew(&ws.control, &pid, &ws.device()).await;
    }
    let _ = workroam_device::registry::touch(&ws.control, &ws.machine.device_id, false).await;
}

async fn adopt_rotation(d: &Arc<Daemon>) {
    let Ok(ws) = d.ws().await else { return };
    let Some(c) = ws.creds.clone() else { return };
    match workroam_device::rotation::adopt(&ws.control, &ws.machine.device_id, &c.access_key).await
    {
        Ok(workroam_device::rotation::Adopt::Switch(k)) => {
            let ok = workroam_platform::secrets::set(
                workroam_platform::secrets::keys::ACCESS_KEY,
                &k.access_key,
            )
            .and_then(|_| {
                workroam_platform::secrets::set(
                    workroam_platform::secrets::keys::SECRET_KEY,
                    &k.secret_key,
                )
            });
            if ok.is_ok() {
                tracing::info!("switched to the rotated storage key");
                d.reset_ws().await;
            }
        }
        Ok(_) => {}
        Err(e) => tracing::debug!("rotation: {}", e.message),
    }
}

async fn scheduled_backup(d: &Arc<Daemon>, m: &MachineConfig) {
    let Some(cfg) = &m.backup else { return };
    let st = workroam_archive::mirror::MirrorState::load_from(
        &workroam_archive::mirror::MirrorState::path(),
    );
    let every = match cfg.schedule.as_str() {
        "hourly" => chrono::Duration::hours(1),
        "weekly" => chrono::Duration::days(7),
        "manual" | "off" => return,
        _ => chrono::Duration::days(1),
    };
    if st.last_ok.map(|t| Utc::now() - t < every).unwrap_or(false) {
        return;
    }
    if let Err(e) = crate::handlers::backup_run(d).await {
        tracing::warn!("scheduled mirror failed: {}", e.message);
    }
}
