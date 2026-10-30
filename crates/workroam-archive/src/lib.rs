//! Project archives (architecture §10.2 – §10.4), the optional off-site
//! mirror (§10.5) and `verify`. Approvals and jobs live in `workroamd`;
//! this crate only does the work.

pub mod archive;
pub mod mirror;
pub mod package;
pub mod verify;

pub use archive::{
    create, delete, extract, list, mark_local, plan, restore, restore_plan, ArchiveEntry,
    ArchiveOptions, ArchiveOutcome, ArchivePlan, DeleteOutcome, Prefer, RestoreArchiveOutcome,
    RestoreArchivePlan, Scenario,
};
pub use mirror::{mirror, MirrorReport, MirrorState};
pub use package::{export, import, ExportOutcome, ImportOutcome};
pub use verify::{verify, VerifyReport};
