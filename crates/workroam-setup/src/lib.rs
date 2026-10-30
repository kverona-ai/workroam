//! First-run setup (architecture §7): the check registry shared by setup,
//! `workroam doctor` and the desktop "Checks" page, and the setup session
//! that both the wizard and `workroam setup` drive step by step.

pub mod checks;
pub mod session;

pub use checks::{CheckResult, CheckStatus};
pub use session::{SetupMode, SetupSession};
