//! Live test against an S3-compatible server with a MinIO-compatible admin API.
//! Runs only when `WORKROAM_TEST_S3=<endpoint>,<access key>,<secret key>` is set,
//! e.g. a local RustFS container.

use workroam_core::config::{Addressing, WorkspaceLocation};
use workroam_core::model::Access;
use workroam_device::provision::{AdminCreds, KeyProvisioner};
use workroam_store::{Creds, S3Client};

#[tokio::test]
async fn provision_scoped_key() {
    let Ok(spec) = std::env::var("WORKROAM_TEST_S3") else {
        return;
    };
    let mut it = spec.split(',');
    let (ep, ak, sk) = (it.next().unwrap(), it.next().unwrap(), it.next().unwrap());
    let admin = Creds {
        access_key: ak.into(),
        secret_key: sk.into(),
    };
    let s3 = S3Client::new(ep, "us-east-1", Addressing::Path, admin.clone()).unwrap();
    let bucket = format!(
        "wr-test-{}",
        hex::encode(workroam_core::crypto::random_bytes::<3>())
    );
    assert!(s3.create_bucket(&bucket).await.unwrap().ok());
    let other = format!("{bucket}-other");
    assert!(s3.create_bucket(&other).await.unwrap().ok());
    let loc = WorkspaceLocation {
        endpoint: ep.into(),
        bucket: bucket.clone(),
        prefix: "ws".into(),
        addressing: Addressing::Path,
        region: "us-east-1".into(),
        workspace_id: "ws_test".into(),
        ..Default::default()
    };
    let ac = AdminCreds {
        access_key: ak.into(),
        secret_key: sk.into(),
        ..Default::default()
    };
    let key = KeyProvisioner::MinioAdmin
        .create_key(&loc, &ac, Access::ReadWrite)
        .await
        .expect("create key");
    let scoped = S3Client::new(
        ep,
        "us-east-1",
        Addressing::Path,
        Creds {
            access_key: key.access_key.clone(),
            secret_key: key.secret_key.clone(),
        },
    )
    .unwrap();
    let ok = scoped
        .put_object(&bucket, "ws/probe", b"x".to_vec(), &[])
        .await
        .unwrap();
    assert!(ok.ok(), "write inside prefix: {}", ok.text());
    let outside = scoped
        .put_object(&other, "x", b"x".to_vec(), &[])
        .await
        .unwrap();
    assert!(!outside.ok(), "write outside the workspace must fail");
    KeyProvisioner::MinioAdmin
        .revoke_key(&loc, &ac, &key)
        .await
        .expect("revoke");
}
