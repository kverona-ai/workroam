//! `--tunnel`: an outbound cloudflared tunnel for a public HTTPS address
//! without an inbound port (architecture §18.2). cloudflared is downloaded
//! into the state directory when it is missing. A quick tunnel gets a new
//! address after each restart; `CLOUDFLARED_TOKEN` runs a named tunnel
//! (give its fixed address with `--public-url`).

use crate::app::Shared;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};

fn exe_name() -> &'static str {
    if cfg!(windows) {
        "cloudflared.exe"
    } else {
        "cloudflared"
    }
}

fn on_path() -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .map(|d| d.join(exe_name()))
        .find(|p| p.is_file())
}

/// The release asset for this platform.
fn asset() -> Option<&'static str> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "cloudflared-linux-amd64",
        ("linux", "aarch64") => "cloudflared-linux-arm64",
        ("linux", "arm") => "cloudflared-linux-arm",
        ("macos", "x86_64") => "cloudflared-darwin-amd64.tgz",
        ("macos", "aarch64") => "cloudflared-darwin-arm64.tgz",
        ("windows", "x86_64") => "cloudflared-windows-amd64.exe",
        _ => return None,
    })
}

async fn download(dir: &Path) -> Result<PathBuf, String> {
    let name =
        asset().ok_or("no cloudflared build for this platform; install it and put it on PATH")?;
    let url = format!("https://github.com/cloudflare/cloudflared/releases/latest/download/{name}");
    eprintln!("downloading cloudflared from {url}");
    let bytes = reqwest::Client::builder()
        .timeout(Duration::from_secs(300))
        .build()
        .map_err(|e| e.to_string())?
        .get(&url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("cannot download cloudflared: {e}"))?
        .bytes()
        .await
        .map_err(|e| e.to_string())?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let target = dir.join(exe_name());
    if name.ends_with(".tgz") {
        let tgz = dir.join(name);
        std::fs::write(&tgz, &bytes).map_err(|e| e.to_string())?;
        let ok = std::process::Command::new("tar")
            .arg("-xzf")
            .arg(&tgz)
            .arg("-C")
            .arg(dir)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        let _ = std::fs::remove_file(&tgz);
        if !ok {
            return Err("cannot unpack cloudflared".into());
        }
    } else {
        std::fs::write(&target, &bytes).map_err(|e| e.to_string())?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| e.to_string())?;
    }
    Ok(target)
}

pub async fn binary(state_dir: &Path) -> Result<PathBuf, String> {
    if let Some(p) = on_path() {
        return Ok(p);
    }
    let local = state_dir.join("bin").join(exe_name());
    if local.is_file() {
        return Ok(local);
    }
    download(&state_dir.join("bin")).await
}

/// `https://<words>.trycloudflare.com` in a cloudflared log line.
pub fn find_url(line: &str) -> Option<String> {
    let i = line.find("https://")?;
    let rest = &line[i..];
    let end = rest
        .find(|c: char| c.is_whitespace() || c == '|' || c == '"')
        .unwrap_or(rest.len());
    let u = &rest[..end];
    u.ends_with(".trycloudflare.com").then(|| u.to_string())
}

/// Keep the tunnel running; set the public URL when it is known.
pub fn spawn(app: Shared, state_dir: PathBuf, port: u16) {
    tokio::spawn(async move {
        let mut backoff = 2u64;
        loop {
            let bin = match binary(&state_dir).await {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("tunnel: {e}");
                    tokio::time::sleep(Duration::from_secs(60)).await;
                    continue;
                }
            };
            let mut cmd = tokio::process::Command::new(&bin);
            cmd.arg("tunnel").arg("--no-autoupdate");
            if let Ok(tok) = std::env::var("CLOUDFLARED_TOKEN") {
                cmd.arg("run").arg("--token").arg(tok);
            } else {
                cmd.arg("--url").arg(format!("http://127.0.0.1:{port}"));
            }
            cmd.stdout(Stdio::null())
                .stderr(Stdio::piped())
                .kill_on_drop(true);
            let mut child = match cmd.spawn() {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("tunnel: cannot start {}: {e}", bin.display());
                    tokio::time::sleep(Duration::from_secs(backoff)).await;
                    backoff = (backoff * 2).min(120);
                    continue;
                }
            };
            if let Some(err) = child.stderr.take() {
                let app = app.clone();
                tokio::spawn(async move {
                    let mut lines = BufReader::new(err).lines();
                    while let Ok(Some(l)) = lines.next_line().await {
                        tracing::debug!("cloudflared: {l}");
                        if let Some(u) = find_url(&l) {
                            let old = app.cfg.public_url.lock().unwrap().replace(u.clone());
                            if old.as_deref() != Some(&u) {
                                println!("public address: {u}");
                                println!("MCP connector:  {u}/mcp");
                            }
                        }
                    }
                });
            }
            let status = child.wait().await;
            eprintln!("tunnel: cloudflared exited ({status:?}); restarting");
            tokio::time::sleep(Duration::from_secs(backoff)).await;
            backoff = (backoff * 2).min(120);
        }
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn finds_quick_tunnel_url() {
        let l = "2026-10-30T08:00:00Z INF |  https://calm-river-1234.trycloudflare.com                    |";
        assert_eq!(
            super::find_url(l).as_deref(),
            Some("https://calm-river-1234.trycloudflare.com")
        );
        assert_eq!(
            super::find_url("INF Requesting new quick Tunnel on trycloudflare.com..."),
            None
        );
        assert_eq!(
            super::find_url("see https://developers.cloudflare.com/x"),
            None
        );
    }
}
