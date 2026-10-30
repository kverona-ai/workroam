//! Agent adaptation layer (architecture §12): declarative adapters, import
//! of existing agent configs, per-machine generation and session migration.

pub mod envblock;
pub mod generate;
pub mod handoff;
pub mod import;
pub mod model;
pub mod registry;
pub mod sessions;

pub use model::Adapter;
pub use registry::{all, by_id, detect, Detected};

/// Workroam Skill templates (AGT-02, architecture §16.2).
pub const SKILL_MD: &str = include_str!("../../../adapters/skills/workroam/SKILL.md");
pub const SKILL_CLOUD_MD: &str = include_str!("../../../adapters/skills/workroam/SKILL.cloud.md");
