//! Archive a project, restore it in each scenario, export and import it.

use std::path::Path;
use std::process::Command;
use workroam_archive::{archive, package, verify, Prefer, Scenario};
use workroam_core::config::{BindingState, Capabilities, MachineConfig, WorkspaceLocation};
use workroam_core::model::HeadState;
use workroam_core::Code;
use workroam_engine::init::{apply_init, bind, plan_init, InitChoices};
use workroam_engine::sync::{pull, push, PullOptions, PushOptions};
use workroam_engine::{Project, Workspace};

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
    assert!(o.status.success(), "git {args:?}");
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

fn machine(name: &str, root: &Path, store: &Path, ws: &str) -> MachineConfig {
    let loc = WorkspaceLocation {
        endpoint: format!("file://{}", store.display()),
        bucket: "bucket".into(),
        workspace_id: ws.into(),
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
async fn archive_restore_export_import() {
    let td = tempfile::tempdir().unwrap();
    std::env::set_var("WORKROAM_HOME", td.path().join("home"));
    std::env::set_var("WORKROAM_SECRETS", "file");
    let store = td.path().join("store");
    let dir = td.path().join("code/done");
    std::fs::create_dir_all(dir.join("notes")).unwrap();
    std::fs::write(dir.join("notes/a.md"), "one\ntwo\nthree\n").unwrap();
    let repo = dir.join("app");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("lib.rs"), "pub fn f() {}\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "first"]);

    let mut m = machine("office", &td.path().join("code"), &store, "ws_a");
    let ws = Workspace::create(m.clone(), None, "pw-archive", "RECOVERY", None)
        .await
        .unwrap();
    let plan = plan_init(&ws, "done", &dir).await.unwrap();
    apply_init(&ws, &plan, &InitChoices::default()).unwrap();
    bind(&mut m, "done", &dir, BindingState::Bound);
    let ws = Workspace {
        machine: m.clone(),
        ..ws
    };
    let p = Project::open(&ws, "done").unwrap();
    push(&p, &PushOptions::default()).await.unwrap();

    // Unsynced change is pushed first, then archived.
    std::fs::write(dir.join("notes/b.md"), "late\n").unwrap();
    let opts = archive::ArchiveOptions {
        note: "shipped".into(),
        ..Default::default()
    };
    let pl = archive::plan(&p, &opts).await.unwrap();
    assert_eq!(pl.unsynced, 1);
    assert!(pl.blocker.is_none());
    let a = archive::create(&p, &opts).await.unwrap();
    assert!(a.pushed.is_some());
    assert_eq!(a.repos, vec!["app".to_string()]);
    let (h, _) = ws.head("done").await.unwrap().unwrap();
    assert_eq!(h.state, HeadState::Archived);
    let e = push(&p, &PushOptions::default()).await.unwrap_err();
    assert_eq!(e.code, Code::Archived);
    let listed = archive::list(&ws, Some("SHIP")).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert!(archive::list(&ws, Some("nothing"))
        .await
        .unwrap()
        .is_empty());
    assert!(verify::verify(&ws, None).await.unwrap().ok);

    // ② unchanged copy.
    let rp = archive::restore_plan(&ws, "done", None).await.unwrap();
    assert_eq!(rp.scenario, Scenario::InSync);

    // ③ local edit on top of the archive.
    std::fs::write(dir.join("notes/a.md"), "one\ntwo\nthree\nfour\n").unwrap();
    let rp = archive::restore_plan(&ws, "done", None).await.unwrap();
    assert_eq!(rp.scenario, Scenario::LocalNewer);

    // ⑥ a directory holding something else.
    let other = td.path().join("other");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(other.join("x.txt"), "x").unwrap();
    let rp6 = archive::restore_plan(&ws, "done", Some(other.clone()))
        .await
        .unwrap();
    assert_eq!(rp6.scenario, Scenario::Occupied);
    let e = archive::restore(&ws, &rp6, &Prefer::Merge, false, None)
        .await
        .unwrap_err();
    assert_eq!(e.code.exit_code(), 2);

    // Extract to another folder: still archived.
    let ex = td.path().join("peek");
    archive::extract(&ws, "done", ex.clone()).await.unwrap();
    assert_eq!(
        std::fs::read_to_string(ex.join("notes/b.md")).unwrap(),
        "late\n"
    );
    assert!(ex.join("app/.git").exists());
    assert!(archive::list(&ws, None).await.unwrap().len() == 1);

    // Export, then restore ① into a fresh folder of a second machine root.
    let file = td.path().join("done.wrarchive");
    package::export(&ws, "done", &file, "export-pw")
        .await
        .unwrap();
    assert!(file.metadata().unwrap().len() > 100);

    // Restore ③ in place: local change survives and is pushed.
    let rp = archive::restore_plan(&ws, "done", None).await.unwrap();
    let r = archive::restore(&ws, &rp, &Prefer::Local, false, None)
        .await
        .unwrap();
    assert!(r.restored);
    assert!(r.snapshot.is_some());
    let (h, _) = ws.head("done").await.unwrap().unwrap();
    assert_eq!(h.state, HeadState::Active);
    assert!(archive::list(&ws, None).await.unwrap().is_empty());
    assert_eq!(
        std::fs::read_to_string(dir.join("notes/a.md")).unwrap(),
        "one\ntwo\nthree\nfour\n"
    );

    // Archive again, change locally, and let the archive win (⑤ via ③).
    let p = Project::open(&ws, "done").unwrap();
    archive::create(&p, &archive::ArchiveOptions::default())
        .await
        .unwrap();
    std::fs::write(dir.join("notes/a.md"), "mine\n").unwrap();
    let rp = archive::restore_plan(&ws, "done", None).await.unwrap();
    let r = archive::restore(&ws, &rp, &Prefer::Archive, false, None)
        .await
        .unwrap();
    let moved = std::path::PathBuf::from(r.moved_to.unwrap());
    assert_eq!(
        std::fs::read_to_string(moved.join("notes/a.md")).unwrap(),
        "mine\n"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("notes/a.md")).unwrap(),
        "one\ntwo\nthree\nfour\n"
    );

    // Import the exported file into another workspace under a new name.
    let store2 = td.path().join("store2");
    let m2 = machine("home", &td.path().join("home2"), &store2, "ws_b");
    let ws2 = Workspace::create(m2.clone(), None, "pw-other", "RECOVERY2", None)
        .await
        .unwrap();
    let e = package::import(&ws2, &file, "wrong", None)
        .await
        .unwrap_err();
    assert_eq!(e.code, Code::Denied);
    let imp = package::import(&ws2, &file, "export-pw", Some("done-copy"))
        .await
        .unwrap();
    assert_eq!(imp.project, "done-copy");
    let rp = archive::restore_plan(&ws2, "done-copy", None)
        .await
        .unwrap();
    assert_eq!(rp.scenario, Scenario::Fresh);
    let r = archive::restore(&ws2, &rp, &Prefer::Merge, false, None)
        .await
        .unwrap();
    assert!(r.restored);
    let fresh = std::path::PathBuf::from(&rp.dir);
    assert_eq!(
        std::fs::read_to_string(fresh.join("notes/b.md")).unwrap(),
        "late\n"
    );
    assert!(fresh.join("app/lib.rs").exists());
    assert!(verify::verify(&ws2, None).await.unwrap().ok);

    // Pulling the restored project works like any other.
    let mut p2 = Project::open(&ws2, "done-copy").unwrap();
    let out = pull(&mut p2, &PullOptions::default()).await.unwrap();
    assert!(out.up_to_date);

    // Delete an archive for good.
    let p = Project::open(&ws, "done").unwrap();
    archive::create(&p, &archive::ArchiveOptions::default())
        .await
        .unwrap();
    let d = archive::delete(&ws, "done").await.unwrap();
    assert!(d.forgotten >= 3);
    assert!(ws.head("done").await.unwrap().is_none());
}
