//! The open workspace: storage, control objects and the snapshot engine.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use workroam_core::config::{MachineConfig, WorkspaceLocation};
use workroam_core::model::{
    AccessKey, ArchiveRecord, CredentialsObject, DeviceRef, Head, HeadState, WorkspaceInfo,
    WorkspaceSettings,
};
use workroam_core::{Code, Error, Result};
use workroam_snapshot::rustic::{backends, RusticEngine};
use workroam_snapshot::SnapshotEngine;
use workroam_store::control::keys;
use workroam_store::{Control, Creds, Store};

/// Run blocking work (snapshot engine, git) from async code.
pub fn blocking<T>(f: impl FnOnce() -> T) -> T {
    match tokio::runtime::Handle::try_current() {
        Ok(h) if h.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(f)
        }
        _ => f(),
    }
}

#[derive(Clone)]
pub struct Workspace {
    pub machine: MachineConfig,
    pub creds: Option<Creds>,
    pub control: Control,
    pub snap: Arc<dyn SnapshotEngine>,
}

/// One line of the project list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectSummary {
    pub name: String,
    pub pid: String,
    pub head: Option<Head>,
    pub archived: bool,
    pub archive: Option<ArchiveRecord>,
}

pub fn control_key(snap: &dyn SnapshotEngine) -> workroam_core::crypto::Key {
    workroam_core::crypto::derive_key(&snap.master_key_material(), "control")
}

pub fn control_store(loc: &WorkspaceLocation, creds: Option<&Creds>) -> Result<Store> {
    Store::open(loc, creds, "control")
}

impl Workspace {
    pub fn device(&self) -> DeviceRef {
        DeviceRef {
            id: self.machine.device_id.clone(),
            name: self.machine.name.clone(),
            os: Some(self.machine.os),
        }
    }

    pub fn location(&self) -> &WorkspaceLocation {
        &self.machine.workspace
    }

    /// Open with the workspace password or the recovery key.
    pub async fn open(
        machine: MachineConfig,
        creds: Option<Creds>,
        secret: &str,
    ) -> Result<Workspace> {
        let loc = machine.workspace.clone();
        let be = backends(&loc, creds.as_ref())?;
        let snap = blocking(|| RusticEngine::open(be, secret)).map_err(|e| {
            if e.message.to_lowercase().contains("password")
                || e.message.to_lowercase().contains("key")
            {
                Error::new(Code::Denied, "wrong workspace password or recovery key")
                    .with_hint(e.message)
            } else {
                e
            }
        })?;
        let mut store = control_store(&loc, creds.as_ref())?;
        store.conditional = loc.capabilities.conditional();
        let control = Control::new(store, control_key(&snap));
        Ok(Workspace {
            machine,
            creds,
            control,
            snap: Arc::new(snap),
        })
    }

    /// Create the repository and the workspace control objects (setup "完成").
    pub async fn create(
        machine: MachineConfig,
        creds: Option<Creds>,
        password: &str,
        recovery: &str,
        key: Option<AccessKey>,
    ) -> Result<Workspace> {
        let loc = machine.workspace.clone();
        let be = backends(&loc, creds.as_ref())?;
        if blocking(|| RusticEngine::exists(&be))? {
            return Err(
                Error::invalid("this location already holds a Workroam workspace")
                    .with_hint("join it with a pairing code instead"),
            );
        }
        let snap = blocking(|| RusticEngine::init(be, password, recovery))?;
        let mut store = control_store(&loc, creds.as_ref())?;
        store.conditional = loc.capabilities.conditional();
        let info = WorkspaceInfo {
            layout: workroam_core::LAYOUT_VERSION,
            workspace_id: loc.workspace_id.clone(),
            created_at: Utc::now(),
            repo_id: snap.repo_id(),
            product: format!("workroam {}", workroam_core::VERSION),
        };
        store
            .put(keys::WORKSPACE, serde_json::to_vec_pretty(&info)?)
            .await?;
        let control = Control::new(store, control_key(&snap));
        control
            .put(keys::SETTINGS, &WorkspaceSettings::default())
            .await?;
        control
            .put(
                keys::CREDENTIALS,
                &CredentialsObject {
                    current: key,
                    ..Default::default()
                },
            )
            .await?;
        Ok(Workspace {
            machine,
            creds,
            control,
            snap: Arc::new(snap),
        })
    }

    /// Plain `control/workspace.json`, readable without the password.
    pub async fn read_info(
        loc: &WorkspaceLocation,
        creds: Option<&Creds>,
    ) -> Result<Option<WorkspaceInfo>> {
        let store = control_store(loc, creds)?;
        match store.get(keys::WORKSPACE).await? {
            Some(o) => Ok(serde_json::from_slice(&o.data).ok()),
            None => Ok(None),
        }
    }

    pub async fn settings(&self) -> Result<WorkspaceSettings> {
        Ok(self
            .control
            .get(keys::SETTINGS)
            .await?
            .map(|(s, _)| s)
            .unwrap_or_default())
    }

    pub async fn save_settings(&self, s: &WorkspaceSettings) -> Result<()> {
        self.control.put(keys::SETTINGS, s).await?;
        Ok(())
    }

    pub fn project_id(&self, name: &str) -> String {
        self.control.project_id(name)
    }

    /// All projects known to the workspace (from their heads).
    pub async fn projects(&self) -> Result<Vec<ProjectSummary>> {
        let keys_ = self.control.list(keys::PROJECTS).await?;
        let mut out = Vec::new();
        for k in keys_.iter().filter(|k| k.ends_with("/head")) {
            let pid = k
                .trim_start_matches(keys::PROJECTS)
                .trim_end_matches("/head")
                .to_string();
            let head = match self.control.get::<Head>(k).await {
                Ok(Some((h, _))) => h,
                _ => continue,
            };
            let archived = head.state == HeadState::Archived;
            let archive = if archived {
                self.control
                    .get::<ArchiveRecord>(&keys::archive(&pid))
                    .await
                    .ok()
                    .flatten()
                    .map(|(a, _)| a)
            } else {
                None
            };
            out.push(ProjectSummary {
                name: head.project.clone(),
                pid,
                head: Some(head),
                archived,
                archive,
            });
        }
        // Projects bound locally that were never pushed.
        for name in self.machine.projects.keys() {
            if !out.iter().any(|p| &p.name == name) {
                out.push(ProjectSummary {
                    name: name.clone(),
                    pid: self.project_id(name),
                    head: None,
                    archived: false,
                    archive: None,
                });
            }
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    pub async fn head(&self, name: &str) -> Result<Option<(Head, String)>> {
        workroam_lease::read_head(&self.control, &self.project_id(name)).await
    }

    pub fn is_read_only(&self) -> bool {
        self.machine.access == workroam_core::model::Access::ReadOnly
    }

    pub fn ensure_writable(&self) -> Result<()> {
        if self.is_read_only() {
            return Err(Error::new(
                Code::ReadOnlyDevice,
                "this device is read-only and cannot push",
            )
            .with_hint("pull, restore to another directory or export instead"));
        }
        Ok(())
    }
}
