//! Built-in adapters, user overrides and agent detection (ADPT-07).

use crate::model::Adapter;
use serde::{Deserialize, Serialize};
use std::time::Duration;

const BUILTIN: &[(&str, &str)] = &[
    (
        "claude-code.toml",
        include_str!("../../../adapters/claude-code.toml"),
    ),
    ("codex.toml", include_str!("../../../adapters/codex.toml")),
    (
        "grok-build.toml",
        include_str!("../../../adapters/grok-build.toml"),
    ),
    (
        "antigravity.toml",
        include_str!("../../../adapters/antigravity.toml"),
    ),
    ("zcode.toml", include_str!("../../../adapters/zcode.toml")),
    ("pi.toml", include_str!("../../../adapters/pi.toml")),
    (
        "grok-bot.toml",
        include_str!("../../../adapters/grok-bot.toml"),
    ),
    ("muse.toml", include_str!("../../../adapters/muse.toml")),
];

pub fn builtin() -> Vec<Adapter> {
    BUILTIN
        .iter()
        .map(|(name, src)| toml::from_str(src).unwrap_or_else(|e| panic!("adapter {name}: {e}")))
        .collect()
}

/// Built-in adapters, overridden by `~/.workroam/adapters/*.toml` with the same id.
pub fn all() -> Vec<Adapter> {
    let mut list = builtin();
    let dir = workroam_core::paths::user_adapters_dir();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            if e.path().extension().map(|x| x == "toml").unwrap_or(false) {
                match std::fs::read_to_string(e.path())
                    .ok()
                    .and_then(|s| toml::from_str::<Adapter>(&s).ok())
                {
                    Some(a) => {
                        list.retain(|b| b.id != a.id);
                        list.push(a);
                    }
                    None => tracing::warn!("ignoring invalid adapter {}", e.path().display()),
                }
            }
        }
    }
    list
}

pub fn by_id(id: &str) -> Option<Adapter> {
    all().into_iter().find(|a| a.id == id)
}

pub fn local() -> Vec<Adapter> {
    all().into_iter().filter(|a| a.is_local()).collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Detected {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub installed: bool,
    pub version: Option<String>,
    pub adapter_version: u32,
    pub supported: Option<bool>,
    pub range: String,
}

/// Run the adapter's detect command (2 s timeout) and parse a version.
pub fn detect(a: &Adapter) -> Detected {
    let mut d = Detected {
        id: a.id.clone(),
        name: a.name.clone(),
        kind: a.connect.kind.clone(),
        installed: false,
        version: None,
        adapter_version: a.adapter_version,
        supported: None,
        range: a.agent_versions.clone(),
    };
    let Some(det) = &a.detect else { return d };
    let mut parts = det.command.split_whitespace();
    let Some(bin) = parts.next() else { return d };
    let args: Vec<&str> = parts.collect();
    if let Some(out) = run_with_timeout(bin, &args, Duration::from_secs(3)) {
        d.installed = true;
        d.version = extract_version(&out);
        d.supported = d
            .version
            .as_deref()
            .map(|v| version_in_range(v, &a.agent_versions));
    }
    d
}

fn run_with_timeout(bin: &str, args: &[&str], timeout: Duration) -> Option<String> {
    let mut cmd = std::process::Command::new(bin);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = cmd.spawn().ok()?;
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() > timeout => {
                let _ = child.kill();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => return None,
        }
    }
    let out = child.wait_with_output().ok()?;
    Some(format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    ))
}

/// First `x.y[.z]` in the text.
pub fn extract_version(s: &str) -> Option<String> {
    let bytes: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == '.') {
                i += 1;
            }
            let cand: String = bytes[start..i]
                .iter()
                .collect::<String>()
                .trim_end_matches('.')
                .to_string();
            if cand.contains('.') {
                return Some(cand);
            }
        }
        i += 1;
    }
    None
}

fn parse_v(v: &str) -> Vec<u64> {
    v.trim()
        .trim_start_matches('v')
        .split('.')
        .map(|p| {
            p.chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse()
                .unwrap_or(0)
        })
        .collect()
}

fn cmp_v(a: &[u64], b: &[u64]) -> std::cmp::Ordering {
    for i in 0..a.len().max(b.len()) {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        if x != y {
            return x.cmp(&y);
        }
    }
    std::cmp::Ordering::Equal
}

/// Ranges like `>=2.1.0`, `>=1.0, <3`, `^0.4`, `=1.2.3`. Empty = any.
pub fn version_in_range(v: &str, range: &str) -> bool {
    use std::cmp::Ordering::*;
    let ver = parse_v(v);
    range
        .split(',')
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .all(|r| {
            let (op, rest) = if let Some(x) = r.strip_prefix(">=") {
                (">=", x)
            } else if let Some(x) = r.strip_prefix("<=") {
                ("<=", x)
            } else if let Some(x) = r.strip_prefix('>') {
                (">", x)
            } else if let Some(x) = r.strip_prefix('<') {
                ("<", x)
            } else if let Some(x) = r.strip_prefix('^') {
                ("^", x)
            } else if let Some(x) = r.strip_prefix('=') {
                ("=", x)
            } else {
                ("=", r)
            };
            let b = parse_v(rest);
            let c = cmp_v(&ver, &b);
            match op {
                ">=" => c != Less,
                "<=" => c != Greater,
                ">" => c == Greater,
                "<" => c == Less,
                "^" => {
                    c != Less
                        && ver.first() == b.first()
                        && (b.first() != Some(&0) || ver.get(1) == b.get(1))
                }
                _ => c == Equal,
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_adapters_parse() {
        let all = builtin();
        assert_eq!(all.len(), 8);
        for id in [
            "claude",
            "codex",
            "grok",
            "antigravity",
            "zcode",
            "pi",
            "grok-bot",
            "muse",
        ] {
            assert!(all.iter().any(|a| a.id == id), "{id}");
        }
        assert_eq!(all.iter().filter(|a| a.is_cloud()).count(), 2);
    }

    #[test]
    fn versions() {
        assert_eq!(
            extract_version("2.1.12 (Claude Code)").as_deref(),
            Some("2.1.12")
        );
        assert_eq!(
            extract_version("codex-cli 0.46.0").as_deref(),
            Some("0.46.0")
        );
        assert!(version_in_range("2.1.12", ">=2.1.0"));
        assert!(!version_in_range("2.0.9", ">=2.1.0"));
        assert!(version_in_range("1.5", ">=1.0, <2"));
        assert!(version_in_range("0.4.9", "^0.4"));
        assert!(!version_in_range("0.5.0", "^0.4"));
        assert!(version_in_range("9.9", ""));
    }
}
