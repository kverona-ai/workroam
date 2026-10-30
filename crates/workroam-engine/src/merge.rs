//! Conflict handling for files changed on both sides (architecture §9.4).

use std::path::Path;

/// Text files are merged three-way; anything else keeps two copies.
pub fn is_text(data: &[u8]) -> bool {
    data.len() <= 4 * 1024 * 1024
        && !data[..data.len().min(8192)].contains(&0)
        && std::str::from_utf8(data).is_ok()
}

pub enum MergeOutcome {
    Clean(Vec<u8>),
    /// Keep both versions.
    Conflict,
}

pub fn merge3(base: &[u8], ours: &[u8], theirs: &[u8]) -> MergeOutcome {
    if !(is_text(base) && is_text(ours) && is_text(theirs)) {
        return MergeOutcome::Conflict;
    }
    let (b, o, t) = (
        std::str::from_utf8(base).unwrap_or_default(),
        std::str::from_utf8(ours).unwrap_or_default(),
        std::str::from_utf8(theirs).unwrap_or_default(),
    );
    match diffy::merge(b, o, t) {
        Ok(m) => MergeOutcome::Clean(m.into_bytes()),
        Err(_) => MergeOutcome::Conflict,
    }
}

/// `dir/name.ext` → `dir/name.conflict-<device>.ext` (unique).
pub fn conflict_path(rel: &str, device: &str, exists: &dyn Fn(&str) -> bool) -> String {
    let dev: String = device
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let (dir, file) = match rel.rsplit_once('/') {
        Some((d, f)) => (format!("{d}/"), f),
        None => (String::new(), rel),
    };
    let (stem, ext) = match file.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s, format!(".{e}")),
        _ => (file, String::new()),
    };
    let mut n = 0;
    loop {
        let suffix = if n == 0 {
            String::new()
        } else {
            format!("-{n}")
        };
        let cand = format!("{dir}{stem}.conflict-{dev}{suffix}{ext}");
        if !exists(&cand) {
            return cand;
        }
        n += 1;
    }
}

pub fn is_conflict_copy(rel: &str) -> bool {
    Path::new(rel)
        .file_name()
        .map(|f| f.to_string_lossy().contains(".conflict-"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges() {
        let base = b"a\nb\nc\n";
        let ours = b"A\nb\nc\n";
        let theirs = b"a\nb\nC\n";
        match merge3(base, ours, theirs) {
            MergeOutcome::Clean(m) => assert_eq!(m, b"A\nb\nC\n"),
            MergeOutcome::Conflict => panic!("expected clean merge"),
        }
        assert!(matches!(
            merge3(base, b"x\nb\nc\n", b"y\nb\nc\n"),
            MergeOutcome::Conflict
        ));
        assert_eq!(
            conflict_path("docs/a.md", "home mac", &|_| false),
            "docs/a.conflict-home-mac.md"
        );
        assert_eq!(
            conflict_path("Makefile", "pc", &|p| p == "Makefile.conflict-pc"),
            "Makefile.conflict-pc-1"
        );
        assert!(is_conflict_copy("docs/a.conflict-pc.md"));
    }
}
