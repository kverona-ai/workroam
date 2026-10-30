//! A pull that died half way is finished by the next pull (architecture
//! §9.3): completed steps are not repeated, and pushing waits until then.

use std::path::Path;
use workroam_core::config::{BindingState, Capabilities, MachineConfig, WorkspaceLocation};
use workroam_core::Code;
use workroam_engine::init::{apply_init, bind, plan_init, InitChoices};
use workroam_engine::local::{clear_stale_syncing, OpEntry, OpLog, SyncState, SYNCING};
use workroam_engine::sync::{interrupted_pull, plan_push, pull, push, PullOptions, PushOptions};
use workroam_engine::{Project, Workspace};

fn machine(name: &str, root: &Path, store: &Path) -> MachineConfig {
    let loc = WorkspaceLocation {
        endpoint: format!("file://{}", store.display()),
        bucket: "bucket".into(),
        workspace_id: "ws_resume".into(),
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
async fn interrupted_pull_is_finished_by_the_next_pull() {
    let td = tempfile::tempdir().unwrap();
    std::env::set_var("WORKROAM_HOME", td.path().join("home"));
    std::env::set_var("WORKROAM_SECRETS", "file");
    let store = td.path().join("store");
    let office_dir = td.path().join("office/notes");
    let home_dir = td.path().join("home-mac/notes");

    std::fs::create_dir_all(office_dir.join("docs")).unwrap();
    for i in 0..5 {
        std::fs::write(
            office_dir.join(format!("docs/f{i}.md")),
            format!("v1 {i}\n"),
        )
        .unwrap();
    }
    std::fs::write(office_dir.join("shared.md"), "a\nb\nc\n").unwrap();

    let mut office_m = machine("office", &td.path().join("office"), &store);
    let office = Workspace::create(office_m.clone(), None, "pw", "RECOVERY", None)
        .await
        .unwrap();
    let plan = plan_init(&office, "notes", &office_dir).await.unwrap();
    apply_init(&office, &plan, &InitChoices::default()).unwrap();
    bind(&mut office_m, "notes", &office_dir, BindingState::Bound);
    let office = Workspace {
        machine: office_m.clone(),
        ..office
    };
    let op = Project::open(&office, "notes").unwrap();
    let first = push(
        &op,
        &PushOptions {
            release: true,
            ..Default::default()
        },
    )
    .await
    .unwrap()
    .snapshot
    .unwrap();

    // Home pulls the first version, then edits shared.md locally.
    let mut home_m = machine("home", &td.path().join("home-mac"), &store);
    bind(&mut home_m, "notes", &home_dir, BindingState::Pending);
    let home = Workspace::open(home_m.clone(), None, "pw").await.unwrap();
    let mut hp = Project::open(&home, "notes").unwrap();
    assert_eq!(
        pull(&mut hp, &PullOptions::default())
            .await
            .unwrap()
            .snapshot
            .as_deref(),
        Some(first.as_str())
    );
    std::fs::write(home_dir.join("shared.md"), "a (home)\nb\nc\n").unwrap();
    push(
        &hp,
        &PushOptions {
            release: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();

    // Office changes every file and the same line of shared.md, then pushes.
    let mut op = Project::open(&office, "notes").unwrap();
    pull(
        &mut op,
        &PullOptions {
            takeover: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    // Office keeps its own line: the pull above merged home's edit in, so
    // overwrite the merged result to force a conflict later.
    std::fs::write(office_dir.join("shared.md"), "a (office)\nb\nc\n").unwrap();
    for i in 0..5 {
        std::fs::write(
            office_dir.join(format!("docs/f{i}.md")),
            format!("v2 {i}\n"),
        )
        .unwrap();
    }
    let second = push(
        &op,
        &PushOptions {
            release: true,
            ..Default::default()
        },
    )
    .await
    .unwrap()
    .snapshot
    .unwrap();

    // Home edits the same line again so the pull produces a conflict copy.
    std::fs::write(home_dir.join("shared.md"), "a (home again)\nb\nc\n").unwrap();
    let before = std::fs::read(SyncState::path(&home_dir)).unwrap();
    let mut hp = Project::open(&home, "notes").unwrap();
    let res = pull(
        &mut hp,
        &PullOptions {
            takeover: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(res.conflicts.len(), 1, "{res:?}");
    let conflict_copy = res.conflicts[0].clone();
    assert!(!res.resumed);
    let copies = || {
        std::fs::read_dir(&home_dir)
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .contains(".conflict-")
            })
            .count()
    };
    assert_eq!(copies(), 1);

    // Rewind to "the process died after docs/f0.md and the conflict copy":
    // old sync state, an unfinished operation log, a leftover SYNCING marker.
    std::fs::write(SyncState::path(&home_dir), &before).unwrap();
    let log = OpLog::new(&home_dir);
    log.begin(&second, "pull").unwrap();
    log.done("docs/f0.md").unwrap();
    log.done("shared.md").unwrap();
    std::fs::write(home_dir.join(SYNCING), "pull 2000-01-01T00:00:00+00:00\n").unwrap();
    std::fs::write(home_dir.join("docs/f3.md"), "v1 3\n").unwrap(); // "not yet written"

    // Pushing refuses until the pull is finished.
    let hp = Project::open(&home, "notes").unwrap();
    assert_eq!(interrupted_pull(&hp).as_deref(), Some(second.as_str()));
    let plan = plan_push(&hp).await.unwrap();
    assert_eq!(plan.interrupted_pull.as_deref(), Some(second.as_str()));
    let err = push(&hp, &PushOptions::default()).await.unwrap_err();
    assert_eq!(err.code, Code::Conflict, "{err:?}");
    assert!(err.message.contains("interrupted"), "{err:?}");

    // A marker written by a process that is still running stays; one with
    // no process (or, in the old format, older than an hour) is stale.
    std::fs::write(
        home_dir.join(SYNCING),
        format!(
            "pull {} pid={}\n",
            chrono::Utc::now().to_rfc3339(),
            std::process::id()
        ),
    )
    .unwrap();
    assert!(!clear_stale_syncing(&home_dir));
    assert!(home_dir.join(SYNCING).exists());
    std::fs::write(
        home_dir.join(SYNCING),
        format!("pull {} pid=4294967295\n", chrono::Utc::now().to_rfc3339()),
    )
    .unwrap();
    assert!(clear_stale_syncing(&home_dir));
    std::fs::write(home_dir.join(SYNCING), "pull 2000-01-01T00:00:00+00:00\n").unwrap();
    assert!(clear_stale_syncing(&home_dir));
    assert!(!home_dir.join(SYNCING).exists());

    // The next pull resumes: files are completed, the conflict copy is not duplicated.
    let mut hp = Project::open(&home, "notes").unwrap();
    let res = pull(&mut hp, &PullOptions::default()).await.unwrap();
    assert!(res.resumed, "{res:?}");
    assert_eq!(res.snapshot.as_deref(), Some(second.as_str()));
    assert_eq!(res.conflicts, Vec::<String>::new(), "{res:?}");
    assert_eq!(copies(), 1);
    assert!(home_dir.join(&conflict_copy).exists());
    for i in 0..5 {
        assert_eq!(
            std::fs::read_to_string(home_dir.join(format!("docs/f{i}.md"))).unwrap(),
            format!("v2 {i}\n")
        );
    }
    assert!(log.read().is_empty());
    assert!(log.pending().is_none());
    assert!(!home_dir.join(SYNCING).exists());
    assert!(interrupted_pull(&hp).is_none());
    assert_eq!(
        SyncState::load(&home_dir).base_snapshot.as_deref(),
        Some(second.as_str())
    );

    // A log whose state update already ran is just cleared.
    log.begin(&second, "pull").unwrap();
    log.append(&OpEntry::Done { path: "x".into() }).unwrap();
    assert!(interrupted_pull(&hp).is_none());
    assert!(log.read().is_empty());
    let plan = plan_push(&hp).await.unwrap();
    assert!(plan.interrupted_pull.is_none());
}
