//! Autostart for `workroamd` as a per-user service (architecture §3.1):
//! launchd on macOS, a logon task on Windows, a systemd user unit on Linux.

use std::path::{Path, PathBuf};
use workroam_core::{Error, Result};

pub const LABEL: &str = "dev.workroam.workroamd";

pub fn daemon_binary() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("workroamd"));
    let name = if cfg!(windows) {
        "workroamd.exe"
    } else {
        "workroamd"
    };
    if exe.file_name().map(|n| n == name).unwrap_or(false) {
        return exe;
    }
    exe.parent()
        .map(|d| d.join(name))
        .filter(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from(name))
}

pub fn install(binary: &Path) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let dir = workroam_core::paths::home_dir().join("Library/LaunchAgents");
        std::fs::create_dir_all(&dir)?;
        let plist = dir.join(format!("{LABEL}.plist"));
        let body = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{LABEL}</string>
  <key>ProgramArguments</key><array><string>{}</string></array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>ProcessType</key><string>Background</string>
</dict>
</plist>
"#,
            binary.display()
        );
        std::fs::write(&plist, body)?;
        let _ = std::process::Command::new("launchctl")
            .arg("unload")
            .arg(&plist)
            .output();
        run("launchctl", &["load", "-w", &plist.to_string_lossy()])
    }
    #[cfg(windows)]
    {
        run(
            "schtasks",
            &[
                "/Create",
                "/F",
                "/SC",
                "ONLOGON",
                "/RL",
                "LIMITED",
                "/TN",
                "Workroam\\workroamd",
                "/TR",
                &format!("\"{}\"", binary.display()),
            ],
        )
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let dir = dirs::config_dir()
            .unwrap_or_else(|| workroam_core::paths::home_dir().join(".config"))
            .join("systemd/user");
        std::fs::create_dir_all(&dir)?;
        let unit = dir.join("workroamd.service");
        let body = format!(
            "[Unit]\nDescription=Workroam background service\nAfter=network-online.target\n\n[Service]\nExecStart={}\nRestart=on-failure\nRestartSec=5\n\n[Install]\nWantedBy=default.target\n",
            binary.display()
        );
        std::fs::write(&unit, body)?;
        let _ = run("systemctl", &["--user", "daemon-reload"]);
        run(
            "systemctl",
            &["--user", "enable", "--now", "workroamd.service"],
        )
    }
}

pub fn uninstall() -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let plist =
            workroam_core::paths::home_dir().join(format!("Library/LaunchAgents/{LABEL}.plist"));
        let _ = std::process::Command::new("launchctl")
            .arg("unload")
            .arg(&plist)
            .output();
        let _ = std::fs::remove_file(plist);
        Ok(())
    }
    #[cfg(windows)]
    {
        let _ = run("schtasks", &["/Delete", "/F", "/TN", "Workroam\\workroamd"]);
        Ok(())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = run(
            "systemctl",
            &["--user", "disable", "--now", "workroamd.service"],
        );
        let unit = dirs::config_dir()
            .unwrap_or_else(|| workroam_core::paths::home_dir().join(".config"))
            .join("systemd/user/workroamd.service");
        let _ = std::fs::remove_file(unit);
        Ok(())
    }
}

pub fn is_installed() -> bool {
    #[cfg(target_os = "macos")]
    {
        workroam_core::paths::home_dir()
            .join(format!("Library/LaunchAgents/{LABEL}.plist"))
            .exists()
    }
    #[cfg(windows)]
    {
        std::process::Command::new("schtasks")
            .args(["/Query", "/TN", "Workroam\\workroamd"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        dirs::config_dir()
            .unwrap_or_else(|| workroam_core::paths::home_dir().join(".config"))
            .join("systemd/user/workroamd.service")
            .exists()
    }
}

#[allow(dead_code)]
fn run(cmd: &str, args: &[&str]) -> Result<()> {
    let out = std::process::Command::new(cmd)
        .args(args)
        .output()
        .map_err(|e| Error::internal(format!("{cmd}: {e}")))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(Error::internal(format!(
            "{cmd} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )))
    }
}
