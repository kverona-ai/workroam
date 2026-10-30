//! Agent session migration (architecture §12.4): collect this project's
//! sessions on push, place them under the local path key on pull and rewrite
//! working-directory fields. Failures only produce warnings.

use crate::model::Adapter;
use serde_json::Value;
use std::path::{Path, PathBuf};

pub const SNAPSHOT_DIR: &str = ".workroam-snapshot/sessions";

/// Directory name an agent derives from a project path.
pub fn path_key(kind: &str, project_dir: &str) -> String {
    match kind {
        "pi-path-key" => {
            let inner: String = project_dir
                .trim_start_matches(['/', '\\'])
                .chars()
                .map(|c| {
                    if matches!(c, '/' | '\\' | ':') {
                        '-'
                    } else {
                        c
                    }
                })
                .collect();
            format!("--{inner}--")
        }
        _ => project_dir
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect(),
    }
}

fn session_root(a: &Adapter, project_dir: &str) -> Option<PathBuf> {
    let s = a.sessions.as_ref()?;
    let dir = s
        .dir
        .replace("{path_key}", &path_key(&s.path_key, project_dir));
    Some(workroam_core::paths::expand_tilde(&dir))
}

fn first_cwd(path: &Path) -> Option<String> {
    use std::io::BufRead;
    let f = std::fs::File::open(path).ok()?;
    for line in std::io::BufReader::new(f)
        .lines()
        .take(5)
        .map_while(|l| l.ok())
    {
        if let Ok(v) = serde_json::from_str::<Value>(&line) {
            if let Some(c) = find_field(&v, "cwd") {
                return Some(c);
            }
        }
    }
    None
}

fn find_field(v: &Value, key: &str) -> Option<String> {
    match v {
        Value::Object(m) => {
            if let Some(Value::String(s)) = m.get(key) {
                return Some(s.clone());
            }
            m.values().find_map(|x| find_field(x, key))
        }
        _ => None,
    }
}

fn same_path(a: &str, b: &str) -> bool {
    let n = |s: &str| {
        s.trim_end_matches(['/', '\\'])
            .replace('\\', "/")
            .to_lowercase()
    };
    n(a) == n(b) || n(a).starts_with(&(n(b) + "/"))
}

/// Session files of this project: (path relative to the agent's session root, absolute path).
pub fn collect(a: &Adapter, project_dir: &Path) -> Vec<(String, PathBuf)> {
    let pd = project_dir.display().to_string();
    let Some(root) = session_root(a, &pd) else {
        return vec![];
    };
    let Some(s) = a.sessions.as_ref() else {
        return vec![];
    };
    if !root.is_dir() {
        return vec![];
    }
    let mut out = Vec::new();
    for e in walkdir::WalkDir::new(&root)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
    {
        let p = e.path();
        if s.path_key == "cwd-filter" && !first_cwd(p).map(|c| same_path(&c, &pd)).unwrap_or(false)
        {
            continue;
        }
        if let Ok(rel) = p.strip_prefix(&root) {
            out.push((rel.to_string_lossy().replace('\\', "/"), p.to_path_buf()));
        }
    }
    out.sort();
    out
}

/// `s` without the leading `prefix`, compared ignoring case and slash
/// direction; `None` when `s` does not start with it.
fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let fold = |c: char| if c == '\\' { '/' } else { c };
    let mut si = s.char_indices();
    for p in prefix.chars() {
        let (_, c) = si.next()?;
        if !fold(c).to_lowercase().eq(fold(p).to_lowercase()) {
            return None;
        }
    }
    Some(match si.next() {
        Some((i, _)) => &s[i..],
        None => "",
    })
}

fn map_path(s: &str, from: &str, to: &str) -> Option<String> {
    let fs = from.trim_end_matches(['/', '\\']);
    let rest = strip_prefix_ci(s, fs)?;
    if !(rest.is_empty() || rest.starts_with(['/', '\\'])) {
        return None;
    }
    let sep = if to.contains('\\') || (to.len() > 1 && to.as_bytes()[1] == b':') {
        '\\'
    } else {
        '/'
    };
    let rest: String = rest
        .chars()
        .map(|c| if c == '/' || c == '\\' { sep } else { c })
        .collect();
    Some(format!("{}{}", to.trim_end_matches(['/', '\\']), rest))
}

fn rewrite_value(v: &mut Value, fields: &[String], from: &str, to: &str) {
    match v {
        Value::Object(m) => {
            for (k, x) in m.iter_mut() {
                if fields.iter().any(|f| f == k) {
                    if let Value::String(s) = x {
                        if let Some(n) = map_path(s, from, to) {
                            *s = n;
                        }
                        continue;
                    }
                }
                rewrite_value(x, fields, from, to);
            }
        }
        Value::Array(a) => a
            .iter_mut()
            .for_each(|x| rewrite_value(x, fields, from, to)),
        _ => {}
    }
}

/// Rewrite working-directory fields in a JSON / JSONL session file.
pub fn rewrite(content: &[u8], fields: &[String], from: &str, to: &str) -> Vec<u8> {
    let Ok(text) = std::str::from_utf8(content) else {
        return content.to_vec();
    };
    if let Ok(mut v) = serde_json::from_str::<Value>(text) {
        rewrite_value(&mut v, fields, from, to);
        return serde_json::to_vec_pretty(&v).unwrap_or_else(|_| content.to_vec());
    }
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let (body, nl) = line
            .strip_suffix('\n')
            .map(|b| (b, "\n"))
            .unwrap_or((line, ""));
        match serde_json::from_str::<Value>(body) {
            Ok(mut v) => {
                rewrite_value(&mut v, fields, from, to);
                out.push_str(&serde_json::to_string(&v).unwrap_or_else(|_| body.to_string()));
            }
            Err(_) => out.push_str(body),
        }
        out.push_str(nl);
    }
    out.into_bytes()
}

/// Place migrated sessions for `a` on this machine. A local session that is
/// at least as long as the incoming copy is kept. Returns warnings.
pub fn restore(
    a: &Adapter,
    files: &[(String, Vec<u8>)],
    from_dir: &str,
    project_dir: &Path,
) -> Vec<String> {
    let to = project_dir.display().to_string();
    let mut warnings = Vec::new();
    let Some(root) = session_root(a, &to) else {
        return warnings;
    };
    let fields = a
        .sessions
        .as_ref()
        .map(|s| s.rewrite_fields.clone())
        .unwrap_or_default();
    for (rel, data) in files {
        if !workroam_core::paths::is_safe_relative(rel) {
            warnings.push(format!("{}: {}: unsafe session path skipped", a.name, rel));
            continue;
        }
        let dest = root.join(rel);
        let body = rewrite(data, &fields, from_dir, &to);
        if let Ok(m) = std::fs::metadata(&dest) {
            if m.len() >= body.len() as u64 {
                continue;
            }
        }
        if let Err(e) = workroam_core::paths::write_atomic(&dest, &body) {
            warnings.push(format!("{}: {}: {}", a.name, rel, e.message));
        }
    }
    warnings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys() {
        assert_eq!(
            path_key("dash-path-key", "/home/me/proj-a"),
            "-home-me-proj-a"
        );
        assert_eq!(
            path_key("dash-path-key", r"D:\work\proj-a"),
            "D--work-proj-a"
        );
        assert_eq!(path_key("pi-path-key", "/home/me/p"), "--home-me-p--");
    }

    #[test]
    fn rewrites_windows_to_unix() {
        let line =
            br#"{"type":"user","cwd":"D:\\work\\proj-a\\src","msg":{"cwd":"D:\\work\\proj-a"}}
not json
"#;
        let out = rewrite(line, &["cwd".into()], r"D:\work\proj-a", "/Users/me/proj-a");
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains(r#""cwd":"/Users/me/proj-a/src""#), "{s}");
        assert!(s.contains(r#"{"cwd":"/Users/me/proj-a"}"#));
        assert!(s.ends_with("not json\n"));
        assert_eq!(map_path("/Users/me/proj-ab", "/Users/me/proj-a", "x"), None);
        // Case folding that changes byte lengths must not slice mid-character.
        assert_eq!(
            map_path("/Users/me/İstanbul/x", "/users/me/istanbul", "/p").as_deref(),
            None
        );
        assert_eq!(
            map_path("/Users/me/Straße/x", "/users/me/straße", "/p").as_deref(),
            Some("/p/x")
        );
        assert_eq!(
            map_path("/Users/me/ÉTÉ/x", "/users/me/été", "/p").as_deref(),
            Some("/p/x")
        );
        assert_eq!(
            map_path("/a/p/x", "/a/p", r"C:\w\p").as_deref(),
            Some(r"C:\w\p\x")
        );
    }
}
