# 持续集成与发布

本文面向维护者：CI 检查什么、如何发布一个版本、发布会产出哪些文件、需要配置哪些 secrets。用户安装说明见 [install.zh-CN.md](install.zh-CN.md)。

## 流水线

| 工作流 | 触发 | 内容 |
| --- | --- | --- |
| [`ci.yml`](../.github/workflows/ci.yml) | 每个 PR、推送到 `main` | `cargo fmt`、`clippy -D warnings`、三系统 `cargo nextest`、`workroam-server` musl 静态构建与 15 MB 体积检查（[架构 §18.1](architecture.md#181-轻量要求)）、`cargo-deny`、三系统桌面应用构建、`actionlint` 与 `shellcheck`、Helm Chart `helm lint`、安装脚本端到端测试（Linux 与 macOS） |
| [`release.yml`](../.github/workflows/release.yml) | 推送 `v*` tag；或手动运行 | 构建全部平台的产物，生成校验和、构建来源证明、包管理器清单与发布说明，发布 GitHub Release 与容器镜像 |

仓库里还没有 `Cargo.toml` 或桌面应用时，CI 中对应的任务会跳过。分支保护只需要勾选 **CI OK** 这一个检查：它在所有任务都通过或跳过时才通过。

## 发布一个版本

1. 确认 `main` 上 CI 全绿，PRD §10 的验收门槛都已达到；
2. 修改根 `Cargo.toml` 中的 `[workspace.package] version`，以及 `apps/desktop/src-tauri/` 下 `Cargo.toml`、`tauri.conf.json` 的版本和 `deploy/helm/workroam/Chart.yaml` 的 `version` / `appVersion`；在根目录运行 `cargo check` 更新根 `Cargo.lock`，在 `apps/desktop/src-tauri/` 运行 `cargo metadata --format-version 1 >/dev/null` 更新桌面应用的 `Cargo.lock`（其中还记录了 `workroam-core`、`workroam-ipc` 等路径依赖的版本，不要手改）；
3. 把英文 `CHANGELOG.md` 中的 `## [Unreleased]` 与中文 `CHANGELOG.zh-CN.md` 中的 `## [未发布]` 都改为 `## [0.1.0] - YYYY-MM-DD`，并在上方各留一个新的空段落。发布说明以英文为主（来自 `CHANGELOG.md`），中文完整版折叠在“简体中文”里（来自 `CHANGELOG.zh-CN.md`），两份要同时更新；
4. 合并上述改动后打 tag 并推送：

   ```sh
   git tag -s v0.1.0 -m "Workroam v0.1.0"
   git push origin v0.1.0
   ```

5. 等 Release 工作流完成（约 40–60 分钟），检查 [Releases](https://github.com/kverona-ai/workroam/releases) 页面。

tag 与 Cargo 版本不一致时，流水线在第一步失败，不会产出任何文件。带 `-` 的版本（如 `v0.1.0-rc.1`）发布为预发布版本，不更新容器镜像的 `latest` 与 Homebrew。

**发布前演练。** 在 Actions → Release → Run workflow 中不填 tag 直接运行：会完整构建所有产物，但不创建 Release、不推送镜像。产物在本次运行的 Artifacts 中（`release-dist` 是汇总后的全部文件），发布说明显示在运行摘要里。

**重新发布。** 某个平台失败、修复后需要重跑：Run workflow 并填写已存在的 tag，流水线会重新构建，并覆盖上传到同一个 Release。

**刷新发布说明。** 修改了 CHANGELOG 或说明模板（`scripts/release/notes.sh`）后，Actions → Release notes → Run workflow 并填写 tag，会用 `main` 上的脚本与 CHANGELOG 重新生成该版本的说明，附件不变。

**从网页发布。** 不方便推送 tag 时，在 Run workflow 中选择 `main` 并填写新的 tag（如 `v0.1.0`）：流水线会先在 `main` 的最新提交上创建 tag，再构建并发布。

## 产物

以 v0.1.0 为例：

| 类别 | 文件 |
| --- | --- |
| 桌面应用（内含 `workroam`、`workroamd`） | `Workroam-0.1.0-macos-{x64,arm64}.dmg`、`.app.tar.gz`；`Workroam-0.1.0-windows-{x64,arm64}-setup.exe`、`Workroam-0.1.0-windows-x64.msi`；`Workroam-0.1.0-linux-{x64,arm64}.{deb,rpm,AppImage}` |
| 命令行 | `workroam-0.1.0-{x86_64,aarch64}-{apple-darwin,unknown-linux-gnu}.tar.gz`、`workroam-0.1.0-{x86_64,aarch64}-pc-windows-msvc.zip` |
| 服务端 | `workroam-server-0.1.0-{x86_64,aarch64}-{unknown-linux-musl,apple-darwin}.tar.gz`、`workroam-server-0.1.0-{x86_64,aarch64}-pc-windows-msvc.zip`；镜像 `ghcr.io/kverona-ai/workroam-server:{0.1.0,0.1,latest}` |
| 源码 | `workroam-0.1.0-source.{tar.gz,zip}`、`workroam-0.1.0-source-vendored.tar.gz` |
| 安装脚本 | `install-cli.sh`、`install-cli.ps1`、`install-server.sh`、`install-server.ps1`；存储一键部署 `install.sh`（`deploy/install.sh` 存在时） |
| 包管理器清单 | `workroam.rb`（Homebrew formula）、`workroam-desktop.rb`（Homebrew cask）、`workroam.json`（Scoop）、`winget-manifests-0.1.0.zip` |
| 校验 | `SHA256SUMS`；每个文件的构建来源证明可用 `gh attestation verify` 验证 |

Windows arm64 只有 NSIS 安装程序（WiX 不支持 ARM64）。Linux 命令行在 Ubuntu 22.04 上构建，要求 glibc 2.35 及以上；`workroam-server` 是静态二进制，没有这个限制。

构建矩阵：

| 平台 | 运行环境 | 方式 |
| --- | --- | --- |
| Linux x64 / arm64 | `ubuntu-22.04` / `ubuntu-22.04-arm` | 原生构建 |
| macOS arm64 / x64 | `macos-14` | arm64 原生，x64 交叉编译 |
| Windows x64 / arm64 | `windows-latest` | x64 原生，arm64 交叉编译 |

## 代码约定

流水线依赖以下约定，改动时请同步修改工作流：

- 根目录是 Cargo workspace，版本写在 `[workspace.package] version`；提交 `Cargo.lock`（构建都带 `--locked`）；
- 二进制名：`workroam`（`crates/workroam-cli`）、`workroamd`、`workroam-server`；`workroam-server` 的完整模式是 cargo feature `full`，容器镜像使用完整模式；
- `workroam-server` 必须能以 `*-unknown-linux-musl` 静态构建（rustls，不依赖 OpenSSL），中继模式不超过 15 MB；
- 桌面应用在 `apps/desktop`，Tauri 配置在 `apps/desktop/src-tauri/tauri.conf.json`；`package.json` 的 `build` 脚本生成前端；
- 桌面应用随附的二进制写在 `bundle.externalBin`（如 `binaries/workroamd`），由 [`desktop-prepare.sh`](../scripts/release/desktop-prepare.sh) 按目标编译并放到 `src-tauri/binaries/<名字>-<目标三元组>`，不提交到仓库。

## Secrets

全部可选：没有配置时照常发布，只是对应的文件不签名或对应的渠道不更新。在 [Settings → Secrets and variables → Actions](https://github.com/kverona-ai/workroam/settings/secrets/actions) 中添加。

| Secret | 用途 | 没有时 |
| --- | --- | --- |
| `APPLE_CERTIFICATE` | Developer ID Application 证书（`.p12` 的 base64） | macOS 应用使用 ad-hoc 签名，首次打开需右键 → 打开 |
| `APPLE_CERTIFICATE_PASSWORD` | 上述 `.p12` 的密码 | |
| `APPLE_SIGNING_IDENTITY` | 例如 `Developer ID Application: Name (TEAMID)` | |
| `APPLE_ID`、`APPLE_PASSWORD`、`APPLE_TEAM_ID` | 公证（notarization）；`APPLE_PASSWORD` 为 App 专用密码 | 不公证 |
| `WINDOWS_CERTIFICATE` | 代码签名证书（`.pfx` 的 base64），用于安装程序与 `.exe` | 不签名，SmartScreen 会提示 |
| `WINDOWS_CERTIFICATE_PASSWORD` | 上述 `.pfx` 的密码 | |
| `TAURI_SIGNING_PRIVATE_KEY`、`TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | 桌面应用自动更新的签名（`npx @tauri-apps/cli signer generate` 生成） | 不生成 `.sig` 更新签名 |
| `HOMEBREW_TAP_TOKEN` | 对 tap 仓库有写权限的 fine-grained token | 不更新 tap，formula 与 cask 仍作为 Release 附件发布 |

| Variable | 用途 | 默认 |
| --- | --- | --- |
| `HOMEBREW_TAP_REPO` | Homebrew tap 仓库 | `kverona-ai/homebrew-tap` |

生成 base64：`base64 -i cert.p12 | pbcopy`（macOS）、`[Convert]::ToBase64String([IO.File]::ReadAllBytes("cert.pfx")) | Set-Clipboard`（Windows）。

`GITHUB_TOKEN` 自带，用于创建 Release、推送 `ghcr.io` 镜像与构建来源证明，不需要配置。首次推送镜像后，到组织的 Packages 页面把 `workroam-server` 设为 Public，用户才能匿名拉取。

## 包管理器渠道

- **Homebrew**：创建公开仓库 `kverona-ai/homebrew-tap` 并配置 `HOMEBREW_TAP_TOKEN` 后，每次正式发布自动更新 `Formula/workroam.rb` 与 `Casks/workroam-desktop.rb`；用户执行 `brew install kverona-ai/tap/workroam`；
- **Scoop**：清单 `workroam.json` 作为 Release 附件发布，可直接 `scoop install <地址>`；它带有 `checkver` 与 `autoupdate`，也可以提交到自建 bucket 或 Scoop Extras；
- **winget**：首次收录需要向 [microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs) 提交 PR，内容即 `winget-manifests-<版本>.zip` 中的清单（可先用 `winget validate` 与 `winget install --manifest` 本地验证）；收录之后的版本可以用 `wingetcreate update` 或 `komac` 自动提交。

## 在本机运行发布脚本

发布流水线的逻辑都在 `scripts/release/` 中，可以在本机直接运行：

```sh
scripts/release/test-install.sh                                   # 安装脚本端到端测试
scripts/release/package-bins.sh workroam 0.1.0 x86_64-unknown-linux-gnu dist workroam workroamd
scripts/release/source.sh 0.1.0 HEAD dist                         # 源码包
scripts/release/manifests.sh 0.1.0 dist                           # 包管理器清单
scripts/release/notes.sh 0.1.0 dist                               # 发布说明
```
