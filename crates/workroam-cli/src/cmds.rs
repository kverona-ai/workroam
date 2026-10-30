//! Command implementations: build IPC parameters, call `workroamd`, print.

use crate::cli::*;
use crate::conn::{self, Conn};
use crate::out::{n, s, short, summary_line, time, Out};
use serde_json::{json, Value};
use workroam_core::text::tr;
use workroam_core::{Code, Error, Result};
use workroam_ipc::ClientKind;

/// Each `--*-stdin` flag reads one line from stdin, in the order the flags are listed in `--help`.
pub fn stdin_line(what: &str) -> Result<String> {
    // Stdin is buffered process-wide, so consecutive calls read consecutive lines.
    let mut l = String::new();
    match std::io::stdin().read_line(&mut l) {
        Ok(n) if n > 0 => Ok(l.trim_end_matches(['\r', '\n']).to_string()),
        _ => Err(Error::invalid(format!("expected {what} on stdin"))),
    }
}

pub fn secret_from(
    flag: bool,
    env: &str,
    what: &str,
    prompt: Option<&str>,
    interactive: bool,
) -> Result<Option<String>> {
    if flag {
        return stdin_line(what).map(Some);
    }
    if let Ok(v) = std::env::var(env) {
        if !v.is_empty() {
            return Ok(Some(v));
        }
    }
    if let (Some(p), true) = (prompt, interactive) {
        return dialoguer::Password::new()
            .with_prompt(p)
            .interact()
            .map(Some)
            .map_err(|e| Error::invalid(e.to_string()));
    }
    Ok(None)
}

pub fn confirm(out: &Out, prompt: &str, default: bool) -> Result<bool> {
    if out.yes {
        return Ok(true);
    }
    if !out.interactive {
        return Ok(default);
    }
    dialoguer::Confirm::new()
        .with_prompt(prompt)
        .default(default)
        .interact()
        .map_err(|e| Error::invalid(e.to_string()))
}

fn cwd() -> String {
    std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default()
}

/// `{project}` from -p or `{cwd}` so the daemon picks the project.
fn scope(cli_project: &[String]) -> Value {
    match cli_project.first() {
        Some(p) => json!({"project": p}),
        None => json!({"cwd": cwd()}),
    }
}

fn merge(mut a: Value, b: Value) -> Value {
    if let (Some(x), Some(y)) = (a.as_object_mut(), b.as_object()) {
        for (k, v) in y {
            x.insert(k.clone(), v.clone());
        }
    }
    a
}

fn name_of(cmd: &Option<Cmd>) -> String {
    let Some(c) = cmd else {
        return "summary".into();
    };
    let dbg = format!("{c:?}");
    let head = dbg
        .split([' ', '(', '{'])
        .next()
        .unwrap_or("")
        .to_lowercase();
    head
}

pub async fn run(cli: Cli) -> i32 {
    let command = name_of(&cli.cmd);
    let out = Out::new(cli.json, cli.quiet, cli.yes, cli.color, &command);
    let banner_setting =
        workroam_core::config::MachineConfig::load(&workroam_core::paths::machine_toml())
            .ok()
            .flatten()
            .map(|m| m.ui.banner)
            .unwrap_or(true);
    let wants_banner = match &cli.cmd {
        None => true,
        Some(Cmd::Setup(_)) | Some(Cmd::Init(_)) => out.interactive && !cli.yes,
        _ => false,
    };
    if wants_banner
        && crate::banner::should_show(cli.json, cli.quiet, cli.no_banner, banner_setting)
    {
        crate::banner::show(cli.color);
    }
    match dispatch(&cli, &out).await {
        Ok(code) => code,
        Err(e) => out.error(&e),
    }
}

async fn connect(kind: ClientKind) -> Result<Conn> {
    conn::connect(kind).await
}

/// Calls whose approval must be answered elsewhere need the background service.
fn require_service(c: &Conn) -> Result<()> {
    if c.in_process && conn::agent_name().is_some() {
        return Err(Error::new(
            Code::DaemonUnavailable,
            "the Workroam background service is not running, so nobody can approve this request",
        )
        .with_hint("start it with `workroamd` or open the Workroam app, then try again"));
    }
    Ok(())
}

async fn dispatch(cli: &Cli, out: &Out) -> Result<i32> {
    let p = &cli.project;
    let Some(cmd) = &cli.cmd else {
        return summary(out).await;
    };
    match cmd {
        Cmd::Completions { shell } => {
            let mut c = crate::command();
            clap_complete::generate(*shell, &mut c, "workroam", &mut std::io::stdout());
            Ok(0)
        }
        Cmd::Mcp => crate::mcp::serve().await.map(|_| 0),
        Cmd::Setup(a) => crate::setup::run(out, a, p).await,
        Cmd::Init(a) => init(out, a).await,
        Cmd::Project { cmd } => project(out, cmd).await,
        Cmd::Status { all } => {
            let c = connect(ClientKind::Cli).await?;
            let mut params = scope(p);
            if *all || p.is_empty() {
                params = merge(params, json!({"all": *all}));
            }
            let v = c.client.call("status", params).await?;
            Ok(out.done(&v, |o, v| print_status(o, v, c.in_process)))
        }
        Cmd::Handoff { template } => {
            let c = connect(ClientKind::Cli).await?;
            let v = c
                .client
                .call(
                    "handoff.read",
                    merge(scope(p), json!({"template": template})),
                )
                .await?;
            Ok(out.done(&v, |o, v| {
                if *template {
                    print!("{}", s(v, "template"));
                    return;
                }
                let h = &v["handoff"];
                if !h["exists"].as_bool().unwrap_or(false) {
                    println!(
                        "{}",
                        o.yellow(
                            "HANDOFF.md is missing; `workroam handoff --template` prints one."
                        )
                    );
                    return;
                }
                println!(
                    "{} {}  {}",
                    o.bold("HANDOFF.md"),
                    o.dim(&time(&h["time"])),
                    o.dim(s(h, "device"))
                );
                if let Some(secs) = h["sections"].as_object() {
                    for (k, val) in secs {
                        let text = val.as_str().unwrap_or("").trim();
                        if !text.is_empty() {
                            println!("\n{}\n{text}", o.bold(k));
                        }
                    }
                }
            }))
        }
        Cmd::Push {
            release,
            all,
            dry_run,
            takeover,
            note,
        } => {
            let c = connect(ClientKind::Cli).await?;
            let params = merge(
                scope(p),
                json!({"release": release, "all": all, "dry_run": dry_run, "takeover": takeover, "note": note}),
            );
            let plans = c.client.call("push.plan", params).await?;
            if *dry_run {
                return Ok(out.done(&plans, print_push_plans));
            }
            require_service(&c)?;
            let results = run_jobs(&c, out, &plans).await?;
            Ok(out.done(&results, print_results))
        }
        Cmd::Pull {
            all,
            dry_run,
            wait,
            takeover,
        } => {
            let c = connect(ClientKind::Cli).await?;
            let params = merge(
                scope(p),
                json!({"all": all, "dry_run": dry_run, "takeover": takeover, "wait": wait}),
            );
            let plans = c.client.call("pull.plan", params).await?;
            if *dry_run {
                return Ok(out.done(&plans, print_pull_plans));
            }
            for pl in plans.as_array().into_iter().flatten() {
                if pl["plan"]["touches_local"].as_bool().unwrap_or(false)
                    && out.interactive
                    && !confirm(out, &tr("approval.touches_local"), true)?
                {
                    return Err(Error::new(Code::Denied, "pull cancelled"));
                }
            }
            require_service(&c)?;
            let results = run_jobs(&c, out, &plans).await?;
            Ok(out.done(&results, print_results))
        }
        Cmd::Lease {
            cmd: LeaseCmd::Takeover,
        } => {
            let c = connect(ClientKind::Cli).await?;
            if !confirm(out, &tr("lease.takeover"), true)? {
                return Err(Error::new(Code::Denied, "cancelled"));
            }
            let v = c.client.call("lease.takeover", scope(p)).await?;
            Ok(out.done(&v, |o, v| {
                println!(
                    "{} {}",
                    o.green("✓"),
                    format_args!("lease of {} is now held by this device", s(v, "project"))
                );
            }))
        }
        Cmd::Approval { cmd } => {
            let c = connect(ClientKind::Cli).await?;
            let v = match cmd {
                ApprovalCmd::List => c.client.call("approval.list", json!({})).await?,
                ApprovalCmd::Show { id } => {
                    c.client.call("approval.list", json!({"id": id})).await?
                }
            };
            Ok(out.done(&v, |o, v| match v.as_array() {
                Some(list) if list.is_empty() => println!("no pending approvals"),
                Some(list) => {
                    for a in list {
                        println!(
                            "{}  {}  {} {}",
                            o.bold(short(s(a, "id"))),
                            s(a, "title"),
                            o.dim("from"),
                            s(a, "initiator")
                        );
                    }
                }
                None => print_approval(o, v),
            }))
        }
        Cmd::Approve {
            id,
            skip_ask,
            access,
            allow_project,
        } => {
            // `--project` (the global -p) names the projects a remote agent may reach.
            let projects: Vec<&String> = allow_project.iter().chain(p.iter()).collect();
            respond(
                out,
                id,
                "approve",
                json!({"skip_ask": skip_ask, "access": access, "projects": projects}),
            )
            .await
        }
        Cmd::Later { id } => respond(out, id, "later", json!({})).await,
        Cmd::Deny { id } => respond(out, id, "deny", json!({})).await,
        Cmd::Job { cmd } => {
            let c = connect(ClientKind::Cli).await?;
            let v = match cmd {
                JobCmd::List { all } => c.client.call("job.list", json!({"all": all})).await?,
                JobCmd::Cancel { id } => c.client.call("job.cancel", json!({"job": id})).await?,
            };
            Ok(out.done(&v, |o, v| {
                let list = v.as_array().cloned().unwrap_or_else(|| vec![v.clone()]);
                if list.is_empty() {
                    println!("no background jobs");
                }
                for j in list {
                    println!(
                        "{}  {:<16} {:<10} {:<12} {}",
                        o.bold(short(s(&j, "id"))),
                        s(&j, "kind"),
                        s(&j, "project"),
                        s(&j, "state"),
                        o.dim(s(&j, "trigger"))
                    );
                }
            }))
        }
        Cmd::History { cmd, limit } => {
            let c = connect(ClientKind::Cli).await?;
            match cmd {
                None => {
                    let v = c
                        .client
                        .call("history.list", merge(scope(p), json!({"limit": limit})))
                        .await?;
                    Ok(out.done(&v, |o, v| {
                        for h in v.as_array().into_iter().flatten() {
                            let mut tags = String::new();
                            if h["archived"].as_bool().unwrap_or(false) {
                                tags.push_str(" [archived]");
                            }
                            if !h["restored_from"].is_null() {
                                tags.push_str(" [restore]");
                            }
                            println!(
                                "{}  {}  {:<12} {}{}  {}",
                                o.bold(short(s(h, "id"))),
                                time(&h["time"]),
                                s(&h["device"], "name"),
                                summary_line(&h["summary"]),
                                o.amber(&tags),
                                o.dim(s(h, "note"))
                            );
                        }
                    }))
                }
                Some(HistoryCmd::Show { snapshot }) => {
                    let v = c
                        .client
                        .call(
                            "history.show",
                            merge(scope(p), json!({"snapshot": snapshot})),
                        )
                        .await?;
                    Ok(out.done(&v, |o, v| {
                        let sn = &v["snapshot"];
                        println!(
                            "{} {} {}",
                            o.bold(short(s(sn, "id"))),
                            time(&sn["time"]),
                            s(&sn["device"], "name")
                        );
                        for ch in v["changes"].as_array().into_iter().flatten() {
                            let mark = match s(ch, "kind") {
                                "added" => o.green("+"),
                                "deleted" => o.red("-"),
                                _ => o.yellow("~"),
                            };
                            println!("  {mark} {}", s(ch, "path"));
                        }
                    }))
                }
            }
        }
        Cmd::Restore {
            snapshot,
            paths,
            to,
            dry_run,
        } => {
            let c = connect(ClientKind::Cli).await?;
            let plan = c
                .client
                .call(
                    "restore.plan",
                    merge(
                        scope(p),
                        json!({"snapshot": snapshot, "paths": paths, "to": to, "dry_run": dry_run}),
                    ),
                )
                .await?;
            let pl = &plan["plan"];
            out.info(&format!(
                "restore {} from {} into {}: {} files{}",
                s(pl, "project"),
                short(s(&pl["snapshot"], "id")),
                s(pl, "dir"),
                n(pl, "files"),
                if pl["saved_local"]
                    .as_array()
                    .map(|a| !a.is_empty())
                    .unwrap_or(false)
                {
                    format!(
                        "; unsynced local files are saved as *.conflict-before-restore first ({})",
                        pl["saved_local"].as_array().unwrap().len()
                    )
                } else {
                    String::new()
                }
            ));
            if *dry_run {
                return Ok(out.done(&plan, |_, _| {}));
            }
            if to.is_none() && !confirm(out, "Restore in place?", false)? {
                return Err(Error::new(Code::Denied, "restore cancelled")
                    .with_hint("add --yes to restore without a terminal"));
            }
            let results = run_jobs(&c, out, &json!([plan])).await?;
            Ok(out.done(&results, print_results))
        }
        Cmd::Archive { cmd } => archive(out, cmd, p).await,
        Cmd::Device { cmd } => device(out, cmd).await,
        Cmd::Doctor => {
            let c = connect(ClientKind::Cli).await?;
            c.client.call("subscribe", json!({})).await?;
            match c.client.call("doctor.run", json!({})).await {
                Ok(v) => Ok(out.done(&v, print_checks)),
                Err(e) if e.code == Code::ChecksFailed => {
                    if let Some(d) = &e.data {
                        if !out.json {
                            print_checks(out, d);
                        }
                    }
                    Err(e)
                }
                Err(e) => Err(e),
            }
        }
        Cmd::Verify { backup, read_data } => {
            let c = connect(ClientKind::Cli).await?;
            out.info("verifying… this reads the index and every snapshot");
            let v = c
                .client
                .call(
                    "verify.run",
                    json!({"backup": backup, "read_data": read_data}),
                )
                .await?;
            Ok(out.done(&v, |o, v| {
                println!(
                    "{} {} snapshots, {} projects, {} control objects",
                    o.green("✓ no problems:"),
                    n(v, "snapshots"),
                    n(v, "projects"),
                    n(v, "control_objects")
                );
                for w in v["warnings"].as_array().into_iter().flatten() {
                    println!("  {} {}", o.yellow("!"), w.as_str().unwrap_or(""));
                }
            }))
        }
        Cmd::Config { cmd } => {
            let c = connect(ClientKind::Cli).await?;
            let pj = p.first().cloned();
            let v = match cmd {
                ConfigCmd::List => c.client.call("config.list", json!({"project": pj})).await?,
                ConfigCmd::Get { key } => {
                    c.client
                        .call("config.get", json!({"key": key, "project": pj}))
                        .await?
                }
                ConfigCmd::Set { key, value } => {
                    c.client
                        .call(
                            "config.set",
                            json!({"key": key, "value": value, "project": pj}),
                        )
                        .await?
                }
            };
            Ok(out.done(&v, |o, v| {
                let list = v.as_array().cloned().unwrap_or_else(|| vec![v.clone()]);
                for x in list {
                    let val = match &x["value"] {
                        Value::String(t) => t.clone(),
                        other => other.to_string(),
                    };
                    println!(
                        "{:<32} {:<24} {}",
                        s(&x, "key"),
                        val,
                        o.dim(s(&x, "source"))
                    );
                }
            }))
        }
        Cmd::Agent { cmd } => agent(out, cmd).await,
        Cmd::Secret { cmd } => secret(out, cmd, p).await,
        Cmd::Credentials {
            cmd:
                CredentialsCmd::Update {
                    access_key,
                    secret_key_stdin,
                },
        } => {
            let sk = secret_from(
                *secret_key_stdin,
                "WORKROAM_SECRET_KEY",
                "the secret key",
                Some("Secret key"),
                out.interactive,
            )?
            .ok_or_else(|| {
                Error::invalid("the secret key is required").with_hint("--secret-key-stdin")
            })?;
            let c = connect(ClientKind::Cli).await?;
            let v = c
                .client
                .call(
                    "credentials.update",
                    json!({"access_key": access_key, "secret_key": sk}),
                )
                .await?;
            Ok(out.done(&v, |o, _| {
                println!("{} access key updated and verified", o.green("✓"))
            }))
        }
        Cmd::Password { cmd } => {
            let c = connect(ClientKind::Cli).await?;
            let v = match cmd {
                PasswordCmd::Change => {
                    if !out.interactive {
                        return Err(Error::invalid("password change needs a terminal"));
                    }
                    let old = dialoguer::Password::new()
                        .with_prompt("Current password")
                        .interact()
                        .map_err(|e| Error::invalid(e.to_string()))?;
                    let new = new_password()?;
                    c.client
                        .call("password.change", json!({"old": old, "new": new}))
                        .await?
                }
                PasswordCmd::Reset { recovery_key_stdin } => {
                    let rk = secret_from(
                        *recovery_key_stdin,
                        "WORKROAM_RECOVERY_KEY",
                        "the recovery key",
                        Some("Recovery key"),
                        out.interactive,
                    )?
                    .ok_or_else(|| {
                        Error::invalid("the recovery key is required")
                            .with_hint("--recovery-key-stdin")
                    })?;
                    let new = match secret_from(false, "WORKROAM_PASSWORD", "", None, false)? {
                        Some(p) => p,
                        None if *recovery_key_stdin && !out.interactive => {
                            stdin_line("the new password")?
                        }
                        None => new_password()?,
                    };
                    c.client
                        .call("password.reset", json!({"recovery_key": rk, "new": new}))
                        .await?
                }
            };
            Ok(out.done(&v, |o, _| {
                println!("{} encryption password updated", o.green("✓"))
            }))
        }
        Cmd::Pause { duration } => {
            let c = connect(ClientKind::Cli).await?;
            let v = c
                .client
                .call("sync.pause", json!({"for": duration}))
                .await?;
            Ok(out.done(&v, |_, v| {
                println!("automatic sync paused until {}", time(&v["paused_until"]))
            }))
        }
        Cmd::Resume => {
            let c = connect(ClientKind::Cli).await?;
            let v = c.client.call("sync.resume", json!({})).await?;
            Ok(out.done(&v, |_, _| println!("automatic sync resumed")))
        }
        Cmd::Prune { dry_run } => {
            let c = connect(ClientKind::Cli).await?;
            let v = c
                .client
                .call("prune.run", json!({"dry_run": dry_run}))
                .await?;
            Ok(out.done(&v, |_, v| {
                println!(
                    "{} snapshots {} across {} projects",
                    n(v, "forgotten"),
                    if *dry_run {
                        "would be removed"
                    } else {
                        "removed"
                    },
                    n(v, "projects")
                )
            }))
        }
        Cmd::Backup { cmd } => backup(out, cmd).await,
    }
}

fn new_password() -> Result<String> {
    loop {
        let pw = dialoguer::Password::new()
            .with_prompt(tr("setup.password"))
            .with_confirmation(tr("setup.password.again"), tr("setup.password.mismatch"))
            .interact()
            .map_err(|e| Error::invalid(e.to_string()))?;
        let (score, text) = workroam_core::crypto::password_strength(&pw);
        eprintln!("  {text}");
        if score >= 2 {
            return Ok(pw);
        }
    }
}

async fn respond(out: &Out, id: &str, decision: &str, extra: Value) -> Result<i32> {
    let c = connect(ClientKind::Cli).await?;
    let v = c
        .client
        .call(
            "approval.respond",
            merge(json!({"id": id, "decision": decision}), extra),
        )
        .await?;
    Ok(out.done(&v, |o, v| println!("{} {}", o.green("✓"), s(v, "decision"))))
}

// ----------------------------------------------------------------- jobs --

/// Start every planned job, show progress and approvals, return the results.
pub async fn run_jobs(c: &Conn, out: &Out, plans: &Value) -> Result<Value> {
    c.client.call("subscribe", json!({})).await?;
    let mut results = Vec::new();
    for pl in plans.as_array().into_iter().flatten() {
        if let Some(b) = pl.get("blocker").filter(|b| !b.is_null()) {
            let e: Error =
                serde_json::from_value(b.clone()).unwrap_or_else(|_| Error::internal("blocked"));
            return Err(e);
        }
        let Some(job) = pl["job"].as_str() else {
            continue;
        };
        let r = run_one(c, out, job).await?;
        results.push(r);
    }
    Ok(json!(results))
}

async fn run_one(c: &Conn, out: &Out, job: &str) -> Result<Value> {
    let bar = (out.interactive && !out.quiet).then(|| {
        let b = indicatif::ProgressBar::new(0);
        b.set_style(
            indicatif::ProgressStyle::with_template(
                "{spinner} {msg:12} {bar:30.cyan/blue} {bytes}/{total_bytes} {bytes_per_sec} {eta}",
            )
            .unwrap_or_else(|_| indicatif::ProgressStyle::default_bar()),
        );
        b
    });
    let call = c
        .client
        .call("job.start", json!({"job": job, "wait": true}));
    tokio::pin!(call);
    let mut last_pct: i64 = -10;
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);
    loop {
        tokio::select! {
            r = &mut call => {
                if let Some(b) = &bar { b.finish_and_clear(); }
                return r;
            }
            _ = &mut ctrl_c => {
                let _ = c.client.call("job.cancel", json!({"job": job})).await;
                if let Some(b) = &bar { b.abandon(); }
                return Err(Error::new(Code::Denied, "cancelled").with_hint("run the same command again to resume"));
            }
            ev = c.client.next_event() => {
                let Some(ev) = ev else { continue };
                let p = &ev.params;
                match ev.method.as_str() {
                    "job.progress" if s(p, "job") == job => {
                        let pr = &p["progress"];
                        let done = n(pr, "done");
                        let total = pr["total"].as_u64();
                        if let Some(b) = &bar {
                            if let Some(t) = total { b.set_length(t); }
                            b.set_position(done);
                            b.set_message(s(pr, "phase").to_string());
                        } else {
                            let pct = total.map(|t| (done * 100).checked_div(t).unwrap_or(100) as i64).unwrap_or(-1);
                            if pct >= last_pct + 10 || pct == 100 && last_pct != 100 {
                                last_pct = pct;
                                out.event(json!({"event": "progress", "job": job, "phase": s(pr, "phase"), "done": done, "total": total, "percent": pct}));
                                out.info(&format!("{} {}%", s(pr, "phase"), pct.max(0)));
                            }
                        }
                    }
                    "approval.requested" if s(p, "job_id") == job => {
                        out.event(json!({"event": "approval_pending", "job_id": job, "approval_id": s(p, "id"), "timeout_s": n(p, "timeout_s")}));
                        let msg = format!(
                            "{} {} — {} (id {}, {})",
                            out.amber("⧗"),
                            s(p, "title"),
                            tr("approval.waiting"),
                            short(s(p, "id")),
                            workroam_core::duration::human(std::time::Duration::from_secs(n(p, "timeout_s")))
                        );
                        if let Some(b) = &bar { b.println(msg); } else { out.info(&msg); }
                    }
                    _ => {}
                }
            }
        }
    }
}

fn print_results(o: &Out, v: &Value) {
    for r in v.as_array().into_iter().flatten() {
        let res = &r["result"];
        match s(r, "kind") {
            "push" => {
                if res["skipped"].as_bool().unwrap_or(false) {
                    println!("{} {} is up to date", o.green("✓"), s(r, "project"));
                } else {
                    println!(
                        "{} pushed {} → {}  {}",
                        o.green("✓"),
                        s(r, "project"),
                        o.bold(short(s(res, "snapshot"))),
                        summary_line(&res["summary"])
                    );
                }
                if res["released"].as_bool().unwrap_or(false) {
                    println!("  lease released");
                }
            }
            "pull" => {
                if res["up_to_date"].as_bool().unwrap_or(false) {
                    println!("{} {} is up to date", o.green("✓"), s(r, "project"));
                } else {
                    println!(
                        "{} pulled {} ← {}  {}",
                        o.green("✓"),
                        s(r, "project"),
                        o.bold(short(s(res, "snapshot"))),
                        summary_line(&res["summary"])
                    );
                }
                for c in res["conflicts"].as_array().into_iter().flatten() {
                    println!(
                        "  {} conflict copy: {}",
                        o.yellow("!"),
                        c.as_str().unwrap_or("")
                    );
                }
                for m in res["merged"].as_array().into_iter().flatten() {
                    println!("  merged: {}", m.as_str().unwrap_or(""));
                }
                if res["handoff_updated"].as_bool().unwrap_or(false) {
                    println!("  HANDOFF.md was updated — `workroam handoff`");
                }
            }
            "archive" => println!(
                "{} archived {} ({})",
                o.green("✓"),
                s(r, "project"),
                short(s(res, "snapshot"))
            ),
            "archive_restore" => {
                println!(
                    "{} restored {} into {}",
                    o.green("✓"),
                    s(r, "project"),
                    s(res, "dir")
                );
                if !s(res, "moved_to").is_empty() {
                    println!("  local files were moved to {}", s(res, "moved_to"));
                }
            }
            _ => println!("{} {} {}", o.green("✓"), s(r, "kind"), s(r, "project")),
        }
        for w in res["warnings"].as_array().into_iter().flatten() {
            println!("  {} {}", o.yellow("!"), w.as_str().unwrap_or(""));
        }
        for nn in res["notes"].as_array().into_iter().flatten() {
            println!("  {}", nn.as_str().unwrap_or(""));
        }
    }
}

fn print_push_plans(o: &Out, v: &Value) {
    for pl in v.as_array().into_iter().flatten() {
        let plan = &pl["plan"];
        println!(
            "{} {}",
            o.bold(s(pl, "project")),
            summary_line(&plan["summary"])
        );
        for ch in plan["changes"].as_array().into_iter().flatten().take(50) {
            println!(
                "  {} {}",
                match s(ch, "kind") {
                    "added" => "+",
                    "deleted" => "-",
                    _ => "~",
                },
                s(ch, "path")
            );
        }
        if let Some(b) = pl.get("blocker").filter(|b| !b.is_null()) {
            println!("  {} {}", o.red("blocked:"), s(b, "message"));
        }
    }
}

fn print_pull_plans(o: &Out, v: &Value) {
    for pl in v.as_array().into_iter().flatten() {
        let plan = &pl["plan"];
        if plan["up_to_date"].as_bool().unwrap_or(false) {
            println!("{} {} is up to date", o.green("✓"), s(pl, "project"));
            continue;
        }
        println!(
            "{} {}  from {}",
            o.bold(s(pl, "project")),
            summary_line(&plan["summary"]),
            s(&plan["head"], "device_name")
        );
        if plan["touches_local"].as_bool().unwrap_or(false) {
            println!(
                "  {} {} local changes will be merged or kept as conflict copies",
                o.yellow("!"),
                n(plan, "local_changes")
            );
        }
        if let Some(b) = pl.get("blocker").filter(|b| !b.is_null()) {
            println!("  {} {}", o.red("blocked:"), s(b, "message"));
        }
    }
}

fn print_approval(o: &Out, a: &Value) {
    println!("{} {}", o.bold(s(a, "title")), o.dim(short(s(a, "id"))));
    println!("  from {}  · {}", s(a, "initiator"), time(&a["created_at"]));
    if let Some(c) = a["code"].as_str() {
        println!("  confirmation code {}", o.amber(c));
    }
    let d = &a["details"];
    if !d["summary"].is_null() {
        println!("  {}", summary_line(&d["summary"]));
    }
    for ch in d["changes"].as_array().into_iter().flatten().take(30) {
        println!(
            "    {} {}",
            match s(ch, "kind") {
                "added" => "+",
                "deleted" => "-",
                _ => "~",
            },
            s(ch, "path")
        );
    }
}

pub fn print_checks(o: &Out, v: &Value) {
    let mut group = String::new();
    for r in v["results"].as_array().into_iter().flatten() {
        if s(r, "group") != group {
            group = s(r, "group").to_string();
            println!("\n{}", o.bold(&group));
        }
        let mark = match s(r, "status") {
            "pass" => o.green("✓"),
            "warn" => o.yellow("!"),
            "fail" => o.red("✗"),
            "info" => o.dim("i"),
            _ => o.dim("-"),
        };
        println!(
            "  {mark} {:<28} {}",
            tr(&format!("check.{}", s(r, "id"))),
            s(r, "message")
        );
        if let Some(h) = r["hint"].as_str() {
            if s(r, "status") != "pass" {
                println!("      {}", o.dim(h));
            }
        }
    }
}

pub fn print_status(o: &Out, v: &Value, in_process: bool) {
    if !v["configured"].as_bool().unwrap_or(false) {
        println!("{}", tr("not_initialized"));
        return;
    }
    let d = &v["daemon"];
    let svc = if in_process {
        o.yellow("not running (ran in-process)")
    } else {
        o.green("running")
    };
    println!(
        "{} {}   {} {}   {} {}",
        o.dim("service"),
        svc,
        o.dim("device"),
        s(&v["device"], "name"),
        o.dim("workspace"),
        s(v, "workspace")
    );
    if !d["paused_until"].is_null() {
        println!(
            "{}",
            o.yellow(&format!(
                "automatic sync paused until {}",
                time(&d["paused_until"])
            ))
        );
    }
    if n(d, "approvals") > 0 {
        println!(
            "{}",
            o.amber(&format!(
                "{} approvals waiting — workroam approval list",
                n(d, "approvals")
            ))
        );
    }
    for p in v["projects"].as_array().into_iter().flatten() {
        println!();
        let lease = &p["lease"];
        let holder = s(&lease["holder"], "name");
        let lease_txt = match s(lease, "state") {
            "mine" => o.amber("● lease: this device"),
            "other" => o.yellow(&format!("lease: {holder}")),
            "stale" => o.dim(&format!("lease: {holder} (stale)")),
            _ => o.dim("lease: free"),
        };
        println!(
            "{}  {}  {}",
            o.bold(s(p, "project")),
            lease_txt,
            o.dim(s(p, "dir"))
        );
        match s(p, "binding") {
            "missing" => println!("  {} {}", o.red("!"), tr("project.missing")),
            "archived" => println!(
                "  {} archived — workroam archive restore {}",
                o.dim("■"),
                s(p, "project")
            ),
            "pending" => println!(
                "  not synced here yet — workroam pull -p {}",
                s(p, "project")
            ),
            _ => {}
        }
        let head = &p["head"];
        if !head.is_null() {
            println!(
                "  latest {} {} from {}",
                short(s(head, "snapshot")),
                time(&head["created_at"]),
                s(head, "device_name")
            );
        }
        let lc = &p["local_changes"];
        if n(lc, "added") + n(lc, "modified") + n(lc, "deleted") > 0 {
            println!("  unsynced: {}", summary_line(lc));
        }
        if p["incoming"].as_bool().unwrap_or(false) {
            println!(
                "  {} new progress from {} — workroam pull",
                o.amber("↓"),
                s(head, "device_name")
            );
        }
        for r in p["repos"].as_array().into_iter().flatten() {
            let push = if r["can_push"].as_bool().unwrap_or(false) {
                o.green("can push")
            } else {
                o.dim("no push from here")
            };
            println!(
                "  repo {:<16} {:<8} {:<10} {}",
                s(r, "name"),
                s(r, "mode"),
                s(r, "branch"),
                push
            );
        }
        if !p["last_failure"].is_null() {
            println!(
                "  {} last job failed: {}",
                o.red("✗"),
                s(&p["last_failure"]["error"], "message")
            );
        }
    }
}

async fn summary(out: &Out) -> Result<i32> {
    if !workroamd::is_configured() {
        if out.json {
            return Ok(out.done(&json!({"configured": false}), |_, _| {}));
        }
        println!("{}", tr("not_initialized"));
        if out.interactive && confirm(out, &tr("start_setup"), true)? {
            let a = crate::cli::Cli::try_parse_from_setup();
            return crate::setup::run(out, &a, &[]).await;
        }
        return Ok(0);
    }
    let c = connect(ClientKind::Cli).await?;
    let v = c.client.call("status", json!({"all": true})).await?;
    Ok(out.done(&v, |o, v| {
        print_status(o, v, c.in_process);
        println!(
            "\n{}",
            o.dim("push · pull · status · history · archive list · doctor · workroam help")
        );
    }))
}

impl crate::cli::Cli {
    fn try_parse_from_setup() -> SetupArgs {
        use clap::Parser;
        match crate::cli::Cli::parse_from(["workroam", "setup"]).cmd {
            Some(Cmd::Setup(a)) => a,
            _ => unreachable!(),
        }
    }
}

// --------------------------------------------------------------- groups --

async fn init(out: &Out, a: &InitArgs) -> Result<i32> {
    let c = connect(ClientKind::Cli).await?;
    let dir = a.dir.clone().unwrap_or_else(cwd);
    let dir = workroam_core::paths::canonical(std::path::Path::new(&dir))
        .display()
        .to_string();
    let plan = c
        .client
        .call("project.scan", json!({"dir": dir, "name": a.name}))
        .await?;
    if !out.json {
        println!(
            "{} {}  {}",
            o_b(out, "project"),
            s(&plan, "project"),
            out.dim(s(&plan, "dir"))
        );
        for i in plan["dir_issues"].as_array().into_iter().flatten() {
            let mark = if i["blocking"].as_bool().unwrap_or(false) {
                out.red("✗")
            } else {
                out.yellow("!")
            };
            println!("  {mark} {}", s(i, "message"));
        }
        for r in plan["repos"].as_array().into_iter().flatten() {
            let rec = &r["recommended"];
            println!(
                "  repo {:<20} → {} {}  {}",
                s(r, "path"),
                s(rec, "mode"),
                s(rec, "history"),
                out.dim(s(r, "reason"))
            );
        }
        for im in plan["imports"]["items"].as_array().into_iter().flatten() {
            println!("  import {} {}", s(im, "kind"), s(im, "source"));
        }
        for pr in plan["problems"].as_array().into_iter().flatten() {
            println!("  {} {}", out.yellow("cross-platform:"), s(pr, "message"));
        }
    }
    if a.dry_run {
        return Ok(out.done(&plan, |_, _| {}));
    }
    // Repository overrides: path=mode[:history].
    let mut repos = serde_json::Map::new();
    for spec in &a.repos {
        let (path, rest) = spec.split_once('=').ok_or_else(|| {
            Error::invalid(format!("--repo {spec}: expected <path>=<mode>[:<history>]"))
        })?;
        let (mode, history) = rest
            .split_once(':')
            .map(|(m, h)| (m, Some(h)))
            .unwrap_or((rest, None));
        let found = plan["repos"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|r| s(r, "path").trim_end_matches('/') == path.trim_end_matches('/'))
            .ok_or_else(|| Error::invalid(format!("no repository at {path}")))?;
        let mut cfg = found["recommended"].clone();
        cfg["path"] = json!(s(found, "path"));
        cfg["mode"] = json!(mode);
        if let Some(h) = history {
            cfg["history"] = json!(h);
        }
        repos.insert(s(found, "name").to_string(), cfg);
    }
    if out.interactive && !out.yes && !confirm(out, "Initialize this project and push it?", true)? {
        return Err(Error::new(Code::Denied, "cancelled"));
    }
    let skip: Vec<String> = a
        .skip_repos
        .iter()
        .filter_map(|p| {
            plan["repos"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|r| s(r, "path") == p.as_str())
                .map(|r| s(r, "name").to_string())
        })
        .collect();
    let choices = json!({
        "repos": repos,
        "skip_repos": skip,
        "take_imported": a.take_imported,
        "agents": if a.agents.is_empty() { Value::Null } else { json!(a.agents) },
    });
    let done = c
        .client
        .call(
            "project.init",
            json!({"dir": dir, "name": a.name, "choices": choices}),
        )
        .await?;
    if a.no_push {
        return Ok(out.done(&done, |o, v| {
            println!("{} initialized {}", o.green("✓"), s(v, "project"))
        }));
    }
    let plans = c
        .client
        .call("push.plan", json!({"project": s(&done, "project")}))
        .await?;
    require_service(&c)?;
    let results = run_jobs(&c, out, &plans).await?;
    Ok(out.done(&json!({"init": done, "push": results}), |o, v| {
        println!("{} initialized {}", o.green("✓"), s(&v["init"], "project"));
        for w in v["init"]["warnings"].as_array().into_iter().flatten() {
            println!("  {} {}", o.yellow("!"), w.as_str().unwrap_or(""));
        }
        print_results(o, &v["push"]);
    }))
}

fn o_b(out: &Out, t: &str) -> String {
    out.bold(t)
}

async fn project(out: &Out, cmd: &ProjectCmd) -> Result<i32> {
    let c = connect(ClientKind::Cli).await?;
    match cmd {
        ProjectCmd::List => {
            let v = c.client.call("project.list", json!({})).await?;
            Ok(out.done(&v, |o, v| {
                for p in v.as_array().into_iter().flatten() {
                    let state = match s(p, "binding") {
                        "bound" => o.green("synced"),
                        "pending" => o.yellow("added, not pulled"),
                        "missing" => o.red("folder missing"),
                        "archived" => o.dim("archived"),
                        _ if p["archived"].as_bool().unwrap_or(false) => o.dim("archived"),
                        _ => o.dim("not on this computer"),
                    };
                    println!(
                        "{:<24} {:<22} {}",
                        o.bold(s(p, "name")),
                        state,
                        o.dim(p["dir"].as_str().unwrap_or(""))
                    );
                }
            }))
        }
        ProjectCmd::Add {
            name,
            path,
            no_pull,
        } => {
            let v = c
                .client
                .call("project.add", json!({"project": name, "path": path}))
                .await?;
            out.info(&format!("added {name} at {}", s(&v, "dir")));
            if *no_pull {
                return Ok(out.done(&v, |_, _| {}));
            }
            let plans = c.client.call("pull.plan", json!({"project": name})).await?;
            require_service(&c)?;
            let r = run_jobs(&c, out, &plans).await?;
            Ok(out.done(&r, print_results))
        }
        ProjectCmd::Move { name, path } => {
            let v = c
                .client
                .call("project.move", json!({"project": name, "path": path}))
                .await?;
            Ok(out.done(&v, |_, v| println!("{name} will sync into {}", s(v, "dir"))))
        }
        ProjectCmd::Remove { name } => {
            if !confirm(
                out,
                &format!("Remove {name} from this computer? Files stay where they are."),
                true,
            )? {
                return Err(Error::new(Code::Denied, "cancelled"));
            }
            let v = c
                .client
                .call("project.remove", json!({"project": name}))
                .await?;
            Ok(out.done(&v, |_, v| {
                println!(
                    "removed {name} from this computer; files kept in {}",
                    s(v, "dir")
                )
            }))
        }
    }
}

async fn archive(out: &Out, cmd: &ArchiveCmd, _p: &[String]) -> Result<i32> {
    let c = connect(ClientKind::Cli).await?;
    match cmd {
        ArchiveCmd::Create {
            name,
            note,
            keep_history,
        } => {
            let v = c
                .client
                .call(
                    "archive.plan",
                    json!({"project": name, "note": note, "keep_history": keep_history}),
                )
                .await?;
            let pl = &v["plan"];
            if let Some(b) = pl["blocker"].as_str() {
                let code = pl["blocker_code"]
                    .as_str()
                    .and_then(|c| serde_json::from_value(json!(c)).ok())
                    .unwrap_or(Code::InvalidArgs);
                return Err(Error::new(code, b));
            }
            out.info(&format!(
                "archive {name}: {} files, {}, repos {}{}; other devices stop syncing it ({})",
                n(pl, "files"),
                workroam_core::duration::human_bytes(n(pl, "size")),
                pl["repos"].as_array().map(|a| a.len()).unwrap_or(0),
                if n(pl, "unsynced") > 0 {
                    format!("; {} unsynced changes are pushed first", n(pl, "unsynced"))
                } else {
                    String::new()
                },
                pl["devices"]
                    .as_array()
                    .map(|a| a
                        .iter()
                        .filter_map(|x| x.as_str())
                        .collect::<Vec<_>>()
                        .join(", "))
                    .unwrap_or_default()
            ));
            if !confirm(out, &format!("Archive {name}?"), false)? {
                return Err(Error::new(Code::Denied, "cancelled")
                    .with_hint("add --yes to archive without a terminal"));
            }
            let r = run_jobs(&c, out, &json!([v])).await?;
            Ok(out.done(&r, print_results))
        }
        ArchiveCmd::List { search } => {
            let v = c
                .client
                .call("archive.list", json!({"search": search}))
                .await?;
            Ok(out.done(&v, |o, v| {
                let list = v.as_array().cloned().unwrap_or_default();
                if list.is_empty() {
                    println!("no archived projects");
                }
                for a in list {
                    let r = &a["record"];
                    println!(
                        "{:<24} {}  {:<10} {}  {}",
                        o.bold(s(&a, "name")),
                        time(&r["archived_at"]),
                        s(&r["device"], "name"),
                        workroam_core::duration::human_bytes(n(r, "size")),
                        o.dim(s(r, "note"))
                    );
                }
            }))
        }
        ArchiveCmd::Restore {
            name,
            to,
            dry_run,
            prefer,
        } => {
            let v = c
                .client
                .call(
                    "archive.restore_plan",
                    json!({"project": name, "to": to, "dry_run": true}),
                )
                .await?;
            let pl = &v["plan"];
            let n_sc = n(pl, "scenario_number");
            let marks = ["", "①", "②", "③", "④", "⑤", "⑥", "⑦"];
            out.info(&format!(
                "{} {}  {}  ({} files differ)",
                marks.get(n_sc as usize).unwrap_or(&""),
                s(pl, "scenario"),
                s(pl, "dir"),
                pl["differing"].as_array().map(|a| a.len()).unwrap_or(0)
            ));
            if s(pl, "scenario") == "occupied" {
                return Err(Error::invalid(s(pl, "reason").to_string())
                    .with_hint("run again with --to <directory>"));
            }
            if *dry_run {
                return Ok(out.done(&v, |_, _| {}));
            }
            let mut prefer = prefer.clone();
            if prefer.is_none()
                && out.interactive
                && matches!(s(pl, "scenario"), "local_newer" | "diverged")
            {
                let opts: Vec<&str> = if s(pl, "scenario") == "local_newer" {
                    vec!["local", "archive"]
                } else {
                    vec!["merge", "archive", "compare"]
                };
                let i = dialoguer::Select::new()
                    .with_prompt("How should the two versions be combined?")
                    .items(&opts)
                    .default(0)
                    .interact()
                    .map_err(|e| Error::invalid(e.to_string()))?;
                prefer = Some(opts[i].to_string());
            }
            let mut params = json!({"project": name, "to": to, "prefer": prefer});
            if prefer.as_deref() == Some("compare") {
                let dir: String = dialoguer::Input::new()
                    .with_prompt("Extract into")
                    .interact_text()
                    .map_err(|e| Error::invalid(e.to_string()))?;
                params["compare_to"] = json!(dir);
            }
            let v = c.client.call("archive.restore_plan", params).await?;
            let r = run_jobs(&c, out, &json!([v])).await?;
            Ok(out.done(&r, print_results))
        }
        ArchiveCmd::Extract { name, to } => {
            let v = c
                .client
                .call("archive.extract", json!({"project": name, "to": to}))
                .await?;
            Ok(out.done(&v, |o, v| {
                println!(
                    "{} extracted {} into {} (still archived)",
                    o.green("✓"),
                    name,
                    s(v, "dir")
                )
            }))
        }
        ArchiveCmd::Delete {
            name,
            confirm: typed,
        } => {
            let typed = match typed {
                Some(t) => t.clone(),
                None if out.interactive => {
                    eprintln!(
                        "{}",
                        workroam_core::text::trf("archive.confirm_name", &[("name", name)])
                    );
                    dialoguer::Input::<String>::new()
                        .with_prompt(name.as_str())
                        .interact_text()
                        .map_err(|e| Error::invalid(e.to_string()))?
                }
                None => {
                    return Err(Error::invalid("type the project name to confirm")
                        .with_hint(format!("--confirm {name}")))
                }
            };
            let v = c
                .client
                .call("archive.delete", json!({"project": name, "confirm": typed}))
                .await?;
            Ok(out.done(&v, |o, v| println!("{} deleted the archive of {name} ({} snapshots); `workroam prune` frees the space", o.green("✓"), n(v, "forgotten"))))
        }
        ArchiveCmd::Export {
            name,
            file,
            password_stdin,
        } => {
            let pw = if *password_stdin {
                Some(stdin_line("the export password")?)
            } else {
                None
            };
            let v = c
                .client
                .call(
                    "archive.export",
                    json!({"project": name, "file": abs(file), "password": pw}),
                )
                .await?;
            Ok(out.done(&v, |o, v| {
                println!(
                    "{} wrote {} ({})",
                    o.green("✓"),
                    s(v, "file"),
                    workroam_core::duration::human_bytes(n(v, "size"))
                )
            }))
        }
        ArchiveCmd::Import {
            file,
            name,
            password_stdin,
        } => {
            let pw = if *password_stdin {
                Some(stdin_line("the export password")?)
            } else {
                None
            };
            let v = c
                .client
                .call(
                    "archive.import",
                    json!({"file": abs(file), "name": name, "password": pw}),
                )
                .await?;
            Ok(out.done(&v, |o, v| {
                println!(
                    "{} imported {} as an archived project; `workroam archive restore {}`",
                    o.green("✓"),
                    s(v, "project"),
                    s(v, "project")
                )
            }))
        }
    }
}

fn abs(p: &str) -> String {
    let path = std::path::Path::new(p);
    if path.is_absolute() {
        p.to_string()
    } else {
        std::env::current_dir()
            .map(|d| d.join(path).display().to_string())
            .unwrap_or_else(|_| p.to_string())
    }
}

async fn device(out: &Out, cmd: &DeviceCmd) -> Result<i32> {
    let c = connect(ClientKind::Cli).await?;
    match cmd {
        DeviceCmd::List => {
            let v = c.client.call("device.list", json!({})).await?;
            Ok(out.done(&v, |o, v| {
                for d in v.as_array().into_iter().flatten() {
                    let me = if d["this_device"].as_bool().unwrap_or(false) {
                        o.amber(" (this device)")
                    } else {
                        String::new()
                    };
                    let leases: Vec<&str> = d["leases"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|x| x.as_str())
                        .collect();
                    println!(
                        "{:<16}{} {:<8} {:<10} last sync {}  {}{}",
                        o.bold(s(d, "name")),
                        me,
                        s(d, "os"),
                        s(d, "access"),
                        time(&d["last_sync"]),
                        o.dim(
                            &d["reachable"]
                                .as_array()
                                .map(|a| a
                                    .iter()
                                    .filter_map(|x| x.as_str())
                                    .collect::<Vec<_>>()
                                    .join(","))
                                .unwrap_or_default()
                        ),
                        if leases.is_empty() {
                            String::new()
                        } else {
                            format!("  lease: {}", leases.join(", "))
                        }
                    );
                }
            }))
        }
        DeviceCmd::Add { read_only, qr } => {
            let v = c
                .client
                .call("device.pairing_code", json!({"read_only": read_only}))
                .await?;
            Ok(out.done(&v, |o, v| {
                println!("{}", tr("setup.pairing"));
                println!("\n{}\n", o.bold(s(v, "pairing_code")));
                if *qr {
                    println!("{}", s(v, "qr_text"));
                }
                if let Some(w) = v["warning"].as_str() {
                    println!("{} {w}", o.yellow("!"));
                }
                if let Some(code) = v["short_code"]["code"].as_str() {
                    let server = s(&v["short_code"], "server");
                    println!(
                        "{} {}  {}",
                        o.bold("Or the 6-digit code:"),
                        o.amber(code),
                        o.dim("(10 minutes, a new key for that device)")
                    );
                    println!(
                        "{}",
                        o.dim(&format!(
                            "On the new computer: workroam setup --join --server {server}"
                        ))
                    );
                    return;
                }
                println!("{}", o.dim("On the new computer: workroam setup --join"));
            }))
        }
        DeviceCmd::Remove {
            device,
            new_access_key,
            new_secret_key_stdin,
        } => {
            eprintln!(
                "{}",
                workroam_core::text::trf("device.remove.confirm", &[("device", device)])
            );
            if !confirm(out, &format!("Remove {device}?"), false)? {
                return Err(Error::new(Code::Denied, "cancelled")
                    .with_hint("add --yes to confirm without a terminal"));
            }
            let sk = if *new_secret_key_stdin {
                Some(stdin_line("the new secret key")?)
            } else {
                None
            };
            let v = c.client.call("device.remove", json!({"device": device, "new_access_key": new_access_key, "new_secret_key": sk})).await?;
            Ok(out.done(&v, |o, v| {
                println!("{} removed {}", o.green("✓"), s(v, "removed"));
                if let Some(h) = v["hint"].as_str() {
                    println!("  {} {h}", o.yellow("!"));
                }
            }))
        }
    }
}

async fn agent(out: &Out, cmd: &AgentCmd) -> Result<i32> {
    let c = connect(ClientKind::Cli).await?;
    match cmd {
        AgentCmd::List => {
            let v = c.client.call("agent.list", json!({})).await?;
            Ok(out.done(&v, |o, v| {
                for a in v["agents"].as_array().into_iter().flatten() {
                    let d = &a["detected"];
                    let state = if a["cloud"].as_bool().unwrap_or(false) {
                        o.dim("cloud")
                    } else if d["installed"].as_bool().unwrap_or(false) {
                        match d["supported"].as_bool() {
                            Some(false) => o.yellow("adapter may not match this version"),
                            _ => o.green("installed"),
                        }
                    } else {
                        o.dim("not found")
                    };
                    let enabled = v["enabled_in"][s(d, "id")]
                        .as_array()
                        .map(|x| x.len())
                        .unwrap_or(0);
                    println!(
                        "{:<14} {:<10} adapter v{}  {}  {}",
                        o.bold(s(d, "name")),
                        d["version"].as_str().unwrap_or("-"),
                        n(d, "adapter_version"),
                        state,
                        if enabled > 0 {
                            format!("in {enabled} projects")
                        } else {
                            String::new()
                        }
                    );
                }
                for g in v["connections"].as_array().into_iter().flatten() {
                    println!(
                        "connection {:<12} {:<10} {}",
                        s(g, "agent_name"),
                        s(g, "access"),
                        g["projects"]
                            .as_array()
                            .map(|a| a
                                .iter()
                                .filter_map(|x| x.as_str())
                                .collect::<Vec<_>>()
                                .join(", "))
                            .unwrap_or_default()
                    );
                }
            }))
        }
        AgentCmd::Connect { agent, relay } => {
            let v = c
                .client
                .call("agent.connect", json!({"agent": agent, "relay": relay}))
                .await?;
            Ok(out.done(&v, |o, v| {
                if v["needs_relay"].as_bool().unwrap_or(false) {
                    println!(
                        "Send this to {agent} (it deploys the relay on its cloud computer):\n"
                    );
                    println!("{}\n", s(v, "instructions"));
                    println!("{} {}", o.dim("then:"), s(v, "next"));
                } else if !s(v, "mcp_url").is_empty() {
                    println!(
                        "{} relay {}",
                        o.green("✓"),
                        if v["connected"].as_bool().unwrap_or(false) {
                            "connected"
                        } else {
                            "configured"
                        }
                    );
                    println!("connector URL: {}", o.bold(s(v, "mcp_url")));
                    if let Some(u) = v["open_url"].as_str() {
                        println!("open: {u}");
                    }
                    println!("{}", s(v, "instructions"));
                    println!("{}", o.dim(s(v, "next")));
                } else {
                    println!(
                        "{} {agent} connected in {} projects",
                        o.green("✓"),
                        v["projects"].as_array().map(|a| a.len()).unwrap_or(0)
                    );
                }
            }))
        }
        AgentCmd::Disconnect { agent } => {
            let v = c
                .client
                .call("agent.disconnect", json!({"agent": agent}))
                .await?;
            Ok(out.done(&v, |o, _| println!("{} {agent} disconnected", o.green("✓"))))
        }
        AgentCmd::Webhook {
            agent,
            url,
            secret_stdin,
        } => {
            let secret = if *secret_stdin {
                Some(stdin_line("the webhook secret")?)
            } else {
                None
            };
            let v = c
                .client
                .call(
                    "agent.webhook",
                    json!({"agent": agent, "url": url, "secret": secret}),
                )
                .await?;
            Ok(out.done(&v, |o, _| {
                println!("{} webhook set for {agent}", o.green("✓"))
            }))
        }
    }
}

/// `${secret:name}` values for the MCP definitions, kept in this computer's keychain.
async fn secret(out: &Out, cmd: &SecretCmd, cli_project: &[String]) -> Result<i32> {
    match cmd {
        SecretCmd::Set { name, value_stdin } => {
            let prompt = format!("Value of {name}");
            let value = secret_from(
                *value_stdin,
                "WORKROAM_SECRET_VALUE",
                "the secret value",
                Some(prompt.as_str()),
                out.interactive,
            )?
            .ok_or_else(|| {
                Error::invalid("the secret value is required").with_hint("--value-stdin")
            })?;
            let c = connect(ClientKind::Cli).await?;
            let v = c
                .client
                .call("secret.set", json!({"name": name, "value": value}))
                .await?;
            Ok(out.done(&v, |o, v| {
                println!("{} secret {name} saved ({})", o.green("✓"), s(v, "backend"))
            }))
        }
        SecretCmd::Remove { name } => {
            let c = connect(ClientKind::Cli).await?;
            let v = c
                .client
                .call("secret.remove", json!({"name": name}))
                .await?;
            Ok(out.done(&v, |o, _| {
                println!("{} secret {name} removed", o.green("✓"))
            }))
        }
        SecretCmd::List => {
            let c = connect(ClientKind::Cli).await?;
            let v = c.client.call("secret.list", scope(cli_project)).await?;
            Ok(out.done(&v, |o, v| {
                let list = v["secrets"].as_array().cloned().unwrap_or_default();
                if list.is_empty() {
                    println!(
                        "{} refers to no ${{secret:…}} in .workroam/mcp.toml",
                        s(v, "project")
                    );
                }
                for x in &list {
                    let servers: Vec<&str> = x["servers"]
                        .as_array()
                        .map(|a| a.iter().filter_map(Value::as_str).collect())
                        .unwrap_or_default();
                    let state = if x["set"].as_bool().unwrap_or(false) {
                        o.green("set")
                    } else {
                        o.yellow("not set")
                    };
                    println!(
                        "{:<24} {:<8} {}",
                        s(x, "name"),
                        state,
                        o.dim(&servers.join(", "))
                    );
                }
            }))
        }
    }
}

async fn backup(out: &Out, cmd: &BackupCmd) -> Result<i32> {
    let c = connect(ClientKind::Cli).await?;
    let v = match cmd {
        BackupCmd::Add {
            kind,
            endpoint,
            bucket,
            prefix,
            region,
            path,
            schedule,
            delete_delay_days,
            access_key,
            secret_key_stdin,
        } => {
            let sk = if *secret_key_stdin {
                Some(stdin_line("the mirror secret key")?)
            } else {
                std::env::var("WORKROAM_BACKUP_SECRET_KEY").ok()
            };
            c.client
                .call("backup.configure", json!({"action": "add", "kind": kind, "endpoint": endpoint, "bucket": bucket, "prefix": prefix, "region": region, "path": path.as_deref().map(abs), "schedule": schedule, "delete_delay_days": delete_delay_days, "access_key": access_key, "secret_key": sk}))
                .await?
        }
        BackupCmd::List => {
            c.client
                .call("backup.configure", json!({"action": "list"}))
                .await?
        }
        BackupCmd::Remove => {
            c.client
                .call("backup.configure", json!({"action": "remove"}))
                .await?
        }
        BackupCmd::Run => {
            out.info("mirroring…");
            c.client.call("backup.run", json!({})).await?
        }
    };
    Ok(out.done(&v, |o, v| match cmd {
        BackupCmd::Run => println!(
            "{} copied {} objects ({}), {} unchanged, {} waiting to be deleted",
            o.green("✓"),
            n(v, "copied"),
            workroam_core::duration::human_bytes(n(v, "bytes")),
            n(v, "unchanged"),
            n(v, "pending_delete")
        ),
        BackupCmd::List => {
            if v["backup"].is_null() {
                println!("no off-site backup configured");
            } else {
                let b = &v["backup"];
                println!(
                    "{} {}{}  schedule {}  last ok {}",
                    s(b, "kind"),
                    s(b, "endpoint"),
                    if s(b, "kind") == "local" {
                        s(b, "path").to_string()
                    } else {
                        format!("/{}", s(b, "bucket"))
                    },
                    s(b, "schedule"),
                    time(&v["last_ok"])
                );
                if let Some(e) = v["last_error"].as_str() {
                    println!("{} {e}", o.red("last error:"));
                }
            }
        }
        _ => println!("{} done", o.green("✓")),
    }))
}
