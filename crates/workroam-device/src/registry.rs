//! Device registry: `control/devices/<id>` (architecture §8.2).

use chrono::Utc;
use workroam_core::config::MachineConfig;
use workroam_core::model::{Access, DeviceRecord};
use workroam_core::Result;
use workroam_store::control::keys;
use workroam_store::Control;

pub fn record_for(
    m: &MachineConfig,
    agents: Vec<String>,
    relay_pubkey: Option<String>,
) -> DeviceRecord {
    let now = Utc::now();
    DeviceRecord {
        id: m.device_id.clone(),
        name: m.name.clone(),
        os: m.os,
        access: m.access,
        joined_at: now,
        last_seen: now,
        last_sync: None,
        reachable: m.reachable.clone(),
        relay_pubkey,
        agents,
    }
}

pub async fn list(control: &Control) -> Result<Vec<DeviceRecord>> {
    let mut v: Vec<DeviceRecord> = control
        .get_all::<DeviceRecord>(keys::DEVICES)
        .await?
        .into_iter()
        .map(|(_, d)| d)
        .collect();
    v.sort_by_key(|a| a.joined_at);
    Ok(v)
}

pub async fn get(control: &Control, id: &str) -> Result<Option<DeviceRecord>> {
    Ok(control
        .get::<DeviceRecord>(&keys::device(id))
        .await?
        .map(|(d, _)| d))
}

/// Create or update this device's record. Read-only devices cannot write.
pub async fn register(control: &Control, rec: &DeviceRecord) -> Result<()> {
    if rec.access == Access::ReadOnly {
        return Ok(());
    }
    let mut r = rec.clone();
    if let Some(old) = get(control, &rec.id).await? {
        r.joined_at = old.joined_at;
        if r.last_sync.is_none() {
            r.last_sync = old.last_sync;
        }
        if r.relay_pubkey.is_none() {
            r.relay_pubkey = old.relay_pubkey;
        }
    }
    control.put(&keys::device(&rec.id), &r).await?;
    Ok(())
}

/// Update last activity (and last sync when `synced`).
pub async fn touch(control: &Control, id: &str, synced: bool) -> Result<()> {
    if let Some(mut r) = get(control, id).await? {
        r.last_seen = Utc::now();
        if synced {
            r.last_sync = Some(r.last_seen);
        }
        control.put(&keys::device(id), &r).await?;
    }
    Ok(())
}

pub async fn remove(control: &Control, id: &str) -> Result<()> {
    control.delete(&keys::device(id)).await
}

/// Find by id or (case-insensitive) name.
pub async fn find(control: &Control, id_or_name: &str) -> Result<Option<DeviceRecord>> {
    Ok(list(control)
        .await?
        .into_iter()
        .find(|d| d.id == id_or_name || d.name.eq_ignore_ascii_case(id_or_name)))
}
