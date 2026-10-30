//! Well-known paths and small filesystem helpers.

use crate::Result;
use std::path::{Component, Path, PathBuf};

pub fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

/// `~/.workroam` (override with `WORKROAM_HOME`, used by tests and portable installs).
pub fn workroam_home() -> PathBuf {
    if let Ok(h) = std::env::var("WORKROAM_HOME") {
        if !h.is_empty() {
            return PathBuf::from(h);
        }
    }
    home_dir().join(".workroam")
}

pub fn machine_toml() -> PathBuf {
    workroam_home().join("machine.toml")
}

pub fn logs_dir() -> PathBuf {
    workroam_home().join("logs")
}

pub fn state_dir() -> PathBuf {
    workroam_home().join("state")
}

pub fn user_adapters_dir() -> PathBuf {
    workroam_home().join("adapters")
}

/// IPC endpoint (architecture §15). On Windows this is a pipe name.
pub fn socket_path() -> PathBuf {
    if let Ok(p) = std::env::var("WORKROAM_SOCKET") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    if std::env::var("WORKROAM_HOME")
        .map(|h| !h.is_empty())
        .unwrap_or(false)
        && !cfg!(windows)
    {
        return workroam_home().join("workroamd.sock");
    }
    #[cfg(target_os = "macos")]
    {
        home_dir().join("Library/Application Support/Workroam/workroamd.sock")
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        match std::env::var("XDG_RUNTIME_DIR") {
            Ok(d) if !d.is_empty() => PathBuf::from(d).join("workroam/workroamd.sock"),
            _ => workroam_home().join("workroamd.sock"),
        }
    }
    #[cfg(windows)]
    {
        let user = std::env::var("USERNAME").unwrap_or_else(|_| "user".into());
        let suffix = if let Ok(h) = std::env::var("WORKROAM_HOME") {
            format!(
                "-{}",
                crate::crypto::sha256_hex(h.as_bytes())[..8].to_string()
            )
        } else {
            String::new()
        };
        PathBuf::from(format!(r"\\.\pipe\workroam-{user}{suffix}"))
    }
}

pub fn expand_tilde(p: &str) -> PathBuf {
    if p == "~" {
        return home_dir();
    }
    if let Some(rest) = p.strip_prefix("~/").or_else(|| p.strip_prefix("~\\")) {
        return home_dir().join(rest);
    }
    PathBuf::from(p)
}

/// Lexically normalise a path (no filesystem access): removes `.` and `..`.
pub fn normalize(p: &Path) -> PathBuf {
    let p = if p.is_relative() {
        std::env::current_dir()
            .map(|c| c.join(p))
            .unwrap_or_else(|_| p.to_path_buf())
    } else {
        p.to_path_buf()
    };
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// The real path when it exists (symlinks, Windows 8.3 short names and
/// letter case resolved), without the `\\?\` prefix; else [`normalize`].
pub fn canonical(p: &Path) -> PathBuf {
    match std::fs::canonicalize(p) {
        Ok(c) => strip_verbatim(c),
        Err(_) => normalize(p),
    }
}

fn strip_verbatim(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        return PathBuf::from(rest);
    }
    p
}

/// Whether `dir` is `root` or inside it, comparing real paths (and ignoring
/// letter case on Windows and macOS, whose file systems usually do too).
pub fn is_within(dir: &Path, root: &Path) -> bool {
    let (d, r) = (canonical(dir), canonical(root));
    if cfg!(any(windows, target_os = "macos")) {
        let lower = |p: &Path| PathBuf::from(p.to_string_lossy().to_lowercase());
        lower(&d).starts_with(lower(&r))
    } else {
        d.starts_with(&r)
    }
}

/// Write a file atomically: temp file in the same directory, then rename.
pub fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(
        ".{}.wr-tmp-{}",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("file"),
        std::process::id()
    ));
    std::fs::write(&tmp, data)?;
    let mut last = None;
    for attempt in 0..10 {
        match std::fs::rename(&tmp, path) {
            Ok(()) => return Ok(()),
            Err(e) => {
                last = Some(e);
                std::thread::sleep(std::time::Duration::from_millis(50 * (attempt + 1)));
            }
        }
    }
    let _ = std::fs::remove_file(&tmp);
    Err(last.unwrap().into())
}

/// Convert an OS path relative to `base` into a snapshot path (`/` separated).
pub fn to_snapshot_path(base: &Path, p: &Path) -> Option<String> {
    let rel = p.strip_prefix(base).ok()?;
    let parts: Vec<String> = rel
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().to_string()),
            _ => None,
        })
        .collect();
    Some(parts.join("/"))
}

/// Convert a snapshot path to an OS path under `base`.
pub fn from_snapshot_path(base: &Path, p: &str) -> PathBuf {
    let mut out = base.to_path_buf();
    for part in p.split('/') {
        if !part.is_empty() && part != "." && part != ".." {
            out.push(part);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn within_compares_real_paths() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("proj");
        std::fs::create_dir_all(root.join("src")).unwrap();
        assert!(is_within(&root.join("src"), &root));
        assert!(is_within(&root.join("src/./../src"), &root));
        assert!(is_within(&canonical(&root.join("src")), &root));
        assert!(!is_within(t.path(), &root));
        assert!(!is_within(&t.path().join("proj2"), &root));
        assert_eq!(
            strip_verbatim(PathBuf::from(r"\\?\C:\x")),
            PathBuf::from(r"C:\x")
        );
    }

    #[test]
    fn snapshot_paths() {
        let base = Path::new("/a/b");
        assert_eq!(
            to_snapshot_path(base, Path::new("/a/b/c/d.txt")).unwrap(),
            "c/d.txt"
        );
        assert_eq!(
            from_snapshot_path(base, "c/d.txt"),
            Path::new("/a/b/c/d.txt")
        );
        assert_eq!(
            from_snapshot_path(base, "../../etc/passwd"),
            Path::new("/a/b/etc/passwd")
        );
    }

    #[test]
    fn atomic_write() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("x/y.txt");
        write_atomic(&p, b"one").unwrap();
        write_atomic(&p, b"two").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"two");
    }
}
