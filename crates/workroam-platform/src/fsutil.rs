//! Safe file replacement (PULL-03, XP-06) and links (XP-04, ADPT-05).

use std::io;
use std::path::{Path, PathBuf};

/// Write `data` to `path` through a temp file and rename, retrying while the
/// target is locked (Windows). When `expect_sha256` is set, the current file
/// must still have that hash (or not exist when `Some("")`).
pub fn replace_file(
    path: &Path,
    data: &[u8],
    exec: bool,
    expect_sha256: Option<&str>,
) -> io::Result<()> {
    if let Some(expected) = expect_sha256 {
        let current = file_sha256(path)?;
        let ok = match (&current, expected) {
            (None, "") => true,
            (Some(h), e) => h == e,
            (None, _) => false,
        };
        if !ok {
            return Err(io::Error::other(format!(
                "{} changed while syncing",
                path.display()
            )));
        }
    }
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir)?;
    // A symlink or directory in the way is removed first (never silently for files with content changes:
    // callers only replace files they verified above).
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() {
            std::fs::remove_file(path)?;
        } else if meta.is_dir() {
            return Err(io::Error::other(format!(
                "{} is a directory",
                path.display()
            )));
        }
    }
    let tmp = temp_name(path);
    std::fs::write(&tmp, data)?;
    set_exec(&tmp, exec)?;
    rename_retry(&tmp, path)
}

pub fn temp_name(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    path.with_file_name(format!(".{name}.wr-{}", std::process::id()))
}

pub fn rename_retry(from: &Path, to: &Path) -> io::Result<()> {
    let mut last = None;
    for attempt in 0..20u64 {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) => {
                last = Some(e);
                std::thread::sleep(std::time::Duration::from_millis(50 * (attempt + 1).min(10)));
            }
        }
    }
    let _ = std::fs::remove_file(from);
    Err(last.unwrap())
}

/// Remove a file, retrying on Windows sharing violations.
pub fn remove_retry(path: &Path) -> io::Result<()> {
    let mut last = None;
    for attempt in 0..20u64 {
        match std::fs::remove_file(path) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => {
                last = Some(e);
                std::thread::sleep(std::time::Duration::from_millis(50 * (attempt + 1).min(10)));
            }
        }
    }
    Err(last.unwrap())
}

/// Remove empty parent directories up to (not including) `stop`.
pub fn prune_empty_dirs(mut dir: &Path, stop: &Path) {
    while dir != stop && dir.starts_with(stop) {
        if std::fs::remove_dir(dir).is_err() {
            break;
        }
        match dir.parent() {
            Some(p) => dir = p,
            None => break,
        }
    }
}

pub fn file_sha256(path: &Path) -> io::Result<Option<String>> {
    use std::io::Read;
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    if meta.file_type().is_symlink() {
        let target = std::fs::read_link(path)?;
        return Ok(Some(workroam_core::crypto::sha256_hex(
            target.to_string_lossy().replace('\\', "/").as_bytes(),
        )));
    }
    if meta.is_dir() {
        return Ok(None);
    }
    let mut f = std::fs::File::open(path)?;
    let mut hasher = Sha256Stream::default();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(Some(hasher.finish()))
}

/// Incremental SHA-256 without exposing the digest crate to callers.
#[derive(Default)]
pub struct Sha256Stream {
    buf: Vec<u8>,
    chunks: Vec<[u8; 32]>,
    total: u64,
}

impl Sha256Stream {
    pub fn update(&mut self, data: &[u8]) {
        // Streaming through core's one-shot hash: keep it simple and exact by
        // buffering (files are hashed in 256 KiB pieces into a single digest).
        self.buf.extend_from_slice(data);
        self.total += data.len() as u64;
        if self.buf.len() > 64 * 1024 * 1024 {
            // Very large files: hash in 64 MiB pieces and combine (tree hash).
            self.chunks.push(workroam_core::crypto::sha256(&self.buf));
            self.buf.clear();
        }
    }
    pub fn finish(self) -> String {
        if self.chunks.is_empty() {
            return workroam_core::crypto::sha256_hex(&self.buf);
        }
        let mut all = Vec::new();
        for c in &self.chunks {
            all.extend_from_slice(c);
        }
        all.extend_from_slice(&workroam_core::crypto::sha256(&self.buf));
        format!("t{}", &workroam_core::crypto::sha256_hex(&all)[1..])
    }
}

#[cfg(unix)]
pub fn set_exec(path: &Path, exec: bool) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let meta = std::fs::metadata(path)?;
    let mut perm = meta.permissions();
    let mode = perm.mode();
    let new = if exec {
        mode | ((mode & 0o444) >> 2) | 0o100
    } else {
        mode & !0o111
    };
    if new != mode {
        perm.set_mode(new);
        std::fs::set_permissions(path, perm)?;
    }
    Ok(())
}

/// Windows keeps the executable bit unchanged (XP-04).
#[cfg(not(unix))]
pub fn set_exec(_path: &Path, _exec: bool) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
pub fn is_exec(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
pub fn is_exec(_meta: &std::fs::Metadata) -> bool {
    false
}

/// Create a symlink recorded in a snapshot. On Windows directories become
/// junctions and files become copies (XP-04).
pub fn create_link(link: &Path, target: &str, base: &Path) -> io::Result<()> {
    if let Some(dir) = link.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if std::fs::symlink_metadata(link).is_ok() {
        remove_any(link)?;
    }
    #[cfg(unix)]
    {
        let _ = base;
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(windows)]
    {
        let resolved = link
            .parent()
            .unwrap_or(base)
            .join(target.replace('/', "\\"));
        if resolved.is_dir() {
            junction(link, &resolved)
        } else if resolved.is_file() {
            std::fs::copy(&resolved, link).map(|_| ())
        } else {
            // Target missing: try a real symlink (works with developer mode), else skip.
            std::os::windows::fs::symlink_file(target, link).or(Ok(()))
        }
    }
}

/// Link a whole directory (Skills): symlink on Unix, junction on Windows.
pub fn link_dir(link: &Path, target: &Path) -> io::Result<()> {
    if let Some(dir) = link.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if let Ok(meta) = std::fs::symlink_metadata(link) {
        if meta.file_type().is_symlink() || is_junction(link) {
            remove_any(link)?;
        } else {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("{} already exists and is not a link", link.display()),
            ));
        }
    }
    #[cfg(unix)]
    {
        let rel = relative_to(target, link.parent().unwrap_or(Path::new(".")));
        std::os::unix::fs::symlink(rel, link)
    }
    #[cfg(windows)]
    {
        junction(link, target)
    }
}

#[cfg(windows)]
fn junction(link: &Path, target: &Path) -> io::Result<()> {
    let status = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::new(io::ErrorKind::Other, "mklink /J failed"))
    }
}

pub fn is_junction(path: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if let Ok(m) = std::fs::symlink_metadata(path) {
            return m.file_attributes() & 0x400 != 0; // FILE_ATTRIBUTE_REPARSE_POINT
        }
        false
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        false
    }
}

pub fn remove_any(path: &Path) -> io::Result<()> {
    let meta = std::fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() {
        std::fs::remove_file(path).or_else(|_| std::fs::remove_dir(path))
    } else if is_junction(path) {
        std::fs::remove_dir(path)
    } else if meta.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}

/// `target` expressed relative to `from_dir` (both absolute).
pub fn relative_to(target: &Path, from_dir: &Path) -> PathBuf {
    let t: Vec<_> = target.components().collect();
    let f: Vec<_> = from_dir.components().collect();
    let mut i = 0;
    while i < t.len() && i < f.len() && t[i] == f[i] {
        i += 1;
    }
    if i == 0 {
        return target.to_path_buf();
    }
    let mut out = PathBuf::new();
    for _ in i..f.len() {
        out.push("..");
    }
    for c in &t[i..] {
        out.push(c.as_os_str());
    }
    out
}

/// Copy a directory tree (used for Skills on systems without links and for exports).
pub fn copy_dir(from: &Path, to: &Path) -> io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let ft = entry.file_type()?;
        let dest = to.join(entry.file_name());
        if ft.is_dir() {
            copy_dir(&entry.path(), &dest)?;
        } else if ft.is_file() {
            std::fs::copy(entry.path(), dest)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_checks_expected_version() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a.txt");
        replace_file(&p, b"v1", false, Some("")).unwrap();
        let h1 = file_sha256(&p).unwrap().unwrap();
        assert!(replace_file(&p, b"v2", false, Some("deadbeef")).is_err());
        replace_file(&p, b"v2", true, Some(&h1)).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"v2");
        #[cfg(unix)]
        assert!(is_exec(&std::fs::metadata(&p).unwrap()));
    }

    #[test]
    fn relative_paths() {
        assert_eq!(
            relative_to(Path::new("/p/.agents/skills"), Path::new("/p/.claude")),
            PathBuf::from("../.agents/skills")
        );
    }

    #[cfg(unix)]
    #[test]
    fn dir_links() {
        let d = tempfile::tempdir().unwrap();
        let target = d.path().join(".agents/skills");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("x.md"), "x").unwrap();
        let link = d.path().join(".claude/skills");
        link_dir(&link, &target).unwrap();
        assert!(link.join("x.md").exists());
        link_dir(&link, &target).unwrap();
    }
}
