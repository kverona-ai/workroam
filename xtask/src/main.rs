//! Repository tasks: `cargo xtask <task>`.
//!
//! - `parity-doc`: regenerate docs/cli-ui-parity.md from crates/workroam-ipc/parity.toml.

fn main() {
    let task = std::env::args().nth(1).unwrap_or_default();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    match task.as_str() {
        "parity-doc" => {
            let raw = std::fs::read_to_string(root.join("crates/workroam-ipc/parity.toml"))
                .expect("read parity.toml");
            let p = workroam_ipc::parity::Parity::parse(&raw).unwrap_or_else(|e| {
                eprintln!("parity.toml: {e}");
                std::process::exit(1)
            });
            let doc = root.join("docs/cli-ui-parity.md");
            std::fs::write(&doc, p.render()).expect("write the doc");
            println!("wrote {}", doc.display());
        }
        _ => {
            eprintln!("usage: cargo xtask parity-doc");
            std::process::exit(2);
        }
    }
}
