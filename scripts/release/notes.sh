#!/usr/bin/env bash
# 生成 GitHub Release 说明，输出到标准输出。
#
# 用法：notes.sh <版本> <产物目录>
#
# 内容：CHANGELOG 中该版本的段落（没有时用“未发布”段落）、按平台分组的下载表
# （只列出产物目录里真实存在的文件）、安装命令与校验方法。
set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "用法：$0 <版本> <产物目录>" >&2
  exit 2
fi

version="$1"; dist="$2"
root="$(cd "$(dirname "$0")/../.." && pwd)"
repo="${GITHUB_REPOSITORY:-kverona-ai/workroam}"
base="https://github.com/$repo/releases/download/v$version"

# CHANGELOG 段落：从 "## [<版本>]" 到下一个 "## ["；找不到时取 "## [未发布]"
section() {
  awk -v head="$1" '
    index($0, "## [" head "]") == 1 { on = 1; next }
    on && /^## \[/ { exit }
    on { print }
  ' "$root/CHANGELOG.md"
}
changes="$(section "$version")"
if [ -z "$(printf "%s" "$changes" | tr -d "[:space:]")" ]; then changes="$(section "未发布")"; fi

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

cat <<NOTES
Workroam v$version：把 AI Agent 工作区带到每一台电脑上（Hand off your AI agent workspace between machines）。

## 更新内容 / What's changed
$changes

## 桌面应用 / Desktop app

桌面应用内含命令行 \`workroam\` 与后台服务 \`workroamd\`，安装后打开即进入初始化向导。

| 系统 | x64 | arm64 |
| --- | --- | --- |
| macOS | $(link "$d-macos-x64.dmg" ".dmg") · $(link "$d-macos-x64.app.tar.gz" ".app.tar.gz") | $(link "$d-macos-arm64.dmg" ".dmg") · $(link "$d-macos-arm64.app.tar.gz" ".app.tar.gz") |
| Windows | $(link "$d-windows-x64-setup.exe" "安装程序 .exe") · $(link "$d-windows-x64.msi" ".msi") | $(link "$d-windows-arm64-setup.exe" "安装程序 .exe") |
| Linux | $(link "$d-linux-x64.deb" ".deb") · $(link "$d-linux-x64.rpm" ".rpm") · $(link "$d-linux-x64.AppImage" ".AppImage") | $(link "$d-linux-arm64.deb" ".deb") · $(link "$d-linux-arm64.rpm" ".rpm") · $(link "$d-linux-arm64.AppImage" ".AppImage") |

## 命令行 / CLI（\`workroam\` + \`workroamd\`）

| 系统 | x64 | arm64 |
| --- | --- | --- |
| macOS | $(link "$c-x86_64-apple-darwin.tar.gz" ".tar.gz") | $(link "$c-aarch64-apple-darwin.tar.gz" ".tar.gz") |
| Windows | $(link "$c-x86_64-pc-windows-msvc.zip" ".zip") | $(link "$c-aarch64-pc-windows-msvc.zip" ".zip") |
| Linux（glibc 2.35+） | $(link "$c-x86_64-unknown-linux-gnu.tar.gz" ".tar.gz") | $(link "$c-aarch64-unknown-linux-gnu.tar.gz" ".tar.gz") |

\`\`\`sh
# macOS / Linux
curl -fsSL https://github.com/$repo/releases/latest/download/install-cli.sh | sh
\`\`\`

\`\`\`powershell
# Windows
irm https://github.com/$repo/releases/latest/download/install-cli.ps1 | iex
\`\`\`

## 轻量服务端 / workroam-server（可选）

单文件二进制，默认中继模式，给云端个人 Agent 用；Linux 为 musl 静态链接，可直接放在 bot 的云端电脑上运行。

| 系统 | x64 | arm64 |
| --- | --- | --- |
| Linux（静态） | $(link "$s-x86_64-unknown-linux-musl.tar.gz" ".tar.gz") | $(link "$s-aarch64-unknown-linux-musl.tar.gz" ".tar.gz") |
| macOS | $(link "$s-x86_64-apple-darwin.tar.gz" ".tar.gz") | $(link "$s-aarch64-apple-darwin.tar.gz" ".tar.gz") |
| Windows | $(link "$s-x86_64-pc-windows-msvc.zip" ".zip") | $(link "$s-aarch64-pc-windows-msvc.zip" ".zip") |

\`\`\`sh
curl -fsSL https://github.com/$repo/releases/latest/download/install-server.sh | sh
\`\`\`

容器镜像（linux/amd64、linux/arm64）：\`docker pull ghcr.io/$repo-server:$version\`

## 自建存储 / Self-hosted storage（可选）

已有 AWS S3、Cloudflare R2、阿里云 OSS 等 S3 兼容存储时不需要部署任何东西。自建时，在有公网 IP 的 Linux 服务器上一条命令部署 RustFS / Garage / SeaweedFS 与 Caddy（自动 HTTPS）：

\`\`\`sh
curl -fsSL $base/install.sh | sh -s -- --domain s3.example.com
\`\`\`

Kubernetes 可用 Helm Chart：[\`deploy/helm/workroam\`](https://github.com/$repo/tree/v$version/deploy/helm/workroam)，说明见 [deploy/README.md](https://github.com/$repo/blob/v$version/deploy/README.md)。

## 源码 / Source

- $(link "$c-source.tar.gz" "$c-source.tar.gz") · $(link "$c-source.zip" ".zip")：完整源码，可直接修改后自行构建
- $(link "$c-source-vendored.tar.gz" "$c-source-vendored.tar.gz")：附带全部 Rust 依赖（\`cargo vendor\`），离线环境可用 \`cargo build --release --offline\` 构建

构建方法见 [从源码构建](https://github.com/$repo/blob/v$version/docs/install.md#从源码构建)。

## 包管理器 / Package managers

- Homebrew：\`brew install kverona-ai/tap/workroam\`（命令行）、\`brew install --cask kverona-ai/tap/workroam-desktop\`（桌面应用）
- Scoop：\`scoop install $base/workroam.json\`
- winget 清单：$(link "winget-manifests-$version.zip" "winget-manifests-$version.zip")

## 校验 / Verify

\`\`\`sh
sha256sum -c SHA256SUMS --ignore-missing          # macOS 用 shasum -a 256 -c
gh attestation verify <文件> --repo $repo         # 验证构建来源（SLSA provenance）
\`\`\`

完整安装说明：[docs/install.md](https://github.com/$repo/blob/v$version/docs/install.md)
NOTES
