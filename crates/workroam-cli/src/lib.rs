//! The `workroam` command line (PRD §8.17). Every command is a thin IPC
//! client of `workroamd`: it collects input, calls one method and prints
//! the result, as text for people or as stable JSON for agents.

pub mod banner;
pub mod cli;
mod cmds;
pub mod conn;
pub mod mcp;
pub mod out;
mod setup;

use clap::{CommandFactory, Parser};
pub use cli::Cli;

pub const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), env!("WORKROAM_VERSION_SUFFIX"));

/// The clap command tree (used by the parity check and completions).
pub fn command() -> clap::Command {
    Cli::command()
}

/// Run with the process arguments; returns the exit code.
pub fn main_with_args(args: Vec<std::ffi::OsString>) -> i32 {
    let cli = match Cli::try_parse_from(args) {
        Ok(c) => c,
        Err(e) => {
            let code = if e.use_stderr() { 2 } else { 0 };
            let _ = e.print();
            return code;
        }
    };
    if let Some(l) = &cli.lang {
        workroam_core::text::set_lang(workroam_core::text::resolve_lang(l));
    } else if let Ok(Some(m)) =
        workroam_core::config::MachineConfig::load(&workroam_core::paths::machine_toml())
    {
        workroam_core::text::set_lang(workroam_core::text::resolve_lang(&m.ui.lang));
    }
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("cannot start: {e}");
            return 1;
        }
    };
    rt.block_on(cmds::run(cli))
}
