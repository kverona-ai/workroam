//! Encrypted control objects (architecture §5, §6.6). Every object is
//! sealed with the control sub key and bound to its key path.

use crate::object::{PutOutcome, Store};
use serde::de::DeserializeOwned;
use serde::Serialize;
use workroam_core::crypto::{self, Key};
use workroam_core::Result;

#[derive(Clone)]
pub struct Control {
    pub store: Store,
    key: Key,
}

impl std::fmt::Debug for Control {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Control({:?})", self.store)
    }
}

fn aad(key: &str) -> String {
    format!("control/{key}")
}

impl Control {
    pub fn new(store: Store, key: Key) -> Control {
        Control { store, key }
    }

    pub fn key(&self) -> &Key {
        &self.key
    }

    pub fn project_id(&self, name: &str) -> String {
        crypto::project_id(&self.key, name)
    }

    pub async fn get<T: DeserializeOwned>(&self, key: &str) -> Result<Option<(T, String)>> {
        match self.store.get(key).await? {
            Some(o) => Ok(Some((
                crypto::open_json(&self.key, &aad(key), &o.data)?,
                o.etag,
            ))),
            None => Ok(None),
        }
    }

    pub async fn put<T: Serialize>(&self, key: &str, value: &T) -> Result<String> {
        self.store
            .put(key, crypto::seal_json(&self.key, &aad(key), value)?)
            .await
    }

    pub async fn put_if_absent<T: Serialize>(&self, key: &str, value: &T) -> Result<PutOutcome> {
        self.store
            .put_if_absent(key, crypto::seal_json(&self.key, &aad(key), value)?)
            .await
    }

    pub async fn put_if_match<T: Serialize>(
        &self,
        key: &str,
        value: &T,
        etag: &str,
    ) -> Result<PutOutcome> {
        self.store
            .put_if_match(key, crypto::seal_json(&self.key, &aad(key), value)?, etag)
            .await
    }

    pub async fn delete(&self, key: &str) -> Result<()> {
        self.store.delete(key).await
    }

    pub async fn list(&self, prefix: &str) -> Result<Vec<String>> {
        self.store.list(prefix).await
    }

    /// Decrypt every object under `prefix`, skipping ones that fail to decrypt.
    pub async fn get_all<T: DeserializeOwned>(&self, prefix: &str) -> Result<Vec<(String, T)>> {
        let mut out = Vec::new();
        for k in self.list(prefix).await? {
            match self.get::<T>(&k).await {
                Ok(Some((v, _))) => out.push((k, v)),
                Ok(None) => {}
                Err(e) => tracing::warn!("skipping control object {k}: {e}"),
            }
        }
        Ok(out)
    }
}

/// Well-known control object keys.
pub mod keys {
    pub const WORKSPACE: &str = "workspace.json";
    pub const SETTINGS: &str = "settings";
    pub const CREDENTIALS: &str = "credentials";
    pub fn device(id: &str) -> String {
        format!("devices/{id}")
    }
    pub const DEVICES: &str = "devices/";
    pub const PROJECTS: &str = "projects/";
    pub fn lease(pid: &str) -> String {
        format!("projects/{pid}/lease")
    }
    pub fn head(pid: &str) -> String {
        format!("projects/{pid}/head")
    }
    pub fn archive(pid: &str) -> String {
        format!("projects/{pid}/archive")
    }
    pub const GRANTS: &str = "grants/";
    pub fn grant(id: &str) -> String {
        format!("grants/{id}")
    }
    pub const RELAYS: &str = "relays/";
    pub fn relay(id: &str) -> String {
        format!("relays/{id}")
    }
}
