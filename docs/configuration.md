# Configuration

**English** · [简体中文](configuration.zh-CN.md)

This page lists every file, setting and environment variable Workroam reads. Most people never edit a file by hand: the desktop **Settings** page and `workroam config` change the same keys, and both validate the value before saving it. For deploying storage and `workroam-server`, see [Deployment](deployment.md).

## Contents

- [Where things are kept](#where-things-are-kept)
- [Changing settings](#changing-settings)
- [Settings reference](#settings-reference)
- [machine.toml (per computer)](#machinetoml-per-computer)
- [workroam.toml (per project)](#workroamtoml-per-project)
- [mcp.toml (MCP servers)](#mcptoml-mcp-servers)
- [Environment variables](#environment-variables)
- [Command-line options](#command-line-options)
- [workroam-server](#workroam-server)
- [Install scripts](#install-scripts)

## Where things are kept

**On each computer** (`~` is your home folder; `WORKROAM_HOME` moves `~/.workroam` elsewhere):

| Path | Contents |
| --- | --- |
| `~/.workroam/machine.toml` | This computer's settings and the workspace location, see [below](#machinetoml-per-computer) |
| `~/.workroam/logs/` | `workroamd.log`, rotated daily |
| `~/.workroam/state/` | Local state of the background service |
| `~/.workroam/adapters/` | Your own agent adapters, which override the built-in ones with the same `id` |
| System keychain | Storage access key, encryption password, MCP secrets (`workroam secret set`) and other secrets: macOS Keychain, Windows Credential Manager, Linux Secret Service. They are never written to `machine.toml` |
| `~/.workroam/secrets.json` | Only when no keychain is available, or with `WORKROAM_SECRETS=file` (mode 0600) |

The background service listens on a local socket: `~/Library/Application Support/Workroam/workroamd.sock` on macOS, `$XDG_RUNTIME_DIR/workroam/workroamd.sock` (or `~/.workroam/workroamd.sock`) on Linux, and the named pipe `\\.\pipe\workroam-<user>` on Windows.

**In each project folder** (synced with the project unless noted):

```text
proj-a/
├── AGENTS.md              # shared rules + this computer's environment block (generated)
├── CLAUDE.md              # a single @AGENTS.md line (generated when needed)
├── .workroam/
│   ├── workroam.toml      # this project's sync policy
│   ├── mcp.toml           # the single MCP definition, with per-OS overrides
│   ├── env/               # windows.md / macos.md / linux.md environment notes
│   └── local/             # this computer only: hash cache, operation log, mappings (not synced)
├── .agents/skills/        # the single source of Skills
├── .handoff/HANDOFF.md    # handoff notes between agents
└── repo-a/                # code repositories (each computer keeps its own .git)
```

Start agents in the project folder, not inside `repo-a/`: some agents only look for rule files from the git root down and would miss the project's `AGENTS.md`.

**In the storage bucket** everything is encrypted. Workspace-wide settings (retention, the default device for cloud agents) live there, so they are the same on every computer.

## Changing settings

```sh
workroam config list                         # every key with its value and where it comes from
workroam config get sync.auto
workroam config set sync.auto auto           # this computer
workroam config set sync.auto off -p proj-a  # override for one project
workroam --json config list                  # machine-readable
```

Each key has a scope:

| Scope | Stored in | Notes |
| --- | --- | --- |
| machine | `machine.toml` | This computer only |
| project | `machine.toml`, with an optional per-project override in `workroam.toml` | Without `-p` it sets this computer's default; with `-p <project>` it sets the project override, which travels with the project |
| project-only | `workroam.toml` | Requires `-p <project>` |
| workspace | The storage bucket | Shared by every computer |

`workroam config list` shows `source` for each value: `machine`, `project`, `workspace` or `default`.

## Settings reference

| Key | Scope | Values | Default | Description |
| --- | --- | --- | --- | --- |
| `device.name` | machine | text | set during setup | This computer's name, shown on every device and in the relay strip |
| `root` | machine | path | `~/projects` | Default folder for new projects (`<root>/<project>`); each project can still have its own folder |
| `reachable` | machine | comma-separated hosts | empty | Internal hosts this computer can reach (e.g. `git.corp.internal`); used to choose the repository mode and where commits can be pushed |
| `autostart` | machine | `true` / `false` | `true` | Start `workroamd` at login (installs or removes the user service) |
| `sync.auto` | project | `off` / `ask` / `auto` | `ask` | Automatic sync: never, ask with a card, or sync without asking |
| `sync.agent_requires_approval` | project | `true` / `false` | `true` | Syncs started by an agent wait for your approval |
| `sync.idle_minutes` | machine | number | `15` | Minutes without changes before an automatic push is offered |
| `sync.on_sleep` | machine | `off` / `ask` / `auto` | `ask` | Push (and release the lease) before the computer sleeps or shuts down. macOS waits up to 25 s for the push; Windows and Linux do not wait, so the job finishes after waking (or fails like any other job if the connection was cut). `ask` shows the usual approval card; before a shutdown only `auto` pushes |
| `sync.on_wake` | machine | `true` / `false` | `true` | Check for new progress after waking up and reconnecting |
| `sync.approval_timeout` | machine | duration (`10m`, `1h`) | `10m` | How long an approval card waits for your answer |
| `notify.ntfy` | machine | URL | empty | ntfy topic for phone notifications (sent directly, or forwarded by a full-mode server) |
| `notify.on_success` | machine | `true` / `false` | `true` | Also notify when a sync finishes successfully |
| `ui.banner` | machine | `true` / `false` | `true` | Show the CLI startup banner |
| `ui.lang` | machine | `zh` / `en` / empty | empty | Interface language; empty follows the system |
| `mcp.enabled` | project | `true` / `false` | `true` | Local MCP server for agents (`workroam mcp`) |
| `remote.enabled` | project-only | `true` / `false` | `false` | Cloud agents may access this project through a relay |
| `remote.default_device` | workspace | device name | empty | The computer that serves cloud agents through the relay |
| `retention.days` | workspace | number | `30` | Keep every snapshot of the last N days |
| `retention.weekly` | workspace | number | `52` | Before that, keep the newest snapshot of each week for N weeks (`0`: keep one per week forever) |

`workroam pause --for 2h` and `workroam resume` (the tray's "Pause automatic sync") write `sync.paused_until`.

## machine.toml (per computer)

`~/.workroam/machine.toml` is written by setup and by `workroam config set`. A full example:

```toml
name = "office-win"
os = "windows"                   # windows | macos | linux
device_id = "d_7f3a…"            # assigned at setup; do not change
access = "read-write"            # read-write | read-only
root = 'D:\projects'
reachable = ["git.corp.internal", "github.com"]
autostart = true

[workspace]
endpoint = "https://s3.example.com"   # or file:///path for a local folder
bucket = "workroam"
prefix = ""                           # optional key prefix inside the bucket
addressing = "path"                   # path | virtual-host (detected at setup)
region = "us-east-1"
preset = "rustfs"                     # rustfs | aws | r2 | oss | other | local
workspace_id = "w_…"
capabilities = { conditional_create = true, conditional_overwrite = true, multipart = true }

[sync]
auto = "ask"
idle_minutes = 15
on_sleep = "ask"
on_wake = true
agent_requires_approval = true
approval_timeout = "10m"
paused_until = ""

[notify]
ntfy = ""
on_success = true

[ui]
banner = true
lang = ""

[mcp]
enabled = true

[remote]                              # written by `workroam agent connect --relay`
relay_url = ""
relay_id = ""

[backup]                              # written by `workroam backup add`
kind = "s3"                           # s3 | local
endpoint = "https://s3.backup.example.com"
bucket = "workroam-mirror"
prefix = ""
region = "us-east-1"
path = ""                             # for kind = "local"
schedule = "daily"                    # hourly | daily | weekly | manual
delete_delay_days = 30                # deletions reach the mirror only after this many days

[projects.proj-a]
path = 'D:\projects\proj-a'
state = "bound"                       # pending | bound | missing | archived
```

The workspace location and the capabilities are found during setup, so do not edit them by hand: run `workroam setup` again to change the storage, and `workroam credentials update` to change the access key. Access keys and the encryption password are never in this file.

## workroam.toml (per project)

`<project>/.workroam/workroam.toml` is created by `workroam init` and syncs with the project, so every computer applies the same policy.

```toml
name = "proj-a"

[sync]
exclude = ["node_modules/", "target/", ".venv/", "dist/", "__pycache__/", ".DS_Store", "Thumbs.db"]
auto = "ask"                     # optional: overrides this computer's sync.auto
agent_requires_approval = true   # optional
sessions = true                  # migrate agent sessions with the project (default true)

[agents]
claude = { state = true }
codex  = { state = true }
grok   = { state = false }

[repos.order-service]            # intranet repository
path = "repo-a"
mode = "bundle"
history = "recent:50"
branches = ["main", "feature/*"]
extras = ["docs/generated/"]

[repos.oss-tool]                 # every computer can reach origin
path = "repo-b"
mode = "remote"
via = "origin"

[repos.scripts]
path = "repo-c"
mode = "files"

[mcp]
enabled = true                   # optional: overrides mcp.enabled

[remote]
enabled = false                  # cloud agents may access this project
```

| Key | Description |
| --- | --- |
| `sync.exclude` | Patterns left out of every snapshot (gitignore style). The default list is shown above |
| `sync.auto`, `sync.agent_requires_approval` | Per-project overrides of the settings with the same names |
| `sync.sessions` | Carry agent sessions with the project, so a conversation can continue on the other computer |
| `agents.<id>.state` | Which agents get generated configs for this project. Built-in ids: `claude` (Claude Code), `codex`, `grok` (Grok Build), `antigravity`, `zcode`, `pi`; cloud agents `grok-bot`, `muse` |
| `repos.<name>.path` | Repository folder, relative to the project |
| `repos.<name>.mode` | `bundle`: commits travel inside the snapshot as git bundles (for intranet repositories). `remote`: commits travel through a git server every computer can reach (`via`). `files`: the working tree is synced as files and `.git` stays local |
| `repos.<name>.history` | For `bundle`: `full`, `recent:N` (only the last N commits, rebased onto the real history at the office) or `snapshot` (no history) |
| `repos.<name>.branches` | Branch names or glob patterns to carry; the checked-out branch always travels. Empty: only the current branch |
| `repos.<name>.extras` | Paths inside the repository that are also synced as plain files (e.g. untracked generated docs) |
| `repos.<name>.via` | For `remote`: the git remote to push and fetch through (default `origin`) |

`workroam init --dry-run` shows the recommended mode for each repository found and why; `--repo <path>=<mode>[:<history>]` and `--skip-repo <path>` override it.

## mcp.toml (MCP servers)

`<project>/.workroam/mcp.toml` defines each MCP server once. After every pull, Workroam writes each enabled agent's native MCP configuration from it, using the right command for the current OS.

```toml
[servers.filesystem]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "{project_dir}/docs"]
targets = ["claude", "codex", "grok", "antigravity"]

[servers.filesystem.windows]
command = "cmd"
args = ["/c", "npx", "-y", "@modelcontextprotocol/server-filesystem", "{project_dir}/docs"]

[servers.jira]
url = "https://jira.example.com/mcp"
headers = { X-Project = "{project}" }
```

| Key | Description |
| --- | --- |
| `command`, `args`, `env` | A local (stdio) server |
| `url`, `headers` | A remote (HTTP) server |
| `targets` | Agent ids that get this server; empty means every agent |
| `windows`, `macos`, `linux` | Per-OS overrides. A `command` replaces command and args, `args` alone replaces the args, `url` replaces the URL, and `env` / `headers` are merged |

In any value, `{project_dir}` becomes the project folder on this computer and `{project}` the project name.

## Environment variables

**Desktop app, CLI and `workroamd`**

| Variable | Description |
| --- | --- |
| `WORKROAM_HOME` | Use another folder instead of `~/.workroam` (portable installs, testing). Keychain entries and the socket are kept separate per folder |
| `WORKROAM_SECRETS=file` | Store secrets in `~/.workroam/secrets.json` (mode 0600) instead of the system keychain, e.g. on a headless Linux server without Secret Service |
| `WORKROAM_SOCKET` | Path of the `workroamd` socket (named pipe on Windows) |
| `WORKROAM_LOG` | Log filter, e.g. `debug` or `workroam_engine=debug,info` (default `info`) |
| `WORKROAM_LANG` | Interface language `zh` or `en`; otherwise `LC_ALL`, `LC_MESSAGES`, `LANG`, `LANGUAGE` are used |
| `WORKROAM_NO_BANNER=1` | Never show the CLI startup banner |
| `WORKROAM_AGENT` | Name of the agent running the CLI. Commands run without a terminal are treated as agent requests and raise approval cards; Workroam recognizes Claude Code, Codex, Grok Build and Antigravity by their own variables, and this sets the name explicitly |
| `WORKROAM_NO_AUTOSTART` | Do not start `workroamd` automatically when a command needs it |
| `WORKROAM_IN_PROCESS=1` | Run the command inside the CLI process without `workroamd` |
| `WORKROAM_SECRET_KEY` | Storage secret key for `setup` and `credentials update` (instead of `--secret-key-stdin`) |
| `WORKROAM_PASSWORD` | Encryption password for `setup` and `password change` (instead of `--password-stdin`) |
| `WORKROAM_RECOVERY_KEY` | Recovery key for `password reset` (instead of `--recovery-key-stdin`) |
| `WORKROAM_ADMIN_TOKEN` | Admin API token, when the storage needs one to create a scoped key |
| `WORKROAM_BACKUP_SECRET_KEY` | Secret key of the off-site backup bucket for `backup add` |
| `WORKROAM_SECRET_VALUE` | Value for `secret set` (instead of `--value-stdin`) |

Prefer stdin or these variables to command-line arguments for secrets: arguments are visible to other users in the process list.

## Command-line options

These work with every `workroam` command:

| Option | Description |
| --- | --- |
| `-p, --project <name>` | The project to act on (default: the project of the current folder) |
| `--json` | Stable JSON on stdout (`schema_version`, `code`), with no progress bars or banner; what agents and scripts should use |
| `-q, --quiet` | Only errors and the final result |
| `-y, --yes` | Answer "yes" to the command's own confirmations (never to sync approvals or typed-name confirmations) |
| `--color auto\|always\|never` | Colored output |
| `--no-banner` | Skip the startup banner this time |
| `--lang zh\|en` | Interface language for this command |

`workroam <command> --help` lists each command's own options, and `workroam completions <shell>` prints a completion script for bash, zsh, fish, PowerShell or elvish. Exit codes are listed in [architecture §15](architecture.md#cli-退出码).

## workroam-server

| Option | Environment | Default | Description |
| --- | --- | --- | --- |
| `--listen <addr>` | `WORKROAM_SERVER_LISTEN` | `127.0.0.1:8787` | Address to listen on |
| `--public-url <url>` | `WORKROAM_SERVER_PUBLIC_URL` | none | The public `https://` address that reaches this port |
| `--tunnel` | | off | Open an outbound Cloudflare tunnel for a public HTTPS address (downloads `cloudflared` when missing). `CLOUDFLARED_TOKEN` runs a named tunnel instead of a quick one |
| `--bind <code>` | | | Bind to a device with the one-time code from `workroam agent connect` |
| `--state <file>` | `WORKROAM_SERVER_STATE` | `~/.workroam-server/state.json` | State file (`WORKROAM_SERVER_HOME` moves the whole folder) |
| `--status` | | | Print the binding, devices and connections, then exit |
| `--full` | | off | Full mode: pairing codes, per-device keys, ntfy, web reader (the container image and `--features full` builds) |
| `--admin-endpoint <url>` | `WORKROAM_ADMIN_ENDPOINT` | the storage endpoint | Full mode: storage admin API, when it differs from the endpoint devices use |
| `--ntfy <url>` | `WORKROAM_NTFY` | none | Full mode: ntfy topic for phone notifications |
| `--web-dir <dir>` | | built in | Full mode: serve the web reader from this folder |
| | `WORKROAM_ADMIN_ACCESS_KEY`, `WORKROAM_ADMIN_SECRET_KEY` | | Full mode: storage admin key used to create per-device keys. Environment only, never a flag or the state file |
| | `WORKROAM_ADMIN_TOKEN` | | Full mode: admin API token, for storages that need one |
| | `WORKROAM_LOG` | `workroam_server=info` | Log filter |

## Install scripts

`install-cli.sh` / `install-server.sh` take `--version <x.y.z>` and `--dir <path>`; the PowerShell scripts read `WORKROAM_VERSION`, `WORKROAM_INSTALL_DIR`, `WORKROAM_DOWNLOAD_URL` and `WORKROAM_COMPONENT`. `WORKROAM_DOWNLOAD_URL` points every script at a mirror. See [Install](install.md#mirrors-and-intranets).

The storage deployment script `install.sh` and its image variables are described in [Deployment](deployment.md#3-self-host-with-one-command).
