//! Leases (architecture §9.1, LEASE-01 … LEASE-04) and the `head` pointer.
//!
//! * Acquire: create with `If-None-Match: *`; an existing `released` lease is
//!   overwritten with `If-Match`.
//! * Renew / release / take over: `If-Match: <etag>`, so only one device wins.
//! * Without conditional writes the workspace runs without leases and forks
//!   are detected through the head's parent snapshot.

use chrono::{Duration, Utc};
use workroam_core::model::{DeviceRef, Head, HeadState, Lease, LeaseState};
use workroam_core::{Code, Error, Result};
use workroam_store::control::keys;
use workroam_store::{Control, PutOutcome};

/// Renew every 10 minutes while online (LEASE-02).
pub const RENEW_EVERY: std::time::Duration = std::time::Duration::from_secs(600);
/// A lease not renewed for 12 hours is "possibly forgotten".
pub const STALE_AFTER_HOURS: i64 = 12;

#[derive(Debug, Clone)]
pub enum LeaseStatus {
    Free,
    Mine(Lease),
    Other { lease: Lease, stale: bool },
}

#[derive(Debug, Clone)]
pub enum Acquire {
    Acquired,
    AlreadyMine,
    HeldByOther {
        lease: Lease,
        stale: bool,
    },
    /// The storage has no conditional writes: leases are not used.
    Unsupported,
}

pub fn is_stale(l: &Lease) -> bool {
    Utc::now() - l.renewed_at > Duration::hours(STALE_AFTER_HOURS)
}

pub async fn read(control: &Control, pid: &str) -> Result<Option<(Lease, String)>> {
    control.get::<Lease>(&keys::lease(pid)).await
}

pub async fn status(control: &Control, pid: &str, me: &str) -> Result<LeaseStatus> {
    Ok(match read(control, pid).await? {
        None => LeaseStatus::Free,
        Some((l, _)) if l.state == LeaseState::Released => LeaseStatus::Free,
        Some((l, _)) if l.holder.id == me => LeaseStatus::Mine(l),
        Some((l, _)) => {
            let stale = is_stale(&l);
            LeaseStatus::Other { lease: l, stale }
        }
    })
}

fn fresh(me: &DeviceRef) -> Lease {
    let now = Utc::now();
    Lease {
        holder: me.clone(),
        acquired_at: now,
        renewed_at: now,
        state: LeaseState::Held,
    }
}

pub async fn acquire(control: &Control, pid: &str, me: &DeviceRef) -> Result<Acquire> {
    if !control.store.conditional {
        return Ok(Acquire::Unsupported);
    }
    let key = keys::lease(pid);
    for _ in 0..5 {
        match control.put_if_absent(&key, &fresh(me)).await? {
            PutOutcome::Written { .. } => return Ok(Acquire::Acquired),
            PutOutcome::PreconditionFailed => {}
        }
        let Some((cur, etag)) = read(control, pid).await? else {
            continue;
        };
        if cur.holder.id == me.id && cur.state == LeaseState::Held {
            let mut l = cur.clone();
            l.renewed_at = Utc::now();
            if let PutOutcome::Written { .. } = control.put_if_match(&key, &l, &etag).await? {
                return Ok(Acquire::AlreadyMine);
            }
            continue;
        }
        if cur.state == LeaseState::Released {
            if let PutOutcome::Written { .. } =
                control.put_if_match(&key, &fresh(me), &etag).await?
            {
                return Ok(Acquire::Acquired);
            }
            continue;
        }
        let stale = is_stale(&cur);
        return Ok(Acquire::HeldByOther { lease: cur, stale });
    }
    Err(Error::network(
        "lease changed repeatedly while acquiring; try again",
    ))
}

/// Renew our lease. Returns false when we no longer hold it.
pub async fn renew(control: &Control, pid: &str, me: &DeviceRef) -> Result<bool> {
    if !control.store.conditional {
        return Ok(true);
    }
    let Some((mut cur, etag)) = read(control, pid).await? else {
        return Ok(false);
    };
    if cur.holder.id != me.id || cur.state != LeaseState::Held {
        return Ok(false);
    }
    cur.renewed_at = Utc::now();
    Ok(matches!(
        control.put_if_match(&keys::lease(pid), &cur, &etag).await?,
        PutOutcome::Written { .. }
    ))
}

/// Release (writes `state = released`; no conditional delete needed).
pub async fn release(control: &Control, pid: &str, me: &DeviceRef) -> Result<bool> {
    if !control.store.conditional {
        return Ok(true);
    }
    let Some((mut cur, etag)) = read(control, pid).await? else {
        return Ok(true);
    };
    if cur.holder.id != me.id {
        return Ok(false);
    }
    if cur.state == LeaseState::Released {
        return Ok(true);
    }
    cur.state = LeaseState::Released;
    cur.renewed_at = Utc::now();
    Ok(matches!(
        control.put_if_match(&keys::lease(pid), &cur, &etag).await?,
        PutOutcome::Written { .. }
    ))
}

/// Force take over after the user confirmed (LEASE-03).
pub async fn takeover(control: &Control, pid: &str, me: &DeviceRef) -> Result<Option<Lease>> {
    if !control.store.conditional {
        return Ok(None);
    }
    let key = keys::lease(pid);
    for _ in 0..5 {
        match read(control, pid).await? {
            None => {
                if let PutOutcome::Written { .. } = control.put_if_absent(&key, &fresh(me)).await? {
                    return Ok(None);
                }
            }
            Some((cur, etag)) => {
                if let PutOutcome::Written { .. } =
                    control.put_if_match(&key, &fresh(me), &etag).await?
                {
                    return Ok(if cur.holder.id != me.id && cur.state == LeaseState::Held {
                        Some(cur)
                    } else {
                        None
                    });
                }
            }
        }
    }
    Err(Error::network(
        "lease changed repeatedly while taking over; try again",
    ))
}

pub async fn read_head(control: &Control, pid: &str) -> Result<Option<(Head, String)>> {
    control.get::<Head>(&keys::head(pid)).await
}

/// Update `head` with a conditional write. `expected` is the ETag we read
/// before (`None` = head must not exist yet). Returns false on a race.
pub async fn write_head(
    control: &Control,
    pid: &str,
    head: &Head,
    expected: Option<&str>,
) -> Result<bool> {
    let key = keys::head(pid);
    let out = match expected {
        None => control.put_if_absent(&key, head).await?,
        Some(etag) => control.put_if_match(&key, head, etag).await?,
    };
    Ok(matches!(out, PutOutcome::Written { .. }))
}

pub fn ensure_active(head: &Head) -> Result<()> {
    if head.state == HeadState::Archived {
        return Err(Error::new(
            Code::Archived,
            format!("project {} is archived", head.project),
        )
        .with_hint("restore it with workroam archive restore"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use workroam_store::Store;

    fn dev(id: &str) -> DeviceRef {
        DeviceRef {
            id: id.into(),
            name: id.into(),
            os: None,
        }
    }

    #[tokio::test]
    async fn lease_lifecycle() {
        let d = tempfile::tempdir().unwrap();
        let c = Control::new(Store::local(d.path()), [1u8; 32]);
        let a = dev("a");
        let b = dev("b");
        assert!(matches!(
            acquire(&c, "p", &a).await.unwrap(),
            Acquire::Acquired
        ));
        assert!(matches!(
            acquire(&c, "p", &a).await.unwrap(),
            Acquire::AlreadyMine
        ));
        match acquire(&c, "p", &b).await.unwrap() {
            Acquire::HeldByOther { lease, stale } => {
                assert_eq!(lease.holder.id, "a");
                assert!(!stale);
            }
            other => panic!("{other:?}"),
        }
        assert!(renew(&c, "p", &a).await.unwrap());
        assert!(!renew(&c, "p", &b).await.unwrap());
        assert!(release(&c, "p", &a).await.unwrap());
        assert!(matches!(
            status(&c, "p", "b").await.unwrap(),
            LeaseStatus::Free
        ));
        assert!(matches!(
            acquire(&c, "p", &b).await.unwrap(),
            Acquire::Acquired
        ));
        let prev = takeover(&c, "p", &a).await.unwrap();
        assert_eq!(prev.unwrap().holder.id, "b");
        assert!(matches!(
            status(&c, "p", "a").await.unwrap(),
            LeaseStatus::Mine(_)
        ));
    }

    #[tokio::test]
    async fn head_races() {
        let d = tempfile::tempdir().unwrap();
        let c = Control::new(Store::local(d.path()), [2u8; 32]);
        let h = Head {
            project: "p".into(),
            snapshot: Some("s1".into()),
            parent: None,
            device_id: "a".into(),
            device_name: "a".into(),
            created_at: Utc::now(),
            state: HeadState::Active,
            summary: None,
        };
        assert!(write_head(&c, "p", &h, None).await.unwrap());
        assert!(!write_head(&c, "p", &h, None).await.unwrap());
        let (_, etag) = read_head(&c, "p").await.unwrap().unwrap();
        let mut h2 = h.clone();
        h2.snapshot = Some("s2".into());
        assert!(write_head(&c, "p", &h2, Some(&etag)).await.unwrap());
        assert!(!write_head(&c, "p", &h2, Some(&etag)).await.unwrap());
    }
}
