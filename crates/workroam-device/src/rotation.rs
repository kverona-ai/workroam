//! Key rotation and device removal (architecture §8.3). The shared
//! `control/credentials` object carries `current` and, during a rotation,
//! `next`; every device adopts `next` on its next sync.

use chrono::{Duration, Utc};
use workroam_core::model::{AccessKey, CredentialsObject};
use workroam_core::Result;
use workroam_store::control::keys;
use workroam_store::Control;

/// Grace period after which the old key is revoked even if a device has not switched.
pub const GRACE_DAYS: i64 = 7;

pub async fn read(control: &Control) -> Result<CredentialsObject> {
    Ok(control
        .get::<CredentialsObject>(keys::CREDENTIALS)
        .await?
        .map(|(c, _)| c)
        .unwrap_or_default())
}

pub async fn write(control: &Control, c: &CredentialsObject) -> Result<()> {
    control.put(keys::CREDENTIALS, c).await?;
    Ok(())
}

/// Start a rotation with a freshly created key.
pub async fn begin(control: &Control, next: AccessKey, me: &str) -> Result<CredentialsObject> {
    let mut c = read(control).await?;
    c.next = Some(next);
    c.rotated_at = Some(Utc::now());
    c.switched = vec![me.to_string()];
    write(control, &c).await?;
    Ok(c)
}

/// What this device should do with the credentials object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Adopt {
    Nothing,
    /// Store this key locally and use it from now on.
    Switch(AccessKey),
}

/// Called on every sync: switch to `next` when it differs from the key in use.
pub async fn adopt(control: &Control, me: &str, in_use: &str) -> Result<Adopt> {
    let mut c = read(control).await?;
    let Some(next) = c.next.clone() else {
        // A finished rotation leaves the new key as `current`.
        return Ok(match &c.current {
            Some(cur)
                if cur.access_key != in_use
                    && c.read_only
                        .as_ref()
                        .map(|r| r.access_key != in_use)
                        .unwrap_or(true) =>
            {
                Adopt::Switch(cur.clone())
            }
            _ => Adopt::Nothing,
        });
    };
    if next.access_key == in_use {
        return Ok(Adopt::Nothing);
    }
    if !c.switched.iter().any(|s| s == me) {
        c.switched.push(me.to_string());
        write(control, &c).await?;
    }
    Ok(Adopt::Switch(next))
}

/// Whether the old key can be revoked: every remaining device switched or the grace period is over.
pub fn ready_to_finish(c: &CredentialsObject, device_ids: &[String]) -> bool {
    if c.next.is_none() {
        return false;
    }
    let all = device_ids.iter().all(|d| c.switched.contains(d));
    let expired = c
        .rotated_at
        .map(|t| Utc::now() - t > Duration::days(GRACE_DAYS))
        .unwrap_or(true);
    all || expired
}

/// Promote `next` to `current`; returns the old key to revoke.
pub async fn finish(control: &Control) -> Result<Option<AccessKey>> {
    let mut c = read(control).await?;
    let Some(next) = c.next.take() else {
        return Ok(None);
    };
    let old = c.current.replace(next);
    c.switched.clear();
    write(control, &c).await?;
    Ok(old)
}

#[cfg(test)]
mod tests {
    use super::*;
    use workroam_core::model::Access;

    fn key(a: &str) -> AccessKey {
        AccessKey {
            access_key: a.into(),
            secret_key: "s".into(),
            scope: Access::ReadWrite,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn rotate() {
        let td = tempfile::tempdir().unwrap();
        let control = Control::new(workroam_store::Store::local(td.path()), [7u8; 32]);
        write(
            &control,
            &CredentialsObject {
                current: Some(key("old")),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        begin(&control, key("new"), "dev_a").await.unwrap();
        assert_eq!(
            adopt(&control, "dev_b", "old").await.unwrap(),
            Adopt::Switch(key("new"))
        );
        assert_eq!(
            adopt(&control, "dev_b", "new").await.unwrap(),
            Adopt::Nothing
        );
        let c = read(&control).await.unwrap();
        assert!(ready_to_finish(&c, &["dev_a".into(), "dev_b".into()]));
        assert!(!ready_to_finish(
            &c,
            &["dev_a".into(), "dev_b".into(), "dev_c".into()]
        ));
        assert_eq!(finish(&control).await.unwrap(), Some(key("old")));
        // A device that missed the rotation adopts the new current key.
        assert_eq!(
            adopt(&control, "dev_c", "old").await.unwrap(),
            Adopt::Switch(key("new"))
        );
    }
}
