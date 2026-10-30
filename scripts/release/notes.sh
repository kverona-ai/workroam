#!/usr/bin/env bash
# 生成 GitHub Release 说明，输出到标准输出。
#
# 用法：notes.sh <版本> <产物目录>
#
# 英文在前，中文完整版折叠在 “简体中文” 里。内容：CHANGELOG.md / CHANGELOG.zh-CN.md
# 中该版本的段落（没有时用 Unreleased / 未发布 段落）、按平台分组的下载表（只列出
# 产物目录里真实存在的文件）、安装命令与校验方法。
#
# DOCS_REF：文档链接指向的 git 引用，默认 v<版本>；为已发布的旧版本刷新说明时可设为 main。
set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "用法：$0 <版本> <产物目录>" >&2
  exit 2
fi

version="$1"; dist="$2"
root="$(cd "$(dirname "$0")/../.." && pwd)"
repo="${GITHUB_REPOSITORY:-kverona-ai/workroam}"
base="https://github.com/$repo/releases/download/v$version"
docs="https://github.com/$repo/blob/${DOCS_REF:-v$version}/docs"

# CHANGELOG 段落：从 "## [<版本>]" 到下一个 "## ["；找不到时取未发布段落
section() { # section <文件> <标题>
  [ -f "$1" ] || return 0
  awk -v head="$2" '
    index($0, "## [" head "]") == 1 { on = 1; next }
    on && /^## \[/ { exit }
    on { print }
  ' "$1"
}
changes() { # changes <文件> <未发布标题>
  local s
  s="$(section "$1" "$version")"
  if [ -z "$(printf "%s" "$s" | tr -d "[:space:]")" ]; then s="$(section "$1" "$2")"; fi
  printf '%s\n' "$s"
}
changes_en="$(changes "$root/CHANGELOG.md" "Unreleased")"
changes_zh="$(changes "$root/CHANGELOG.zh-CN.md" "未发布")"

link() {
  # link <文件名> <显示文字>：文件存在时输出链接，否则输出 “—”
  if [ -f "$dist/$1" ]; then
    printf '[%s](%s/%s)' "$2" "$base" "$1"
  else
    printf '—'
  fi
}

d="Workroam-$version"
c="workroam-$version"
s="workroam-server-$version"

# 两种语言共用的下载表（第一列的系统名按语言传入）
desktop_table() { # desktop_table <表头> <安装程序文字> <Linux>
  cat <<EOF
| $1 | x64 | arm64 |
| --- | --- | --- |
| macOS | $(link "$d-macos-x64.dmg" ".dmg") · $(link "$d-macos-x64.app.tar.gz" ".app.tar.gz") | $(link "$d-macos-arm64.dmg" ".dmg") · $(link "$d-macos-arm64.app.tar.gz" ".app.tar.gz") |
| Windows | $(link "$d-windows-x64-setup.exe" "$2 .exe") · $(link "$d-windows-x64.msi" ".msi") | $(link "$d-windows-arm64-setup.exe" "$2 .exe") |
| $3 | $(link "$d-linux-x64.deb" ".deb") · $(link "$d-linux-x64.rpm" ".rpm") · $(link "$d-linux-x64.AppImage" ".AppImage") | $(link "$d-linux-arm64.deb" ".deb") · $(link "$d-linux-arm64.rpm" ".rpm") · $(link "$d-linux-arm64.AppImage" ".AppImage") |
EOF
}
cli_table() { # cli_table <表头> <Linux 行名>
  cat <<EOF
| $1 | x64 | arm64 |
| --- | --- | --- |
| macOS | $(link "$c-x86_64-apple-darwin.tar.gz" ".tar.gz") | $(link "$c-aarch64-apple-darwin.tar.gz" ".tar.gz") |
| Windows | $(link "$c-x86_64-pc-windows-msvc.zip" ".zip") | $(link "$c-aarch64-pc-windows-msvc.zip" ".zip") |
| $2 | $(link "$c-x86_64-unknown-linux-gnu.tar.gz" ".tar.gz") | $(link "$c-aarch64-unknown-linux-gnu.tar.gz" ".tar.gz") |
EOF
}
server_table() { # server_table <表头> <Linux 行名>
  cat <<EOF
| $1 | x64 | arm64 |
| --- | --- | --- |
| $2 | $(link "$s-x86_64-unknown-linux-musl.tar.gz" ".tar.gz") | $(link "$s-aarch64-unknown-linux-musl.tar.gz" ".tar.gz") |
| macOS | $(link "$s-x86_64-apple-darwin.tar.gz" ".tar.gz") | $(link "$s-aarch64-apple-darwin.tar.gz" ".tar.gz") |
| Windows | $(link "$s-x86_64-pc-windows-msvc.zip" ".zip") | $(link "$s-aarch64-pc-windows-msvc.zip" ".zip") |
EOF
}

cat <<NOTES
Workroam v$version: hand off your AI agent workspace between machines.

> 中文说明：展开文末的 **简体中文**。

## What's changed
$changes_en

## Desktop app

The desktop app includes the \`workroam\` CLI and the \`workroamd\` background service, and opens the setup wizard on first launch.

$(desktop_table "System" "Installer" "Linux")

## CLI (\`workroam\` + \`workroamd\`)

$(cli_table "System" "Linux (glibc 2.35+)")

\`\`\`sh
# macOS / Linux
curl -fsSL https://github.com/$repo/releases/latest/download/install-cli.sh | sh
\`\`\`

\`\`\`powershell
# Windows
irm https://github.com/$repo/releases/latest/download/install-cli.ps1 | iex
\`\`\`

## workroam-server (optional)

A single-file binary that runs in relay mode by default, for cloud personal agents. The Linux builds are statically linked with musl and run directly on a bot's cloud computer.

$(server_table "System" "Linux (static)")

\`\`\`sh
curl -fsSL https://github.com/$repo/releases/latest/download/install-server.sh | sh
\`\`\`

Container image (linux/amd64, linux/arm64): \`docker pull ghcr.io/$repo-server:$version\`

## Self-hosted storage (optional)

If you already use AWS S3, Cloudflare R2, Alibaba Cloud OSS or another S3-compatible service, there is nothing to deploy. To self-host, one command on a Linux server with a public IP deploys RustFS / Garage / SeaweedFS with Caddy (automatic HTTPS):

\`\`\`sh
curl -fsSL $base/install.sh | sh -s -- --domain s3.example.com
\`\`\`

For Kubernetes, use the Helm chart [\`deploy/helm/workroam\`](https://github.com/$repo/tree/v$version/deploy/helm/workroam). See the [deployment guide]($docs/deployment.md).

## Source

- $(link "$c-source.tar.gz" "$c-source.tar.gz") · $(link "$c-source.zip" ".zip"): the complete source, ready to edit and build yourself
- $(link "$c-source-vendored.tar.gz" "$c-source-vendored.tar.gz"): also contains every Rust dependency (\`cargo vendor\`), so \`cargo build --release --offline\` works without network access

See [Build from source]($docs/install.md#build-from-source).

## Package managers

- Homebrew: \`brew install kverona-ai/tap/workroam\` (CLI), \`brew install --cask kverona-ai/tap/workroam-desktop\` (desktop app)
- Scoop: \`scoop install $base/workroam.json\`
- winget manifests: $(link "winget-manifests-$version.zip" "winget-manifests-$version.zip")

## Verify

\`\`\`sh
sha256sum -c SHA256SUMS --ignore-missing          # on macOS: shasum -a 256 -c
gh attestation verify <file> --repo $repo         # check build provenance (SLSA)
\`\`\`

Full guides: [Install]($docs/install.md) · [Deployment]($docs/deployment.md) · [Configuration]($docs/configuration.md)

<details>
<summary><b>简体中文</b></summary>

Workroam v$version：把 AI Agent 工作区带到每一台电脑上。

### 更新内容
$(printf '%s\n' "$changes_zh" | sed 's/^### /#### /')

### 桌面应用

桌面应用内含命令行 \`workroam\` 与后台服务 \`workroamd\`，安装后打开即进入初始化向导。

$(desktop_table "系统" "安装程序" "Linux")

### 命令行（\`workroam\` + \`workroamd\`）

$(cli_table "系统" "Linux（glibc 2.35+）")

\`\`\`sh
# macOS / Linux
curl -fsSL https://github.com/$repo/releases/latest/download/install-cli.sh | sh
\`\`\`

\`\`\`powershell
# Windows
irm https://github.com/$repo/releases/latest/download/install-cli.ps1 | iex
\`\`\`

### 轻量服务端 workroam-server（可选）

单文件二进制，默认中继模式，给云端个人 Agent 用；Linux 为 musl 静态链接，可直接放在 bot 的云端电脑上运行。

$(server_table "系统" "Linux（静态）")

\`\`\`sh
curl -fsSL https://github.com/$repo/releases/latest/download/install-server.sh | sh
\`\`\`

容器镜像（linux/amd64、linux/arm64）：\`docker pull ghcr.io/$repo-server:$version\`

### 自建存储（可选）

已有 AWS S3、Cloudflare R2、阿里云 OSS 等 S3 兼容存储时不需要部署任何东西。自建时，在有公网 IP 的 Linux 服务器上一条命令部署 RustFS / Garage / SeaweedFS 与 Caddy（自动 HTTPS）：

\`\`\`sh
curl -fsSL $base/install.sh | sh -s -- --domain s3.example.com
\`\`\`

Kubernetes 可用 Helm Chart：[\`deploy/helm/workroam\`](https://github.com/$repo/tree/v$version/deploy/helm/workroam)，说明见 [部署指南]($docs/deployment.zh-CN.md)。

### 源码

- $(link "$c-source.tar.gz" "$c-source.tar.gz") · $(link "$c-source.zip" ".zip")：完整源码，可直接修改后自行构建
- $(link "$c-source-vendored.tar.gz" "$c-source-vendored.tar.gz")：附带全部 Rust 依赖（\`cargo vendor\`），离线环境可用 \`cargo build --release --offline\` 构建

构建方法见 [从源码构建]($docs/install.zh-CN.md#从源码构建)。

### 包管理器

- Homebrew：\`brew install kverona-ai/tap/workroam\`（命令行）、\`brew install --cask kverona-ai/tap/workroam-desktop\`（桌面应用）
- Scoop：\`scoop install $base/workroam.json\`
- winget 清单：$(link "winget-manifests-$version.zip" "winget-manifests-$version.zip")

### 校验

\`\`\`sh
sha256sum -c SHA256SUMS --ignore-missing          # macOS 用 shasum -a 256 -c
gh attestation verify <文件> --repo $repo         # 验证构建来源（SLSA provenance）
\`\`\`

完整文档：[安装]($docs/install.zh-CN.md) · [部署指南]($docs/deployment.zh-CN.md) · [配置说明]($docs/configuration.zh-CN.md)

</details>
NOTES
