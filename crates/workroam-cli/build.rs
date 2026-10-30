// Development builds show `-dev+<short commit>` in the banner and --version.
fn main() {
    println!("cargo:rerun-if-env-changed=WORKROAM_RELEASE");
    let release = std::env::var("PROFILE")
        .map(|p| p == "release")
        .unwrap_or(false)
        || std::env::var("WORKROAM_RELEASE").is_ok();
    let suffix = if release {
        String::new()
    } else {
        std::process::Command::new("git")
            .args(["rev-parse", "--short", "HEAD"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| format!("-dev+{}", String::from_utf8_lossy(&o.stdout).trim()))
            .unwrap_or_else(|| "-dev".into())
    };
    println!("cargo:rustc-env=WORKROAM_VERSION_SUFFIX={suffix}");
}
