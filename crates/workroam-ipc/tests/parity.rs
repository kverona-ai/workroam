//! Architecture §17.4: the feature list, the IPC methods, the CLI and the
//! desktop app's actions agree, and the parity doc is generated from it.

use std::collections::{BTreeMap, BTreeSet};
use workroam_ipc::parity::{cli_args, Parity};

fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn parity_methods_are_covered() {
    let p = Parity::load().unwrap();
    let used: BTreeSet<&str> = p
        .features()
        .flat_map(|r| r.ipc.iter().map(String::as_str))
        .collect();
    for m in workroam_ipc::METHODS.iter().filter(|m| m.user_facing) {
        assert!(
            used.contains(m.name),
            "user-facing method {} is in no parity.toml feature",
            m.name
        );
    }
    for m in &used {
        assert!(
            workroam_ipc::is_known(m) || workroam_ipc::EVENTS.contains(m),
            "parity.toml names unknown IPC method or event {m}"
        );
    }
}

#[test]
fn parity_ids_are_unique_and_complete() {
    let p = Parity::load().unwrap();
    let mut seen = BTreeSet::new();
    let cli_exempt: Vec<String> = p.exempt("cli").map(|r| r.cells[0].clone()).collect();
    for r in p.features() {
        let id = r.id.as_deref().expect("every feature has an id");
        assert!(seen.insert(id), "duplicate feature id {id}");
        assert_eq!(r.since.as_deref(), Some("0.1"), "{id}: since");
        assert!(!r.cli.is_empty(), "{id}: no cli entry");
        if r.desktop.is_empty() {
            let why = r
                .desktop_exempt
                .as_deref()
                .unwrap_or_else(|| panic!("{id}: no desktop entry and no exemption"));
            assert!(
                cli_exempt.iter().any(|c| c.contains(why)),
                "{id}: {why} is not in the CLI-only exemption list"
            );
        }
    }
}

#[test]
fn parity_cli_entries_parse() {
    let p = Parity::load().unwrap();
    for r in p.features() {
        for ex in &r.cli {
            let args = cli_args(ex);
            assert_eq!(args[0], "workroam", "{ex}");
            if let Err(e) = workroam_cli::command().try_get_matches_from(&args) {
                use clap::error::ErrorKind;
                // `--version` ends parsing with "display version", which is a success.
                if e.kind() != ErrorKind::DisplayVersion {
                    panic!(
                        "{}: `{ex}` does not parse:\n{e}",
                        r.id.as_deref().unwrap_or("?")
                    );
                }
            }
        }
    }
}

#[test]
fn parity_desktop_actions_exist() {
    let p = Parity::load().unwrap();
    let path = root().join("apps/desktop/actions.json");
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let actions: BTreeMap<String, String> = v["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| {
            (
                a["id"].as_str().unwrap().to_string(),
                a["since"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect();
    for r in p.features() {
        for d in &r.desktop {
            let since = actions.get(d).unwrap_or_else(|| {
                panic!("desktop action {d} is not in apps/desktop/actions.json")
            });
            assert_eq!(
                Some(since.as_str()),
                r.since.as_deref(),
                "{d}: since differs between the CLI and the desktop app"
            );
        }
    }
}

/// Every action in actions.json is wired in the desktop UI: the id is passed
/// to `act("<id>", …)` (directly, or through a helper that forwards it), and
/// every `act` id in the UI is a listed action.
#[test]
fn parity_desktop_actions_are_wired() {
    let raw = std::fs::read_to_string(root().join("apps/desktop/actions.json")).unwrap();
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let listed: BTreeSet<String> = v["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["id"].as_str().unwrap().to_string())
        .collect();
    let mut js = String::new();
    let dir = root().join("apps/desktop/ui/js");
    for e in std::fs::read_dir(&dir).unwrap() {
        let p = e.unwrap().path();
        if p.extension().is_some_and(|x| x == "js") {
            js.push_str(&std::fs::read_to_string(&p).unwrap());
        }
    }
    for id in &listed {
        assert!(
            js.contains(&format!("\"{id}\"")),
            "desktop action {id} is not wired in apps/desktop/ui/js"
        );
    }
    for (i, _) in js.match_indices("act(\"") {
        let rest = &js[i + 5..];
        let id = &rest[..rest.find('"').unwrap()];
        assert!(
            listed.contains(id),
            "act(\"{id}\") in apps/desktop/ui/js is not in apps/desktop/actions.json"
        );
    }
}

#[test]
fn parity_doc_is_generated() {
    let p = Parity::load().unwrap();
    let doc = root().join("docs/cli-ui-parity.md");
    let want = p.render();
    if std::env::var_os("WORKROAM_BLESS").is_some() {
        std::fs::write(&doc, &want).unwrap();
    }
    let have = std::fs::read_to_string(&doc).unwrap().replace("\r\n", "\n");
    assert!(
        have == want,
        "docs/cli-ui-parity.md is out of date: run `cargo xtask parity-doc`"
    );
}
