//! Every IPC method and event (architecture §15). User-facing methods must
//! appear in `parity.toml` with both a CLI and a desktop entry (§17.4).

#[derive(Debug, Clone, Copy)]
pub struct Method {
    pub name: &'static str,
    /// Reached by a person from the desktop app and the CLI.
    pub user_facing: bool,
    /// Agents (MCP / relay) may call it without approval.
    pub read_only: bool,
}

const fn m(name: &'static str, user_facing: bool, read_only: bool) -> Method {
    Method {
        name,
        user_facing,
        read_only,
    }
}

pub const METHODS: &[Method] = &[
    // Protocol.
    m("hello", false, true),
    m("subscribe", false, true),
    m("daemon.info", false, true),
    m("daemon.shutdown", false, false),
    // Setup and checks.
    m("setup.begin", true, false),
    m("setup.step", true, false),
    m("setup.check", true, false),
    m("setup.commit", true, false),
    m("setup.cancel", true, false),
    m("doctor.run", true, true),
    // Devices.
    m("device.pairing_code", true, false),
    m("device.list", true, true),
    m("device.remove", true, false),
    // Projects.
    m("project.scan", true, true),
    m("project.init", true, false),
    m("project.add", true, false),
    m("project.list", true, true),
    m("project.move", true, false),
    m("project.remove", true, false),
    m("status", true, true),
    m("handoff.read", true, true),
    // Sync.
    m("lease.takeover", true, false),
    m("push.plan", true, true),
    m("pull.plan", true, true),
    m("restore.plan", true, true),
    m("job.start", true, false),
    m("job.cancel", true, false),
    m("job.list", true, true),
    m("approval.list", true, true),
    m("approval.respond", true, false),
    m("history.list", true, true),
    m("history.show", true, true),
    // Archives.
    m("archive.plan", true, true),
    m("archive.list", true, true),
    m("archive.restore_plan", true, true),
    m("archive.restore", true, false),
    m("archive.extract", true, false),
    m("archive.delete", true, false),
    m("archive.export", true, false),
    m("archive.import", true, false),
    // Backup and maintenance.
    m("backup.configure", true, false),
    m("backup.run", true, false),
    m("verify.run", true, true),
    m("prune.run", true, false),
    // Settings and secrets.
    m("config.list", true, true),
    m("config.get", true, true),
    m("config.set", true, false),
    m("credentials.update", true, false),
    m("password.change", true, false),
    m("password.reset", true, false),
    m("sync.pause", true, false),
    m("sync.resume", true, false),
    // Agents.
    m("agent.list", true, true),
    m("agent.connect", true, false),
    m("agent.disconnect", true, false),
    m("agent.webhook", true, false),
];

/// Server → client notifications.
pub const EVENTS: &[&str] = &[
    "setup.check_result",
    "approval.requested",
    "approval.resolved",
    "job.progress",
    "job.finished",
    "job.failed",
    "arrival",
    "status.changed",
];

pub fn find(name: &str) -> Option<&'static Method> {
    METHODS.iter().find(|m| m.name == name)
}

pub fn is_known(name: &str) -> bool {
    find(name).is_some()
}
