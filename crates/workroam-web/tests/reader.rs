//! The reader decrypts what rustic and workroam-core write.

use workroam_snapshot::rustic::{local_backends, RusticEngine};
use workroam_snapshot::{EntryKind, EntrySource, SnapshotEngine, SourceEntry};
use workroam_web::Reader;

fn files(dir: &std::path::Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(files(&p));
        } else {
            out.push((
                p.file_name().unwrap().to_string_lossy().into_owned(),
                std::fs::read(&p).unwrap(),
            ));
        }
    }
    out
}

#[test]
fn unlock_list_read_and_control() {
    let d = tempfile::tempdir().unwrap();
    std::env::set_var("WORKROAM_HOME", d.path().join("home"));
    let repo = d.path().join("repo");
    let eng = RusticEngine::init(
        local_backends(&repo).unwrap(),
        "pw-123456789",
        "ABCDE-FGHJK-MNPQR-STVWX-YZ012-34567-89ABC-DEFGH",
    )
    .unwrap();
    let big: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    let entries = vec![
        SourceEntry::bytes("HANDOFF.md", b"# HANDOFF\n\nnext: ship it\n".to_vec()),
        SourceEntry {
            path: "src/big.bin".into(),
            kind: EntryKind::File {
                source: EntrySource::Bytes(big.clone()),
                size: big.len() as u64,
                exec: false,
            },
            mtime: None,
        },
    ];
    let tags = vec!["workroam".to_string(), "workroam:project=abc".to_string()];
    let id = eng
        .backup(entries, &tags, None, "office", "abc", "", None)
        .unwrap();

    let mut r = Reader::new();
    let keys = files(&repo.join("keys"));
    assert_eq!(keys.len(), 2, "password and recovery key");
    assert!(keys.iter().all(|(_, k)| r.unlock(k, "wrong").is_err()));
    assert!(
        keys.iter().any(|(_, k)| r
            .unlock(k, "abcdefghjkmnpqrstvwxyz0123456789abcdefgh")
            .is_err()),
        "normalize first"
    );
    let rk = Reader::normalize_recovery("abcde fghjk mnpqr stvwx yz012 34567 89abc defgh");
    assert!(
        keys.iter().any(|(_, k)| r.unlock(k, &rk).is_ok()),
        "recovery key unlocks"
    );
    r.lock();
    assert!(keys
        .iter()
        .any(|(_, k)| r.unlock(k, "pw-123456789").is_ok()));

    // Snapshots and indexes.
    let snaps = files(&repo.join("snapshots"));
    assert_eq!(snaps.len(), 1);
    let snap: serde_json::Value = serde_json::from_slice(&r.file(&snaps[0].1).unwrap()).unwrap();
    assert!(id.starts_with(&snaps[0].0[..8]) || snaps[0].0.starts_with(&id[..8]));
    assert!(snap["tags"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t == "workroam:project=abc"));
    for (_, ix) in files(&repo.join("index")) {
        let plain = r.file(&ix).unwrap();
        assert!(r.add_index(&plain).unwrap() > 0);
    }
    let packs: std::collections::HashMap<String, Vec<u8>> =
        files(&repo.join("data")).into_iter().collect();
    let read_blob = |r: &Reader, id: &str| {
        let (pack, loc) = r.locate(id).unwrap();
        let data = &packs[&pack][loc.offset as usize..(loc.offset + loc.length) as usize];
        r.blob(id, data).unwrap()
    };
    // Walk the tree down to the files.
    let mut tree: serde_json::Value =
        serde_json::from_slice(&read_blob(&r, snap["tree"].as_str().unwrap())).unwrap();
    let mut found = std::collections::HashMap::new();
    let mut stack = vec![tree.take()];
    while let Some(t) = stack.pop() {
        for n in t["nodes"].as_array().unwrap() {
            match n["type"].as_str().unwrap() {
                "dir" => stack.push(
                    serde_json::from_slice(&read_blob(&r, n["subtree"].as_str().unwrap())).unwrap(),
                ),
                "file" => {
                    let mut body = Vec::new();
                    for b in n["content"].as_array().into_iter().flatten() {
                        body.extend(read_blob(&r, b.as_str().unwrap()));
                    }
                    found.insert(n["name"].as_str().unwrap().to_string(), body);
                }
                _ => {}
            }
        }
    }
    assert_eq!(found["HANDOFF.md"], b"# HANDOFF\n\nnext: ship it\n");
    assert_eq!(found["big.bin"], big);

    // Control objects and project ids match workroam-core.
    let ck = workroam_core::crypto::derive_key(&eng.master_key_material(), "control");
    let sealed =
        workroam_core::crypto::seal(&ck, b"control/projects/p1/head", br#"{"project":"demo"}"#);
    assert_eq!(
        r.control("projects/p1/head", &sealed).unwrap(),
        br#"{"project":"demo"}"#
    );
    assert!(
        r.control("projects/p2/head", &sealed).is_err(),
        "bound to its key"
    );
    assert_eq!(
        r.project_id("demo").unwrap(),
        workroam_core::crypto::project_id(&ck, "demo")
    );
}

#[test]
fn pairing_code_opens() {
    let sealed =
        workroam_core::crypto::password_seal_raw("pw-123456789", br#"{"endpoint":"https://s3"}"#);
    let code = format!(
        "wr1:{}",
        data_encoding::BASE32_NOPAD.encode(&sealed).to_lowercase()
    );
    assert_eq!(
        Reader::open_pairing(&code, "pw-123456789").unwrap(),
        br#"{"endpoint":"https://s3"}"#
    );
    assert!(Reader::open_pairing(&code, "nope").is_err());
}
