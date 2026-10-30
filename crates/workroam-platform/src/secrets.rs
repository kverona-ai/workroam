//! Secrets live in the system keychain (macOS Keychain, Windows Credential
//! Manager, Linux Secret Service). Headless machines without a keychain fall
//! back to a 0600 file in `~/.workroam/` (set `WORKROAM_SECRETS=file` to force).

use std::collections::BTreeMap;
use std::path::PathBuf;
use workroam_core::{Error, Result};

const SERVICE: &str = "dev.workroam";

pub mod keys {
    pub const ACCESS_KEY: &str = "storage.access_key";
    pub const SECRET_KEY: &str = "storage.secret_key";
    pub const PASSWORD: &str = "workspace.password";
    pub const BACKUP_ACCESS_KEY: &str = "backup.access_key";
    pub const BACKUP_SECRET_KEY: &str = "backup.secret_key";
    pub const RELAY_KEY: &str = "relay.device_key";
    pub fn webhook(agent: &str) -> String {
        format!("webhook.{agent}")
    }
    pub fn secret(name: &str) -> String {
        format!("secret.{name}")
    }
}

fn file_path() -> PathBuf {
    workroam_core::paths::workroam_home().join("secrets.json")
}

fn force_file() -> bool {
    matches!(std::env::var("WORKROAM_SECRETS").as_deref(), Ok("file"))
}

fn read_file() -> BTreeMap<String, String> {
    std::fs::read(file_path())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn write_file(map: &BTreeMap<String, String>) -> Result<()> {
    let path = file_path();
    workroam_core::paths::write_atomic(&path, &serde_json::to_vec_pretty(map)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// Run a keyring call on its own thread: some backends drive their own
/// async runtime and must not run inside the daemon's Tokio threads.
fn on_thread<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::spawn(f)
        .join()
        .expect("keychain thread panicked")
}

fn user_for(key: &str) -> String {
    // Separate entries per WORKROAM_HOME so test and portable setups do not collide.
    match std::env::var("WORKROAM_HOME") {
        Ok(h) if !h.is_empty() => format!(
            "{key}@{}",
            &workroam_core::crypto::sha256_hex(h.as_bytes())[..8]
        ),
        _ => key.to_string(),
    }
}

pub fn set(key: &str, value: &str) -> Result<()> {
    if !force_file() {
        let (k, v) = (user_for(key), value.to_string());
        let ok = on_thread(move || {
            keyring::Entry::new(SERVICE, &k)
                .and_then(|e| e.set_password(&v))
                .is_ok()
        });
        if ok {
            // Remove any stale file copy.
            let mut map = read_file();
            if map.remove(key).is_some() {
                write_file(&map)?;
            }
            return Ok(());
        }
        tracing::warn!(
            "system keychain unavailable, storing {key} in {}",
            file_path().display()
        );
    }
    let mut map = read_file();
    map.insert(key.to_string(), value.to_string());
    write_file(&map)
}

pub fn get(key: &str) -> Result<Option<String>> {
    if !force_file() {
        let k = user_for(key);
        let v = on_thread(move || {
            keyring::Entry::new(SERVICE, &k)
                .and_then(|e| e.get_password())
                .ok()
        });
        if v.is_some() {
            return Ok(v);
        }
    }
    Ok(read_file().get(key).cloned())
}

pub fn require(key: &str) -> Result<String> {
    get(key)?.ok_or_else(|| {
        Error::invalid(format!("secret {key} is not in the keychain"))
            .with_hint("run workroam credentials update or workroam setup")
    })
}

pub fn delete(key: &str) -> Result<()> {
    if !force_file() {
        let k = user_for(key);
        on_thread(move || {
            if let Ok(e) = keyring::Entry::new(SERVICE, &k) {
                let _ = e.delete_credential();
            }
        });
    }
    let mut map = read_file();
    if map.remove(key).is_some() {
        write_file(&map)?;
    }
    Ok(())
}

/// Where secrets end up on this machine (shown by `doctor`).
pub fn backend_name() -> String {
    if force_file() {
        return format!("file ({})", file_path().display());
    }
    let ok = on_thread(|| {
        keyring::Entry::new(SERVICE, "probe")
            .map(|e| e.get_password().is_ok() || true)
            .unwrap_or(false)
    });
    if ok {
        "system keychain".into()
    } else {
        format!("file ({})", file_path().display())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_backend_roundtrip() {
        let d = tempfile::tempdir().unwrap();
        std::env::set_var("WORKROAM_HOME", d.path());
        std::env::set_var("WORKROAM_SECRETS", "file");
        set("x", "1").unwrap();
        assert_eq!(get("x").unwrap().as_deref(), Some("1"));
        delete("x").unwrap();
        assert_eq!(get("x").unwrap(), None);
    }
}
