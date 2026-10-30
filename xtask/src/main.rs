//! Repository tasks: `cargo xtask <task>`.
//!
//! - `parity-doc`: regenerate docs/cli-ui-parity.md from crates/workroam-ipc/parity.toml.
//! - `web`: rebuild apps/web/workroam_web.wasm (the web reader's decryption core).

fn main() {
    let task = std::env::args().nth(1).unwrap_or_default();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the repository");
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
        "web" => {
            let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
            let ok = std::process::Command::new(cargo)
                .current_dir(root)
                .args(["build", "--release", "--locked", "-p", "workroam-web"])
                .args(["--lib", "--target", "wasm32-unknown-unknown"])
                .status()
                .expect("run cargo")
                .success();
            if !ok {
                std::process::exit(1);
            }
            let wasm = root.join("target/wasm32-unknown-unknown/release/workroam_web.wasm");
            let to = root.join("apps/web/workroam_web.wasm");
            std::fs::copy(&wasm, &to).expect("copy the wasm");
            println!("wrote {}", to.display());
        }
        _ => {
            eprintln!("usage: cargo xtask parity-doc | web");
            std::process::exit(2);
        }
    }
}
