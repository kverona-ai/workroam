# Install

**English** · [简体中文](install.zh-CN.md)

Every Workroam release is published on [GitHub Releases](https://github.com/kverona-ai/workroam/releases), with builds for all three systems on both x64 and arm64. Every file is listed in `SHA256SUMS` and carries a GitHub build provenance attestation (SLSA provenance).

| You want to | Install |
| --- | --- |
| Use Workroam on your computer (recommended) | **Desktop app**: includes the `workroam` CLI and the `workroamd` background service, and opens the setup wizard on first launch |
| Use only the command line, on a server or a machine without a desktop | **CLI**: `workroam` + `workroamd` |
| Let a cloud personal agent connect (optional) | **`workroam-server`**: relay mode on a bot's cloud computer or any VPS |
| Self-host your storage (optional) | `install.sh`: one command deploys RustFS / Garage / SeaweedFS with Caddy; see the [deployment guide](deployment.md) |

## Desktop app

Download the file for your system from the Release page. File names follow `Workroam-<version>-<os>-<arch>.<ext>`:

| System | x64 | arm64 | Notes |
| --- | --- | --- | --- |
| macOS | `macos-x64.dmg` | `macos-arm64.dmg` | Choose arm64 for Apple silicon, x64 for Intel |
| Windows 10 1809+ / 11 | `windows-x64-setup.exe`, `windows-x64.msi` | `windows-arm64-setup.exe` | `setup.exe` installs for the current user without admin rights; the `msi` suits managed deployment |
| Linux | `linux-x64.deb` / `.rpm` / `.AppImage` | `linux-arm64.deb` / `.rpm` / `.AppImage` | Needs WebKitGTK 4.1 (Ubuntu 22.04+, Debian 12+, Fedora 38+) |

```sh
# Debian / Ubuntu
sudo apt install ./Workroam-0.1.0-linux-x64.deb
# Fedora / openSUSE
sudo dnf install ./Workroam-0.1.0-linux-x64.rpm
# Any distribution
chmod +x Workroam-0.1.0-linux-x64.AppImage && ./Workroam-0.1.0-linux-x64.AppImage
```

Package managers:

```sh
brew install --cask kverona-ai/tap/workroam-desktop   # macOS
winget install Kverona.Workroam                        # Windows (once the manifest is accepted into winget)
```

**Unsigned builds.** Until the project has code signing certificates configured:

- macOS says it "cannot verify the developer". Right-click Workroam in Finder → Open, or run `xattr -dr com.apple.quarantine /Applications/Workroam.app`;
- Windows SmartScreen says "Windows protected your PC". Click "More info" → "Run anyway".

## Command line

One command installs to `~/.local/bin`, needs no root and verifies SHA256:

```sh
# macOS / Linux
curl -fsSL https://github.com/kverona-ai/workroam/releases/latest/download/install-cli.sh | sh

# A specific version or directory
curl -fsSL https://github.com/kverona-ai/workroam/releases/latest/download/install-cli.sh | sh -s -- --version 0.1.0 --dir /usr/local/bin
```

```powershell
# Windows (PowerShell): installs to %LOCALAPPDATA%\Programs\Workroam\bin and adds it to the user PATH
irm https://github.com/kverona-ai/workroam/releases/latest/download/install-cli.ps1 | iex
```

Package managers:

```sh
brew install kverona-ai/tap/workroam                                                          # macOS / Linux
scoop install https://github.com/kverona-ai/workroam/releases/latest/download/workroam.json   # Windows
```

Manual install: download `workroam-<version>-<target>.tar.gz` (`.zip` on Windows), extract it, and put `workroam` and `workroamd` on your `PATH`.

| System | x64 | arm64 |
| --- | --- | --- |
| macOS | `x86_64-apple-darwin` | `aarch64-apple-darwin` |
| Windows | `x86_64-pc-windows-msvc` | `aarch64-pc-windows-msvc` |
| Linux (glibc 2.35+) | `x86_64-unknown-linux-gnu` | `aarch64-unknown-linux-gnu` |

Then run `workroam setup` to set up (the same steps as the desktop wizard) and `workroam doctor` to check the environment. Configuration files, settings and environment variables are described in [Configuration](configuration.md).

## workroam-server (optional)

A single-file binary. It runs in relay mode by default and needs no Docker, database or root. The Linux builds are statically linked with musl, so they run directly on a bot's cloud computer:

```sh
curl -fsSL https://github.com/kverona-ai/workroam/releases/latest/download/install-server.sh | sh
workroam-server --bind <bind code> --tunnel
```

The desktop app generates the bind code. The whole flow is in [Deployment · Cloud agent relay](deployment.md#5-cloud-agent-relay-workroam-server-relay-mode).

Container image (linux/amd64, linux/arm64):

```sh
docker run -d -p 8787:8787 -v workroam-server:/home/nonroot/.workroam-server \
  ghcr.io/kverona-ai/workroam-server:0.1.0 --listen 0.0.0.0:8787 --public-url https://relay.example.com
```

| System | x64 | arm64 |
| --- | --- | --- |
| Linux (static) | `x86_64-unknown-linux-musl` | `aarch64-unknown-linux-musl` |
| macOS | `x86_64-apple-darwin` | `aarch64-apple-darwin` |
| Windows | `x86_64-pc-windows-msvc` | `aarch64-pc-windows-msvc` |

## Verify downloads

```sh
# After downloading SHA256SUMS, run in the same directory (on macOS: shasum -a 256 -c)
sha256sum -c SHA256SUMS --ignore-missing

# Check that a file was built by this repository's release workflow
gh attestation verify Workroam-0.1.0-linux-x64.deb --repo kverona-ai/workroam
```

The install scripts check `SHA256SUMS` themselves and write nothing when a checksum does not match.

## Mirrors and intranets

Put the Release files at your own address as `v<version>/<file name>`, and the install scripts will download from there:

```sh
export WORKROAM_DOWNLOAD_URL=https://mirror.example.com/workroam
curl -fsSL https://mirror.example.com/workroam/v0.1.0/install-cli.sh | sh -s -- --version 0.1.0
```

The PowerShell script reads the same environment variables: `WORKROAM_VERSION`, `WORKROAM_INSTALL_DIR`, `WORKROAM_DOWNLOAD_URL` and `WORKROAM_COMPONENT` (`cli` or `server`).

## Build from source

Every Release has two kinds of source package:

- `workroam-<version>-source.tar.gz` / `.zip`: the complete source of that version, ready to edit;
- `workroam-<version>-source-vendored.tar.gz`: also contains every Rust dependency (`vendor/` and `.cargo/config.toml`), so it builds without network access.

You can also clone the repository: `git clone https://github.com/kverona-ai/workroam && git checkout v0.1.0`.

**Prerequisites**

- Rust stable (install with `rustup`) and git;
- for the desktop app, also Node.js 22+ and the [Tauri v2 prerequisites](https://v2.tauri.app/start/prerequisites/) for your system. On Ubuntu / Debian:

  ```sh
  sudo apt install build-essential pkg-config libwebkit2gtk-4.1-dev libsoup-3.0-dev \
    libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev libdbus-1-dev
  ```

**Build the CLI and the server**

```sh
cargo build --release --locked --bin workroam --bin workroamd
cargo build --release --locked --bin workroam-server                                  # relay mode
cargo build --release --locked --bin workroam-server --features workroam-server/full   # full mode
# Offline build (with the vendored source package)
cargo build --release --offline --bin workroam --bin workroamd
```

The binaries are in `target/release/`. For a statically linked server, run `rustup target add x86_64-unknown-linux-musl` and add `--target x86_64-unknown-linux-musl`.

**Build the desktop app**

```sh
scripts/release/desktop-prepare.sh "" build   # builds the UI, compiles and places the bundled workroam / workroamd
cd apps/desktop
npx @tauri-apps/cli@^2 build                  # installers land in src-tauri/target/release/bundle/
```

Before opening a PR, run `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace`; see the [contributing guide](../CONTRIBUTING.md).
