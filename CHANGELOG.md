# Changelog

**English** · [简体中文](CHANGELOG.zh-CN.md)

All notable changes to this project are recorded here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html). Each release's notes on GitHub are generated from this file (English) and `CHANGELOG.zh-CN.md` (Chinese).

## [Unreleased]

### Changed

- Docs: new bilingual deployment guide (`docs/deployment.md`) and configuration reference (`docs/configuration.md`), plus an English install guide; the README points to the published release
- Release notes are now in English by default, with the full Chinese version folded under "简体中文"; the changelog is split into English `CHANGELOG.md` and Chinese `CHANGELOG.zh-CN.md`
- New "Release notes" workflow to regenerate the notes of a published release

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
