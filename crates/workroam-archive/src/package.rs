//! `.wrarchive` files (architecture §10.4): a temporary rustic repository
//! holding only the archive snapshot plus the password-sealed archive
//! record, packed into one file.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use workroam_core::crypto;
use workroam_core::model::{
    project_tag, ArchiveRecord, Head, HeadState, TAG_ARCHIVED, TAG_WORKROAM,
};
use workroam_core::{Code, Error, Result};
use workroam_engine::{blocking, Workspace};
use workroam_store::control::keys;

const MAGIC: &[u8] = b"WRARCHIVE\x01\n";
const RECORD: &str = "record.bin";
const REPO: &str = "repo";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Packed {
    record: ArchiveRecord,
    exported_at: chrono::DateTime<Utc>,
    workspace: String,
}

fn files_under(root: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d)? {
            let e = e?;
            let p = e.path();
            if e.file_type()?.is_dir() {
                stack.push(p);
            } else {
                out.push(p);
            }
        }
    }
    out.sort();
    Ok(out)
}

fn pack(src: &Path, file: &Path) -> Result<u64> {
    let tmp = file.with_extension("wrarchive.part");
    let mut w = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
    w.write_all(MAGIC)?;
    for p in files_under(src)? {
        let rel = p
            .strip_prefix(src)
            .map_err(|e| Error::internal(e.to_string()))?
            .to_string_lossy()
            .replace('\\', "/");
        let data = std::fs::read(&p)?;
        w.write_all(&(rel.len() as u32).to_le_bytes())?;
        w.write_all(rel.as_bytes())?;
        w.write_all(&(data.len() as u64).to_le_bytes())?;
        w.write_all(&data)?;
    }
    w.flush()?;
    drop(w);
    std::fs::rename(&tmp, file)?;
    Ok(std::fs::metadata(file)?.len())
}

fn unpack(file: &Path, dest: &Path) -> Result<()> {
    let bad = || {
        Error::new(
            Code::Corrupted,
            format!("{} is not a Workroam archive file", file.display()),
        )
    };
    let mut r = std::io::BufReader::new(std::fs::File::open(file)?);
    let mut magic = vec![0u8; MAGIC.len()];
    r.read_exact(&mut magic).map_err(|_| bad())?;
    if magic != MAGIC {
        return Err(bad());
    }
    loop {
        let mut len = [0u8; 4];
        match r.read_exact(&mut len) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e.into()),
        }
        let mut name = vec![0u8; u32::from_le_bytes(len) as usize];
        r.read_exact(&mut name).map_err(|_| bad())?;
        let name = String::from_utf8(name).map_err(|_| bad())?;
        if name.split('/').any(|c| c == ".." || c.is_empty()) || name.starts_with('/') {
            return Err(bad());
        }
        let mut size = [0u8; 8];
        r.read_exact(&mut size).map_err(|_| bad())?;
        let mut data = vec![0u8; u64::from_le_bytes(size) as usize];
        r.read_exact(&mut data).map_err(|_| bad())?;
        let out = dest.join(&name);
        if let Some(d) = out.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(out, data)?;
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportOutcome {
    pub project: String,
    pub file: String,
    pub size: u64,
    pub snapshot: String,
}

/// Write the archive of `name` into `file`, encrypted with `password`.
pub async fn export(
    ws: &Workspace,
    name: &str,
    file: &Path,
    password: &str,
) -> Result<ExportOutcome> {
    let pid = ws.project_id(name);
    let (rec, _) = ws
        .control
        .get::<ArchiveRecord>(&keys::archive(&pid))
        .await?
        .ok_or_else(|| {
            Error::invalid(format!("project {name} is not archived"))
                .with_hint("archive it first with workroam archive")
        })?;
    if password.is_empty() {
        return Err(Error::invalid("an export password is required"));
    }
    let tmp = tempfile::tempdir()?;
    let repo = tmp.path().join(REPO);
    let ids = vec![rec.snapshot.clone()];
    blocking(|| ws.snap.export_to(&repo, password, &ids))?;
    let packed = Packed {
        record: rec.clone(),
        exported_at: Utc::now(),
        workspace: ws.machine.workspace.workspace_id.clone(),
    };
    std::fs::write(
        tmp.path().join(RECORD),
        crypto::password_seal(password, &serde_json::to_vec(&packed)?),
    )?;
    if let Some(d) = file.parent() {
        if !d.as_os_str().is_empty() {
            std::fs::create_dir_all(d)?;
        }
    }
    let size = pack(tmp.path(), file)?;
    Ok(ExportOutcome {
        project: name.to_string(),
        file: file.display().to_string(),
        size,
        snapshot: rec.snapshot,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportOutcome {
    pub project: String,
    pub snapshot: String,
    pub from_workspace: String,
}

/// Import a `.wrarchive` file into this workspace as an archived project
/// (optionally under another name).
pub async fn import(
    ws: &Workspace,
    file: &Path,
    password: &str,
    rename: Option<&str>,
) -> Result<ImportOutcome> {
    ws.ensure_writable()?;
    let tmp = tempfile::tempdir()?;
    unpack(file, tmp.path())?;
    let sealed = std::fs::read(tmp.path().join(RECORD))
        .map_err(|_| Error::new(Code::Corrupted, "the archive file has no record"))?;
    let packed: Packed = serde_json::from_slice(
        &crypto::password_open(password, &sealed)
            .map_err(|_| Error::new(Code::Denied, "wrong export password"))?,
    )?;
    let name = rename.unwrap_or(&packed.record.name).to_string();
    workroam_core::config::validate_project_name(&name)?;
    let pid = ws.project_id(&name);
    if let Some((h, _)) = ws.head(&name).await? {
        return Err(Error::new(
            Code::Conflict,
            format!(
                "a project named {name} already exists ({})",
                if h.state == HeadState::Archived {
                    "archived"
                } else {
                    "active"
                }
            ),
        )
        .with_hint("import under another name with --name"));
    }
    let repo = tmp.path().join(REPO);
    let ids = blocking(|| ws.snap.import_from(&repo, password))?;
    let mut snapshot = None;
    for id in ids {
        let meta = blocking(|| ws.snap.get(&id))?;
        let old: Vec<String> = meta
            .tags
            .iter()
            .filter(|t| t.starts_with(&project_tag("")))
            .cloned()
            .collect();
        let add = vec![
            TAG_WORKROAM.to_string(),
            TAG_ARCHIVED.to_string(),
            project_tag(&pid),
        ];
        let new_id = blocking(|| ws.snap.retag(&id, &add, &old))?;
        snapshot = Some(new_id);
    }
    let snapshot = snapshot
        .ok_or_else(|| Error::new(Code::Corrupted, "the archive file holds no snapshot"))?;
    let rec = ArchiveRecord {
        name: name.clone(),
        snapshot: snapshot.clone(),
        ..packed.record
    };
    ws.control.put(&keys::archive(&pid), &rec).await?;
    let head = Head {
        project: name.clone(),
        snapshot: Some(snapshot.clone()),
        parent: None,
        device_id: ws.machine.device_id.clone(),
        device_name: ws.machine.name.clone(),
        created_at: Utc::now(),
        state: HeadState::Archived,
        summary: None,
    };
    if !workroam_lease::write_head(&ws.control, &pid, &head, None).await? {
        return Err(Error::new(
            Code::Conflict,
            "another device created the project meanwhile",
        ));
    }
    Ok(ImportOutcome {
        project: name,
        snapshot,
        from_workspace: packed.workspace,
    })
}
