# 安装

Workroam 的每个版本都发布在 [GitHub Releases](https://github.com/kverona-ai/workroam/releases)，三个系统各提供 x64 与 arm64 两种架构。所有文件都列在 `SHA256SUMS` 中，并附带 GitHub 构建来源证明（SLSA provenance）。

| 你要用的 | 安装什么 |
| --- | --- |
| 在电脑上使用 Workroam（推荐） | **桌面应用**：内含命令行 `workroam` 与后台服务 `workroamd`，打开后进入初始化向导 |
| 只用命令行、服务器或无图形界面的电脑 | **命令行**：`workroam` + `workroamd` |
| 让云端个人 Agent 连接（可选） | **`workroam-server`**：在 bot 的云端电脑或任意 VPS 上运行中继模式 |
| 自建存储（可选） | `install.sh`：一条命令部署 RustFS / Garage / SeaweedFS 与 Caddy（见 [技术架构 §20](architecture.md#20-部署架构)） |

## 桌面应用

从 Release 页面下载对应文件，文件名格式为 `Workroam-<版本>-<系统>-<架构>.<扩展名>`：

| 系统 | x64 | arm64 | 说明 |
| --- | --- | --- | --- |
| macOS | `macos-x64.dmg` | `macos-arm64.dmg` | Apple 芯片选 arm64，Intel 芯片选 x64 |
| Windows 10 1809+ / 11 | `windows-x64-setup.exe`、`windows-x64.msi` | `windows-arm64-setup.exe` | `setup.exe` 为当前用户安装，不需要管理员权限；`msi` 适合统一部署 |
| Linux | `linux-x64.deb` / `.rpm` / `.AppImage` | `linux-arm64.deb` / `.rpm` / `.AppImage` | 需要 WebKitGTK 4.1（Ubuntu 22.04+、Debian 12+、Fedora 38+） |

```sh
# Debian / Ubuntu
sudo apt install ./Workroam-0.1.0-linux-x64.deb
# Fedora / openSUSE
sudo dnf install ./Workroam-0.1.0-linux-x64.rpm
# 任意发行版
chmod +x Workroam-0.1.0-linux-x64.AppImage && ./Workroam-0.1.0-linux-x64.AppImage
```

包管理器：

```sh
brew install --cask kverona-ai/tap/workroam-desktop   # macOS
winget install Kverona.Workroam                        # Windows（清单收录进 winget 仓库后可用）
```

**未签名版本的提示。** 项目还没有配置代码签名证书时：

- macOS 会提示“无法验证开发者”。在“访达”中右键点 Workroam → 打开，或执行 `xattr -dr com.apple.quarantine /Applications/Workroam.app`；
- Windows SmartScreen 会提示“已保护你的电脑”，点“更多信息” → “仍要运行”。

## 命令行

一条命令安装到 `~/.local/bin`，不需要 root，会校验 SHA256：

```sh
# macOS / Linux
curl -fsSL https://github.com/kverona-ai/workroam/releases/latest/download/install-cli.sh | sh

# 指定版本或目录
curl -fsSL https://github.com/kverona-ai/workroam/releases/latest/download/install-cli.sh | sh -s -- --version 0.1.0 --dir /usr/local/bin
```

```powershell
# Windows（PowerShell），安装到 %LOCALAPPDATA%\Programs\Workroam\bin 并加入用户 PATH
irm https://github.com/kverona-ai/workroam/releases/latest/download/install-cli.ps1 | iex
```

包管理器：

```sh
brew install kverona-ai/tap/workroam                                                   # macOS / Linux
scoop install https://github.com/kverona-ai/workroam/releases/latest/download/workroam.json   # Windows
```

手动安装：下载 `workroam-<版本>-<目标>.tar.gz`（Windows 为 `.zip`），解压后把 `workroam` 与 `workroamd` 放进 `PATH`。

| 系统 | x64 | arm64 |
| --- | --- | --- |
| macOS | `x86_64-apple-darwin` | `aarch64-apple-darwin` |
| Windows | `x86_64-pc-windows-msvc` | `aarch64-pc-windows-msvc` |
| Linux（glibc 2.35+） | `x86_64-unknown-linux-gnu` | `aarch64-unknown-linux-gnu` |

装好后运行 `workroam setup` 开始初始化（与桌面向导的步骤完全一致），`workroam doctor` 检查环境。

## workroam-server（可选）

单文件二进制，默认中继模式，不需要 Docker、数据库或 root。Linux 版本为 musl 静态链接，可以直接在 bot 的云端电脑上运行：

```sh
curl -fsSL https://github.com/kverona-ai/workroam/releases/latest/download/install-server.sh | sh
workroam-server --bind <绑定码> --tunnel
```

绑定码由桌面应用生成，完整流程见 [技术架构 §18.3](architecture.md#183-部署到-bot-的云端电脑与绑定)。

容器镜像（linux/amd64、linux/arm64）：

```sh
docker run -d -p 8787:8787 -v workroam-server:/home/nonroot/.workroam-server \
  ghcr.io/kverona-ai/workroam-server:0.1.0 --listen 0.0.0.0:8787 --public-url https://relay.example.com
```

| 系统 | x64 | arm64 |
| --- | --- | --- |
| Linux（静态） | `x86_64-unknown-linux-musl` | `aarch64-unknown-linux-musl` |
| macOS | `x86_64-apple-darwin` | `aarch64-apple-darwin` |
| Windows | `x86_64-pc-windows-msvc` | `aarch64-pc-windows-msvc` |

## 校验下载

```sh
# 下载 SHA256SUMS 后，在同一目录执行（macOS 用 shasum -a 256 -c）
sha256sum -c SHA256SUMS --ignore-missing

# 验证文件确实由本仓库的发布流水线构建
gh attestation verify Workroam-0.1.0-linux-x64.deb --repo kverona-ai/workroam
```

安装脚本会自动核对 `SHA256SUMS`，校验不通过时不会写入任何文件。

## 镜像与内网

把 Release 中的文件按 `v<版本>/<文件名>` 放到自己的地址下，安装脚本即可从那里下载：

```sh
export WORKROAM_DOWNLOAD_URL=https://mirror.example.com/workroam
curl -fsSL https://mirror.example.com/workroam/v0.1.0/install-cli.sh | sh -s -- --version 0.1.0
```

PowerShell 脚本支持同名环境变量：`WORKROAM_VERSION`、`WORKROAM_INSTALL_DIR`、`WORKROAM_DOWNLOAD_URL`、`WORKROAM_COMPONENT`（`cli` 或 `server`）。

## 从源码构建

每个 Release 附带两种源码包：

- `workroam-<版本>-source.tar.gz` / `.zip`：该版本的完整源码，可直接修改；
- `workroam-<版本>-source-vendored.tar.gz`：另附全部 Rust 依赖（`vendor/` 与 `.cargo/config.toml`），没有网络时也能构建。

也可以直接克隆仓库：`git clone https://github.com/kverona-ai/workroam && git checkout v0.1.0`。

**准备**

- Rust stable（`rustup` 安装）、git；
- 桌面应用另需 Node.js 22+，以及各系统的 [Tauri v2 依赖](https://v2.tauri.app/start/prerequisites/)。Ubuntu / Debian：

  ```sh
  sudo apt install build-essential pkg-config libwebkit2gtk-4.1-dev libsoup-3.0-dev \
    libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev libdbus-1-dev
  ```

**构建命令行与服务端**

```sh
cargo build --release --locked --bin workroam --bin workroamd
cargo build --release --locked --bin workroam-server                      # 中继模式
cargo build --release --locked --bin workroam-server --features workroam-server/full   # 完整模式
# 离线构建（使用 vendored 源码包时）
cargo build --release --offline --bin workroam --bin workroamd
```

产物在 `target/release/`。静态链接的服务端：`rustup target add x86_64-unknown-linux-musl`，再加 `--target x86_64-unknown-linux-musl`。

**构建桌面应用**

```sh
scripts/release/desktop-prepare.sh "" build   # 构建前端，编译并放好随附的 workroam / workroamd
cd apps/desktop
npx @tauri-apps/cli@^2 build                  # 安装包在 src-tauri/target/release/bundle/
```

修改代码后提交 PR 前，请运行 `cargo fmt --all`、`cargo clippy --workspace --all-targets -- -D warnings` 与 `cargo test --workspace`，详见 [贡献指南](../CONTRIBUTING.md)。
