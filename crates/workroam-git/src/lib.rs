//! Code repositories (architecture §11). Every machine keeps its own `.git`;
//! Workroam talks to the local `git` binary only.

pub mod cmd;
pub mod scan;
pub mod sync;

pub use cmd::Git;
