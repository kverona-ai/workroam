//! Project directory checks (PROJ-06, architecture §6.7).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DirIssue {
    /// `not_writable`, `nested`, `contains_project`, `forbidden`, `cloud_sync`, `long_path`, `not_dir`
    pub kind: String,
    pub blocking: bool,
    pub message: String,
}

pub fn is_empty_or_missing(dir: &Path) -> bool {
    match std::fs::read_dir(dir) {
        Ok(mut it) => it.next().is_none(),
        Err(_) => !dir.exists(),
    }
}

fn forbidden_dirs() -> Vec<PathBuf> {
    let mut v = vec![workroam_core::paths::home_dir()];
    if cfg!(windows) {
        for var in [
            "SystemRoot",
            "ProgramFiles",
            "ProgramFiles(x86)",
            "ProgramData",
        ] {
            if let Ok(p) = std::env::var(var) {
                v.push(PathBuf::from(p));
            }
        }
    } else {
        for p in [
            "/",
            "/bin",
            "/boot",
            "/dev",
            "/etc",
            "/lib",
            "/proc",
            "/sbin",
            "/sys",
            "/usr",
            "/var",
            "/System",
            "/Library",
            "/Applications",
            "/private",
        ] {
            v.push(PathBuf::from(p));
        }
    }
    v
}

/// Check a directory chosen for a project. `others` are the other project
/// directories bound on this device.
pub fn check_project_dir(dir: &Path, others: &[PathBuf]) -> Vec<DirIssue> {
    let mut issues = Vec::new();
    let dir = workroam_core::paths::normalize(dir);
    if dir.parent().is_none() || dir.components().count() <= 1 {
        issues.push(DirIssue {
            kind: "forbidden".into(),
            blocking: true,
            message: "不能使用磁盘根目录 / cannot use a drive root".into(),
        });
    }
    for f in forbidden_dirs() {
        if workroam_core::paths::normalize(&f) == dir {
            issues.push(DirIssue {
                kind: "forbidden".into(),
                blocking: true,
                message: format!(
                    "不能使用用户主目录或系统目录 / cannot use the home or a system directory: {}",
                    dir.display()
                ),
            });
        }
    }
    if dir.exists() && !dir.is_dir() {
        issues.push(DirIssue {
            kind: "not_dir".into(),
            blocking: true,
            message: format!("{} 不是目录 / is not a directory", dir.display()),
        });
    }
    for o in others {
        let o = workroam_core::paths::normalize(o);
        if o == dir {
            continue;
        }
        if dir.starts_with(&o) {
            issues.push(DirIssue {
                kind: "nested".into(),
                blocking: true,
                message: format!(
                    "位于另一个项目目录 {} 之内 / inside another project",
                    o.display()
                ),
            });
        } else if o.starts_with(&dir) {
            issues.push(DirIssue {
                kind: "contains_project".into(),
                blocking: true,
                message: format!(
                    "包含另一个项目目录 {} / contains another project",
                    o.display()
                ),
            });
        }
    }
    // Writable: the dir or its nearest existing ancestor.
    let mut probe = dir.clone();
    while !probe.exists() {
        if !probe.pop() {
            break;
        }
    }
    let test = probe.join(format!(".workroam-write-test-{}", std::process::id()));
    match std::fs::write(&test, b"") {
        Ok(()) => {
            let _ = std::fs::remove_file(&test);
        }
        Err(_) => issues.push(DirIssue {
            kind: "not_writable".into(),
            blocking: true,
            message: format!("目录不可写 / not writable: {}", probe.display()),
        }),
    }
    let s = dir.to_string_lossy().to_lowercase();
    for marker in [
        "onedrive",
        "icloud",
        "mobile documents",
        "dropbox",
        "google drive",
        "googledrive",
        "坚果云",
        "baidunetdisk",
    ] {
        if s.contains(marker) {
            issues.push(DirIssue {
                kind: "cloud_sync".into(),
                blocking: false,
                message: "位于网盘同步文件夹内，两套同步可能互相冲突 / inside a cloud-sync folder; two sync tools may conflict".into(),
            });
            break;
        }
    }
    if cfg!(windows) && dir.to_string_lossy().len() > 160 && !long_paths_enabled() {
        issues.push(DirIssue {
            kind: "long_path".into(),
            blocking: false,
            message: "路径较长，建议开启 LongPathsEnabled 并设置 core.longpaths=true / long path: enable LongPathsEnabled".into(),
        });
    }
    issues
}

/// Windows `LongPathsEnabled` (always true elsewhere).
pub fn long_paths_enabled() -> bool {
    #[cfg(windows)]
    {
        let out = std::process::Command::new("reg")
            .args([
                "query",
                r"HKLM\SYSTEM\CurrentControlSet\Control\FileSystem",
                "/v",
                "LongPathsEnabled",
            ])
            .output();
        match out {
            Ok(o) => String::from_utf8_lossy(&o.stdout).contains("0x1"),
            Err(_) => false,
        }
    }
    #[cfg(not(windows))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_projects() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a");
        let issues = check_project_dir(&a.join("b"), &[a.clone()]);
        assert!(issues.iter().any(|i| i.kind == "nested" && i.blocking));
        let issues = check_project_dir(&a, &[a.join("b")]);
        assert!(issues.iter().any(|i| i.kind == "contains_project"));
        assert!(check_project_dir(&d.path().join("ok"), &[])
            .iter()
            .all(|i| !i.blocking));
    }

    #[test]
    fn home_is_forbidden() {
        let issues = check_project_dir(&workroam_core::paths::home_dir(), &[]);
        assert!(issues.iter().any(|i| i.kind == "forbidden"));
    }
}
