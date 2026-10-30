#!/bin/sh
# Workroam 安装脚本（macOS / Linux）。
#
# 发布时会生成两份：
#   install-cli.sh     安装命令行 workroam 与后台服务 workroamd
#   install-server.sh  安装可选的 workroam-server（中继模式，Linux 为静态二进制）
#
#   curl -fsSL https://github.com/kverona-ai/workroam/releases/latest/download/install-cli.sh | sh
#   curl -fsSL https://github.com/kverona-ai/workroam/releases/latest/download/install-server.sh | sh
#
# 传参：curl ... | sh -s -- --version 0.1.0 --dir ~/bin
#
# 选项（也可用环境变量）：
#   --version <版本>    WORKROAM_VERSION       默认最新版本
#   --dir <目录>        WORKROAM_INSTALL_DIR   默认 ~/.local/bin，不需要 root
#   --component <cli|server>  WORKROAM_COMPONENT
#   WORKROAM_DOWNLOAD_URL   下载地址前缀（镜像或内网），其下按 v<版本>/<文件名> 组织
set -eu

REPO="kverona-ai/workroam"
COMPONENT="${WORKROAM_COMPONENT:-cli}"
VERSION="${WORKROAM_VERSION:-latest}"
INSTALL_DIR="${WORKROAM_INSTALL_DIR:-$HOME/.local/bin}"
DOWNLOAD_URL="${WORKROAM_DOWNLOAD_URL:-https://github.com/$REPO/releases/download}"

say() { printf '%s\n' "workroam-install: $*" >&2; }
die() { say "错误：$*"; exit 1; }

while [ "$#" -gt 0 ]; do
  case "$1" in
    --version) VERSION="${2:?--version 需要一个值}"; shift 2 ;;
    --dir) INSTALL_DIR="${2:?--dir 需要一个值}"; shift 2 ;;
    --component) COMPONENT="${2:?--component 需要一个值}"; shift 2 ;;
    --cli) COMPONENT=cli; shift ;;
    --server) COMPONENT=server; shift ;;
    -h|--help)
      echo "用法：install.sh [--cli|--server] [--version <版本>] [--dir <目录>]"
      exit 0 ;;
    *) die "未知参数：$1" ;;
  esac
done

case "$COMPONENT" in
  cli) PACKAGE=workroam; BINS="workroam workroamd" ;;
  server) PACKAGE=workroam-server; BINS="workroam-server" ;;
  *) die "--component 只能是 cli 或 server" ;;
esac

if command -v curl >/dev/null 2>&1; then
  fetch() { curl -fsSL --retry 3 -o "$2" "$1"; }
  final_url() { curl -fsSLI -o /dev/null -w '%{url_effective}' "$1"; }
elif command -v wget >/dev/null 2>&1; then
  fetch() { wget -q -O "$2" "$1"; }
  final_url() { wget -q -S --spider "$1" 2>&1 | sed -n 's/^ *[Ll]ocation: *//p' | tail -n 1 | tr -d '\r'; }
else
  die "需要 curl 或 wget"
fi

if command -v sha256sum >/dev/null 2>&1; then
  sha256() { sha256sum "$1" | cut -d' ' -f1; }
elif command -v shasum >/dev/null 2>&1; then
  sha256() { shasum -a 256 "$1" | cut -d' ' -f1; }
else
  die "需要 sha256sum 或 shasum 来校验下载内容"
fi

# 平台
os="$(uname -s)"
arch="$(uname -m)"
case "$arch" in
  x86_64|amd64) arch=x86_64 ;;
  aarch64|arm64) arch=aarch64 ;;
  *) die "不支持的处理器架构：$arch（支持 x86_64 与 aarch64）" ;;
esac
case "$os" in
  Darwin)
    # 在 Rosetta 下运行的终端也装原生 arm64 版本
    if [ "$arch" = x86_64 ] && [ "$(sysctl -n hw.optional.arm64 2>/dev/null || echo 0)" = 1 ]; then
      arch=aarch64
    fi
    target="$arch-apple-darwin"
    ;;
  Linux)
    if [ "$COMPONENT" = server ]; then
      target="$arch-unknown-linux-musl"
    else
      target="$arch-unknown-linux-gnu"
      if ldd --version 2>&1 | grep -qi musl; then
        die "命令行需要 glibc 2.35 或更新版本；当前系统使用 musl（如 Alpine），请从源码构建"
      fi
    fi
    ;;
  MINGW*|MSYS*|CYGWIN*) die "Windows 请在 PowerShell 中运行：irm https://github.com/$REPO/releases/latest/download/install-cli.ps1 | iex" ;;
  *) die "不支持的系统：$os" ;;
esac

# 版本
if [ "$VERSION" = latest ]; then
  url="$(final_url "https://github.com/$REPO/releases/latest" || true)"
  VERSION="${url##*/tag/v}"
  case "$VERSION" in
    ''|*/*) die "无法获取最新版本号，请用 --version 指定" ;;
  esac
fi
VERSION="${VERSION#v}"

archive="$PACKAGE-$VERSION-$target.tar.gz"
base="$DOWNLOAD_URL/v$VERSION"
tmp="$(mktemp -d 2>/dev/null || mktemp -d -t workroam)"
trap 'rm -rf "$tmp"' EXIT INT TERM

say "下载 $archive"
fetch "$base/$archive" "$tmp/$archive" || die "下载失败：$base/$archive"
fetch "$base/SHA256SUMS" "$tmp/SHA256SUMS" || die "下载校验和失败：$base/SHA256SUMS"

expected="$(awk -v f="$archive" '{ n = $2; sub(/^\*/, "", n) } n == f { print $1 }' "$tmp/SHA256SUMS")"
[ -n "$expected" ] || die "SHA256SUMS 中没有 $archive"
actual="$(sha256 "$tmp/$archive")"
[ "$expected" = "$actual" ] || die "校验和不一致：期望 $expected，实际 $actual"

tar -xzf "$tmp/$archive" -C "$tmp"
mkdir -p "$INSTALL_DIR"
for bin in $BINS; do
  src="$tmp/$PACKAGE-$VERSION-$target/$bin"
  [ -f "$src" ] || die "压缩包中缺少 $bin"
  # 先写临时文件再改名，正在运行的旧版本不受影响
  cp "$src" "$INSTALL_DIR/.$bin.new"
  chmod 0755 "$INSTALL_DIR/.$bin.new"
  mv -f "$INSTALL_DIR/.$bin.new" "$INSTALL_DIR/$bin"
  say "已安装 $INSTALL_DIR/$bin"
done

case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *) say "提示：$INSTALL_DIR 不在 PATH 中，请把下面这行加入 shell 配置文件："
     say "  export PATH=\"$INSTALL_DIR:\$PATH\"" ;;
esac

if [ "$COMPONENT" = server ]; then
  say "完成。启动中继：workroam-server --bind <绑定码> --tunnel"
else
  say "完成。运行 workroam setup 开始初始化，或安装桌面应用使用图形向导。"
fi
