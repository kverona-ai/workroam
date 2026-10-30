//! Reach `workroamd`: the background service when it runs (starting it
//! when this computer is set up), otherwise an in-process daemon for this
//! one command.

use std::io::IsTerminal;
use std::time::Duration;
use workroam_core::Result;
use workroam_ipc::{Client, ClientKind};

pub struct Conn {
    pub client: Client,
    pub in_process: bool,
}

/// Agents run the CLI without a terminal (UX-06); they get approval cards.
pub fn agent_name() -> Option<String> {
    if let Ok(a) = std::env::var("WORKROAM_AGENT") {
        if !a.is_empty() {
            return Some(a);
        }
    }
    if std::io::stdout().is_terminal() && std::io::stdin().is_terminal() {
        return None;
    }
    let known = [
        ("CLAUDECODE", "claude-code"),
        ("CODEX_SANDBOX", "codex"),
        ("CODEX_HOME", "codex"),
        ("GROK_BUILD", "grok-build"),
        ("ANTIGRAVITY", "antigravity"),
        ("PI_AGENT", "pi"),
    ];
    for (var, name) in known {
        if std::env::var_os(var).is_some() {
            return Some(name.to_string());
        }
    }
    Some("agent".into())
}

fn daemon_binary() -> Option<std::path::PathBuf> {
    let name = if cfg!(windows) {
        "workroamd.exe"
    } else {
        "workroamd"
    };
    if let Ok(exe) = std::env::current_exe() {
        if let Some(d) = exe.parent() {
            let p = d.join(name);
            if p.exists() {
                return Some(p);
            }
        }
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|d| d.join(name))
            .find(|p| p.exists())
    })
}

fn spawn_daemon() -> bool {
    let Some(bin) = daemon_binary() else {
        return false;
    };
    let mut c = std::process::Command::new(bin);
    c.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW);
    }
    c.spawn().is_ok()
}

pub async fn connect(kind: ClientKind) -> Result<Conn> {
    let agent = agent_name();
    if std::env::var("WORKROAM_IN_PROCESS").as_deref() != Ok("1") {
        if let Ok(c) = Client::connect(kind, agent.clone()).await {
            return Ok(Conn {
                client: c,
                in_process: false,
            });
        }
        if workroamd::is_configured()
            && std::env::var("WORKROAM_NO_AUTOSTART").is_err()
            && spawn_daemon()
        {
            for _ in 0..30 {
                tokio::time::sleep(Duration::from_millis(100)).await;
                if let Ok(c) = Client::connect(kind, agent.clone()).await {
                    return Ok(Conn {
                        client: c,
                        in_process: false,
                    });
                }
            }
        }
    }
    Ok(Conn {
        client: workroamd::server::in_process(kind, agent).await?,
        in_process: true,
    })
}
