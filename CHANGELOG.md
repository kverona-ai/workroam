# Changelog

**English** · [简体中文](CHANGELOG.zh-CN.md)

All notable changes to this project are recorded here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html). Each release's notes on GitHub are generated from this file (English) and `CHANGELOG.zh-CN.md` (Chinese).

## [Unreleased]

### Changed

- Release signing: macOS CLI and `workroam-server` binaries are now signed and notarized too, the `.dmg` is notarized and stapled, and notarization can use an App Store Connect API key; Windows files are signed through SignPath (executables first, then the installers built from them) instead of a `.pfx`; an optional GPG key signs `SHA256SUMS`; each Release run shows which signing is configured. See `docs/signing.md`

## [0.1.1] - 2026-10-30

A full review of the code base after v0.1.0 (performance, design, security, feature completeness); this release closes the gaps it found.

### Added

- Push before lid close, sleep or shutdown (`sync.on_sleep`) now really happens: on macOS the IOKit power notification holds the system for up to 25 seconds to finish the push and release the lease; on Windows the suspend / resume notification and on Linux logind's `PrepareForSleep` / `PrepareForShutdown` start it (the system does not wait, the push job carries on after wake, and a failure from the network going away is reported like any other job); after wake the service checks for new progress right away (`sync.on_wake`)
- `workroamd` stops in order on `SIGTERM` (logout / shutdown) or the Windows shutdown / logoff event: with `sync.on_sleep = auto` it pushes first, then waits up to 20 seconds for running jobs
- Recovery after an interrupted pull (crash, power loss; architecture §9.3): the service replays the operation log at start and finishes the remaining steps; steps already done (including conflict copies already written) are not repeated; until then a push is refused with a hint to pull first, and `workroam pull` picks up where it stopped
- The `.workroam/SYNCING` marker records the process that wrote it and is cleared once that process is gone, so agents are no longer paused forever
- MCP secrets: the values that `${secret:name}` in `.workroam/mcp.toml` refers to can now be saved in this computer's keychain, listed per project and removed, with `workroam secret set / remove / list` and Settings → Agent → MCP secrets in the desktop app; the earlier hint pointed at a command that did not exist

### Changed

- Snapshots keep file and directory modification times: files the previous snapshot already knows are not read or chunked again, which makes pushes of large projects clearly faster; a directory takes the newest time of its files, so unchanged directory trees are not uploaded again
- `/healthz` of `workroam-server` reports only the number of online devices, not their names
- Docs: new bilingual deployment guide (`docs/deployment.md`) and configuration reference (`docs/configuration.md`), plus an English install guide; the README points to the published release
- Release notes are now in English by default, with the full Chinese version folded under "简体中文"; the changelog is split into English `CHANGELOG.md` and Chinese `CHANGELOG.zh-CN.md`
- New "Release notes" workflow to regenerate the notes of a published release

### Fixed

- Unpacking a project archive (`.wrarchive`) no longer trusts entry names and sizes blindly (name length, relative path, backslash and drive letter checks, size verified against the data actually read): a corrupt or malicious file cannot cause a huge allocation or a write outside the directory
- Session migration skips session paths with backslashes, absolute paths or drive letters; the case-insensitive comparison of Windows and Unix path prefixes no longer splits bytes (paths with non-ASCII characters do not crash)
- `workroam-server`: a signed request in full mode cannot be replayed within its 5-minute window; open OAuth authorization requests are capped (20 in total, 3 per client) so an unauthenticated caller cannot flood the approval cards; the unused attempt counter on pairing codes is gone
- Duration parsing (for example `sync.approval_timeout`) reports an error on overflow instead of crashing
- `doctor` really probes the system keychain when deciding whether it is usable instead of always reporting "system keychain"
- Desktop app: a builder wrapped as an action returned a promise instead of an element, so the Settings page showed `[object Promise]` where the Agent section (agents, MCP switch, connections, webhook, default device) and the off-site backup section belong, the add-project sheet listed no repositories, the archive restore sheet had no preference control, and the setup wizard's "workspace found" and "broad key" notices failed to show

## [0.1.0] - 2026-10-30

The first release: every feature of PRD v0.1, delivered at once (acceptance gates in PRD §10).

### Added

- Desktop app (Tauri v2, macOS / Windows / Linux): tray icon, first-run setup wizard, main window and corner cards; includes the `workroam` CLI and the `workroamd` background service
- `workroam` CLI and `workroamd` service: setup and `doctor` checks, device pairing, adding projects, push / pull / lease, snapshots and restore, history, project archive and restore, optional off-site backup, code repository sync, adapters and session migration for six local agents, automatic sync; `--json` output for local agents through the Workroam Skill
- `workroam-server`: relay mode (a single musl static binary that can run on a bot's cloud computer, for cloud personal agents through a remote MCP connector) and full mode (pairing codes, per-device keys, notification forwarding, a read-only web reader that decrypts in the browser)
- Self-hosted storage: `deploy/install.sh` deploys RustFS / Garage / SeaweedFS with Caddy in one command, Docker Compose fragments, Kubernetes Helm chart (`deploy/helm/workroam`)
- v0.1 docs: requirements (`docs/prd.md`), architecture (`docs/architecture.md`), desktop / CLI parity (`docs/cli-ui-parity.md`), architecture decision records ADR-0001 to ADR-0009
- High-fidelity desktop prototype (`docs/prototype/`) with design rules, logo, README screenshots and a CLI banner sample (`docs/assets/cli-banner.svg`)
- CI/CD: every PR checks formatting, clippy, tests, the static `workroam-server` build and the desktop build on three systems; a `v*` tag publishes a release with x64 and arm64 desktop installers for macOS / Windows / Linux, CLI and `workroam-server` binaries, a `ghcr.io` container image, source packages (including a vendored one), `SHA256SUMS` and build provenance, install scripts and Homebrew / Scoop / winget manifests (`docs/install.md`, `docs/release.md`)
- README (English, with a Chinese `README.zh-CN.md`), contributing guide, code of conduct, security policy, issue and PR templates
