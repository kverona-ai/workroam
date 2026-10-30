//! Pairing codes: `wr1:` + Base32(salt ‖ nonce ‖ XChaCha20-Poly1305 ciphertext),
//! keyed by scrypt(workspace password, salt) (architecture §8.1).

use data_encoding::BASE32_NOPAD;
use serde::{Deserialize, Serialize};
use workroam_core::config::{Addressing, Capabilities, WorkspaceLocation};
use workroam_core::model::Access;
use workroam_core::{Code, Error, Result};

pub const PREFIX: &str = "wr1:";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PairingPayload {
    pub endpoint: String,
    pub bucket: String,
    #[serde(default)]
    pub prefix: String,
    pub addressing: Addressing,
    #[serde(default)]
    pub region: String,
    pub access_key: String,
    pub secret_key: String,
    pub scope: Access,
    pub workspace_id: String,
    #[serde(default)]
    pub preset: String,
    #[serde(default)]
    pub capabilities: Capabilities,
}

impl PairingPayload {
    pub fn from_location(
        loc: &WorkspaceLocation,
        access_key: &str,
        secret_key: &str,
        scope: Access,
    ) -> Self {
        PairingPayload {
            endpoint: loc.endpoint.clone(),
            bucket: loc.bucket.clone(),
            prefix: loc.prefix.clone(),
            addressing: loc.addressing,
            region: loc.region.clone(),
            access_key: access_key.into(),
            secret_key: secret_key.into(),
            scope,
            workspace_id: loc.workspace_id.clone(),
            preset: loc.preset.clone(),
            capabilities: loc.capabilities,
        }
    }

    pub fn location(&self) -> WorkspaceLocation {
        WorkspaceLocation {
            endpoint: self.endpoint.clone(),
            bucket: self.bucket.clone(),
            prefix: self.prefix.clone(),
            addressing: self.addressing,
            region: self.region.clone(),
            capabilities: self.capabilities,
            workspace_id: self.workspace_id.clone(),
            preset: self.preset.clone(),
        }
    }
}

pub struct PairingCode;

impl PairingCode {
    pub fn encode(payload: &PairingPayload, password: &str) -> Result<String> {
        let plain = serde_json::to_vec(payload)?;
        let sealed = workroam_core::crypto::password_seal_raw(password, &plain);
        Ok(format!("{PREFIX}{}", BASE32_NOPAD.encode(&sealed)))
    }

    /// Whitespace, line breaks and lowercase are tolerated (copied from a terminal).
    pub fn decode(code: &str, password: &str) -> Result<PairingPayload> {
        let raw = Self::raw(code)?;
        let plain = workroam_core::crypto::password_open_raw(password, &raw).map_err(|_| {
            Error::new(
                Code::Denied,
                "the pairing code does not match this password",
            )
            .with_hint("check the workspace password, or generate a new pairing code")
        })?;
        Ok(serde_json::from_slice(&plain)?)
    }

    /// Syntax check without the password.
    pub fn looks_valid(code: &str) -> bool {
        Self::raw(code).map(|r| r.len() > 40 + 16).unwrap_or(false)
    }

    fn raw(code: &str) -> Result<Vec<u8>> {
        let c: String = code.chars().filter(|c| !c.is_whitespace()).collect();
        let body = c
            .strip_prefix(PREFIX)
            .or_else(|| c.strip_prefix("WR1:"))
            .ok_or_else(|| Error::invalid("a pairing code starts with wr1:"))?;
        BASE32_NOPAD
            .decode(body.to_ascii_uppercase().as_bytes())
            .map_err(|_| Error::invalid("the pairing code is damaged; copy it again"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let p = PairingPayload {
            endpoint: "https://s3.example.com".into(),
            bucket: "workroam-office-ab12".into(),
            prefix: String::new(),
            addressing: Addressing::Path,
            region: "us-east-1".into(),
            access_key: "AK".into(),
            secret_key: "SK".into(),
            scope: Access::ReadWrite,
            workspace_id: "ws_1".into(),
            preset: "rustfs".into(),
            capabilities: Capabilities::default(),
        };
        let code = PairingCode::encode(&p, "correct horse").unwrap();
        assert!(code.starts_with("wr1:"));
        assert!(PairingCode::looks_valid(&code));
        let messy = format!("  {}\n{} ", &code[..30], code[30..].to_lowercase());
        assert_eq!(PairingCode::decode(&messy, "correct horse").unwrap(), p);
        assert_eq!(
            PairingCode::decode(&code, "wrong").unwrap_err().code,
            Code::Denied
        );
    }
}
