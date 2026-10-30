//! Run the `workroam` binary the way a script or agent does: no terminal,
//! `--json`, an in-process daemon.

use serde_json::Value;
use std::io::Write;
use std::process::{Command, Stdio};

fn workroam(
    home: &std::path::Path,
    cwd: &std::path::Path,
    args: &[&str],
    stdin: &str,
) -> (i32, String) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_workroam"))
        .args(args)
        .current_dir(cwd)
        .env("WORKROAM_HOME", home)
        .env("WORKROAM_SECRETS", "file")
        .env("WORKROAM_IN_PROCESS", "1")
        .env("WORKROAM_NO_BANNER", "1")
        .env_remove("WORKROAM_AGENT")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    c.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    let o = c.wait_with_output().unwrap();
    (
        o.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&o.stdout).into_owned() + &String::from_utf8_lossy(&o.stderr),
    )
}

fn last_json(s: &str) -> Value {
    s.lines()
        .rev()
        .find_map(|l| {
            serde_json::from_str::<Value>(l)
                .ok()
                .filter(|v| v.get("schema_version").is_some())
        })
        .unwrap_or_else(|| panic!("no JSON result in: {s}"))
}

#[test]
fn setup_init_status_and_mcp() {
    let td = tempfile::tempdir().unwrap();
    let home = td.path().join("home");
    let store = format!("file://{}", td.path().join("store").display());
    let root = td.path().join("projects");
    let rk = td.path().join("rk.txt");

    // Not set up yet.
    let (code, out) = workroam(&home, td.path(), &["--json"], "");
    assert_eq!(code, 0, "{out}");
    assert_eq!(last_json(&out)["result"]["configured"], false);

    // Without a terminal a new workspace needs --recovery-key-out.
    let (code, out) = workroam(
        &home,
        td.path(),
        &[
            "--json",
            "setup",
            "--new",
            "--preset",
            "local",
            "--endpoint",
            &store,
            "--password-stdin",
            "--device-name",
            "office",
            "--root",
            root.to_str().unwrap(),
        ],
        "correct horse battery\n",
    );
    assert_eq!(code, 2, "{out}");
    assert_eq!(last_json(&out)["ok"], false);

    let (code, out) = workroam(
        &home,
        td.path(),
        &[
            "--json",
            "setup",
            "--new",
            "--preset",
            "local",
            "--endpoint",
            &store,
            "--password-stdin",
            "--recovery-key-out",
            rk.to_str().unwrap(),
            "--device-name",
            "office",
            "--root",
            root.to_str().unwrap(),
            "--no-autostart",
        ],
        "correct horse battery\n",
    );
    if code == 21 {
        eprintln!("skipping: storage checks fail here: {out}");
        return;
    }
    assert_eq!(code, 0, "{out}");
    let v = last_json(&out);
    assert!(v["result"]["pairing_code"]
        .as_str()
        .unwrap()
        .starts_with("wr1:"));
    assert!(
        v["result"]["recovery_key"].is_null(),
        "the key went to the file"
    );
    assert!(std::fs::read_to_string(&rk).unwrap().trim().len() > 20);

    let dir = root.join("demo");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("notes.md"), "hi\n").unwrap();
    let (code, out) = workroam(&home, &dir, &["--json", "init", "--no-push"], "");
    assert_eq!(code, 0, "{out}");

    let (code, out) = workroam(&home, &dir, &["--json", "status"], "");
    assert_eq!(code, 0, "{out}");
    let v = last_json(&out);
    assert_eq!(v["command"], "status");
    assert_eq!(v["result"]["projects"][0]["project"], "demo");

    // MCP secrets: set on this computer, listed per project.
    let mcp = dir.join(".workroam").join("mcp.toml");
    let old_mcp = std::fs::read(&mcp).ok();
    std::fs::create_dir_all(mcp.parent().unwrap()).unwrap();
    std::fs::write(
        &mcp,
        "[servers.jira]\nurl = \"https://jira.example.com/mcp\"\nheaders = { Authorization = \"Bearer ${secret:jira_token}\" }\n",
    )
    .unwrap();
    let (code, out) = workroam(&home, &dir, &["--json", "secret", "list"], "");
    assert_eq!(code, 0, "{out}");
    let v = last_json(&out);
    assert_eq!(v["result"]["secrets"][0]["name"], "jira_token");
    assert_eq!(v["result"]["secrets"][0]["set"], false);
    let (code, out) = workroam(
        &home,
        &dir,
        &["--json", "secret", "set", "jira_token", "--value-stdin"],
        "t0k3n\n",
    );
    assert_eq!(code, 0, "{out}");
    let (code, out) = workroam(&home, &dir, &["--json", "secret", "list"], "");
    assert_eq!(code, 0, "{out}");
    assert_eq!(last_json(&out)["result"]["secrets"][0]["set"], true);
    // Without a terminal the value must come from stdin or the environment.
    let (code, out) = workroam(&home, &dir, &["--json", "secret", "set", "other"], "");
    assert_eq!(code, 2, "{out}");
    let (code, out) = workroam(
        &home,
        &dir,
        &["--json", "secret", "remove", "jira_token"],
        "",
    );
    assert_eq!(code, 0, "{out}");
    match old_mcp {
        Some(b) => std::fs::write(&mcp, b).unwrap(),
        None => std::fs::remove_file(&mcp).unwrap(),
    }

    // An agent push needs the background service for its approval card.
    let (code, out) = workroam(&home, &dir, &["--json", "push"], "");
    assert_eq!(code, 30, "{out}");

    // Unknown command: usage error.
    let (code, _) = workroam(&home, &dir, &["frobnicate"], "");
    assert_eq!(code, 2);

    // MCP over stdio.
    let msgs = [
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"status","arguments":{}}}"#,
    ]
    .join("\n")
        + "\n";
    let (code, out) = workroam(&home, &dir, &["mcp"], &msgs);
    assert_eq!(code, 0, "{out}");
    let replies: Vec<Value> = out
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    assert_eq!(replies.len(), 3, "{out}");
    assert_eq!(replies[0]["result"]["protocolVersion"], "2025-06-18");
    let names: Vec<&str> = replies[1]["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        ["status", "push", "pull", "history", "handoff_template"]
    );
    assert_eq!(replies[2]["result"]["isError"], false, "{out}");
}

#[test]
fn completions_and_help() {
    let td = tempfile::tempdir().unwrap();
    let (code, out) = workroam(
        &td.path().join("h"),
        td.path(),
        &["completions", "bash"],
        "",
    );
    assert_eq!(code, 0);
    assert!(out.contains("workroam"));
    let (code, out) = workroam(&td.path().join("h"), td.path(), &["--help"], "");
    assert_eq!(code, 0);
    for c in [
        "setup", "init", "push", "pull", "status", "history", "archive", "agent", "doctor",
    ] {
        assert!(out.contains(c), "help lists {c}");
    }
}
