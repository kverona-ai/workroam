//! Find repositories in a project and recommend a sync mode (§11.6, PROJ-02).

use crate::cmd::{remote_host, Git};
use serde::{Deserialize, Serialize};
use std::path::Path;
use workroam_core::config::RepoConfig;
use workroam_core::model::RepoMode;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FoundRepo {
    pub name: String,
    pub path: String,
    pub origin: Option<String>,
    pub host: Option<String>,
    pub branch: Option<String>,
    pub commits: u64,
    pub recommended: RepoConfig,
    pub reason: String,
    /// Can this device push to the origin?
    pub can_push_here: bool,
}

const SKIP: &[&str] = &[
    "node_modules",
    "target",
    ".venv",
    "dist",
    ".workroam",
    ".agents",
    ".handoff",
    ".git",
    "vendor",
    "__pycache__",
];

/// Directories under `project` that contain a `.git` (max depth 4).
pub fn find_repos(project: &Path) -> Vec<String> {
    let mut out = Vec::new();
    walk(project, project, 0, &mut out);
    out.sort();
    out
}

fn walk(root: &Path, dir: &Path, depth: usize, out: &mut Vec<String>) {
    if depth > 4 {
        return;
    }
    if depth > 0 && dir.join(".git").exists() {
        if let Some(rel) = workroam_core::paths::to_snapshot_path(root, dir) {
            out.push(rel);
        }
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if SKIP.contains(&name.as_str()) || name.starts_with('.') && depth == 0 && name != ".github"
        {
            continue;
        }
        if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            walk(root, &e.path(), depth + 1, out);
        }
    }
}

/// Hosts that look private / intranet only.
pub fn looks_internal(host: &str) -> bool {
    let h = host.to_lowercase();
    if h.ends_with(".internal")
        || h.ends_with(".local")
        || h.ends_with(".corp")
        || h.ends_with(".lan")
        || h.ends_with(".intranet")
        || h.ends_with(".home.arpa")
        || !h.contains('.')
    {
        return true;
    }
    if let Ok(ip) = h.parse::<std::net::Ipv4Addr>() {
        return ip.is_private() || ip.is_loopback() || ip.is_link_local();
    }
    false
}

/// Recommend a mode (architecture §11.6). `reachable_everywhere(host)` says
/// whether every device of the workspace can reach the host.
pub fn recommend(
    project: &Path,
    rel: &str,
    reachable_here: &dyn Fn(&str) -> bool,
    reachable_everywhere: &dyn Fn(&str) -> bool,
) -> FoundRepo {
    let dir = workroam_core::paths::from_snapshot_path(project, rel);
    let git = Git::new(&dir);
    let origin = git.remote_url("origin");
    let host = origin.as_deref().and_then(remote_host);
    let branch = git.current_branch();
    let commits = git.count(&["--all"]);
    let name = rel.rsplit('/').next().unwrap_or(rel).to_string();
    let (mode, history, reason) = match &host {
        None => (RepoMode::Files, String::new(), "没有远端，按文件同步 / no remote: files mode".to_string()),
        Some(h) if !looks_internal(h) && reachable_everywhere(h) => (RepoMode::Remote, String::new(), format!("两地都能连接 {h}，经 origin 同步 / {h} is reachable from every device: remote mode")),
        Some(h) if looks_internal(h) || !reachable_everywhere(h) => (RepoMode::Bundle, "recent:50".to_string(), format!("{h} 是内网仓库，打包最近 50 次提交 / {h} is internal: bundle with the last 50 commits")),
        Some(h) => (RepoMode::Bundle, "full".to_string(), format!("{h}：完整历史打包 / full history bundle")),
    };
    let can_push_here = host.as_deref().map(reachable_here).unwrap_or(false);
    let recommended = RepoConfig {
        path: rel.to_string(),
        mode,
        history,
        branches: branch.clone().map(|b| vec![b]).unwrap_or_default(),
        via: if mode == RepoMode::Remote {
            Some("origin".into())
        } else {
            None
        },
        ..Default::default()
    };
    FoundRepo {
        name,
        path: rel.to_string(),
        origin,
        host,
        branch,
        commits,
        recommended,
        reason,
        can_push_here,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_hosts() {
        assert!(looks_internal("git.corp.internal"));
        assert!(looks_internal("10.0.0.5"));
        assert!(looks_internal("gitlab"));
        assert!(!looks_internal("github.com"));
    }

    #[test]
    fn finds_nested_repos() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("repo-a/.git")).unwrap();
        std::fs::create_dir_all(d.path().join("libs/repo-b/.git")).unwrap();
        std::fs::create_dir_all(d.path().join("node_modules/x/.git")).unwrap();
        assert_eq!(find_repos(d.path()), vec!["libs/repo-b", "repo-a"]);
    }
}
