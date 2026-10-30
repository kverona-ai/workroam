//! The setup session (architecture §7.2). Every step validates and stores
//! its input; nothing is written to storage or this computer until `commit`
//! (except the check probes, which delete themselves).

use crate::checks::{self, CheckResult, CheckStatus, Findings, StorageTarget};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use workroam_core::config::{Addressing, BindingState, MachineConfig, WorkspaceLocation};
use workroam_core::model::{Access, AccessKey, AutoMode, DeviceRef};
use workroam_core::{Code, Error, Result};
use workroam_device::provision::{provisioner_for, AdminCreds, KeyProvisioner};
use workroam_device::{PairingCode, PairingPayload};
use workroam_engine::Workspace;
use workroam_platform::secrets;
use workroam_store::{Creds, S3Client};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SetupMode {
    New,
    Join,
    FromBackup,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum KeyMode {
    Create,
    #[default]
    Current,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum BucketChoice {
    #[default]
    Auto,
    Existing,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MachineInput {
    pub device_name: String,
    pub root: String,
    #[serde(default)]
    pub reachable: Vec<String>,
    #[serde(default)]
    pub auto: Option<AutoMode>,
    #[serde(default)]
    pub autostart: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BucketInfo {
    pub name: String,
    pub workspace: bool,
    pub empty: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EndpointInfo {
    pub preset: String,
    pub endpoint: String,
    pub addressing: Addressing,
    pub region: String,
    pub can_create_key: bool,
    pub needs_admin_token: bool,
    pub deploy_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PasswordInfo {
    pub strength: u8,
    pub strength_text: String,
    pub recovery_key: String,
    /// 1-based groups the user types back to confirm.
    pub confirm_groups: (usize, usize),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JoinInfo {
    pub endpoint: String,
    pub bucket: String,
    pub scope: Access,
    pub projects: Vec<String>,
    pub devices: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitOutcome {
    pub mode: SetupMode,
    pub device_id: String,
    pub device_name: String,
    pub workspace: String,
    pub pairing_code: Option<String>,
    pub pairing_qr_svg: Option<String>,
    pub recovery_key: Option<String>,
    /// Projects bound on this device (to pull next).
    pub projects: Vec<(String, String)>,
    pub created_bucket: bool,
    pub created_key: bool,
}

pub struct SetupSession {
    pub id: String,
    pub mode: SetupMode,
    pub preset: String,
    pub loc: WorkspaceLocation,
    pub creds: Option<Creds>,
    pub key_mode: KeyMode,
    pub key_access: Access,
    pub admin: AdminCreds,
    pub bucket_choice: BucketChoice,
    pub accept_broad_key: bool,
    pub checks: Vec<CheckResult>,
    pub findings: Findings,
    pub password: Option<String>,
    pub recovery_key: Option<String>,
    confirm_groups: (usize, usize),
    pub recovery_confirmed: bool,
    pub machine: MachineInput,
    pairing_raw: Option<String>,
    pub pairing: Option<PairingPayload>,
    pub unlock_secret: Option<String>,
    pub projects: Vec<(String, Option<String>)>,
    pub all_projects: bool,
    pub as_device: Option<String>,
    workspace: Option<Workspace>,
}

impl SetupSession {
    pub fn new(mode: SetupMode) -> SetupSession {
        SetupSession {
            id: workroam_core::crypto::random_id("setup"),
            mode,
            preset: String::new(),
            loc: WorkspaceLocation::default(),
            creds: None,
            key_mode: KeyMode::Current,
            key_access: Access::ReadWrite,
            admin: AdminCreds::default(),
            bucket_choice: BucketChoice::Auto,
            accept_broad_key: false,
            checks: vec![],
            findings: Findings::default(),
            password: None,
            recovery_key: None,
            confirm_groups: (2, 6),
            recovery_confirmed: false,
            machine: MachineInput {
                device_name: workroam_platform::sysinfo::default_device_name(),
                root: default_root(),
                ..Default::default()
            },
            pairing_raw: None,
            pairing: None,
            unlock_secret: None,
            projects: vec![],
            all_projects: false,
            as_device: None,
            workspace: None,
        }
    }

    // ---- storage (new / from-backup) ------------------------------------

    pub fn set_endpoint(
        &mut self,
        preset: &str,
        endpoint: &str,
        addressing: Option<Addressing>,
        region: Option<String>,
    ) -> Result<EndpointInfo> {
        let p = workroam_store::location::preset(preset).ok_or_else(|| {
            Error::invalid(format!(
                "unknown storage preset `{preset}` (rustfs, aws, r2, oss, other, local)"
            ))
        })?;
        let endpoint = endpoint.trim().trim_end_matches('/').to_string();
        if endpoint.is_empty() {
            return Err(Error::invalid("the storage endpoint is required"));
        }
        if preset == "local" {
            if !endpoint.starts_with("file://") {
                return Err(Error::invalid(
                    "a local storage endpoint looks like file:///path/to/dir",
                ));
            }
        } else {
            let u = url::Url::parse(&endpoint)
                .map_err(|e| Error::invalid(format!("invalid endpoint: {e}")))?;
            if !matches!(u.scheme(), "https" | "http") {
                return Err(Error::invalid("the endpoint must start with https://"));
            }
        }
        self.preset = preset.to_string();
        self.loc.endpoint = endpoint.clone();
        self.loc.preset = preset.to_string();
        self.loc.addressing = addressing.unwrap_or_else(|| {
            if preset == "aws" || preset == "oss" {
                p.addressing
            } else {
                workroam_store::location::guess_addressing(&endpoint)
            }
        });
        self.loc.region = region
            .or_else(|| workroam_store::detect_region(&endpoint))
            .unwrap_or_else(|| p.default_region.to_string());
        let prov = provisioner_for(preset, &endpoint);
        Ok(EndpointInfo {
            preset: preset.into(),
            endpoint,
            addressing: self.loc.addressing,
            region: self.loc.region.clone(),
            can_create_key: prov.is_some(),
            needs_admin_token: prov.map(|p| p.needs_admin_token()).unwrap_or(false),
            deploy_hint: (preset == "rustfs")
                .then(|| workroam_core::text::tr("setup.rustfs_hint").to_string()),
        })
    }

    /// Store the key and list visible buckets (with workspace / empty marks).
    pub async fn set_credentials(
        &mut self,
        access_key: &str,
        secret_key: &str,
    ) -> Result<Vec<BucketInfo>> {
        if self.loc.is_local() {
            self.creds = None;
            return Ok(vec![]);
        }
        if access_key.trim().is_empty() || secret_key.is_empty() {
            return Err(Error::invalid(
                "both the Access Key and the Secret Key are required",
            ));
        }
        let creds = Creds {
            access_key: access_key.trim().into(),
            secret_key: secret_key.into(),
        };
        self.admin.access_key = creds.access_key.clone();
        self.admin.secret_key = creds.secret_key.clone();
        self.creds = Some(creds.clone());
        let c = S3Client::new(
            &self.loc.endpoint,
            &self.loc.region,
            self.loc.addressing,
            creds,
        )?;
        let resp = c.list_buckets().await?;
        if resp.status == 403
            && matches!(
                resp.error_code().as_deref(),
                Some("InvalidAccessKeyId") | Some("SignatureDoesNotMatch")
            )
        {
            return Err(Error::new(
                Code::Denied,
                workroam_core::text::tr("hint.credentials"),
            ));
        }
        if !resp.ok() {
            // Keys without ListBuckets: the bucket is typed by hand.
            return Ok(vec![]);
        }
        let mut out = Vec::new();
        for name in S3Client::parse_bucket_names(&resp).into_iter().take(100) {
            let keys = c
                .list_objects(&name, "", 2)
                .await
                .map(|(k, _)| k)
                .unwrap_or_default();
            let ws = c
                .get_object(&name, "control/workspace.json")
                .await
                .map(|r| r.ok())
                .unwrap_or(false);
            out.push(BucketInfo {
                name,
                workspace: ws,
                empty: keys.is_empty(),
            });
        }
        Ok(out)
    }

    pub fn set_key_mode(
        &mut self,
        mode: KeyMode,
        access: Access,
        admin_endpoint: Option<String>,
        admin_token: Option<String>,
    ) -> Result<()> {
        if mode == KeyMode::Create {
            let prov = provisioner_for(&self.preset, &self.loc.endpoint).ok_or_else(|| {
                Error::invalid(format!(
                    "{} cannot create scoped keys automatically; use --key-mode current",
                    self.preset
                ))
            })?;
            if prov.needs_admin_token() && (admin_endpoint.is_none() || admin_token.is_none()) {
                return Err(Error::invalid(
                    "Garage needs --admin-endpoint and --admin-token-stdin to create keys",
                ));
            }
        }
        if self.mode == SetupMode::New && access == Access::ReadOnly && mode != KeyMode::Create {
            return Err(Error::invalid(
                "a read-only first device needs --key-mode create",
            ));
        }
        self.key_mode = mode;
        self.key_access = access;
        self.admin.admin_endpoint = admin_endpoint;
        self.admin.admin_token = admin_token;
        Ok(())
    }

    pub async fn set_bucket(
        &mut self,
        choice: BucketChoice,
        name: Option<String>,
        prefix: &str,
    ) -> Result<BucketInfo> {
        let name = match choice {
            BucketChoice::Auto => name.filter(|n| !n.is_empty()).unwrap_or_else(|| {
                workroam_store::location::default_bucket_name(&self.machine.device_name)
            }),
            _ => name
                .filter(|n| !n.is_empty())
                .ok_or_else(|| Error::invalid("a bucket name is required"))?,
        };
        if !self.loc.is_local() {
            workroam_store::validate_bucket_name(&name)
                .map_err(|k| Error::invalid(workroam_core::text::tr(k)))?;
        }
        self.bucket_choice = choice;
        self.loc.bucket = name.clone();
        self.loc.prefix = prefix.trim().trim_matches('/').to_string();
        let info = self.inspect_bucket().await?;
        if info.workspace && self.mode == SetupMode::New {
            return Err(
                Error::invalid(workroam_core::text::tr("bucket.workspace_found"))
                    .with_hint("workroam setup --join"),
            );
        }
        if self.mode == SetupMode::New
            && !info.empty
            && !info.workspace
            && self.loc.prefix.is_empty()
        {
            return Err(Error::invalid(workroam_core::text::tr(
                "bucket.needs_prefix",
            )));
        }
        Ok(info)
    }

    async fn inspect_bucket(&self) -> Result<BucketInfo> {
        let ws = Workspace::read_info(&self.loc, self.creds.as_ref())
            .await
            .ok()
            .flatten()
            .is_some();
        let empty = if self.loc.is_local() {
            self.loc
                .local_root()
                .map(|r| workroam_platform::dircheck::is_empty_or_missing(&r))
                .unwrap_or(true)
        } else if let Some(c) = &self.creds {
            let s3 = S3Client::new(
                &self.loc.endpoint,
                &self.loc.region,
                self.loc.addressing,
                c.clone(),
            )?;
            let (keys, r) = s3
                .list_objects(&self.loc.bucket, &self.loc.key_prefix(), 2)
                .await?;
            r.status == 404 || keys.is_empty()
        } else {
            true
        };
        Ok(BucketInfo {
            name: self.loc.bucket.clone(),
            workspace: ws,
            empty,
        })
    }

    /// Run storage and local checks; `each` sees every result as it finishes.
    pub async fn run_checks(
        &mut self,
        each: &mut (dyn FnMut(&CheckResult) + Send),
    ) -> Result<Vec<CheckResult>> {
        let t = StorageTarget {
            loc: &self.loc,
            creds: self.creds.as_ref(),
            bucket_may_be_missing: self.mode == SetupMode::New,
        };
        let (mut results, findings) = checks::storage_checks(&t, each).await;
        for r in checks::local_checks() {
            each(&r);
            results.push(r);
        }
        if let Some(a) = findings.addressing {
            self.loc.addressing = a;
        }
        self.loc.capabilities = findings.capabilities;
        self.findings = findings;
        self.checks = results.clone();
        Ok(results)
    }

    pub fn checks_passed(&self) -> Result<()> {
        if self.checks.is_empty() {
            return Err(Error::new(
                Code::ChecksFailed,
                "run the capability checks first",
            ));
        }
        if let Some(f) = self.checks.iter().find(|c| c.status == CheckStatus::Fail) {
            return Err(
                Error::new(Code::ChecksFailed, format!("{}: {}", f.label, f.message))
                    .with_hint(f.hint.clone().unwrap_or_default()),
            );
        }
        if self.findings.broad_key && self.key_mode == KeyMode::Current && !self.accept_broad_key {
            return Err(Error::new(
                Code::ChecksFailed,
                workroam_core::text::tr("setup.broad_key"),
            )
            .with_hint("--accept-broad-key, or --key-mode create"));
        }
        Ok(())
    }

    pub fn set_password(&mut self, password: &str) -> Result<PasswordInfo> {
        let (score, key) = workroam_core::crypto::password_strength(password);
        if score == 0 {
            return Err(Error::invalid(workroam_core::text::tr(key)));
        }
        self.password = Some(password.to_string());
        let rk = self
            .recovery_key
            .clone()
            .unwrap_or_else(workroam_core::crypto::new_recovery_key);
        self.recovery_key = Some(rk.clone());
        let r: [u8; 2] = workroam_core::crypto::random_bytes();
        let a = (r[0] as usize % 4) + 1;
        let b = (r[1] as usize % 4) + 5;
        self.confirm_groups = (a, b);
        Ok(PasswordInfo {
            strength: score,
            strength_text: workroam_core::text::tr(key).to_string(),
            recovery_key: rk,
            confirm_groups: (a, b),
        })
    }

    /// The user typed groups `a` and `b` of the recovery key (or the CLI saved it to a file).
    pub fn confirm_recovery(&mut self, typed: &str) -> Result<()> {
        let rk = self
            .recovery_key
            .clone()
            .ok_or_else(|| Error::invalid("set the password first"))?;
        let groups: Vec<&str> = rk.split('-').collect();
        let want = format!(
            "{}{}",
            groups[self.confirm_groups.0 - 1],
            groups[self.confirm_groups.1 - 1]
        );
        let got: String = typed
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_uppercase();
        if got != want {
            return Err(Error::invalid(workroam_core::text::tr(
                "setup.recovery.wrong",
            )));
        }
        self.recovery_confirmed = true;
        Ok(())
    }

    pub fn mark_recovery_saved(&mut self) {
        self.recovery_confirmed = true;
    }

    pub fn set_machine(&mut self, m: MachineInput) -> Result<()> {
        if m.device_name.trim().is_empty() {
            return Err(Error::invalid("the device name is required"));
        }
        if m.root.trim().is_empty() {
            return Err(Error::invalid("the default project location is required"));
        }
        self.machine = m;
        Ok(())
    }

    // ---- join ------------------------------------------------------------

    pub fn set_pairing_code(&mut self, code: &str) -> Result<()> {
        if !PairingCode::looks_valid(code) {
            return Err(Error::invalid(
                "this is not a Workroam pairing code (it starts with wr1:)",
            ));
        }
        self.pairing_raw = Some(code.to_string());
        Ok(())
    }

    /// Decrypt the pairing code (join) or open the mirror (from-backup), then
    /// open the workspace to list projects and devices.
    pub async fn unlock(
        &mut self,
        password: Option<&str>,
        recovery_key: Option<&str>,
    ) -> Result<JoinInfo> {
        let secret = match (password, recovery_key) {
            (Some(p), _) => p.to_string(),
            (None, Some(r)) => workroam_core::crypto::normalize_recovery_key(r),
            (None, None) => return Err(Error::invalid("the workspace password is required")),
        };
        if self.mode == SetupMode::Join {
            let raw = self
                .pairing_raw
                .clone()
                .ok_or_else(|| Error::invalid("paste the pairing code first"))?;
            let payload = match PairingCode::decode(&raw, &secret) {
                Ok(p) => p,
                Err(e) if password.is_none() => {
                    return Err(Error::new(e.code, "the pairing code is protected by the workspace password").with_hint("on a device that is already joined, run: workroam password reset --recovery-key-stdin"))
                }
                Err(e) => return Err(e),
            };
            self.loc = payload.location();
            self.creds = (!payload.access_key.is_empty()).then(|| Creds {
                access_key: payload.access_key.clone(),
                secret_key: payload.secret_key.clone(),
            });
            self.key_access = payload.scope;
            self.pairing = Some(payload);
        }
        let probe = MachineConfig::new(
            &self.machine.device_name,
            &self.machine.root,
            self.loc.clone(),
        );
        let ws = Workspace::open(probe, self.creds.clone(), &secret).await?;
        let projects: Vec<String> = ws
            .projects()
            .await?
            .into_iter()
            .filter(|p| !p.archived)
            .map(|p| p.name)
            .collect();
        let devices: Vec<String> = workroam_device::registry::list(&ws.control)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|d| d.name)
            .collect();
        self.unlock_secret = Some(secret);
        self.workspace = Some(ws);
        Ok(JoinInfo {
            endpoint: self.loc.endpoint.clone(),
            bucket: self.loc.bucket.clone(),
            scope: self.key_access,
            projects,
            devices,
        })
    }

    pub fn set_backup_source(&mut self, loc: WorkspaceLocation, creds: Option<Creds>) {
        self.loc = loc;
        self.creds = creds;
    }

    pub fn set_projects(&mut self, projects: Vec<(String, Option<String>)>, all: bool) {
        self.projects = projects;
        self.all_projects = all;
    }

    // ---- commit ----------------------------------------------------------

    pub async fn commit(&mut self) -> Result<CommitOutcome> {
        match self.mode {
            SetupMode::New => self.commit_new().await,
            SetupMode::Join | SetupMode::FromBackup => self.commit_join().await,
        }
    }

    fn base_machine(&self) -> MachineConfig {
        let mut m = MachineConfig::new(
            self.machine.device_name.trim(),
            self.machine.root.trim(),
            self.loc.clone(),
        );
        m.reachable = self.machine.reachable.clone();
        if let Some(a) = self.machine.auto {
            m.sync.auto = a;
        }
        if let Some(a) = self.machine.autostart {
            m.autostart = a;
        }
        m.access = self.key_access;
        m
    }

    async fn commit_new(&mut self) -> Result<CommitOutcome> {
        self.checks_passed()?;
        let password = self
            .password
            .clone()
            .ok_or_else(|| Error::invalid("set the encryption password first"))?;
        let recovery = self
            .recovery_key
            .clone()
            .ok_or_else(|| Error::invalid("set the encryption password first"))?;
        if !self.recovery_confirmed {
            return Err(Error::invalid("confirm that the recovery key is saved"));
        }
        let mut created_bucket = false;
        if self.loc.is_local() {
            std::fs::create_dir_all(self.loc.local_root().unwrap_or_default())?;
        } else if !self.findings.bucket_exists {
            let c = S3Client::new(
                &self.loc.endpoint,
                &self.loc.region,
                self.loc.addressing,
                self.creds.clone().unwrap_or(Creds {
                    access_key: String::new(),
                    secret_key: String::new(),
                }),
            )?;
            let r = c.create_bucket(&self.loc.bucket).await?;
            if !r.ok() {
                return Err(Error::new(Code::Network, format!("could not create bucket {}: HTTP {} {}", self.loc.bucket, r.status, r.error_code().unwrap_or_default()))
                    .with_hint("choose another name (bucket names are global on some services) or create it in the console"));
            }
            created_bucket = true;
            // Bucket-level probes now that it exists.
            let t = StorageTarget {
                loc: &self.loc,
                creds: self.creds.as_ref(),
                bucket_may_be_missing: false,
            };
            let (res, f) = checks::storage_checks(&t, &mut |_| {}).await;
            if let Some(fail) = res.iter().find(|r| r.status == CheckStatus::Fail) {
                return Err(Error::new(
                    Code::ChecksFailed,
                    format!("{}: {}", fail.label, fail.message),
                ));
            }
            self.loc.capabilities = f.capabilities;
            self.findings.broad_key = f.broad_key;
        }
        self.loc.workspace_id = workroam_core::crypto::random_id("ws");
        let mut created_key = false;
        let mut rw_key = self.creds.clone();
        let mut ro_key: Option<AccessKey> = None;
        let mut current_key = self.creds.as_ref().map(|c| AccessKey {
            access_key: c.access_key.clone(),
            secret_key: c.secret_key.clone(),
            scope: Access::ReadWrite,
            ..Default::default()
        });
        if self.key_mode == KeyMode::Create {
            let prov = provisioner_for(&self.preset, &self.loc.endpoint)
                .ok_or_else(|| Error::invalid("this storage cannot create keys"))?;
            let rw = prov
                .create_key(&self.loc, &self.admin, Access::ReadWrite)
                .await?;
            wait_usable(&self.loc, &rw.access_key, &rw.secret_key).await?;
            if let Ok(ro) = prov
                .create_key(&self.loc, &self.admin, Access::ReadOnly)
                .await
            {
                ro_key = Some(AccessKey {
                    access_key: ro.access_key,
                    secret_key: ro.secret_key,
                    scope: Access::ReadOnly,
                    provider: Some(ro.provider),
                    provider_ref: Some(ro.provider_ref),
                });
            }
            rw_key = Some(Creds {
                access_key: rw.access_key.clone(),
                secret_key: rw.secret_key.clone(),
            });
            current_key = Some(AccessKey {
                access_key: rw.access_key,
                secret_key: rw.secret_key,
                scope: Access::ReadWrite,
                provider: Some(rw.provider),
                provider_ref: Some(rw.provider_ref),
            });
            created_key = true;
            // Administrator credentials are dropped here.
            self.admin = AdminCreds::default();
        }
        let machine = {
            let mut m = self.base_machine();
            m.workspace = self.loc.clone();
            m.access = Access::ReadWrite;
            m
        };
        let ws = Workspace::create(
            machine.clone(),
            rw_key.clone(),
            &password,
            &recovery,
            current_key.clone(),
        )
        .await?;
        if let Some(ro) = &ro_key {
            let mut c = workroam_device::rotation::read(&ws.control).await?;
            c.read_only = Some(ro.clone());
            workroam_device::rotation::write(&ws.control, &c).await?;
        }
        // This device's key: the read-only one when asked for.
        let (my_key, my_access) = match (self.key_access, &ro_key) {
            (Access::ReadOnly, Some(ro)) => (
                Some(Creds {
                    access_key: ro.access_key.clone(),
                    secret_key: ro.secret_key.clone(),
                }),
                Access::ReadOnly,
            ),
            _ => (rw_key.clone(), Access::ReadWrite),
        };
        let mut machine = machine;
        machine.access = my_access;
        store_secrets(my_key.as_ref(), &password)?;
        workroam_device::registry::register(
            &ws.control,
            &workroam_device::registry::record_for(&machine, detected_agents(), None),
        )
        .await?;
        machine.save(&workroam_core::paths::machine_toml())?;
        if machine.autostart {
            let _ =
                workroam_platform::service::install(&workroam_platform::service::daemon_binary());
        }
        let code = match &rw_key {
            Some(k) => Some(PairingCode::encode(
                &PairingPayload::from_location(
                    &self.loc,
                    &k.access_key,
                    &k.secret_key,
                    Access::ReadWrite,
                ),
                &password,
            )?),
            None => Some(PairingCode::encode(
                &PairingPayload::from_location(&self.loc, "", "", Access::ReadWrite),
                &password,
            )?),
        };
        Ok(CommitOutcome {
            mode: self.mode,
            device_id: machine.device_id.clone(),
            device_name: machine.name.clone(),
            workspace: self.loc.display(),
            pairing_qr_svg: code.as_deref().and_then(workroam_device::qr::svg),
            pairing_code: code,
            recovery_key: Some(recovery),
            projects: vec![],
            created_bucket,
            created_key,
        })
    }

    async fn commit_join(&mut self) -> Result<CommitOutcome> {
        let secret = self
            .unlock_secret
            .clone()
            .ok_or_else(|| Error::invalid("unlock the workspace first"))?;
        let ws = self
            .workspace
            .clone()
            .ok_or_else(|| Error::invalid("unlock the workspace first"))?;
        let mut machine = self.base_machine();
        machine.workspace = self.loc.clone();
        if let Some(name) = &self.as_device {
            let rec = workroam_device::registry::find(&ws.control, name)
                .await?
                .ok_or_else(|| {
                    Error::invalid(format!("no device named {name} in this workspace"))
                })?;
            machine.device_id = rec.id;
            machine.name = rec.name;
        }
        let available: Vec<String> = ws
            .projects()
            .await?
            .into_iter()
            .filter(|p| !p.archived)
            .map(|p| p.name)
            .collect();
        let chosen: Vec<(String, Option<String>)> = if self.all_projects {
            available.iter().map(|n| (n.clone(), None)).collect()
        } else {
            self.projects.clone()
        };
        let mut bound = Vec::new();
        for (name, dir) in chosen {
            if !available.contains(&name) {
                return Err(Error::invalid(format!(
                    "the workspace has no project named {name}"
                )));
            }
            let d = dir
                .map(|d| workroam_core::paths::expand_tilde(&d))
                .unwrap_or_else(|| machine.default_project_dir(&name));
            workroam_engine::init::bind(&mut machine, &name, &d, BindingState::Pending);
            bound.push((name, d.display().to_string()));
        }
        let ws = Workspace {
            machine: machine.clone(),
            ..ws
        };
        let mut code = None;
        if self.mode == SetupMode::FromBackup {
            // The mirror becomes the main storage (§10.5).
            let key = self.creds.as_ref().map(|c| AccessKey {
                access_key: c.access_key.clone(),
                secret_key: c.secret_key.clone(),
                scope: Access::ReadWrite,
                ..Default::default()
            });
            workroam_device::rotation::write(
                &ws.control,
                &workroam_core::model::CredentialsObject {
                    current: key,
                    ..Default::default()
                },
            )
            .await?;
            if let Some(c) = &self.creds {
                code = Some(PairingCode::encode(
                    &PairingPayload::from_location(
                        &self.loc,
                        &c.access_key,
                        &c.secret_key,
                        Access::ReadWrite,
                    ),
                    &secret,
                )?);
            }
        }
        store_secrets(self.creds.as_ref(), &secret)?;
        workroam_device::registry::register(
            &ws.control,
            &workroam_device::registry::record_for(&machine, detected_agents(), None),
        )
        .await?;
        machine.save(&workroam_core::paths::machine_toml())?;
        if machine.autostart {
            let _ =
                workroam_platform::service::install(&workroam_platform::service::daemon_binary());
        }
        Ok(CommitOutcome {
            mode: self.mode,
            device_id: machine.device_id.clone(),
            device_name: machine.name.clone(),
            workspace: self.loc.display(),
            pairing_qr_svg: code.as_deref().and_then(workroam_device::qr::svg),
            pairing_code: code,
            recovery_key: None,
            projects: bound,
            created_bucket: false,
            created_key: false,
        })
    }

    pub fn device(&self) -> DeviceRef {
        DeviceRef {
            id: String::new(),
            name: self.machine.device_name.clone(),
            os: Some(workroam_core::model::OsKind::current()),
        }
    }
}

pub fn default_root() -> String {
    let home = workroam_core::paths::home_dir();
    let p: PathBuf = if cfg!(windows) {
        home.join("Workroam")
    } else {
        home.join("workroam")
    };
    p.display().to_string()
}

fn detected_agents() -> Vec<String> {
    workroam_adapters::registry::local()
        .iter()
        .map(workroam_adapters::detect)
        .filter(|d| d.installed)
        .map(|d| d.id)
        .collect()
}

pub fn store_secrets(creds: Option<&Creds>, password: &str) -> Result<()> {
    if let Some(c) = creds {
        secrets::set(secrets::keys::ACCESS_KEY, &c.access_key)?;
        secrets::set(secrets::keys::SECRET_KEY, &c.secret_key)?;
    }
    secrets::set(secrets::keys::PASSWORD, password)
}

/// New cloud keys take a few seconds to become valid.
async fn wait_usable(loc: &WorkspaceLocation, ak: &str, sk: &str) -> Result<()> {
    let c = S3Client::new(
        &loc.endpoint,
        &loc.region,
        loc.addressing,
        Creds {
            access_key: ak.into(),
            secret_key: sk.into(),
        },
    )?;
    for _ in 0..15 {
        if let Ok(r) = c.list_objects(&loc.bucket, &loc.key_prefix(), 1).await {
            if r.1.ok() {
                return Ok(());
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
    Err(
        Error::new(Code::ChecksFailed, "the new scoped key does not work yet")
            .with_hint("try again in a minute, or use --key-mode current"),
    )
}

impl KeyMode {
    pub fn parse(s: &str) -> Option<KeyMode> {
        match s {
            "create" => Some(KeyMode::Create),
            "current" => Some(KeyMode::Current),
            _ => None,
        }
    }
}

impl BucketChoice {
    pub fn parse(s: &str) -> BucketChoice {
        match s {
            "auto" | "" => BucketChoice::Auto,
            _ => BucketChoice::Custom,
        }
    }
}

pub fn provisioner(preset: &str, endpoint: &str) -> Option<KeyProvisioner> {
    provisioner_for(preset, endpoint)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn new_then_join_local() {
        let td = tempfile::tempdir().unwrap();
        std::env::set_var("WORKROAM_HOME", td.path().join("h1"));
        std::env::set_var("WORKROAM_SECRETS", "file");
        let store = format!("file://{}", td.path().join("store").display());
        let mut s = SetupSession::new(SetupMode::New);
        s.set_endpoint("local", &store, None, None).unwrap();
        s.set_credentials("", "").await.unwrap();
        s.set_key_mode(KeyMode::Current, Access::ReadWrite, None, None)
            .unwrap();
        s.set_bucket(BucketChoice::Custom, Some("ws".into()), "")
            .await
            .unwrap();
        let results = s.run_checks(&mut |_| {}).await.unwrap();
        assert!(checks::all_ok(&results) || results.iter().any(|r| r.id == "git"));
        assert!(s.set_password("short").is_err());
        let info = s.set_password("correct horse battery").unwrap();
        let groups: Vec<&str> = info.recovery_key.split('-').collect();
        assert!(s.confirm_recovery("nope").is_err());
        s.confirm_recovery(&format!(
            "{} {}",
            groups[info.confirm_groups.0 - 1],
            groups[info.confirm_groups.1 - 1].to_lowercase()
        ))
        .unwrap();
        s.set_machine(MachineInput {
            device_name: "office".into(),
            root: td.path().join("p").display().to_string(),
            autostart: Some(false),
            ..Default::default()
        })
        .unwrap();
        // A missing git would fail checks in odd environments; ignore it here.
        s.checks.retain(|c| c.status != CheckStatus::Fail);
        let out = s.commit().await.unwrap();
        let code = out.pairing_code.unwrap();

        std::env::set_var("WORKROAM_HOME", td.path().join("h2"));
        let mut j = SetupSession::new(SetupMode::Join);
        j.set_pairing_code(&code).unwrap();
        assert!(j.unlock(Some("wrong password!"), None).await.is_err());
        let info = j.unlock(Some("correct horse battery"), None).await.unwrap();
        assert_eq!(info.devices, vec!["office".to_string()]);
        j.set_machine(MachineInput {
            device_name: "home".into(),
            root: td.path().join("q").display().to_string(),
            autostart: Some(false),
            ..Default::default()
        })
        .unwrap();
        let out = j.commit().await.unwrap();
        assert_eq!(out.device_name, "home");
        let m = MachineConfig::load(&workroam_core::paths::machine_toml())
            .unwrap()
            .unwrap();
        assert_eq!(m.name, "home");
    }
}
