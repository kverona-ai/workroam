<div align="center">

<img src="docs/assets/logo.svg" width="128" alt="Workroam logo">

# Workroam

**Hand off your AI agent workspace between machines.**

One S3 bucket of your own. Tell any agent on any computer to "sync down",<br>
and your projects, code, rules, MCP / Skills and context pick up where you left off.

[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Status](https://img.shields.io/badge/status-design%20phase-orange.svg)](docs/prd.md)
[![Rust](https://img.shields.io/badge/Rust-stable-dea584.svg?logo=rust&logoColor=white)](https://www.rust-lang.org)
[![Platforms](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey.svg)](#)
[![Storage](https://img.shields.io/badge/storage-any%20S3--compatible-2F6BFF.svg)](docs/architecture.md)
[![PRs Welcome](https://img.shields.io/badge/PRs-welcome-1FB6E8.svg)](CONTRIBUTING.md)

**English** · [简体中文](README.zh-CN.md)

[Requirements](docs/prd.md) · [Architecture](docs/architecture.md) · [Decisions](docs/adr/README.md) · [Contributing](CONTRIBUTING.md)

<br>

<img src="docs/assets/screenshots/main-window.webp" width="860" alt="Workroam main window with an approval card in the corner">

</div>

> [!NOTE]
> Workroam is in the **design phase** and has no usable release yet. Feedback on the design is welcome via issues. The design documents are currently written in Chinese.

---

## Why Workroam

You probably use several coding agents (Claude Code, Codex, Antigravity, Grok Build, …) and switch between an office machine and a home machine:

- only the office machine can reach the intranet and internal git, so **remote development is not an option**: work has to stay local;
- cloud drives, Syncthing and rclone **don't understand** agent rules, MCP, Skills, sessions or git, and syncing `.git` directly corrupts repositories;
- Windows, macOS and Linux disagree on paths, line endings, case sensitivity and symlinks;
- after switching machines, your agent **forgets** where it left off.

Workroam turns "leave the office → get home → keep going → push at the office" into two sentences.

## Features

|  |  |
| --- | --- |
| 🔐 **End-to-end encryption** | Data is encrypted on the client and the server only stores ciphertext. Credentials never sync. |
| 🪶 **Minimal server** | Any S3-compatible storage (RustFS, Garage, SeaweedFS, AWS S3, R2, OSS), deployed with one command. |
| 🏃 **Handoff, not real-time sync** | A lease ensures only one machine writes at a time. Nothing is ever silently overwritten. |
| 🌿 **Git-aware** | Every machine keeps its own `.git`, with bundle, remote and files modes. Intranet repos can carry only the last N commits and are rebased back onto real history at the office. |
| 🤖 **Agent-aware** | One `AGENTS.md`, one MCP definition and one Skills folder, rendered into each agent's native config. `HANDOFF.md` lets the next agent continue. |
| 🖥️ **Cross-platform** | Case conflicts, invalid Windows file names and long paths are caught before you push. Links and executable bits are handled per OS on pull. |
| 🙋 **You stay in the loop** | Approve every sync, including agent-initiated ones, watch progress, and get notified on success or failure. |
| ⏪ **Rollback & archive** | Every sync is an immutable snapshot you can return to. When you decide a project is done, archive it: syncing stops, a self-contained copy is kept forever, and you can resume it any time. Local files are never touched. |

## Product preview

High-fidelity designs from the [clickable prototype](docs/prototype/README.md) (open `docs/prototype/index.html` in a browser). Light and dark themes, Liquid Glass style; the prototype UI is currently in Chinese.

<table>
<tr>
<td width="50%"><img src="docs/assets/screenshots/setup-wizard.webp" alt="First-run setup: 16 storage capability checks"><br><sub>First-run setup: 16 storage capability checks</sub></td>
<td width="50%"><img src="docs/assets/screenshots/corner-cards.webp" alt="Corner cards: approve, progress, done, failed, arrival, lease conflict"><br><sub>Corner cards: approve, progress, done, failed, arrival, lease conflict</sub></td>
</tr>
<tr>
<td width="50%"><img src="docs/assets/screenshots/archive-restore-dark.webp" alt="Restoring an archive: what happens to your local folder (dark)"><br><sub>Restoring an archive: what happens to your local folder (dark)</sub></td>
<td width="50%"><img src="docs/assets/screenshots/tray-menu-dark.webp" alt="Tray menu and icon states (dark)"><br><sub>Tray menu and icon states (dark)</sub></td>
</tr>
</table>

## How it works

```mermaid
flowchart LR
    O["🏢 Office<br/>workroam push --release"] -- encrypted snapshot --> S[("S3-compatible storage")]
    S -- incremental pull --> H["🏠 Home<br/>workroam pull"]
    H -- encrypted snapshot --> S
    S -- pull & rebase --> O
    O -- git push --> G["Intranet git"]
```

1. **Leaving the office**: tell any agent "I'm heading home". It updates `.handoff/HANDOFF.md` and runs `workroam push --release`, and you click "Approve" on the card that pops up.
2. **Arriving home**: the tray tells you there is new progress on the server. Say "sync down" and Workroam pulls files, imports commits, and generates MCP configs, Skills links and an environment note for this machine.
3. **Keep going**: open any agent. It reads `AGENTS.md` and `HANDOFF.md`, reports where things stand, and continues.
4. **Back at the office**: pull the home progress. Home commits are rebased back onto real history, then you `git push` to the intranet.

## Quick start (planned)

```bash
# Server: deploy storage and HTTPS with one command
curl -fsSL https://<project-domain>/install.sh | sh -s -- --domain sync.example.com --storage rustfs

# Client: the desktop app opens a setup wizard on first launch
# (headless machines: run the same steps in the terminal;
#  every desktop action has an equivalent workroam command)
workroam setup
cd ~/projects/proj-a && workroam init

# Hand off
workroam push --release     # when leaving
workroam pull               # when arriving (takes the lease)
```

## Roadmap

| Version | Theme | Gate |
| --- | --- | --- |
| **v0.1** | Dogfooding: setup wizard & checks, pairing codes, CLI, snapshots, lease, encryption, bundle mode, Claude Code & Codex adapters, approval cards | Two weeks of daily Windows ↔ Mac handoffs without data loss |
| v0.2 | More agents: Grok Build, Antigravity, ZCode, Pi; session migration; remote & files modes; scoped keys, read-only devices, key rotation; project archive & restore | Six agents work right after switching machines |
| **v0.3** | Hands-free (target product): automatic sync, arrival cards, sync history, MCP server for local agents, remote connector for cloud personal agents (Grok Bot, Muse), mobile push, optional server with pairing codes and a web UI; optional off-site mirror | Almost no manual commands |

See [PRD · Roadmap](docs/prd.md#10-版本规划与验收门槛) for details.

## Repository layout

```text
.
├── docs/            # Docs: PRD, architecture, ADRs, prototype, assets
├── crates/          # Rust code (planned, see crates/README.md)
├── .github/         # Issue / PR templates
├── CONTRIBUTING.md  # Contribution guide
└── LICENSE          # Apache-2.0
```

## Contributing

Contributions of every kind are welcome, especially **agent adapters**: they are plain data files, the easiest place to start and the most valuable. Please read [CONTRIBUTING.md](CONTRIBUTING.md) first. All commits require a DCO `Signed-off-by`.

Report security issues privately as described in [SECURITY.md](SECURITY.md), not in public issues.

## License

[Apache License 2.0](LICENSE). Standalone Rust libraries will be dual-licensed under `MIT OR Apache-2.0`.

> Before taking intranet code off your company network, check your company's policies.
