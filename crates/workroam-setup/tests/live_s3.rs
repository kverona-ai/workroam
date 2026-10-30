//! Setup against a real S3-compatible server (RustFS / MinIO): new
//! workspace with a scoped key, then join. Runs only with
//! `WORKROAM_TEST_S3=<endpoint>,<access key>,<secret key>`.

use workroam_core::model::Access;
use workroam_setup::checks::CheckStatus;
use workroam_setup::session::{BucketChoice, KeyMode, MachineInput, SetupMode, SetupSession};

#[tokio::test(flavor = "multi_thread")]
async fn new_workspace_with_scoped_key() {
    let Ok(spec) = std::env::var("WORKROAM_TEST_S3") else {
        return;
    };
    let mut it = spec.split(',');
    let (ep, ak, sk) = (it.next().unwrap(), it.next().unwrap(), it.next().unwrap());
    let td = tempfile::tempdir().unwrap();
    std::env::set_var("WORKROAM_HOME", td.path().join("h1"));
    std::env::set_var("WORKROAM_SECRETS", "file");
    let mut s = SetupSession::new(SetupMode::New);
    s.set_machine(MachineInput {
        device_name: "Office PC".into(),
        root: td.path().join("p").display().to_string(),
        autostart: Some(false),
        ..Default::default()
    })
    .unwrap();
    let info = s.set_endpoint("rustfs", ep, None, None).unwrap();
    assert!(info.can_create_key);
    s.set_credentials(ak, sk).await.unwrap();
    s.set_key_mode(KeyMode::Create, Access::ReadWrite, None, None)
        .unwrap();
    let b = s.set_bucket(BucketChoice::Auto, None, "").await.unwrap();
    assert!(b.name.starts_with("workroam-office-pc-"), "{}", b.name);
    let results = s
        .run_checks(&mut |r| eprintln!("{:?} {} {}", r.status, r.id, r.message))
        .await
        .unwrap();
    assert!(!results
        .iter()
        .any(|r| r.status == CheckStatus::Fail && r.group == "storage"));
    s.checks.retain(|c| c.group == "storage");
    let pw = s.set_password("correct horse battery staple").unwrap();
    let _ = pw;
    s.mark_recovery_saved();
    let out = s.commit().await.unwrap();
    assert!(out.created_bucket && out.created_key);
    let code = out.pairing_code.unwrap();
    assert!(code.starts_with("wr1:"));

    // The pairing code carries the scoped key, not the administrator's.
    std::env::set_var("WORKROAM_HOME", td.path().join("h2"));
    let mut j = SetupSession::new(SetupMode::Join);
    j.set_pairing_code(&code).unwrap();
    let info = j
        .unlock(Some("correct horse battery staple"), None)
        .await
        .unwrap();
    assert_eq!(info.devices, vec!["Office PC".to_string()]);
    assert_ne!(j.creds.as_ref().unwrap().access_key, ak);
    j.set_machine(MachineInput {
        device_name: "Home Mac".into(),
        root: td.path().join("q").display().to_string(),
        autostart: Some(false),
        ..Default::default()
    })
    .unwrap();
    j.commit().await.unwrap();
}
