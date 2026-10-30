//! Errors and the stable CLI exit codes (architecture §15).

use serde::{Deserialize, Serialize};
use std::fmt;

/// Stable result codes. The numeric value is the CLI exit code and the
/// `code` field in JSON output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Code {
    Ok,
    InvalidArgs,
    Denied,
    ApprovalTimeout,
    LeaseHeld,
    CrossPlatform,
    Conflict,
    GitInProgress,
    ReadOnlyDevice,
    Archived,
    DirectoryLocked,
    Network,
    ChecksFailed,
    Corrupted,
    DaemonUnavailable,
    Internal,
}

impl Code {
    pub fn exit_code(self) -> i32 {
        match self {
            Code::Ok => 0,
            Code::InvalidArgs => 2,
            Code::Denied => 10,
            Code::ApprovalTimeout => 11,
            Code::LeaseHeld => 12,
            Code::CrossPlatform => 13,
            Code::Conflict => 14,
            Code::GitInProgress => 15,
            Code::ReadOnlyDevice => 16,
            Code::Archived => 17,
            Code::DirectoryLocked => 18,
            Code::Network => 20,
            Code::ChecksFailed => 21,
            Code::Corrupted => 22,
            Code::DaemonUnavailable => 30,
            Code::Internal => 1,
        }
    }

    pub fn from_exit_code(code: i32) -> Code {
        match code {
            0 => Code::Ok,
            2 => Code::InvalidArgs,
            10 => Code::Denied,
            11 => Code::ApprovalTimeout,
            12 => Code::LeaseHeld,
            13 => Code::CrossPlatform,
            14 => Code::Conflict,
            15 => Code::GitInProgress,
            16 => Code::ReadOnlyDevice,
            17 => Code::Archived,
            18 => Code::DirectoryLocked,
            20 => Code::Network,
            21 => Code::ChecksFailed,
            22 => Code::Corrupted,
            30 => Code::DaemonUnavailable,
            _ => Code::Internal,
        }
    }
}

/// A Workroam error: a stable code, a human message and an optional fix hint.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Error {
    pub code: Code,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl Error {
    pub fn new(code: Code, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            hint: None,
            data: None,
        }
    }
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
    pub fn with_data(mut self, data: serde_json::Value) -> Self {
        self.data = Some(data);
        self
    }
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(Code::InvalidArgs, message)
    }
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(Code::Internal, message)
    }
    pub fn network(message: impl Into<String>) -> Self {
        Self::new(Code::Network, message)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)?;
        if let Some(h) = &self.hint {
            write!(f, " ({h})")?;
        }
        Ok(())
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::internal(format!("I/O error: {e}"))
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::internal(format!("JSON error: {e}"))
    }
}

impl From<toml::de::Error> for Error {
    fn from(e: toml::de::Error) -> Self {
        Error::invalid(format!("TOML error: {e}"))
    }
}

impl From<toml::ser::Error> for Error {
    fn from(e: toml::ser::Error) -> Self {
        Error::internal(format!("TOML error: {e}"))
    }
}

/// Attach context to any error, keeping the code when it is already ours.
pub trait Context<T> {
    fn ctx(self, what: &str) -> Result<T>;
}

impl<T, E: std::fmt::Display> Context<T> for std::result::Result<T, E> {
    fn ctx(self, what: &str) -> Result<T> {
        self.map_err(|e| Error::internal(format!("{what}: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_roundtrip() {
        for c in [
            Code::Ok,
            Code::InvalidArgs,
            Code::Denied,
            Code::ApprovalTimeout,
            Code::LeaseHeld,
            Code::CrossPlatform,
            Code::Conflict,
            Code::GitInProgress,
            Code::ReadOnlyDevice,
            Code::Archived,
            Code::DirectoryLocked,
            Code::Network,
            Code::ChecksFailed,
            Code::Corrupted,
            Code::DaemonUnavailable,
        ] {
            assert_eq!(Code::from_exit_code(c.exit_code()), c);
        }
        assert_eq!(Code::DirectoryLocked.exit_code(), 18);
        assert_eq!(Code::DaemonUnavailable.exit_code(), 30);
    }
}
