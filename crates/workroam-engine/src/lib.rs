//! Workroam sync engine (architecture §9 – §11): the open workspace,
//! projects, push, pull, conflicts, restore and history. Approvals, jobs and
//! triggers live in `workroamd`; this crate only does the work.

pub mod history;
pub mod init;
pub mod local;
pub mod merge;
pub mod project;
pub mod restore;
pub mod scan;
pub mod status;
pub mod sync;
pub mod workspace;

pub use project::Project;
pub use workspace::{blocking, ProjectSummary, Workspace};
