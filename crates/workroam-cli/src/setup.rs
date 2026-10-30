//! `workroam setup`: the same three paths as the desktop wizard (PRD §8.1),
//! over the `setup.*` methods. Flags answer questions up front; anything
//! left open is asked when a person is at the terminal, and is an error
//! (or a safe default) otherwise.

use crate::cli::SetupArgs;
use crate::cmds::{confirm, print_checks, secret_from, stdin_line};
use crate::conn;
use crate::out::{s, Out};
use serde_json::{json, Value};
use workroam_core::text::{tr, trf};
use workroam_core::{Code, Error, Result};
use workroam_ipc::{Client, ClientKind};

fn dlg<T>(r: std::result::Result<T, dialoguer::Error>) -> Result<T> {
    r.map_err(|e| Error::invalid(e.to_string()))
}

/// A flag's value, else a prompt (with `default`), else `default`.
fn ask(out: &Out, flag: &Option<String>, prompt: &str, default: &str) -> Result<String> {
    if let Some(v) = flag {
        return Ok(v.clone());
    }
    if out.interactive {
        let mut i = dialoguer::Input::<String>::new()
            .with_prompt(prompt)
            .allow_empty(true);
        if !default.is_empty() {
            i = i.default(default.to_string());
        }
        return dlg(i.interact_text());
    }
    Ok(default.to_string())
}

fn required(
    out: &Out,
    flag: &Option<String>,
    prompt: &str,
    default: &str,
    name: &str,
) -> Result<String> {
    let v = ask(out, flag, prompt, default)?;
    if v.trim().is_empty() {
        return Err(Error::invalid(format!("{name} is required"))
            .with_hint(format!("pass --{}", name.replace(' ', "-"))));
    }
    Ok(v)
}

/// A flag's value, else a menu of `(value, label)`, else the first choice.
fn choose(
    out: &Out,
    flag: &Option<String>,
    prompt: &str,
    items: &[(&str, String)],
    default: usize,
) -> Result<String> {
    if let Some(v) = flag {
        return Ok(v.clone());
    }
    if out.interactive {
        let labels: Vec<&str> = items.iter().map(|(_, l)| l.as_str()).collect();
        let i = dlg(dialoguer::Select::new()
            .with_prompt(prompt)
            .items(&labels)
            .default(default)
            .interact())?;
        return Ok(items[i].0.to_string());
    }
    Ok(items[default].0.to_string())
}

struct Session<'a> {
    client: &'a Client,
    id: String,
}

impl Session<'_> {
    async fn step(&self, step: &str, extra: Value) -> Result<Value> {
        let mut p = json!({"session": self.id, "step": step});
        if let (Some(o), Some(e)) = (p.as_object_mut(), extra.as_object()) {
            for (k, v) in e {
                o.insert(k.clone(), v.clone());
            }
        }
        self.client.call("setup.step", p).await
    }
}

pub async fn run(out: &Out, a: &SetupArgs, projects: &[String]) -> Result<i32> {
    if workroamd::is_configured()
        && !out.yes
        && !confirm(
            out,
            "This computer is already set up. Set it up again?",
            false,
        )?
    {
        return Err(Error::new(Code::Denied, "this computer is already set up")
            .with_hint("pass -y to set it up again"));
    }
    let mode: String = if a.new {
        "new".into()
    } else if a.join || a.pairing_code_stdin {
        "join".into()
    } else if a.from_backup {
        "from_backup".into()
    } else {
        let items = [
            ("new", tr("setup.path.new")),
            ("join", tr("setup.path.join")),
            ("from_backup", tr("setup.path.backup")),
        ];
        choose(out, &None, &tr("setup.path"), &items, 0)?
    };
    let c = conn::connect(ClientKind::Cli).await?;
    let begin = c.client.call("setup.begin", json!({"mode": mode})).await?;
    let sess = Session {
        client: &c.client,
        id: s(&begin, "session").to_string(),
    };
    let r = match mode.as_str() {
        "new" => new(out, a, &sess, &begin).await,
        "join" => join(out, a, &sess, &begin, projects).await,
        _ => from_backup(out, a, &sess, &begin, projects).await,
    };
    if r.is_err() {
        let _ = c
            .client
            .call("setup.cancel", json!({"session": sess.id}))
            .await;
    }
    let done = r?;
    Ok(out.done(&done, |o, v| print_done(o, a, v)))
}

/// Returns the default project location the user chose.
async fn machine(out: &Out, a: &SetupArgs, sess: &Session<'_>, begin: &Value) -> Result<String> {
    let d = &begin["defaults"];
    let name = required(
        out,
        &a.device_name,
        &tr("setup.device_name"),
        s(d, "device_name"),
        "device name",
    )?;
    let root = required(out, &a.root, &tr("setup.root"), s(d, "root"), "root")?;
    let reachable = if !a.reachable.is_empty() || !out.interactive {
        a.reachable.clone()
    } else {
        ask(out, &None, &tr("setup.reachable"), "")?
            .split(',')
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
            .collect()
    };
    let auto = match &a.auto {
        Some(v) => Some(v.clone()),
        None if out.interactive => {
            let items = [
                ("ask", tr("auto.ask")),
                ("auto", tr("auto.auto")),
                ("off", tr("auto.off")),
            ];
            Some(choose(out, &None, &tr("setup.auto"), &items, 0)?)
        }
        None => None,
    };
    let autostart = if a.no_autostart {
        false
    } else if a.autostart || !out.interactive {
        true
    } else {
        confirm(out, &tr("setup.autostart"), true)?
    };
    sess.step(
        "machine",
        json!({"machine": {"device_name": name, "root": root, "reachable": reachable, "auto": auto, "autostart": autostart}}),
    )
    .await?;
    Ok(root)
}

async fn new(out: &Out, a: &SetupArgs, sess: &Session<'_>, begin: &Value) -> Result<Value> {
    // Storage.
    let presets: Vec<String> = begin["defaults"]["presets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| p.as_str().map(String::from))
        .collect();
    let items: Vec<(&str, String)> = presets
        .iter()
        .map(|p| (p.as_str(), tr(&format!("preset.{p}"))))
        .collect();
    let preset = choose(out, &a.preset, &tr("setup.preset"), &items, 0)?;
    if preset == "rustfs" && a.endpoint.is_none() {
        out.info(&out.dim(&tr("setup.rustfs_hint")));
    }
    let ep_prompt = if preset == "local" {
        tr("preset.local")
    } else {
        tr("setup.endpoint")
    };
    let endpoint = match preset.as_str() {
        "aws" | "r2" | "oss" => ask(out, &a.endpoint, &ep_prompt, "")?,
        _ => required(out, &a.endpoint, &ep_prompt, "", "endpoint")?,
    };
    let ep = sess
        .step(
            "endpoint",
            json!({"preset": preset, "endpoint": endpoint, "addressing": a.addressing, "region": a.region}),
        )
        .await?;
    if preset != "local" {
        let ak = required(
            out,
            &a.access_key,
            &tr("setup.access_key"),
            "",
            "access key",
        )?;
        let sk = secret_from(
            a.secret_key_stdin,
            "WORKROAM_SECRET_KEY",
            "the secret key",
            Some(&tr("setup.secret_key")),
            out.interactive,
        )?
        .ok_or_else(|| {
            Error::invalid("the secret key is required")
                .with_hint("pass --secret-key-stdin or set WORKROAM_SECRET_KEY")
        })?;
        sess.step("credentials", json!({"access_key": ak, "secret_key": sk}))
            .await?;
    } else {
        sess.step("credentials", json!({})).await?;
    }
    // Key permissions.
    let can_create = ep["can_create_key"].as_bool().unwrap_or(false);
    let key_mode = if can_create {
        let items = [
            ("create", tr("key_mode.create")),
            ("current", tr("key_mode.current")),
        ];
        choose(out, &a.key_mode, &tr("setup.key_mode"), &items, 0)?
    } else {
        a.key_mode.clone().unwrap_or_else(|| "current".into())
    };
    let admin_token = if key_mode == "create" && ep["needs_admin_token"].as_bool().unwrap_or(false)
    {
        secret_from(
            false,
            "WORKROAM_ADMIN_TOKEN",
            "",
            Some("Admin API token"),
            out.interactive,
        )?
    } else {
        None
    };
    sess.step(
        "key_mode",
        json!({"mode": key_mode, "access": a.key_access, "admin_endpoint": a.admin_endpoint, "admin_token": admin_token, "accept_broad_key": a.accept_broad_key}),
    )
    .await?;
    // The machine first: the default bucket name uses the device name.
    machine(out, a, sess, begin).await?;
    // Bucket.
    if preset != "local" {
        let bucket = match &a.bucket {
            Some(b) => b.clone(),
            None if out.interactive => {
                let items = [("auto", tr("bucket.auto")), ("custom", tr("bucket.custom"))];
                match choose(out, &None, &tr("setup.bucket"), &items, 0)?.as_str() {
                    "auto" => "auto".into(),
                    _ => required(out, &None, &tr("setup.bucket"), "", "bucket")?,
                }
            }
            None => "auto".into(),
        };
        let prefix = ask(out, &a.prefix, &tr("setup.prefix"), "")?;
        let (choice, name) = if bucket == "auto" {
            ("auto", None)
        } else {
            ("custom", Some(bucket))
        };
        sess.step(
            "bucket",
            json!({"choice": choice, "name": name, "prefix": prefix}),
        )
        .await?;
    } else {
        sess.step(
            "bucket",
            json!({"choice": "custom", "name": "workspace", "prefix": a.prefix}),
        )
        .await?;
    }
    // Checks.
    loop {
        out.info(&format!("\n{}", out.bold(&tr("setup.checks"))));
        let chk = sess
            .client
            .call("setup.check", json!({"session": sess.id}))
            .await?;
        if !out.json {
            print_checks(out, &chk);
            println!();
        }
        if chk["passed"].as_bool().unwrap_or(false) {
            break;
        }
        if chk["broad_key"].as_bool().unwrap_or(false) && !a.accept_broad_key {
            out.info(&out.yellow(&tr("setup.broad_key")));
            if out.interactive && confirm(out, &tr("setup.broad_key.confirm"), false)? {
                sess.step("accept_broad_key", json!({})).await?;
                continue;
            }
        }
        if out.interactive && confirm(out, &tr("setup.checks.retry"), true)? {
            continue;
        }
        let e: Error = serde_json::from_value(chk["blocker"].clone())
            .unwrap_or_else(|_| Error::new(Code::ChecksFailed, "some checks did not pass"));
        return Err(e);
    }
    // Encryption password and recovery key.
    let pw = match secret_from(
        a.password_stdin,
        "WORKROAM_PASSWORD",
        "the encryption password",
        None,
        false,
    )? {
        Some(p) => p,
        None if out.interactive => dlg(dialoguer::Password::new()
            .with_prompt(tr("setup.password"))
            .with_confirmation(tr("setup.password.again"), tr("setup.password.mismatch"))
            .interact())?,
        None => {
            return Err(Error::invalid("the encryption password is required")
                .with_hint("pass --password-stdin or set WORKROAM_PASSWORD"))
        }
    };
    let info = sess.step("password", json!({"password": pw})).await?;
    let rk = s(&info, "recovery_key").to_string();
    if let Some(path) = &a.recovery_key_out {
        write_private(path, &format!("{rk}\n"))?;
        sess.step("recovery_saved", json!({})).await?;
        out.info(&format!("recovery key written to {path}"));
    } else if out.interactive {
        println!(
            "\n{}\n\n    {}\n",
            tr("setup.recovery"),
            out.amber(&out.bold(&rk))
        );
        let g = &info["confirm_groups"];
        let (ga, gb) = (
            g[0].as_u64().unwrap_or(1).to_string(),
            g[1].as_u64().unwrap_or(5).to_string(),
        );
        loop {
            let typed = ask(
                out,
                &None,
                &trf("setup.recovery.confirm", &[("a", &ga), ("b", &gb)]),
                "",
            )?;
            match sess.step("confirm_recovery", json!({"typed": typed})).await {
                Ok(_) => break,
                Err(e) => out.info(&out.red(&e.message)),
            }
        }
    } else {
        return Err(
            Error::invalid("without a terminal the recovery key must be saved to a file")
                .with_hint("pass --recovery-key-out <file>"),
        );
    }
    let mut done = sess
        .client
        .call("setup.commit", json!({"session": sess.id}))
        .await?;
    if a.recovery_key_out.is_some() {
        // Never print the key when it went to a file.
        done["recovery_key"] = Value::Null;
    }
    Ok(done)
}

async fn unlock(out: &Out, a: &SetupArgs, sess: &Session<'_>) -> Result<Value> {
    if a.recovery_key_stdin {
        let rk = stdin_line("the recovery key")?;
        return sess.step("unlock", json!({"recovery_key": rk})).await;
    }
    loop {
        let pw = secret_from(
            a.password_stdin,
            "WORKROAM_PASSWORD",
            "the encryption password",
            Some(&tr("setup.unlock")),
            out.interactive,
        )?
        .ok_or_else(|| {
            Error::invalid("the encryption password is required")
                .with_hint("pass --password-stdin or set WORKROAM_PASSWORD")
        })?;
        match sess.step("unlock", json!({"password": pw})).await {
            Ok(v) => return Ok(v),
            Err(e) if out.interactive && !a.password_stdin => {
                out.info(&out.red(&e.message));
                if confirm(out, &tr("setup.unlock.forgot"), false)? {
                    let rk = dlg(dialoguer::Password::new()
                        .with_prompt("Recovery key")
                        .interact())?;
                    return sess.step("unlock", json!({"recovery_key": rk})).await;
                }
            }
            Err(e) => return Err(e),
        }
    }
}

/// Which projects this device takes: -p, --all-projects, or a checklist.
async fn pick_projects(
    out: &Out,
    a: &SetupArgs,
    sess: &Session<'_>,
    info: &Value,
    projects: &[String],
    as_device: Option<String>,
    root: &str,
) -> Result<()> {
    let all: Vec<String> = info["projects"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| p.as_str().map(String::from))
        .collect();
    let chosen: Vec<String> = if a.all_projects {
        all.clone()
    } else if !projects.is_empty() {
        // `--project <name>[=<dir>]`
        let mut list = Vec::new();
        for p in projects {
            let (name, dir) = match p.split_once('=') {
                Some((n, d)) => (n.to_string(), Some(d.to_string())),
                None => (p.clone(), None),
            };
            if !all.contains(&name) {
                return Err(Error::new(
                    Code::InvalidArgs,
                    format!("the workspace has no project {name}"),
                )
                .with_data(json!(all)));
            }
            list.push(json!({"name": name, "dir": dir}));
        }
        return sess
            .step(
                "projects",
                json!({"projects": list, "all": false, "as_device": as_device}),
            )
            .await
            .map(|_| ());
    } else if out.interactive && !all.is_empty() {
        let picks = dlg(dialoguer::MultiSelect::new()
            .with_prompt(tr("setup.projects"))
            .items(&all)
            .defaults(&vec![true; all.len()])
            .interact())?;
        let root = workroam_core::paths::expand_tilde(root);
        let mut list = Vec::new();
        for i in picks {
            let def = root.join(&all[i]).display().to_string();
            let dir = ask(out, &None, &format!("{} →", all[i]), &def)?;
            list.push(json!({"name": all[i], "dir": dir}));
        }
        return sess
            .step(
                "projects",
                json!({"projects": list, "all": false, "as_device": as_device}),
            )
            .await
            .map(|_| ());
    } else {
        vec![]
    };
    let list: Vec<Value> = chosen.iter().map(|n| json!({"name": n})).collect();
    sess.step(
        "projects",
        json!({"projects": list, "all": a.all_projects, "as_device": as_device}),
    )
    .await?;
    Ok(())
}

async fn join(
    out: &Out,
    a: &SetupArgs,
    sess: &Session<'_>,
    begin: &Value,
    projects: &[String],
) -> Result<Value> {
    let code = if a.pairing_code_stdin {
        stdin_line("the pairing code")?
    } else {
        required(out, &None, &tr("setup.pairing.paste"), "", "pairing code")
            .map_err(|e| e.with_hint("pass --pairing-code-stdin"))?
    };
    let code = code.trim().to_string();
    let six = code.chars().filter(|c| c.is_ascii_digit()).count() == 6 && !code.starts_with("wr1:");
    let server = if six {
        Some(required(
            out,
            &a.server,
            "Server address (https://…)",
            "",
            "server",
        )?)
    } else {
        None
    };
    let device_name = a
        .device_name
        .clone()
        .unwrap_or_else(|| s(&begin["defaults"], "device_name").to_string());
    sess.step(
        "pairing_code",
        json!({"code": code, "server": server, "device_name": device_name}),
    )
    .await?;
    let root = machine(out, a, sess, begin).await?;
    let info = unlock(out, a, sess).await?;
    out.info(&format!(
        "{} {} / {}  ({} projects, devices: {})",
        out.green("✓"),
        s(&info, "endpoint"),
        s(&info, "bucket"),
        info["projects"].as_array().map(|x| x.len()).unwrap_or(0),
        info["devices"]
            .as_array()
            .map(|d| d
                .iter()
                .filter_map(|x| x.as_str())
                .collect::<Vec<_>>()
                .join(", "))
            .unwrap_or_default()
    ));
    pick_projects(out, a, sess, &info, projects, None, &root).await?;
    sess.client
        .call("setup.commit", json!({"session": sess.id}))
        .await
}

async fn from_backup(
    out: &Out,
    a: &SetupArgs,
    sess: &Session<'_>,
    begin: &Value,
    projects: &[String],
) -> Result<Value> {
    let endpoint = required(
        out,
        &a.endpoint,
        "Backup endpoint (or directory)",
        "",
        "endpoint",
    )?;
    let bucket = ask(out, &a.bucket, "Backup bucket", "")?;
    let prefix = ask(out, &a.prefix, &tr("setup.prefix"), "")?;
    let preset = a.preset.clone().unwrap_or_else(|| {
        if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
            "other".into()
        } else {
            "local".into()
        }
    });
    let (ak, sk) = if preset == "local" {
        (None, None)
    } else {
        let ak = required(
            out,
            &a.access_key,
            &tr("setup.access_key"),
            "",
            "access key",
        )?;
        let sk = secret_from(
            a.secret_key_stdin,
            "WORKROAM_SECRET_KEY",
            "the secret key",
            Some(&tr("setup.secret_key")),
            out.interactive,
        )?;
        (Some(ak), sk)
    };
    sess.step(
        "backup_source",
        json!({"endpoint": endpoint, "bucket": bucket, "prefix": prefix, "region": a.region, "preset": preset, "access_key": ak, "secret_key": sk}),
    )
    .await?;
    let root = machine(out, a, sess, begin).await?;
    let info = unlock(out, a, sess).await?;
    let as_device = match &a.as_device {
        Some(d) => Some(d.clone()),
        None if out.interactive => {
            let devs: Vec<String> = info["devices"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|x| x.as_str().map(String::from))
                .collect();
            if devs.is_empty() {
                None
            } else {
                let mut items: Vec<(&str, String)> = vec![("", "A new device".to_string())];
                items.extend(
                    devs.iter()
                        .map(|d| (d.as_str(), format!("{}: {d}", tr("setup.as_device")))),
                );
                Some(choose(out, &None, &tr("setup.as_device"), &items, 0)?)
                    .filter(|d| !d.is_empty())
            }
        }
        None => None,
    };
    pick_projects(out, a, sess, &info, projects, as_device, &root).await?;
    sess.client
        .call("setup.commit", json!({"session": sess.id}))
        .await
}

fn write_private(path: &str, body: &str) -> Result<()> {
    let p = std::path::Path::new(path);
    if let Some(d) = p.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(d)?;
    }
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(p)?;
        f.write_all(body.as_bytes())?;
    }
    #[cfg(not(unix))]
    std::fs::write(p, body)?;
    Ok(())
}

fn print_done(o: &Out, a: &SetupArgs, v: &Value) {
    println!(
        "\n{} {} · {}",
        o.green("✓"),
        s(v, "device_name"),
        s(v, "workspace")
    );
    if v["created_bucket"].as_bool().unwrap_or(false) {
        println!("  bucket created");
    }
    if v["created_key"].as_bool().unwrap_or(false) {
        println!("  scoped key created; the admin key was discarded");
    }
    if let Some(code) = v["pairing_code"].as_str() {
        println!("\n{}\n  {}", o.bold(&tr("setup.pairing")), code);
        if a.qr {
            if let Some(q) = workroam_device::qr::terminal(code) {
                println!("{q}");
            }
        }
    }
    let bound = v["projects"].as_array().map(|x| x.len()).unwrap_or(0);
    if s(v, "mode") == "new" {
        println!("\n{}", tr("setup.done"));
    } else if bound > 0 {
        println!("\n{bound} projects bound on this device. Next: workroam pull --all");
    } else {
        println!("\nNext: workroam project list · workroam pull -p <name>");
    }
}
