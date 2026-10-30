//! Two devices hand a project back and forth through a shared local store.

use std::path::Path;
use std::process::Command;
use workroam_core::config::{BindingState, Capabilities, MachineConfig, WorkspaceLocation};
use workroam_core::Code;
use workroam_engine::init::{apply_init, bind, plan_init, InitChoices};
use workroam_engine::restore::{plan_restore, restore, RestoreTarget};
use workroam_engine::sync::{plan_pull, pull, push, PullOptions, PushOptions};
use workroam_engine::{history, Project, Workspace};

fn git(dir: &Path, args: &[&str]) -> String {
    let o = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@x")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@x")
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

fn machine(name: &str, root: &Path, store: &Path) -> MachineConfig {
    let loc = WorkspaceLocation {
        endpoint: format!("file://{}", store.display()),
        bucket: "bucket".into(),
        workspace_id: "ws_test".into(),
        capabilities: Capabilities {
            conditional_create: true,
            conditional_overwrite: true,
            multipart: true,
        },
        preset: "local".into(),
        ..Default::default()
    };
    let mut m = MachineConfig::new(name, &root.display().to_string(), loc);
    m.reachable = vec![];
    m
}

#[tokio::test(flavor = "multi_thread")]
async fn office_home_office() {
    let td = tempfile::tempdir().unwrap();
    std::env::set_var("WORKROAM_HOME", td.path().join("home"));
    std::env::set_var("WORKROAM_SECRETS", "file");
    let store = td.path().join("store");
    let office_dir = td.path().join("office/proj-a");
    let home_dir = td.path().join("home-mac/code/proj-a");

    // Office: a project with docs and an internal repository.
    std::fs::create_dir_all(office_dir.join("docs")).unwrap();
    std::fs::write(office_dir.join("docs/plan.md"), "line 1\nline 2\nline 3\n").unwrap();
    std::fs::write(office_dir.join("CLAUDE.md"), "Prefer small commits.\n").unwrap();
    let repo = office_dir.join("app");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(
        &repo,
        &[
            "remote",
            "add",
            "origin",
            "https://git.corp.internal/team/app.git",
        ],
    );
    std::fs::write(repo.join("main.rs"), "fn main() {}\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "first"]);
    std::fs::write(repo.join("wip.rs"), "// not committed\n").unwrap();

    let mut office_m = machine("office", &td.path().join("office"), &store);
    let office = Workspace::create(
        office_m.clone(),
        None,
        "pw-correct-horse",
        "RECOVERY-KEY",
        None,
    )
    .await
    .unwrap();
    let plan = plan_init(&office, "proj-a", &office_dir).await.unwrap();
    assert_eq!(plan.repos.len(), 1);
    assert_eq!(
        plan.repos[0].recommended.mode,
        workroam_core::model::RepoMode::Bundle
    );
    let out = apply_init(
        &office,
        &plan,
        &InitChoices {
            agents: Some(vec!["claude".into(), "codex".into()]),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(out.imported.iter().any(|i| i.contains("CLAUDE.md")));
    bind(&mut office_m, "proj-a", &office_dir, BindingState::Bound);
    let office = Workspace {
        machine: office_m.clone(),
        ..office
    };
    let p = Project::open(&office, "proj-a").unwrap();
    let r = push(
        &p,
        &PushOptions {
            release: true,
            note: "first".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let first = r.snapshot.clone().unwrap();
    assert!(r.released);
    assert!(!r.skipped);
    // Nothing changed: no new snapshot, even a second later (git commit
    // dates must not leak into the comparison).
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    let again = push(
        &p,
        &PushOptions {
            release: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(again.skipped);

    // Home: join with the password and pull into an empty directory.
    let mut home_m = machine("home", &td.path().join("home-mac/code"), &store);
    bind(&mut home_m, "proj-a", &home_dir, BindingState::Pending);
    let home = Workspace::open(home_m.clone(), None, "pw-correct-horse")
        .await
        .unwrap();
    assert_eq!(home.projects().await.unwrap().len(), 1);
    let mut hp = Project::open(&home, "proj-a").unwrap();
    let plan = plan_pull(&hp, None).await.unwrap();
    assert!(plan.summary.added > 3, "{:?}", plan.summary);
    let res = pull(&mut hp, &PullOptions::default()).await.unwrap();
    assert_eq!(res.snapshot.as_deref(), Some(first.as_str()));
    assert_eq!(
        std::fs::read_to_string(home_dir.join("docs/plan.md")).unwrap(),
        "line 1\nline 2\nline 3\n"
    );
    assert_eq!(
        std::fs::read_to_string(home_dir.join("app/wip.rs")).unwrap(),
        "// not committed\n"
    );
    assert_eq!(
        git(&home_dir.join("app"), &["log", "--format=%s", "-1"]),
        "first"
    );
    assert!(std::fs::read_to_string(home_dir.join("AGENTS.md"))
        .unwrap()
        .contains("home"));
    assert_eq!(
        std::fs::read_to_string(home_dir.join("CLAUDE.md")).unwrap(),
        "@AGENTS.md\n"
    );
    assert!(home_dir.join(".mcp.json").exists());

    // Office still believes it holds nothing; home works and pushes.
    std::fs::write(
        home_dir.join("docs/plan.md"),
        "line 1 (home)\nline 2\nline 3\n",
    )
    .unwrap();
    git(&home_dir.join("app"), &["add", "."]);
    git(&home_dir.join("app"), &["commit", "-q", "-m", "home work"]);
    let hr = push(
        &hp,
        &PushOptions {
            release: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(hr.snapshot.is_some() && hr.snapshot.as_deref() != Some(first.as_str()));

    // Office edited another line meanwhile: pushing is refused until it pulls.
    std::fs::write(
        office_dir.join("docs/plan.md"),
        "line 1\nline 2\nline 3 (office)\n",
    )
    .unwrap();
    let err = push(&p, &PushOptions::default()).await.unwrap_err();
    assert_eq!(err.code, Code::Conflict);
    let mut p = Project::open(&office, "proj-a").unwrap();
    let res = pull(&mut p, &PullOptions::default()).await.unwrap();
    assert_eq!(res.merged, vec!["docs/plan.md".to_string()]);
    assert_eq!(
        std::fs::read_to_string(office_dir.join("docs/plan.md")).unwrap(),
        "line 1 (home)\nline 2\nline 3 (office)\n"
    );
    assert_eq!(
        git(&repo, &["log", "--format=%s", "-1", "main"]),
        "home work"
    );
    let r2 = push(&p, &PushOptions::default()).await.unwrap();
    assert!(!r2.skipped);

    // History and restore into another directory.
    let hist = history::list(&p, None).unwrap();
    assert_eq!(hist.len(), 3);
    assert_eq!(hist[2].note, "first");
    let copy = td.path().join("copy");
    let rp = plan_restore(&p, &first[..8], vec![], RestoreTarget::Dir(copy.clone()))
        .await
        .unwrap();
    restore(&mut p, &rp).await.unwrap();
    assert_eq!(
        std::fs::read_to_string(copy.join("docs/plan.md")).unwrap(),
        "line 1\nline 2\nline 3\n"
    );
    assert_eq!(
        git(&copy.join("app"), &["log", "--format=%s", "-1"]),
        "first"
    );

    // Restore in place: the result is a new snapshot recording where it came from.
    let rp = plan_restore(&p, &first, vec!["docs".into()], RestoreTarget::InPlace)
        .await
        .unwrap();
    let ro = restore(&mut p, &rp).await.unwrap();
    assert_eq!(
        std::fs::read_to_string(office_dir.join("docs/plan.md")).unwrap(),
        "line 1\nline 2\nline 3\n"
    );
    let hist = history::list(&p, None).unwrap();
    assert_eq!(hist[0].restored_from.as_deref(), Some(first.as_str()));
    assert_eq!(ro.snapshot.as_deref(), Some(hist[0].id.as_str()));

    // Office still holds the lease: home must take over to pull, then home's lease blocks office.
    let mut hp = Project::open(&home, "proj-a").unwrap();
    assert_eq!(
        pull(&mut hp, &PullOptions::default())
            .await
            .unwrap_err()
            .code,
        Code::LeaseHeld
    );
    let res = pull(
        &mut hp,
        &PullOptions {
            takeover: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        res.took_over_from.map(|d| d.name).as_deref(),
        Some("office")
    );
    assert_eq!(
        std::fs::read_to_string(home_dir.join("docs/plan.md")).unwrap(),
        "line 1\nline 2\nline 3\n"
    );
    let err = pull(&mut p, &PullOptions::default()).await.unwrap_err();
    assert_eq!(err.code, Code::LeaseHeld);
    pull(
        &mut p,
        &PullOptions {
            takeover: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
}
