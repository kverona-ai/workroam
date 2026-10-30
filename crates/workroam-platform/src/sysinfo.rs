//! Facts about this machine for the environment block and device records.

use workroam_core::model::OsKind;

/// Is a process with this id still running? Used to tell a sync that is
/// in progress from the marker a dead process left behind.
pub fn process_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        // SAFETY: plain Win32 calls; a null handle means no such process.
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return false;
            }
            CloseHandle(h);
            true
        }
    }
    #[cfg(unix)]
    {
        if pid > i32::MAX as u32 {
            return false;
        }
        // SAFETY: signal 0 sends nothing; it only reports whether the
        // process exists (EPERM: it exists but belongs to someone else).
        let r = unsafe { libc::kill(pid as libc::pid_t, 0) };
        r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(not(any(windows, unix)))]
    {
        true
    }
}

pub fn hostname() -> String {
    for var in ["COMPUTERNAME", "HOSTNAME"] {
        if let Ok(h) = std::env::var(var) {
            if !h.is_empty() {
                return h;
            }
        }
    }
    if let Ok(h) = std::fs::read_to_string("/etc/hostname") {
        let h = h.trim();
        if !h.is_empty() {
            return h.to_string();
        }
    }
    std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "workroam-device".into())
}

/// Device name derived from the hostname: lowercase, `-` separated.
pub fn default_device_name() -> String {
    let h = hostname();
    let h = h.split('.').next().unwrap_or(&h);
    let mut out = String::new();
    for c in h.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "device".into()
    } else {
        out
    }
}

pub fn shell() -> String {
    if cfg!(windows) {
        if std::env::var("PSModulePath")
            .map(|p| p.contains("PowerShell"))
            .unwrap_or(false)
        {
            return "PowerShell".into();
        }
        return "cmd".into();
    }
    std::env::var("SHELL")
        .ok()
        .and_then(|s| s.rsplit('/').next().map(str::to_string))
        .unwrap_or_else(|| "sh".into())
}

pub fn os_version() -> String {
    let os = OsKind::current();
    let detail = match os {
        OsKind::Macos => cmd_out("sw_vers", &["-productVersion"]),
        OsKind::Linux => std::fs::read_to_string("/etc/os-release")
            .ok()
            .and_then(|s| {
                s.lines().find(|l| l.starts_with("PRETTY_NAME=")).map(|l| {
                    l.trim_start_matches("PRETTY_NAME=")
                        .trim_matches('"')
                        .to_string()
                })
            }),
        OsKind::Windows => cmd_out("cmd", &["/C", "ver"]),
    };
    match detail {
        Some(d) if !d.is_empty() => format!("{} {} ({})", os.display(), d, std::env::consts::ARCH),
        _ => format!("{} ({})", os.display(), std::env::consts::ARCH),
    }
}

fn cmd_out(cmd: &str, args: &[&str]) -> Option<String> {
    std::process::Command::new(cmd)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
}

/// Can we open a TCP connection to `host` (port 443, then 22) quickly?
pub fn host_reachable(host: &str) -> bool {
    use std::net::ToSocketAddrs;
    let (h, ports): (&str, Vec<u16>) = match host.rsplit_once(':') {
        Some((h, p)) if p.parse::<u16>().is_ok() => (h, vec![p.parse().unwrap()]),
        _ => (host, vec![443, 22, 80]),
    };
    for port in ports {
        if let Ok(addrs) = (h, port).to_socket_addrs() {
            for a in addrs {
                if std::net::TcpStream::connect_timeout(&a, std::time::Duration::from_millis(1500))
                    .is_ok()
                {
                    return true;
                }
            }
        }
    }
    false
}
