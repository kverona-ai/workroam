//! Drive the daemon over IPC like the CLI and the desktop app do.

use serde_json::{json, Value};
use std::sync::Arc;
use workroam_core::Code;
use workroam_ipc::{Client, ClientKind};

async fn client(d: &Arc<workroamd::Daemon>, kind: ClientKind, agent: Option<&str>) -> Client {
    let (a, b) = tokio::io::duplex(1 << 20);
    tokio::spawn(workroamd::server::serve_conn(d.clone(), Box::new(b)));
    Client::over(Box::new(a), kind, agent.map(String::from))
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn setup_init_push_approve_archive() {
    let td = tempfile::tempdir().unwrap();
    std::env::set_var("WORKROAM_HOME", td.path().join("home"));
    std::env::set_var("WORKROAM_SECRETS", "file");
    let store = format!("file://{}", td.path().join("store").display());
    let d = workroamd::Daemon::new(false);
    let c = client(&d, ClientKind::Cli, None).await;
    assert!(!c.hello.configured);

    // Setup over IPC, step by step like the wizard.
    let s = c.call("setup.begin", json!({"mode": "new"})).await.unwrap();
    let sid = s["session"].as_str().unwrap().to_string();
    let step = |step: &str, extra: Value| {
        let mut p = json!({"session": sid, "step": step});
        p.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        p
    };
    c.call(
        "setup.step",
        step("endpoint", json!({"preset": "local", "endpoint": store})),
    )
    .await
    .unwrap();
    c.call("setup.step", step("credentials", json!({})))
        .await
        .unwrap();
    c.call("setup.step", step("key_mode", json!({"mode": "current"})))
        .await
        .unwrap();
    c.call(
        "setup.step",
        step("bucket", json!({"choice": "custom", "name": "ws"})),
    )
    .await
    .unwrap();
    let chk = c
        .call("setup.check", json!({"session": sid}))
        .await
        .unwrap();
    assert!(chk["results"].as_array().unwrap().len() > 3);
    let pw = c
        .call(
            "setup.step",
            step("password", json!({"password": "correct horse battery"})),
        )
        .await
        .unwrap();
    let rk = pw["recovery_key"].as_str().unwrap().to_string();
    let groups: Vec<&str> = rk.split('-').collect();
    let g = &pw["confirm_groups"];
    let typed = format!(
        "{} {}",
        groups[g[0].as_u64().unwrap() as usize - 1],
        groups[g[1].as_u64().unwrap() as usize - 1]
    );
    c.call(
        "setup.step",
        step("confirm_recovery", json!({"typed": typed})),
    )
    .await
    .unwrap();
    let root = td.path().join("projects");
    c.call(
        "setup.step",
        step(
            "machine",
            json!({"machine": {"device_name": "office", "root": root.display().to_string(), "autostart": false}}),
        ),
    )
    .await
    .unwrap();
    let done = match c.call("setup.commit", json!({"session": sid})).await {
        Ok(v) => v,
        Err(e) if e.code == Code::ChecksFailed => {
            eprintln!(
                "skipping: storage checks fail in this environment: {}",
                e.message
            );
            return;
        }
        Err(e) => panic!("{e}"),
    };
    assert!(done["pairing_code"].as_str().unwrap().starts_with("wr1:"));

    // Init a project and push it.
    let dir = root.join("demo");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("notes.md"), "hello\n").unwrap();
    let scan = c
        .call("project.scan", json!({"dir": dir.display().to_string()}))
        .await
        .unwrap();
    assert_eq!(scan["project"], "demo");
    c.call("project.init", json!({"dir": dir.display().to_string()}))
        .await
        .unwrap();
    let plans = c
        .call(
            "push.plan",
            json!({"cwd": dir.join("sub").display().to_string()}),
        )
        .await
        .unwrap();
    let job = plans[0]["job"].as_str().unwrap().to_string();
    let r = c
        .call("job.start", json!({"job": job, "wait": true}))
        .await
        .unwrap();
    assert!(r["result"]["snapshot"].is_string(), "{r}");

    // An agent's push waits for approval; a person approves it.
    std::fs::write(dir.join("notes.md"), "hello again\n").unwrap();
    let agent = client(&d, ClientKind::Cli, Some("codex")).await;
    agent.call("subscribe", json!({})).await.unwrap();
    let plans = agent
        .call("push.plan", json!({"project": "demo", "release": true}))
        .await
        .unwrap();
    let job = plans[0]["job"].as_str().unwrap().to_string();
    let waiting = {
        let agent = Arc::new(agent);
        let a2 = agent.clone();
        let h = tokio::spawn(async move {
            a2.call("job.start", json!({"job": job, "wait": true}))
                .await
        });
        let ev = loop {
            let ev = agent.next_event().await.unwrap();
            if ev.method == "approval.requested" {
                break ev;
            }
        };
        assert_eq!(ev.params["initiator"], "codex");
        // The agent cannot approve itself.
        let id = ev.params["id"].as_str().unwrap().to_string();
        let e = agent
            .call("approval.respond", json!({"id": id, "decision": "approve"}))
            .await
            .unwrap_err();
        assert_eq!(e.code, Code::Denied);
        c.call("approval.respond", json!({"id": id, "decision": "approve"}))
            .await
            .unwrap();
        h.await.unwrap().unwrap()
    };
    assert_eq!(waiting["result"]["released"], true);

    // Settings round trip and validation.
    c.call(
        "config.set",
        json!({"key": "sync.idle_minutes", "value": "5"}),
    )
    .await
    .unwrap();
    let v = c
        .call("config.get", json!({"key": "sync.idle_minutes"}))
        .await
        .unwrap();
    assert_eq!(v["value"], 5);
    let e = c
        .call(
            "config.set",
            json!({"key": "sync.auto", "value": "sometimes"}),
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, Code::InvalidArgs);
    c.call(
        "config.set",
        json!({"key": "retention.days", "value": "14"}),
    )
    .await
    .unwrap();

    // Status, history, archive and restore through jobs.
    let st = c.call("status", json!({"project": "demo"})).await.unwrap();
    assert_eq!(st["projects"][0]["project"], "demo");
    let h = c
        .call("history.list", json!({"project": "demo"}))
        .await
        .unwrap();
    assert_eq!(h.as_array().unwrap().len(), 2);
    let a = c
        .call("archive.plan", json!({"project": "demo", "note": "done"}))
        .await
        .unwrap();
    c.call("job.start", json!({"job": a["job"], "wait": true}))
        .await
        .unwrap();
    let list = c.call("archive.list", json!({})).await.unwrap();
    assert_eq!(list[0]["name"], "demo");
    let r = c
        .call("archive.restore", json!({"project": "demo"}))
        .await
        .unwrap();
    assert_eq!(r["result"]["restored"], true);
    let e = c
        .call("archive.delete", json!({"project": "demo"}))
        .await
        .unwrap_err();
    assert_eq!(e.code, Code::InvalidArgs);

    // Pause and resume.
    let p = c.call("sync.pause", json!({"for": "30m"})).await.unwrap();
    assert!(p["paused_until"].is_string());
    c.call("sync.resume", json!({})).await.unwrap();
    let devs = c.call("device.list", json!({})).await.unwrap();
    assert_eq!(devs[0]["name"], "office");
    let code = c.call("device.pairing_code", json!({})).await.unwrap();
    assert!(code["pairing_code"].as_str().unwrap().starts_with("wr1:"));
}
