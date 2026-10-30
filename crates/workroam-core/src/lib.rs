//! Shared types for every Workroam component.
//!
//! Nothing in here touches the network or the storage; it only describes
//! the workspace, its projects and devices, and how they are configured.

pub mod config;
pub mod crypto;
pub mod duration;
pub mod error;
pub mod model;
pub mod paths;
pub mod text;

pub use error::{Code, Error, Result};

/// Product version, shared by every binary.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// IPC protocol major version (architecture §15).
pub const PROTOCOL_VERSION: u32 = 1;

/// JSON output schema version for `--json` (architecture §16.1).
pub const JSON_SCHEMA_VERSION: u32 = 1;

/// Snapshot manifest format version.
pub const MANIFEST_FORMAT: u32 = 1;

/// Storage layout version written to `control/workspace.json`.
pub const LAYOUT_VERSION: u32 = 1;
