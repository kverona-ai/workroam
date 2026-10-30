//! Cross-platform checks run before every push (XP-01 … XP-05).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use unicode_normalization::UnicodeNormalization;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum XpProblemKind {
    /// Two paths differ only by case (XP-01).
    CaseConflict,
    /// Reserved names, `<>:"\|?*`, trailing dot or space (XP-02).
    WindowsIllegal,
    /// Path longer than 260 characters on Windows (XP-03, warning only).
    LongPath,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct XpProblem {
    pub kind: XpProblemKind,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub other: Option<String>,
    /// Blocking problems refuse the push (exit code 13).
    pub blocking: bool,
}

const RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Is one path component illegal on Windows?
pub fn windows_illegal_component(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    if name.ends_with('.') || name.ends_with(' ') {
        return true;
    }
    if name
        .chars()
        .any(|c| matches!(c, '<' | '>' | ':' | '"' | '\\' | '|' | '?' | '*') || (c as u32) < 32)
    {
        return true;
    }
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    RESERVED.contains(&stem.as_str())
}

/// NFC normalise a snapshot path (XP-05).
pub fn nfc(path: &str) -> String {
    path.nfc().collect()
}

/// Check a set of snapshot paths. `windows_root_len` is the length of the
/// project directory on the Windows device when known (0 otherwise).
pub fn check_paths<'a>(
    paths: impl IntoIterator<Item = &'a str>,
    windows_root_len: usize,
) -> Vec<XpProblem> {
    let mut problems = Vec::new();
    let mut lower: HashMap<String, String> = HashMap::new();
    for p in paths {
        // Case conflicts on every prefix (directories too).
        let mut acc = String::new();
        for (i, comp) in p.split('/').enumerate() {
            if i > 0 {
                acc.push('/');
            }
            acc.push_str(comp);
            let key = nfc(&acc).to_lowercase();
            match lower.get(&key) {
                Some(existing) if existing != &acc => {
                    if !problems
                        .iter()
                        .any(|q: &XpProblem| q.kind == XpProblemKind::CaseConflict && q.path == acc)
                    {
                        problems.push(XpProblem {
                            kind: XpProblemKind::CaseConflict,
                            path: acc.clone(),
                            other: Some(existing.clone()),
                            blocking: true,
                        });
                    }
                }
                Some(_) => {}
                None => {
                    lower.insert(key, acc.clone());
                }
            }
            if windows_illegal_component(comp)
                && !problems
                    .iter()
                    .any(|q| q.kind == XpProblemKind::WindowsIllegal && q.path == acc)
            {
                problems.push(XpProblem {
                    kind: XpProblemKind::WindowsIllegal,
                    path: acc.clone(),
                    other: None,
                    blocking: true,
                });
            }
        }
        let total = if windows_root_len > 0 {
            windows_root_len + 1 + p.len()
        } else {
            p.len() + 40
        };
        if total > 260 {
            problems.push(XpProblem {
                kind: XpProblemKind::LongPath,
                path: p.to_string(),
                other: None,
                blocking: false,
            });
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn illegal_names() {
        assert!(windows_illegal_component("CON"));
        assert!(windows_illegal_component("con.txt"));
        assert!(windows_illegal_component("a:b"));
        assert!(windows_illegal_component("trailing."));
        assert!(windows_illegal_component("trailing "));
        assert!(!windows_illegal_component("console.log"));
        assert!(!windows_illegal_component("README.md"));
    }

    #[test]
    fn case_conflicts() {
        let p = check_paths(["src/Main.rs", "src/main.rs", "docs/a.md"], 0);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].kind, XpProblemKind::CaseConflict);
        let p = check_paths(["Docs/a.md", "docs/b.md"], 0);
        assert_eq!(p[0].kind, XpProblemKind::CaseConflict);
    }

    #[test]
    fn long_paths_warn() {
        let long = "a/".repeat(140) + "f.txt";
        let p = check_paths([long.as_str()], 20);
        assert!(p
            .iter()
            .any(|x| x.kind == XpProblemKind::LongPath && !x.blocking));
    }

    #[test]
    fn nfc_normalises() {
        assert_eq!(nfc("cafe\u{301}"), "caf\u{e9}");
    }
}
