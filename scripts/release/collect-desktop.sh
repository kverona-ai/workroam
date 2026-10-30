#!/usr/bin/env bash
# 收集 Tauri 打出的桌面安装包，按统一的名字复制到输出目录。
#
# 用法：collect-desktop.sh <版本> <目标三元组> <平台> <架构> <输出目录>
# 例如：collect-desktop.sh 0.1.0 aarch64-apple-darwin macos arm64 dist
#
# 产物命名：Workroam-<版本>-<平台>-<架构>.<扩展名>
#   macOS    .dmg、.app.tar.gz
#   Windows  .msi、-setup.exe
#   Linux    .deb、.rpm、.AppImage
# 如果 Tauri 生成了更新签名（.sig），一并按新名字复制。
set -euo pipefail

if [ "$#" -ne 5 ]; then
  echo "用法：$0 <版本> <目标三元组> <平台> <架构> <输出目录>" >&2
  exit 2
fi

version="$1"; target="$2"; platform="$3"; arch="$4"; out="$5"
root="$(cd "$(dirname "$0")/../.." && pwd)"
bundle="${CARGO_TARGET_DIR:-$root/target}/$target/release/bundle"
stem="Workroam-$version-$platform-$arch"
mkdir -p "$out"
found=0

copy() {
  # copy <源文件> <新文件名>
  cp "$1" "$out/$2"
  if [ -f "$1.sig" ]; then cp "$1.sig" "$out/$2.sig"; fi
  echo "$out/$2"
  found=$((found + 1))
}

first() {
  # 返回匹配的第一个文件；没有时返回空
  for f in "$@"; do
    if [ -e "$f" ]; then echo "$f"; return 0; fi
  done
  return 0
}

pick() {
  # pick <新文件名> <候选...>：复制第一个存在的候选文件
  local name="$1" f; shift
  f="$(first "$@")"
  if [ -n "$f" ]; then copy "$f" "$name"; fi
}

if [ ! -d "$bundle" ]; then
  echo "找不到打包目录：$bundle" >&2
  exit 1
fi

case "$platform" in
  macos)
    pick "$stem.dmg" "$bundle"/dmg/*.dmg
    app="$(first "$bundle"/macos/*.app)"
    if [ -n "$app" ]; then
      tar -czf "$out/$stem.app.tar.gz" -C "$(dirname "$app")" "$(basename "$app")"
      echo "$out/$stem.app.tar.gz"
      found=$((found + 1))
    fi
    ;;
  windows)
    pick "$stem.msi" "$bundle"/msi/*.msi
    pick "$stem-setup.exe" "$bundle"/nsis/*-setup.exe "$bundle"/nsis/*.exe
    ;;
  linux)
    pick "$stem.deb" "$bundle"/deb/*.deb
    pick "$stem.rpm" "$bundle"/rpm/*.rpm
    pick "$stem.AppImage" "$bundle"/appimage/*.AppImage
    ;;
  *)
    echo "未知平台：$platform" >&2
    exit 2
    ;;
esac

if [ "$found" -eq 0 ]; then
  echo "在 $bundle 下没有找到 $platform 安装包" >&2
  exit 1
fi
