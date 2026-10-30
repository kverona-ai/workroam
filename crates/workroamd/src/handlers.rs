//! Every IPC method (architecture §15). Business logic, validation and
//! defaults live here so the desktop app and the CLI behave the same.

use crate::approvals::{Answer, Decision};
use crate::jobs::{self, JobSpec};
use crate::{p_bool, p_req, p_str, to_value, Ctx, Daemon};
use chrono::Utc;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use workroam_archive::archive::{self as arch, Prefer};
use workroam_core::config::{BackupConfig, BindingState, MachineConfig, ProjectConfig};
use workroam_core::model::{Access, Grant, HeadState};
use workroam_core::{Code, Error, Result};
use workroam_engine::restore::RestoreTarget;
use workroam_engine::{history, init, sync, Project};
use workroam_platform::secrets;
use workroam_setup::session::{BucketChoice, KeyMode, MachineInput, SetupMode, SetupSession};

pub async fn handle(d: &Arc<Daemon>, ctx: &Ctx, method: &str, p: Value) -> Result<Value> {
    let Some(m) = workroam_ipc::methods::find(method) else {
        return Err(Error::invalid(format!("unknown method {method}")));
    };
    // Remote callers only reach what their grant allows (§16.4).
    if ctx.is_remote() && !m.read_only && !matches!(method, "push.plan" | "pull.plan" | "job.start")
    {
        return Err(Error::new(
            Code::Denied,
            format!("{method} is not available remotely"),
        ));
    }
    match method {
        "daemon.info" => Ok(json!({
            "version": env!("CARGO_PKG_VERSION"),
            "service": d.service,
            "started_at": d.started_at,
            "configured": crate::is_configured(),
            "paused": d.is_paused(),
            "paused_until": *d.paused_until.lock().unwrap(),
            "socket": workroam_core::paths::socket_path(),
            "relay": d.relay.status(),
        })),
        "daemon.shutdown" => {
            if ctx.is_agent() {
                return Err(Error::new(Code::Denied, "agents cannot stop the service"));
            }
            tokio::spawn(async {
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                std::process::exit(0);
            });
            Ok(json!({"stopping": true}))
        }
        m if m.starts_with("setup.") => setup(d, m, &p).await,
        "doctor.run" => doctor(d).await,
        "device.pairing_code" => pairing_code(d, p_bool(&p, "read_only")).await,
        "device.list" => device_list(d).await,
        "device.remove" => device_remove(d, &p).await,
        "project.scan" => project_scan(d, &p).await,
        "project.init" => project_init(d, &p).await,
        "project.add" => project_add(d, &p).await,
        "project.list" => project_list(d).await,
        "project.move" => {
            let name = p_req(&p, "project")?;
            let path = PathBuf::from(p_req(&p, "path")?);
            let mut err = None;
            let m = d
                .update_machine(|m| {
                    if let Err(e) = init::move_dir(m, &name, &path) {
                        err = Some(e);
                    }
                })
                .await?;
            if let Some(e) = err {
                return Err(e);
            }
            Ok(json!({"project": name, "dir": m.projects.get(&name).map(|b| b.path.clone())}))
        }
        "project.remove" => {
            let name = project_name(&p)?;
            let mut removed = None;
            d.update_machine(|m| removed = init::unbind(m, &name))
                .await?;
            let b = removed
                .ok_or_else(|| Error::invalid(format!("project {name} is not on this device")))?;
            if let Ok(ws) = d.ws().await {
                let pid = ws.project_id(&name);
                let _ = workroam_lease::release(&ws.control, &pid, &ws.device()).await;
            }
            Ok(json!({"project": name, "dir": b.path, "files_kept": true}))
        }
        "status" => status(d, &p).await,
        "handoff.read" => handoff(d, &p).await,
        "lease.takeover" => {
            let ws = d.ws().await?;
            ws.ensure_writable()?;
            let name = project_name(&p)?;
            let pr = Project::open(&ws, &name)?;
            let prev = sync::take_lease(&pr, true).await?;
            Ok(
                json!({"project": name, "took_over_from": prev, "lease": sync::lease_view(&pr).await?}),
            )
        }
        "push.plan" => push_plan(d, ctx, &p).await,
        "pull.plan" => pull_plan(d, ctx, &p).await,
        "restore.plan" => restore_plan(d, ctx, &p).await,
        "job.start" => {
            let id = p_req(&p, "job")?;
            if let Some(j) = d.jobs.get(&id) {
                if ctx.is_remote() && !matches!(j.kind.as_str(), "push" | "pull") {
                    return Err(Error::new(
                        Code::Denied,
                        "only push and pull can be requested remotely",
                    ));
                }
            }
            jobs::start(d, ctx, &id, p_bool(&p, "ask"), p_bool(&p, "wait")).await
        }
        "job.cancel" => to_value(&d.jobs.cancel(d, &p_req(&p, "job")?)?),
        "job.list" => {
            if let Some(id) = p_str(&p, "job") {
                return to_value(
                    &d.jobs
                        .get(&id)
                        .ok_or_else(|| Error::invalid(format!("no job {id}")))?,
                );
            }
            to_value(&d.jobs.list(p_bool(&p, "all")))
        }
        "approval.list" => {
            if let Some(id) = p_str(&p, "id") {
                return to_value(
                    &d.approvals
                        .get(&id)
                        .ok_or_else(|| Error::invalid(format!("no pending approval {id}")))?,
                );
            }
            to_value(&d.approvals.list())
        }
        "approval.respond" => {
            if ctx.is_agent() {
                return Err(
                    Error::new(Code::Denied, "an agent cannot answer its own approval")
                        .with_hint("answer on the card or with workroam approve in your terminal"),
                );
            }
            let ans = Answer {
                decision: Decision::parse(&p_req(&p, "decision")?)?,
                skip_ask: p_str(&p, "skip_ask"),
                access: p_str(&p, "access"),
                projects: p
                    .get("projects")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default(),
            };
            let a = d.approvals.respond(&p_req(&p, "id")?, ans.clone())?;
            Ok(json!({"id": a.id, "decision": ans.decision, "job": a.job_id}))
        }
        "history.list" => {
            let ws = d.ws().await?;
            let pr = Project::open(&ws, &project_name(&p)?)?;
            let limit = p.get("limit").and_then(|v| v.as_u64()).map(|n| n as usize);
            to_value(&history::list(&pr, limit.or(Some(20)))?)
        }
        "history.show" => {
            let ws = d.ws().await?;
            let pr = Project::open(&ws, &project_name(&p)?)?;
            let id = history::resolve(&pr, &p_req(&p, "snapshot")?).await?;
            let meta = history::ensure_exists(&ws, &id)?;
            let changes = history::diff(&pr, meta.parent.as_deref(), &id)?;
            Ok(json!({"snapshot": history::info(&meta), "changes": changes}))
        }
        "archive.plan" => archive_plan(d, ctx, &p).await,
        "archive.list" => {
            to_value(&arch::list(&*d.ws().await?, p_str(&p, "search").as_deref()).await?)
        }
        "archive.restore_plan" | "archive.restore" => archive_restore(d, ctx, method, &p).await,
        "archive.extract" => {
            let ws = d.ws().await?;
            to_value(
                &arch::extract(&ws, &p_req(&p, "project")?, PathBuf::from(p_req(&p, "to")?))
                    .await?,
            )
        }
        "archive.delete" => {
            let ws = d.ws().await?;
            let name = p_req(&p, "project")?;
            if p_str(&p, "confirm").as_deref() != Some(name.as_str()) {
                return Err(
                    Error::invalid("type the project name to confirm the deletion")
                        .with_hint(format!("--confirm {name}")),
                );
            }
            let out = arch::delete(&ws, &name).await?;
            d.update_machine(|m| {
                if m.projects.get(&name).map(|b| b.state) == Some(BindingState::Archived) {
                    m.projects.remove(&name);
                }
            })
            .await?;
            to_value(&out)
        }
        "archive.export" => {
            let ws = d.ws().await?;
            let pw = match p_str(&p, "password") {
                Some(pw) => pw,
                None => secrets::require(secrets::keys::PASSWORD)?,
            };
            to_value(
                &workroam_archive::package::export(
                    &ws,
                    &p_req(&p, "project")?,
                    Path::new(&p_req(&p, "file")?),
                    &pw,
                )
                .await?,
            )
        }
        "archive.import" => {
            let ws = d.ws().await?;
            let pw = match p_str(&p, "password") {
                Some(pw) => pw,
                None => secrets::require(secrets::keys::PASSWORD)?,
            };
            to_value(
                &workroam_archive::package::import(
                    &ws,
                    Path::new(&p_req(&p, "file")?),
                    &pw,
                    p_str(&p, "name").as_deref(),
                )
                .await?,
            )
        }
        "backup.configure" => backup_configure(d, &p).await,
        "backup.run" => backup_run(d).await,
        "verify.run" => {
            let pct = p.get("read_data").and_then(|v| v.as_f64());
            let ws = if p_bool(&p, "backup") {
                mirror_workspace(d).await?
            } else {
                d.ws().await?
            };
            to_value(
                &workroam_archive::verify::verify(&ws, pct)
                    .await?
                    .into_result()?,
            )
        }
        "prune.run" => {
            let ws = d.ws().await?;
            to_value(&history::prune(&ws, p_bool(&p, "dry_run")).await?)
        }
        "config.list" => {
            to_value(&crate::settings::list(d, p_str(&p, "project").as_deref()).await?)
        }
        "config.get" => to_value(
            &crate::settings::get(d, &p_req(&p, "key")?, p_str(&p, "project").as_deref()).await?,
        ),
        "config.set" => {
            let v = p
                .get("value")
                .map(|v| match v {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                })
                .ok_or_else(|| Error::invalid("missing parameter `value`"))?;
            let out =
                crate::settings::set(d, &p_req(&p, "key")?, &v, p_str(&p, "project").as_deref())
                    .await?;
            d.emit("status.changed", json!({"setting": out.key}));
            to_value(&out)
        }
        "credentials.update" => credentials_update(d, &p).await,
        "password.change" => password_change(d, &p).await,
        "password.reset" => password_reset(d, &p).await,
        "sync.pause" => {
            let dur =
                workroam_core::duration::parse(&p_str(&p, "for").unwrap_or_else(|| "1h".into()))?;
            let until = Utc::now() + chrono::Duration::from_std(dur).unwrap_or_default();
            *d.paused_until.lock().unwrap() = Some(until);
            d.update_machine(|m| m.sync.paused_until = until.to_rfc3339())
                .await?;
            d.emit("status.changed", json!({"paused_until": until}));
            Ok(json!({"paused_until": until}))
        }
        "sync.resume" => {
            *d.paused_until.lock().unwrap() = None;
            d.update_machine(|m| m.sync.paused_until.clear()).await?;
            d.emit("status.changed", json!({"paused_until": null}));
            Ok(json!({"paused_until": null}))
        }
        "agent.list" => agent_list(d).await,
        "agent.connect" => crate::relay::connect(d, ctx, &p).await,
        "agent.disconnect" => crate::relay::disconnect(d, &p).await,
        "agent.webhook" => {
            let ws = d.ws().await?;
            let agent = p_req(&p, "agent")?;
            let url = p_req(&p, "url")?;
            url::Url::parse(&url).map_err(|_| Error::invalid(format!("{url} is not a URL")))?;
            let mut n = 0;
            for (k, mut g) in ws
                .control
                .get_all::<Grant>(workroam_store::control::keys::GRANTS)
                .await?
            {
                if g.agent == agent && !g.revoked {
                    g.webhook_url = Some(url.clone());
                    ws.control.put(&k, &g).await?;
                    n += 1;
                }
            }
            if n == 0 {
                return Err(Error::invalid(format!("{agent} is not connected"))
                    .with_hint(format!("workroam agent connect {agent}")));
            }
            if let Some(s) = p_str(&p, "secret") {
                secrets::set(&secrets::keys::webhook(&agent), &s)?;
            }
            Ok(json!({"agent": agent, "url": url, "connections": n}))
        }
        "hello" | "subscribe" => Ok(Value::Null),
        other => Err(Error::internal(format!("{other} is not implemented"))),
    }
}

// ------------------------------------------------------------ helpers --

/// Project from `project`, or the bound project whose directory holds `cwd`.
pub fn project_name(p: &Value) -> Result<String> {
    if let Some(n) = p_str(p, "project") {
        return Ok(n);
    }
    let m = crate::load_machine()?;
    if let Some(cwd) = p_str(p, "cwd") {
        if let Some(n) = project_for_dir(&m, Path::new(&cwd)) {
            return Ok(n);
        }
    }
    Err(
        Error::invalid("this directory is not inside a Workroam project")
            .with_hint("cd into a project, or pass -p <project>"),
    )
}

pub fn project_for_dir(m: &MachineConfig, dir: &Path) -> Option<String> {
    let dir = workroam_core::paths::normalize(dir);
    let mut best: Option<(usize, String)> = None;
    for (name, b) in &m.projects {
        let root = workroam_core::paths::normalize(&workroam_core::paths::expand_tilde(&b.path));
        if dir.starts_with(&root) {
            let len = root.as_os_str().len();
            if best.as_ref().map(|(l, _)| len > *l).unwrap_or(true) {
                best = Some((len, name.clone()));
            }
        }
    }
    best.map(|(_, n)| n)
}

fn targets(m: &MachineConfig, p: &Value) -> Result<Vec<String>> {
    if p_bool(p, "all") {
        let v: Vec<String> = m
            .projects
            .iter()
            .filter(|(_, b)| b.state == BindingState::Bound)
            .map(|(n, _)| n.clone())
            .collect();
        if v.is_empty() {
            return Err(Error::invalid("no project is synced on this device yet"));
        }
        return Ok(v);
    }
    Ok(vec![project_name(p)?])
}

// --------------------------------------------------------------- setup --

async fn setup(d: &Arc<Daemon>, method: &str, p: &Value) -> Result<Value> {
    if method == "setup.begin" {
        let mode = match p_str(p, "mode").as_deref().unwrap_or("new") {
            "new" => SetupMode::New,
            "join" => SetupMode::Join,
            "from_backup" | "from-backup" => SetupMode::FromBackup,
            other => return Err(Error::invalid(format!("unknown setup mode {other}"))),
        };
        let s = SetupSession::new(mode);
        let out = json!({
            "session": s.id,
            "mode": mode,
            "configured": crate::is_configured(),
            "defaults": {
                "device_name": workroam_platform::sysinfo::default_device_name(),
                "root": workroam_setup::session::default_root(),
                "presets": workroam_store::location::presets().iter().map(|p| p.id.to_string()).collect::<Vec<_>>(),
            },
            "agents": workroam_adapters::registry::all().iter().map(workroam_adapters::registry::detect).collect::<Vec<_>>(),
        });
        d.setup.lock().await.insert(s.id.clone(), s);
        return Ok(out);
    }
    let id = p_req(p, "session")?;
    let mut map = d.setup.lock().await;
    let s = map.get_mut(&id).ok_or_else(|| {
        Error::invalid(format!("no setup session {id}")).with_hint("start again with setup.begin")
    })?;
    match method {
        "setup.cancel" => {
            map.remove(&id);
            Ok(json!({"cancelled": true}))
        }
        "setup.check" => {
            let d2 = d.clone();
            let sid = id.clone();
            let mut each = move |r: &workroam_setup::checks::CheckResult| {
                d2.emit("setup.check_result", json!({"session": sid, "result": r}));
            };
            let results = s.run_checks(&mut each).await?;
            let passed = s.checks_passed();
            Ok(json!({
                "results": results,
                "passed": passed.is_ok(),
                "blocker": passed.err(),
                "broad_key": s.findings.broad_key,
            }))
        }
        "setup.commit" => {
            let out = s.commit().await?;
            map.remove(&id);
            drop(map);
            d.reset_ws().await;
            if d.service {
                crate::relay::ensure_running(d);
            }
            to_value(&out)
        }
        "setup.step" => setup_step(s, p).await,
        _ => Err(Error::invalid(format!("unknown method {method}"))),
    }
}

async fn setup_step(s: &mut SetupSession, p: &Value) -> Result<Value> {
    let step = p_req(p, "step")?;
    match step.as_str() {
        "endpoint" => {
            let addressing = match p_str(p, "addressing").as_deref() {
                Some("path") => Some(workroam_core::config::Addressing::Path),
                Some("virtual-host") | Some("virtual_host") => {
                    Some(workroam_core::config::Addressing::VirtualHost)
                }
                Some(other) => return Err(Error::invalid(format!("unknown addressing {other}"))),
                None => None,
            };
            to_value(&s.set_endpoint(
                &p_req(p, "preset")?,
                &p_str(p, "endpoint").unwrap_or_default(),
                addressing,
                p_str(p, "region"),
            )?)
        }
        "credentials" => to_value(
            &s.set_credentials(
                &p_str(p, "access_key").unwrap_or_default(),
                &p_str(p, "secret_key").unwrap_or_default(),
            )
            .await?,
        ),
        "key_mode" => {
            let mode = KeyMode::parse(&p_str(p, "mode").unwrap_or_else(|| "current".into()))
                .ok_or_else(|| Error::invalid("key mode is create or current"))?;
            let access = match p_str(p, "access").as_deref() {
                Some("read-only") | Some("read_only") => Access::ReadOnly,
                _ => Access::ReadWrite,
            };
            s.set_key_mode(
                mode,
                access,
                p_str(p, "admin_endpoint"),
                p_str(p, "admin_token"),
            )?;
            if p_bool(p, "accept_broad_key") {
                s.accept_broad_key = true;
            }
            Ok(json!({"mode": mode, "access": access}))
        }
        "accept_broad_key" => {
            s.accept_broad_key = true;
            Ok(json!({"accepted": true}))
        }
        "bucket" => to_value(
            &s.set_bucket(
                BucketChoice::parse(&p_str(p, "choice").unwrap_or_else(|| "auto".into())),
                p_str(p, "name"),
                &p_str(p, "prefix").unwrap_or_default(),
            )
            .await?,
        ),
        "password" => to_value(&s.set_password(&p_req(p, "password")?)?),
        "confirm_recovery" => {
            s.confirm_recovery(&p_req(p, "typed")?)?;
            Ok(json!({"confirmed": true}))
        }
        "recovery_saved" => {
            s.mark_recovery_saved();
            Ok(json!({"confirmed": true}))
        }
        "machine" => {
            let m: MachineInput =
                serde_json::from_value(p.get("machine").cloned().unwrap_or_else(|| p.clone()))
                    .map_err(|e| Error::invalid(format!("machine settings: {e}")))?;
            s.set_machine(m)?;
            Ok(json!({"ok": true}))
        }
        "pairing_code" => {
            s.set_pairing_code(&p_req(p, "code")?)?;
            Ok(json!({"ok": true}))
        }
        "unlock" => to_value(
            &s.unlock(
                p_str(p, "password").as_deref(),
                p_str(p, "recovery_key").as_deref(),
            )
            .await?,
        ),
        "backup_source" => {
            let loc = workroam_core::config::WorkspaceLocation {
                endpoint: p_req(p, "endpoint")?,
                bucket: p_str(p, "bucket").unwrap_or_default(),
                prefix: p_str(p, "prefix").unwrap_or_default(),
                region: p_str(p, "region").unwrap_or_default(),
                preset: p_str(p, "preset").unwrap_or_else(|| "other".into()),
                ..Default::default()
            };
            let creds = p_str(p, "access_key").map(|ak| workroam_store::Creds {
                access_key: ak,
                secret_key: p_str(p, "secret_key").unwrap_or_default(),
            });
            s.set_backup_source(loc, creds);
            Ok(json!({"ok": true}))
        }
        "projects" => {
            let list: Vec<(String, Option<String>)> = p
                .get("projects")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| {
                            let name = x.get("name").and_then(|n| n.as_str())?;
                            Some((
                                name.to_string(),
                                x.get("dir").and_then(|d| d.as_str()).map(String::from),
                            ))
                        })
                        .collect()
                })
                .unwrap_or_default();
            s.set_projects(list, p_bool(p, "all"));
            if let Some(dev) = p_str(p, "as_device") {
                s.as_device = Some(dev);
            }
            Ok(json!({"ok": true}))
        }
        other => Err(Error::invalid(format!("unknown setup step {other}"))),
    }
}

async fn doctor(d: &Arc<Daemon>) -> Result<Value> {
    let mut results = workroam_setup::checks::local_checks();
    let d2 = d.clone();
    let mut each = move |r: &workroam_setup::checks::CheckResult| {
        d2.emit(
            "setup.check_result",
            json!({"session": "doctor", "result": r}),
        );
    };
    if let Ok(m) = crate::load_machine() {
        let creds = crate::storage_creds(&m).ok().flatten();
        let t = workroam_setup::checks::StorageTarget {
            loc: &m.workspace,
            creds: creds.as_ref(),
            bucket_may_be_missing: false,
        };
        let (r, _) = workroam_setup::checks::storage_checks(&t, &mut each).await;
        results.extend(r);
        if m.backup.is_some() {
            let st = workroam_archive::mirror::MirrorState::load_from(
                &workroam_archive::mirror::MirrorState::path(),
            );
            let reach = match &st.last_error {
                Some(e) => Err(e.clone()),
                None => Ok(()),
            };
            results.push(workroam_setup::checks::backup_check(st.last_ok, reach).await);
        }
    }
    let ok = workroam_setup::checks::all_ok(&results);
    let out = json!({"results": results, "passed": ok, "ran_at": Utc::now()});
    if !ok {
        return Err(Error::new(Code::ChecksFailed, "some checks did not pass").with_data(out));
    }
    Ok(out)
}

// ------------------------------------------------------------- devices --

async fn pairing_code(d: &Arc<Daemon>, read_only: bool) -> Result<Value> {
    let ws = d.ws().await?;
    let password = secrets::require(secrets::keys::PASSWORD)?;
    let creds = workroam_device::rotation::read(&ws.control).await?;
    let (ak, sk, scope) = if read_only {
        match &creds.read_only {
            Some(k) => (k.access_key.clone(), k.secret_key.clone(), Access::ReadOnly),
            None => {
                let c = ws.creds.clone().unwrap_or_default_creds();
                (c.0, c.1, Access::ReadOnly)
            }
        }
    } else {
        match &creds.current {
            Some(k) => (
                k.access_key.clone(),
                k.secret_key.clone(),
                Access::ReadWrite,
            ),
            None => {
                let c = ws.creds.clone().unwrap_or_default_creds();
                (c.0, c.1, Access::ReadWrite)
            }
        }
    };
    let payload = workroam_device::PairingPayload::from_location(ws.location(), &ak, &sk, scope);
    let code = workroam_device::PairingCode::encode(&payload, &password)?;
    let shared_key = read_only && creds.read_only.is_none() && !ws.location().is_local();
    Ok(json!({
        "pairing_code": code,
        "qr_svg": workroam_device::qr::svg(&code),
        "qr_text": workroam_device::qr::terminal(&code),
        "read_only": read_only,
        "warning": if shared_key { Some("this storage has no separate read-only key; the device is read-only by policy but holds the read-write key") } else { None },
    }))
}

trait CredsExt {
    fn unwrap_or_default_creds(self) -> (String, String);
}

impl CredsExt for Option<workroam_store::Creds> {
    fn unwrap_or_default_creds(self) -> (String, String) {
        self.map(|c| (c.access_key, c.secret_key))
            .unwrap_or_default()
    }
}

async fn device_list(d: &Arc<Daemon>) -> Result<Value> {
    let ws = d.ws().await?;
    let devices = workroam_device::registry::list(&ws.control).await?;
    let mut leases = Vec::new();
    for pr in ws.projects().await? {
        if let Some((l, _)) = workroam_lease::read(&ws.control, &pr.pid).await? {
            if !workroam_lease::is_stale(&l) {
                leases.push((pr.name.clone(), l.holder.id.clone()));
            }
        }
    }
    let rows: Vec<Value> = devices
        .into_iter()
        .map(|dev| {
            let held: Vec<&String> = leases
                .iter()
                .filter(|(_, id)| *id == dev.id)
                .map(|(n, _)| n)
                .collect();
            json!({
                "id": dev.id,
                "name": dev.name,
                "os": dev.os,
                "access": dev.access,
                "reachable": dev.reachable,
                "last_seen": dev.last_seen,
                "last_sync": dev.last_sync,
                "joined_at": dev.joined_at,
                "leases": held,
                "this_device": dev.id == ws.machine.device_id,
            })
        })
        .collect();
    Ok(json!(rows))
}

async fn device_remove(d: &Arc<Daemon>, p: &Value) -> Result<Value> {
    let ws = d.ws().await?;
    ws.ensure_writable()?;
    let who = p_req(p, "device")?;
    let dev = workroam_device::registry::find(&ws.control, &who)
        .await?
        .ok_or_else(|| Error::invalid(format!("no device named {who}")))?;
    if dev.id == ws.machine.device_id {
        return Err(
            Error::invalid("this is the current device").with_hint("remove it from another device")
        );
    }
    workroam_device::registry::remove(&ws.control, &dev.id).await?;
    // Drop its leases so nobody waits for a removed device.
    for pr in ws.projects().await? {
        if let Some((l, _)) = workroam_lease::read(&ws.control, &pr.pid).await? {
            if l.holder.id == dev.id {
                let _ = ws
                    .control
                    .delete(&workroam_store::control::keys::lease(&pr.pid))
                    .await;
            }
        }
    }
    let mut rotation = false;
    if let (Some(ak), Some(sk)) = (p_str(p, "new_access_key"), p_str(p, "new_secret_key")) {
        workroam_device::rotation::begin(
            &ws.control,
            workroam_core::model::AccessKey {
                access_key: ak,
                secret_key: sk,
                ..Default::default()
            },
            &ws.machine.device_id,
        )
        .await?;
        rotation = true;
    }
    Ok(json!({
        "removed": dev.name,
        "rotation_started": rotation,
        "hint": if rotation { None } else { Some("rotate the storage key so the removed device loses access: workroam credentials update") },
    }))
}

// ------------------------------------------------------------ projects --

fn dir_param(p: &Value) -> Result<PathBuf> {
    let d = p_str(p, "dir")
        .or_else(|| p_str(p, "cwd"))
        .ok_or_else(|| Error::invalid("missing parameter `dir`"))?;
    Ok(workroam_core::paths::normalize(
        &workroam_core::paths::expand_tilde(&d),
    ))
}

async fn project_scan(d: &Arc<Daemon>, p: &Value) -> Result<Value> {
    let ws = d.ws().await?;
    let dir = dir_param(p)?;
    let name = p_str(p, "name").unwrap_or_else(|| {
        dir.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "project".into())
    });
    to_value(&init::plan_init(&ws, &name, &dir).await?)
}

async fn project_init(d: &Arc<Daemon>, p: &Value) -> Result<Value> {
    let ws = d.ws().await?;
    ws.ensure_writable()?;
    let dir = dir_param(p)?;
    let name = p_str(p, "name").unwrap_or_else(|| {
        dir.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "project".into())
    });
    let plan = init::plan_init(&ws, &name, &dir).await?;
    let choices: init::InitChoices = p
        .get("choices")
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|e| Error::invalid(format!("init choices: {e}")))?
        .unwrap_or_default();
    let out = init::apply_init(&ws, &plan, &choices)?;
    d.update_machine(|m| init::bind(m, &name, &dir, BindingState::Bound))
        .await?;
    d.emit("status.changed", json!({"project": name}));
    to_value(&out)
}

async fn project_add(d: &Arc<Daemon>, p: &Value) -> Result<Value> {
    let ws = d.ws().await?;
    let name = p_req(p, "project")?;
    if ws.head(&name).await?.is_none() {
        return Err(
            Error::invalid(format!("the workspace has no project named {name}"))
                .with_hint("workroam project list shows the projects"),
        );
    }
    let dir = match p_str(p, "path") {
        Some(s) => workroam_core::paths::normalize(&workroam_core::paths::expand_tilde(&s)),
        None => ws.machine.default_project_dir(&name),
    };
    let issues =
        workroam_platform::dircheck::check_project_dir(&dir, &init::other_dirs(&ws.machine, &name));
    if let Some(i) = issues.iter().find(|i| i.blocking) {
        return Err(Error::invalid(i.message.clone()));
    }
    d.update_machine(|m| init::bind(m, &name, &dir, BindingState::Pending))
        .await?;
    Ok(json!({"project": name, "dir": dir, "next": "pull"}))
}

async fn project_list(d: &Arc<Daemon>) -> Result<Value> {
    let ws = d.ws().await?;
    let states = init::refresh_bindings(&ws);
    if states
        .iter()
        .any(|(n, s)| ws.machine.projects.get(n).map(|b| b.state) != Some(*s))
    {
        let st = states.clone();
        d.update_machine(|m| {
            for (n, s) in st {
                if let Some(b) = m.projects.get_mut(&n) {
                    b.state = s;
                }
            }
        })
        .await?;
    }
    let rows: Vec<Value> = ws
        .projects()
        .await?
        .into_iter()
        .map(|pr| {
            let local = ws.machine.projects.get(&pr.name);
            json!({
                "name": pr.name,
                "pid": pr.pid,
                "on_this_device": local.is_some(),
                "dir": local.map(|b| b.path.clone()),
                "binding": states.get(&pr.name).map(|s| s.as_str()).unwrap_or("not_added"),
                "archived": pr.archived,
                "head": pr.head,
                "archive": pr.archive,
            })
        })
        .collect();
    Ok(json!(rows))
}

async fn status(d: &Arc<Daemon>, p: &Value) -> Result<Value> {
    let daemon = json!({
        "running": true,
        "service": d.service,
        "version": env!("CARGO_PKG_VERSION"),
        "paused_until": if d.is_paused() { *d.paused_until.lock().unwrap() } else { None },
        "jobs": d.jobs.list(false),
        "approvals": d.approvals.list().len(),
    });
    if !crate::is_configured() {
        return Ok(json!({"configured": false, "daemon": daemon, "projects": []}));
    }
    let ws = d.ws().await?;
    let states = init::refresh_bindings(&ws);
    let names: Vec<String> =
        if p_bool(p, "all") || (p_str(p, "project").is_none() && p_str(p, "cwd").is_none()) {
            ws.machine.projects.keys().cloned().collect()
        } else {
            match project_name(p) {
                Ok(n) => vec![n],
                Err(_) => ws.machine.projects.keys().cloned().collect(),
            }
        };
    let mut rows = Vec::new();
    for n in names {
        let pr = Project::open(&ws, &n)?;
        let mut st = to_value(&workroam_engine::status::project_status(&pr).await?)?;
        if let Some(s) = states.get(&n) {
            st["binding"] = json!(s.as_str());
        }
        st["last_failure"] = d
            .last_failure
            .lock()
            .unwrap()
            .get(&n)
            .cloned()
            .unwrap_or(Value::Null);
        st["job_running"] = json!(d.jobs.running_for(&n));
        rows.push(st);
    }
    Ok(json!({
        "configured": true,
        "daemon": daemon,
        "workspace": ws.location().display(),
        "device": {"id": ws.machine.device_id, "name": ws.machine.name, "access": ws.machine.access},
        "projects": rows,
    }))
}

async fn handoff(d: &Arc<Daemon>, p: &Value) -> Result<Value> {
    if p_bool(p, "template") {
        let project = project_name(p).unwrap_or_else(|_| "project".into());
        return Ok(json!({
            "template": workroam_adapters::handoff::template(&project),
            "sections": workroam_adapters::handoff::SECTIONS,
            "hint": "Fill every section before pushing; keep it short and concrete.",
        }));
    }
    let ws = d.ws().await?;
    let pr = Project::open(&ws, &project_name(p)?)?;
    let h = workroam_adapters::handoff::read(&pr.dir);
    Ok(json!({"project": pr.name, "path": workroam_adapters::handoff::PATH, "handoff": h}))
}

// ---------------------------------------------------------------- jobs --

async fn push_plan(d: &Arc<Daemon>, ctx: &Ctx, p: &Value) -> Result<Value> {
    let ws = d.ws().await?;
    let mut out = Vec::new();
    for name in targets(&ws.machine, p)? {
        check_remote_project(d, ctx, &name).await?;
        let pr = Project::open(&ws, &name)?;
        let plan = sync::plan_push(&pr).await?;
        let blocker = plan.blocker();
        let job = if p_bool(p, "dry_run") || blocker.is_some() {
            None
        } else {
            Some(d.jobs.create(
                JobSpec::Push {
                    project: name.clone(),
                    release: p_bool(p, "release"),
                    takeover: p_bool(p, "takeover"),
                    note: p_str(p, "note").unwrap_or_default(),
                },
                to_value(&plan)?,
                ctx,
                p_str(p, "trigger").as_deref().unwrap_or("manual"),
            ))
        };
        out.push(
            json!({"project": name, "plan": plan, "blocker": blocker, "job": job.map(|j| j.id)}),
        );
    }
    Ok(json!(out))
}

async fn pull_plan(d: &Arc<Daemon>, ctx: &Ctx, p: &Value) -> Result<Value> {
    let ws = d.ws().await?;
    let mut out = Vec::new();
    for name in targets(&ws.machine, p)? {
        check_remote_project(d, ctx, &name).await?;
        let pr = Project::open(&ws, &name)?;
        let plan = sync::plan_pull(&pr, None).await?;
        let blocker = plan.blocker();
        let wait_s = match p.get("wait") {
            Some(Value::String(s)) => Some(workroam_core::duration::parse(s)?.as_secs()),
            Some(Value::Bool(true)) => Some(30 * 60),
            Some(Value::Number(n)) => n.as_u64(),
            _ => None,
        };
        let job = if p_bool(p, "dry_run") || blocker.is_some() {
            None
        } else {
            Some(d.jobs.create(
                JobSpec::Pull {
                    project: name.clone(),
                    takeover: p_bool(p, "takeover"),
                    wait_s,
                },
                to_value(&plan)?,
                ctx,
                p_str(p, "trigger").as_deref().unwrap_or("manual"),
            ))
        };
        out.push(
            json!({"project": name, "plan": plan, "blocker": blocker, "job": job.map(|j| j.id)}),
        );
    }
    Ok(json!(out))
}

async fn restore_plan(d: &Arc<Daemon>, ctx: &Ctx, p: &Value) -> Result<Value> {
    let ws = d.ws().await?;
    let name = project_name(p)?;
    let pr = Project::open(&ws, &name)?;
    let paths: Vec<String> = p
        .get("paths")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let target = match p_str(p, "to") {
        Some(t) => RestoreTarget::Dir(workroam_core::paths::expand_tilde(&t)),
        None => RestoreTarget::InPlace,
    };
    let plan =
        workroam_engine::restore::plan_restore(&pr, &p_req(p, "snapshot")?, paths, target).await?;
    let job = (!p_bool(p, "dry_run")).then(|| {
        d.jobs.create(
            JobSpec::Restore {
                project: name.clone(),
                plan: plan.clone(),
            },
            to_value(&plan).unwrap_or_default(),
            ctx,
            "manual",
        )
    });
    Ok(json!({"project": name, "plan": plan, "job": job.map(|j| j.id)}))
}

async fn archive_plan(d: &Arc<Daemon>, ctx: &Ctx, p: &Value) -> Result<Value> {
    let ws = d.ws().await?;
    let name = project_name(p)?;
    let pr = Project::open(&ws, &name)?;
    let opts = arch::ArchiveOptions {
        note: p_str(p, "note").unwrap_or_default(),
        keep_history: p_bool(p, "keep_history"),
        takeover: p_bool(p, "takeover"),
        progress: None,
    };
    let plan = arch::plan(&pr, &opts).await?;
    let job = (!p_bool(p, "dry_run") && plan.blocker.is_none()).then(|| {
        d.jobs.create(
            JobSpec::Archive {
                project: name.clone(),
                note: opts.note.clone(),
                keep_history: opts.keep_history,
                takeover: opts.takeover,
            },
            to_value(&plan).unwrap_or_default(),
            ctx,
            "manual",
        )
    });
    Ok(json!({"project": name, "plan": plan, "job": job.map(|j| j.id)}))
}

async fn archive_restore(d: &Arc<Daemon>, ctx: &Ctx, method: &str, p: &Value) -> Result<Value> {
    let ws = d.ws().await?;
    let name = p_req(p, "project")?;
    let to = p_str(p, "to").map(|t| workroam_core::paths::expand_tilde(&t));
    let plan = arch::restore_plan(&ws, &name, to).await?;
    let prefer = match p_str(p, "prefer").as_deref() {
        None | Some("merge") => Prefer::Merge,
        Some("local") => Prefer::Local,
        Some("archive") => Prefer::Archive,
        Some("compare") => Prefer::Compare(PathBuf::from(p_req(p, "compare_to")?)),
        Some(other) => {
            return Err(Error::invalid(format!(
                "--prefer is local, merge or archive, not {other}"
            )))
        }
    };
    let job = (!p_bool(p, "dry_run") && plan.scenario != arch::Scenario::Occupied).then(|| {
        d.jobs.create(
            JobSpec::ArchiveRestore {
                project: name.clone(),
                plan: plan.clone(),
                prefer: prefer.clone(),
                takeover: p_bool(p, "takeover"),
            },
            to_value(&plan).unwrap_or_default(),
            ctx,
            "manual",
        )
    });
    if method == "archive.restore" {
        if plan.scenario == arch::Scenario::Occupied {
            return Err(Error::invalid(plan.reason.clone().unwrap_or_default())
                .with_hint("run again with --to <directory>"));
        }
        if let Some(j) = job {
            return jobs::start(d, ctx, &j.id, false, true).await;
        }
    }
    Ok(json!({"project": name, "plan": plan, "prefer": prefer, "job": job.map(|j| j.id)}))
}

/// Remote calls only reach projects the connection was granted.
async fn check_remote_project(d: &Arc<Daemon>, ctx: &Ctx, project: &str) -> Result<()> {
    let Some(o) = &ctx.origin else { return Ok(()) };
    crate::relay::check_grant(d, &o.connection_id, project, true).await
}

// -------------------------------------------------------------- backup --

async fn backup_configure(d: &Arc<Daemon>, p: &Value) -> Result<Value> {
    match p_str(p, "action").as_deref().unwrap_or("list") {
        "list" => {
            let m = crate::load_machine()?;
            let st = workroam_archive::mirror::MirrorState::load_from(
                &workroam_archive::mirror::MirrorState::path(),
            );
            Ok(
                json!({"backup": m.backup, "last_ok": st.last_ok, "last_error": st.last_error, "pending_delete": st.pending_delete.len()}),
            )
        }
        "remove" => {
            d.update_machine(|m| m.backup = None).await?;
            let _ = secrets::delete(secrets::keys::BACKUP_ACCESS_KEY);
            let _ = secrets::delete(secrets::keys::BACKUP_SECRET_KEY);
            Ok(json!({"removed": true}))
        }
        "add" => {
            let kind = p_str(p, "kind").unwrap_or_else(|| {
                if p_str(p, "path").is_some() {
                    "local".into()
                } else {
                    "s3".into()
                }
            });
            let cfg = BackupConfig {
                kind: kind.clone(),
                endpoint: p_str(p, "endpoint").unwrap_or_default(),
                bucket: p_str(p, "bucket").unwrap_or_default(),
                prefix: p_str(p, "prefix").unwrap_or_default(),
                region: p_str(p, "region").unwrap_or_default(),
                path: p_str(p, "path").unwrap_or_default(),
                schedule: p_str(p, "schedule").unwrap_or_else(|| "daily".into()),
                delete_delay_days: p
                    .get("delete_delay_days")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(30) as u32,
                ..Default::default()
            };
            match kind.as_str() {
                "local" if cfg.path.is_empty() => {
                    return Err(Error::invalid("--path is required for a local mirror"))
                }
                "s3" if cfg.endpoint.is_empty() || cfg.bucket.is_empty() => {
                    return Err(Error::invalid(
                        "--endpoint and --bucket are required for an S3 mirror",
                    ))
                }
                "local" | "s3" => {}
                other => {
                    return Err(Error::invalid(format!(
                        "unknown mirror kind {other} (s3 or local)"
                    )))
                }
            }
            if kind == "s3" {
                secrets::set(secrets::keys::BACKUP_ACCESS_KEY, &p_req(p, "access_key")?)?;
                secrets::set(secrets::keys::BACKUP_SECRET_KEY, &p_req(p, "secret_key")?)?;
            }
            let c2 = cfg.clone();
            d.update_machine(move |m| m.backup = Some(c2)).await?;
            Ok(json!({"backup": cfg}))
        }
        other => Err(Error::invalid(format!("unknown backup action {other}"))),
    }
}

fn mirror_creds(cfg: &BackupConfig) -> Result<Option<workroam_store::Creds>> {
    if cfg.kind == "local" {
        return Ok(None);
    }
    Ok(Some(workroam_store::Creds {
        access_key: secrets::require(secrets::keys::BACKUP_ACCESS_KEY)?,
        secret_key: secrets::require(secrets::keys::BACKUP_SECRET_KEY)?,
    }))
}

pub async fn backup_run(d: &Arc<Daemon>) -> Result<Value> {
    let m = crate::load_machine()?;
    let cfg = m.backup.clone().ok_or_else(|| {
        Error::invalid("no off-site backup is configured").with_hint("workroam backup add")
    })?;
    let creds = crate::storage_creds(&m)?;
    let report = workroam_archive::mirror::mirror(
        &m.workspace,
        creds.as_ref(),
        &cfg,
        mirror_creds(&cfg)?.as_ref(),
        &workroam_archive::mirror::MirrorState::path(),
    )
    .await?;
    let _ = d;
    to_value(&report)
}

async fn mirror_workspace(d: &Arc<Daemon>) -> Result<Arc<workroam_engine::Workspace>> {
    let ws = d.ws().await?;
    let cfg = ws
        .machine
        .backup
        .clone()
        .ok_or_else(|| Error::invalid("no off-site backup is configured"))?;
    let mut m = ws.machine.clone();
    m.workspace = workroam_archive::mirror::location(&cfg, &ws.location().workspace_id);
    m.workspace.capabilities = ws.location().capabilities;
    let password = secrets::require(secrets::keys::PASSWORD)?;
    Ok(Arc::new(
        workroam_engine::Workspace::open(m, mirror_creds(&cfg)?, &password).await?,
    ))
}

// ----------------------------------------------------- keys and secrets --

async fn credentials_update(d: &Arc<Daemon>, p: &Value) -> Result<Value> {
    let m = crate::load_machine()?;
    let creds = workroam_store::Creds {
        access_key: p_req(p, "access_key")?,
        secret_key: p_req(p, "secret_key")?,
    };
    // Validate before saving: the new key must read the workspace.
    let store = workroam_store::object::Store::open(&m.workspace, Some(&creds), "control")?;
    store
        .get(workroam_store::control::keys::WORKSPACE)
        .await?
        .ok_or_else(|| Error::new(Code::ChecksFailed, "the new key cannot read this workspace"))?;
    secrets::set(secrets::keys::ACCESS_KEY, &creds.access_key)?;
    secrets::set(secrets::keys::SECRET_KEY, &creds.secret_key)?;
    d.reset_ws().await;
    Ok(json!({"updated": true, "access_key": creds.access_key}))
}

async fn replace_password_keys(ws: &workroam_engine::Workspace, new: &str) -> Result<()> {
    let snap = ws.snap.clone();
    let new2 = new.to_string();
    workroam_engine::blocking(move || -> Result<()> {
        let old: Vec<String> = snap
            .keys()?
            .into_iter()
            .filter(|k| k.role == workroam_snapshot::KeyRole::Password)
            .map(|k| k.id)
            .collect();
        let added = snap.add_key(&new2, workroam_snapshot::KeyRole::Password)?;
        for id in old.into_iter().filter(|i| *i != added) {
            snap.delete_key(&id)?;
        }
        Ok(())
    })?;
    secrets::set(secrets::keys::PASSWORD, new)?;
    Ok(())
}

async fn password_change(d: &Arc<Daemon>, p: &Value) -> Result<Value> {
    let ws = d.ws().await?;
    let new = p_req(p, "new")?;
    if let Some(old) = p_str(p, "old") {
        if Some(old) != secrets::get(secrets::keys::PASSWORD)? {
            return Err(Error::new(Code::Denied, "the current password is wrong"));
        }
    }
    let (score, text) = workroam_core::crypto::password_strength(&new);
    if score < 2 {
        return Err(Error::invalid(format!(
            "the new password is too weak ({text})"
        )));
    }
    replace_password_keys(&ws, &new).await?;
    d.reset_ws().await;
    Ok(json!({"changed": true, "note": "pairing codes made earlier still need the old password"}))
}

async fn password_reset(d: &Arc<Daemon>, p: &Value) -> Result<Value> {
    let m = crate::load_machine()?;
    let recovery = workroam_core::crypto::normalize_recovery_key(&p_req(p, "recovery_key")?);
    let new = p_req(p, "new")?;
    let ws = workroam_engine::Workspace::open(m.clone(), crate::storage_creds(&m)?, &recovery)
        .await
        .map_err(|_| {
            Error::new(
                Code::Denied,
                "the recovery key does not match this workspace",
            )
        })?;
    replace_password_keys(&ws, &new).await?;
    d.reset_ws().await;
    Ok(json!({"reset": true}))
}

// -------------------------------------------------------------- agents --

async fn agent_list(d: &Arc<Daemon>) -> Result<Value> {
    let detected: Vec<Value> = workroam_adapters::registry::all()
        .iter()
        .map(|a| {
            let det = workroam_adapters::registry::detect(a);
            json!({"detected": det, "mcp": a.connect.mcp, "skill": a.connect.skill, "cloud": a.is_cloud()})
        })
        .collect();
    let grants = match d.ws().await {
        Ok(ws) => ws
            .control
            .get_all::<Grant>(workroam_store::control::keys::GRANTS)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|(_, g)| g)
            .filter(|g| !g.revoked)
            .collect(),
        Err(_) => vec![],
    };
    // Which bound projects enable each local agent.
    let mut enabled: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    if let Ok(m) = crate::load_machine() {
        for (n, b) in &m.projects {
            if let Ok(Some(c)) = ProjectConfig::load(&workroam_core::paths::expand_tilde(&b.path)) {
                for (id, t) in c.agents {
                    if t.state {
                        enabled.entry(id).or_default().push(n.clone());
                    }
                }
            }
        }
    }
    Ok(
        json!({"agents": detected, "connections": grants, "enabled_in": enabled, "relay": d.relay.status()}),
    )
}

/// Enable a local agent in every bound project and regenerate its files.
pub async fn connect_local(d: &Arc<Daemon>, agent: &workroam_adapters::Adapter) -> Result<Value> {
    let ws = d.ws().await?;
    let mut done = Vec::new();
    let mut warnings = Vec::new();
    for (n, b) in &ws.machine.projects {
        if b.state != BindingState::Bound {
            continue;
        }
        let dir = workroam_core::paths::expand_tilde(&b.path);
        let mut c = match ProjectConfig::load(&dir)? {
            Some(c) => c,
            None => continue,
        };
        c.agents.insert(
            agent.id.clone(),
            workroam_core::config::AgentToggle { state: true },
        );
        c.save(&dir)?;
        let pr = Project::open(&ws, n)?;
        match pr.generate() {
            Ok(g) => warnings.extend(g.warnings),
            Err(e) => warnings.push(format!("{n}: {}", e.message)),
        }
        done.push(n.clone());
    }
    Ok(json!({"agent": agent.id, "projects": done, "warnings": warnings}))
}

pub async fn disconnect_local(
    d: &Arc<Daemon>,
    agent: &workroam_adapters::Adapter,
) -> Result<Value> {
    let ws = d.ws().await?;
    let mut done = Vec::new();
    for (n, b) in &ws.machine.projects {
        let dir = workroam_core::paths::expand_tilde(&b.path);
        if let Ok(Some(mut c)) = ProjectConfig::load(&dir) {
            if c.agents.get(&agent.id).map(|t| t.state).unwrap_or(false) {
                c.agents.insert(
                    agent.id.clone(),
                    workroam_core::config::AgentToggle { state: false },
                );
                c.save(&dir)?;
                let pr = Project::open(&ws, n)?;
                let _ = pr.generate();
                done.push(n.clone());
            }
        }
    }
    Ok(json!({"agent": agent.id, "projects": done}))
}

/// Heads that are archived: auto-sync and arrivals skip them.
pub fn is_archived(h: &workroam_core::model::Head) -> bool {
    h.state == HeadState::Archived
}
