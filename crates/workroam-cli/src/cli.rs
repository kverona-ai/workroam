//! Command definitions (PRD §8.17, cli-ui-parity.md). Every command maps to
//! one IPC method of `workroamd`; this file only describes arguments.

use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(
    name = "workroam",
    version = crate::VERSION,
    about = "Hand off your AI agent workspace between machines.",
    long_about = "Workroam keeps a project, its git repositories, agent rules, MCP config, skills and HANDOFF.md in sync between your computers through your own S3 storage, end-to-end encrypted.\n\nRun `workroam` without arguments for a status summary.",
    after_help = "Docs: https://github.com/kverona-ai/workroam"
)]
pub struct Cli {
    /// Project (default: the project of the current directory). `setup`
    /// accepts it several times as <project>[=<dir>].
    #[arg(short = 'p', long = "project", global = true, value_name = "PROJECT")]
    pub project: Vec<String>,
    /// Stable JSON on stdout (schema_version, code); no progress bars or banner.
    #[arg(long, global = true)]
    pub json: bool,
    /// Only errors and the final result.
    #[arg(short, long, global = true)]
    pub quiet: bool,
    /// Answer "yes" to this command's own confirmations (never to sync approvals or typed-name confirmations).
    #[arg(short = 'y', long, global = true)]
    pub yes: bool,
    #[arg(long, global = true, value_enum, default_value_t = ColorWhen::Auto)]
    pub color: ColorWhen,
    /// Do not show the startup banner this time.
    #[arg(long, global = true)]
    pub no_banner: bool,
    /// Interface language.
    #[arg(long, global = true, value_parser = ["zh", "en"])]
    pub lang: Option<String>,
    #[command(subcommand)]
    pub cmd: Option<Cmd>,
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
pub enum ColorWhen {
    Auto,
    Always,
    Never,
}

#[derive(Debug, Subcommand)]
pub enum Cmd {
    /// First-time setup, or join an existing workspace.
    Setup(SetupArgs),
    /// Bring a folder under Workroam and push it for the first time.
    Init(InitArgs),
    /// Projects of the workspace and their folders on this computer.
    Project {
        #[command(subcommand)]
        cmd: ProjectCmd,
    },
    /// Service, lease, latest snapshot, local changes, repositories, new progress.
    Status {
        /// Every project on this computer.
        #[arg(long)]
        all: bool,
    },
    /// Structured summary of HANDOFF.md.
    Handoff {
        /// Print the HANDOFF.md template instead.
        #[arg(long)]
        template: bool,
    },
    /// Push a snapshot.
    Push {
        /// Release the lease after pushing.
        #[arg(long)]
        release: bool,
        /// Every project on this computer.
        #[arg(long)]
        all: bool,
        /// Only show what would be pushed.
        #[arg(long)]
        dry_run: bool,
        /// Take over the lease from another device.
        #[arg(long)]
        takeover: bool,
        /// Note recorded with the snapshot.
        #[arg(long, short = 'm')]
        note: Option<String>,
    },
    /// Pull the latest snapshot and take the lease.
    Pull {
        #[arg(long)]
        all: bool,
        #[arg(long)]
        dry_run: bool,
        /// Wait for the lease to be released (default 30m).
        #[arg(long, num_args = 0..=1, default_missing_value = "30m", value_name = "DURATION", conflicts_with = "takeover")]
        wait: Option<String>,
        /// Take over the lease (conflict copies are kept).
        #[arg(long)]
        takeover: bool,
    },
    /// Lease operations.
    Lease {
        #[command(subcommand)]
        cmd: LeaseCmd,
    },
    /// Pending approvals.
    Approval {
        #[command(subcommand)]
        cmd: ApprovalCmd,
    },
    /// Approve a pending request.
    Approve {
        id: String,
        /// Don't ask again for this project for this long (e.g. 1h).
        #[arg(long, value_name = "DURATION")]
        skip_ask: Option<String>,
        /// Remote connections: read-only or sync.
        #[arg(long, value_parser = ["read-only", "sync"])]
        access: Option<String>,
        /// Remote connections: projects the agent may access (repeatable).
        #[arg(long = "allow-project", value_name = "PROJECT")]
        allow_project: Vec<String>,
    },
    /// Postpone a pending request.
    Later { id: String },
    /// Deny a pending request.
    Deny { id: String },
    /// Background jobs.
    Job {
        #[command(subcommand)]
        cmd: JobCmd,
    },
    /// Snapshot history of the project.
    History {
        #[command(subcommand)]
        cmd: Option<HistoryCmd>,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Restore from a snapshot, in place or into another directory.
    Restore {
        /// Snapshot id, latest, ~N, or @<time>.
        snapshot: String,
        /// Only these files or directories (repeatable).
        #[arg(long = "path", value_name = "PATH")]
        paths: Vec<String>,
        /// Export into this directory instead of restoring in place.
        #[arg(long)]
        to: Option<String>,
        #[arg(long)]
        dry_run: bool,
    },
    /// Archive projects and restore them.
    Archive {
        #[command(subcommand)]
        cmd: ArchiveCmd,
    },
    /// Devices of the workspace.
    Device {
        #[command(subcommand)]
        cmd: DeviceCmd,
    },
    /// Run every check.
    Doctor,
    /// Check the integrity of the stored data.
    Verify {
        /// Verify the off-site mirror instead of the primary storage.
        #[arg(long)]
        backup: bool,
        /// Also read this percentage of the data packs.
        #[arg(long, value_name = "PERCENT")]
        read_data: Option<f64>,
    },
    /// Read and change settings (same keys as the desktop settings page).
    Config {
        #[command(subcommand)]
        cmd: ConfigCmd,
    },
    /// Local and cloud agents.
    Agent {
        #[command(subcommand)]
        cmd: AgentCmd,
    },
    /// Storage access key saved on this computer.
    Credentials {
        #[command(subcommand)]
        cmd: CredentialsCmd,
    },
    /// Workspace encryption password.
    Password {
        #[command(subcommand)]
        cmd: PasswordCmd,
    },
    /// Pause automatic sync (default 1 hour).
    Pause {
        #[arg(long = "for", value_name = "DURATION", default_value = "1h")]
        duration: String,
    },
    /// Resume automatic sync.
    Resume,
    /// Apply the retention policy and free unused data.
    Prune {
        #[arg(long)]
        dry_run: bool,
    },
    /// Off-site backup mirror.
    Backup {
        #[command(subcommand)]
        cmd: BackupCmd,
    },
    /// MCP server over stdio for local agents.
    Mcp,
    /// Shell completion script.
    Completions {
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
}

#[derive(Debug, Args)]
pub struct SetupArgs {
    #[arg(long, group = "path")]
    pub new: bool,
    #[arg(long, group = "path")]
    pub join: bool,
    #[arg(long, group = "path")]
    pub from_backup: bool,
    #[arg(long, value_parser = ["rustfs", "aws", "r2", "oss", "other", "local"])]
    pub preset: Option<String>,
    #[arg(long)]
    pub endpoint: Option<String>,
    #[arg(long, value_parser = ["path", "virtual-host"])]
    pub addressing: Option<String>,
    #[arg(long)]
    pub region: Option<String>,
    #[arg(long)]
    pub access_key: Option<String>,
    /// Read the secret key from stdin (or WORKROAM_SECRET_KEY).
    #[arg(long)]
    pub secret_key_stdin: bool,
    #[arg(long, value_parser = ["create", "current"])]
    pub key_mode: Option<String>,
    #[arg(long, value_parser = ["read-write", "read-only"], default_value = "read-write")]
    pub key_access: String,
    /// Admin API endpoint for key creation (when it differs from the S3 endpoint).
    #[arg(long)]
    pub admin_endpoint: Option<String>,
    /// Keep using a key that can see more than this workspace.
    #[arg(long)]
    pub accept_broad_key: bool,
    /// auto or a bucket name.
    #[arg(long)]
    pub bucket: Option<String>,
    #[arg(long)]
    pub prefix: Option<String>,
    /// Read the encryption password from stdin (or WORKROAM_PASSWORD).
    #[arg(long)]
    pub password_stdin: bool,
    /// Write the recovery key to this file (required without a terminal).
    #[arg(long)]
    pub recovery_key_out: Option<String>,
    #[arg(long)]
    pub pairing_code_stdin: bool,
    /// Unlock with the recovery key instead of the password.
    #[arg(long)]
    pub recovery_key_stdin: bool,
    #[arg(long)]
    pub device_name: Option<String>,
    #[arg(long)]
    pub root: Option<String>,
    /// Internal hosts this computer reaches, comma separated.
    #[arg(long, value_delimiter = ',')]
    pub reachable: Vec<String>,
    #[arg(long, value_parser = ["off", "ask", "auto"])]
    pub auto: Option<String>,
    #[arg(long, overrides_with = "no_autostart")]
    pub autostart: bool,
    #[arg(long)]
    pub no_autostart: bool,
    /// Join: every project at its default folder.
    #[arg(long)]
    pub all_projects: bool,
    /// Rejoin after restoring from a backup as this former device.
    #[arg(long)]
    pub as_device: Option<String>,
    /// Show the pairing QR code as text.
    #[arg(long)]
    pub qr: bool,
}

#[derive(Debug, Args)]
pub struct InitArgs {
    /// Directory (default: the current one).
    pub dir: Option<String>,
    /// Project name (default: the directory name).
    #[arg(long)]
    pub name: Option<String>,
    /// Only scan and report; write nothing.
    #[arg(long)]
    pub dry_run: bool,
    /// Override a repository: <path>=<mode>[:<history>] (repeatable).
    #[arg(long = "repo", value_name = "PATH=MODE[:HISTORY]")]
    pub repos: Vec<String>,
    /// Leave a repository out (repeatable).
    #[arg(long = "skip-repo", value_name = "PATH")]
    pub skip_repos: Vec<String>,
    /// Take the agents' versions of conflicting rules, MCP and skills.
    #[arg(long)]
    pub take_imported: bool,
    /// Enabled agents, comma separated (default: the installed ones).
    #[arg(long, value_delimiter = ',')]
    pub agents: Vec<String>,
    /// Do not push after initializing.
    #[arg(long)]
    pub no_push: bool,
}

#[derive(Debug, Subcommand)]
pub enum ProjectCmd {
    /// Every project of the workspace, including ones not on this computer.
    List,
    /// Add an existing project on this computer, then pull it.
    Add {
        name: String,
        #[arg(long)]
        path: Option<String>,
        /// Only add; do not pull now.
        #[arg(long)]
        no_pull: bool,
    },
    /// Change the folder (only before the first sync).
    Move {
        name: String,
        #[arg(long)]
        path: String,
    },
    /// Remove the project from this computer (files stay).
    Remove { name: String },
}

#[derive(Debug, Subcommand)]
pub enum LeaseCmd {
    /// Take over the lease without pulling.
    Takeover,
}

#[derive(Debug, Subcommand)]
pub enum ApprovalCmd {
    List,
    Show { id: String },
}

#[derive(Debug, Subcommand)]
pub enum JobCmd {
    List {
        /// Include finished jobs.
        #[arg(long)]
        all: bool,
    },
    Cancel {
        id: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum HistoryCmd {
    /// Files changed by one snapshot.
    Show { snapshot: String },
}

#[derive(Debug, Subcommand)]
pub enum ArchiveCmd {
    /// Archive a project.
    Create {
        name: String,
        #[arg(long)]
        note: Option<String>,
        /// Keep every earlier snapshot too.
        #[arg(long)]
        keep_history: bool,
    },
    List {
        #[arg(long)]
        search: Option<String>,
    },
    /// Restore an archived project.
    Restore {
        name: String,
        #[arg(long)]
        to: Option<String>,
        #[arg(long)]
        dry_run: bool,
        #[arg(long, value_parser = ["local", "merge", "archive"])]
        prefer: Option<String>,
    },
    /// Copy an archive into another folder; it stays archived.
    Extract {
        name: String,
        #[arg(long)]
        to: String,
    },
    /// Delete an archive for good.
    Delete {
        name: String,
        /// The project name, to confirm without a terminal.
        #[arg(long)]
        confirm: Option<String>,
    },
    /// Export an archive into a .wrarchive file.
    Export {
        name: String,
        file: String,
        /// Read an export password from stdin (default: the workspace password).
        #[arg(long)]
        password_stdin: bool,
    },
    /// Import a .wrarchive file.
    Import {
        file: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        password_stdin: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum DeviceCmd {
    List,
    /// Pairing code for a new device.
    Add {
        #[arg(long)]
        read_only: bool,
        /// Show the QR code as text.
        #[arg(long)]
        qr: bool,
    },
    /// Remove a device and rotate the storage key.
    Remove {
        device: String,
        /// New access key to rotate to.
        #[arg(long)]
        new_access_key: Option<String>,
        #[arg(long)]
        new_secret_key_stdin: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum ConfigCmd {
    List,
    Get { key: String },
    Set { key: String, value: String },
}

#[derive(Debug, Subcommand)]
pub enum AgentCmd {
    /// Detected agents, versions and adapter matches; cloud connections.
    List,
    /// Connect a local agent, or a cloud agent through a relay.
    Connect {
        agent: String,
        /// Relay address reported by the bot.
        #[arg(long)]
        relay: Option<String>,
    },
    Disconnect {
        agent: String,
    },
    /// Event webhook of a cloud agent.
    Webhook {
        agent: String,
        #[arg(long)]
        url: String,
        #[arg(long)]
        secret_stdin: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum CredentialsCmd {
    Update {
        #[arg(long)]
        access_key: String,
        #[arg(long)]
        secret_key_stdin: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum PasswordCmd {
    Change,
    Reset {
        #[arg(long)]
        recovery_key_stdin: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum BackupCmd {
    /// Mirror to an S3 bucket or a local folder.
    Add {
        #[arg(long, value_parser = ["s3", "local"])]
        kind: Option<String>,
        #[arg(long)]
        endpoint: Option<String>,
        #[arg(long)]
        bucket: Option<String>,
        #[arg(long)]
        prefix: Option<String>,
        #[arg(long)]
        region: Option<String>,
        #[arg(long)]
        path: Option<String>,
        #[arg(long, value_parser = ["hourly", "daily", "weekly", "manual"], default_value = "daily")]
        schedule: String,
        #[arg(long, default_value_t = 30)]
        delete_delay_days: u32,
        #[arg(long)]
        access_key: Option<String>,
        #[arg(long)]
        secret_key_stdin: bool,
    },
    List,
    Remove,
    Run,
}
