//! Thin wrapper around the `git` command line.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use workroam_core::{Code, Error, Result};

#[derive(Debug, Clone)]
pub struct Git {
    pub dir: PathBuf,
}

pub fn git_cmd() -> Command {
    let mut c = Command::new("git");
    c.env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null());
    c
}

/// `git --version` as (major, minor, patch).
pub fn version() -> Option<(u32, u32, u32)> {
    let out = git_cmd().arg("--version").output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    let v = s.split_whitespace().nth(2)?;
    let mut it = v.split('.').map(|p| {
        p.chars()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse::<u32>()
            .unwrap_or(0)
    });
    Some((it.next()?, it.next().unwrap_or(0), it.next().unwrap_or(0)))
}

pub fn is_repo(dir: &Path) -> bool {
    dir.join(".git").exists()
}

impl Git {
    pub fn new(dir: impl Into<PathBuf>) -> Git {
        Git { dir: dir.into() }
    }

    fn command(&self) -> Command {
        let mut c = git_cmd();
        c.current_dir(&self.dir);
        c
    }

    pub fn run(&self, args: &[&str]) -> Result<String> {
        self.run_env(args, &[])
    }

    pub fn run_env(&self, args: &[&str], env: &[(&str, &str)]) -> Result<String> {
        let mut c = self.command();
        c.args(args);
        for (k, v) in env {
            c.env(k, v);
        }
        let out = c.output().map_err(|e| {
            Error::internal(format!("cannot run git: {e}")).with_hint("install git 2.30 or newer")
        })?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
        } else {
            Err(Error::internal(format!(
                "git {} failed: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            )))
        }
    }

    pub fn ok(&self, args: &[&str]) -> bool {
        self.command()
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    pub fn git_dir(&self) -> PathBuf {
        match self.run(&["rev-parse", "--absolute-git-dir"]) {
            Ok(p) => PathBuf::from(p),
            Err(_) => self.dir.join(".git"),
        }
    }

    /// rebase / merge / cherry-pick in progress (PUSH-01).
    pub fn in_progress(&self) -> Option<&'static str> {
        let g = self.git_dir();
        if g.join("rebase-merge").exists() || g.join("rebase-apply").exists() {
            Some("rebase")
        } else if g.join("MERGE_HEAD").exists() {
            Some("merge")
        } else if g.join("CHERRY_PICK_HEAD").exists() {
            Some("cherry-pick")
        } else if g.join("REVERT_HEAD").exists() {
            Some("revert")
        } else {
            None
        }
    }

    pub fn ensure_not_in_progress(&self, name: &str) -> Result<()> {
        if let Some(what) = self.in_progress() {
            return Err(Error::new(
                Code::GitInProgress,
                format!("repository {name} is in the middle of a {what}"),
            )
            .with_hint("finish or abort it, then sync again"));
        }
        Ok(())
    }

    pub fn current_branch(&self) -> Option<String> {
        self.run(&["symbolic-ref", "--quiet", "--short", "HEAD"])
            .ok()
            .filter(|s| !s.is_empty())
    }

    pub fn head(&self) -> Option<String> {
        self.rev_parse("HEAD")
    }

    pub fn rev_parse(&self, rev: &str) -> Option<String> {
        self.run(&[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{rev}^{{commit}}"),
        ])
        .ok()
        .filter(|s| !s.is_empty())
    }

    pub fn tree_of(&self, rev: &str) -> Option<String> {
        self.run(&[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{rev}^{{tree}}"),
        ])
        .ok()
        .filter(|s| !s.is_empty())
    }

    pub fn exists(&self, obj: &str) -> bool {
        self.ok(&["cat-file", "-e", &format!("{obj}^{{commit}}")])
    }

    /// Local branches and their tips.
    pub fn branches(&self) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        if let Ok(s) = self.run(&[
            "for-each-ref",
            "--format=%(refname:short) %(objectname)",
            "refs/heads/",
        ]) {
            for line in s.lines() {
                if let Some((b, c)) = line.split_once(' ') {
                    out.insert(b.to_string(), c.to_string());
                }
            }
        }
        out
    }

    pub fn refs_under(&self, prefix: &str) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        if let Ok(s) = self.run(&["for-each-ref", "--format=%(refname) %(objectname)", prefix]) {
            for line in s.lines() {
                if let Some((r, c)) = line.split_once(' ') {
                    out.insert(r.to_string(), c.to_string());
                }
            }
        }
        out
    }

    pub fn remote_url(&self, remote: &str) -> Option<String> {
        self.run(&["remote", "get-url", remote])
            .ok()
            .filter(|s| !s.is_empty())
    }

    pub fn config_get(&self, key: &str) -> Option<String> {
        self.run(&["config", "--get", key])
            .ok()
            .filter(|s| !s.is_empty())
    }

    pub fn is_ancestor(&self, a: &str, b: &str) -> bool {
        self.ok(&["merge-base", "--is-ancestor", a, b])
    }

    pub fn count(&self, range: &[&str]) -> u64 {
        let mut args = vec!["rev-list", "--count"];
        args.extend_from_slice(range);
        self.run(&args)
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0)
    }

    /// Working tree differs from HEAD (tracked changes or untracked, non-ignored files).
    pub fn is_dirty(&self) -> bool {
        self.run(&["status", "--porcelain", "--untracked-files=normal"])
            .map(|s| !s.is_empty())
            .unwrap_or(false)
    }

    /// `git ls-files -co --exclude-standard` (files mode, GIT-07).
    pub fn ls_files(&self) -> Result<Vec<String>> {
        let s = self.run(&["ls-files", "-co", "--exclude-standard", "-z"])?;
        Ok(s.split('\0')
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .collect())
    }

    /// Files ignored by git (for `extras`).
    pub fn ignored_files(&self) -> Result<Vec<String>> {
        let s = self.run(&["ls-files", "-oi", "--exclude-standard", "--directory", "-z"])?;
        Ok(s.split('\0')
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .collect())
    }

    /// Commit message + author/committer of a commit, for exact rebuilds.
    pub fn commit_info(&self, c: &str) -> Result<CommitInfo> {
        let s = self.run(&[
            "show",
            "-s",
            "--format=%T%x00%an%x00%ae%x00%aI%x00%cn%x00%ce%x00%cI%x00%B",
            c,
        ])?;
        let parts: Vec<&str> = s.splitn(8, '\0').collect();
        if parts.len() < 8 {
            return Err(Error::internal(format!("cannot read commit {c}")));
        }
        Ok(CommitInfo {
            tree: parts[0].into(),
            author_name: parts[1].into(),
            author_email: parts[2].into(),
            author_date: parts[3].into(),
            committer_name: parts[4].into(),
            committer_email: parts[5].into(),
            committer_date: parts[6].into(),
            message: parts[7].to_string(),
        })
    }

    /// Recreate a commit with the same tree, message and identities on new parents.
    pub fn recommit(&self, info: &CommitInfo, parents: &[String]) -> Result<String> {
        let mut args: Vec<String> = vec!["commit-tree".into(), info.tree.clone()];
        for p in parents {
            args.push("-p".into());
            args.push(p.clone());
        }
        let msg = if info.message.is_empty() {
            "(no message)".to_string()
        } else {
            info.message.clone()
        };
        args.push("-m".into());
        args.push(msg);
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        self.run_env(
            &refs,
            &[
                ("GIT_AUTHOR_NAME", &info.author_name),
                ("GIT_AUTHOR_EMAIL", &info.author_email),
                ("GIT_AUTHOR_DATE", &info.author_date),
                ("GIT_COMMITTER_NAME", &info.committer_name),
                ("GIT_COMMITTER_EMAIL", &info.committer_email),
                ("GIT_COMMITTER_DATE", &info.committer_date),
            ],
        )
    }

    /// First-parent chain ending at `tip`, oldest first, at most `max` commits.
    pub fn first_parent_chain(&self, tip: &str, max: usize) -> Result<Vec<String>> {
        let max = max.max(1).to_string();
        let s = self.run(&["rev-list", "--first-parent", "--max-count", &max, tip])?;
        let mut v: Vec<String> = s.lines().map(str::to_string).collect();
        v.reverse();
        Ok(v)
    }

    pub fn update_ref(&self, name: &str, new: &str) -> Result<()> {
        self.run(&["update-ref", name, new]).map(|_| ())
    }

    pub fn delete_ref(&self, name: &str) {
        let _ = self.run(&["update-ref", "-d", name]);
    }
}

#[derive(Debug, Clone)]
pub struct CommitInfo {
    pub tree: String,
    pub author_name: String,
    pub author_email: String,
    pub author_date: String,
    pub committer_name: String,
    pub committer_email: String,
    pub committer_date: String,
    pub message: String,
}

/// Host of a git remote URL (`git@host:path`, `ssh://`, `https://`).
pub fn remote_host(url: &str) -> Option<String> {
    if let Ok(u) = url::Url::parse(url) {
        if let Some(h) = u.host_str() {
            return Some(h.to_string());
        }
    }
    // scp-like: user@host:path
    let rest = url.split_once('@').map(|(_, r)| r).unwrap_or(url);
    let host = rest.split_once(':').map(|(h, _)| h)?;
    if host.contains('/') || host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts() {
        assert_eq!(
            remote_host("git@github.com:kverona-ai/workroam.git").as_deref(),
            Some("github.com")
        );
        assert_eq!(
            remote_host("https://git.corp.internal/a/b.git").as_deref(),
            Some("git.corp.internal")
        );
        assert_eq!(
            remote_host("ssh://git@host:2222/x.git").as_deref(),
            Some("host")
        );
        assert_eq!(remote_host("/local/path"), None);
    }

    #[test]
    fn version_is_known() {
        assert!(version().is_some());
    }
}
