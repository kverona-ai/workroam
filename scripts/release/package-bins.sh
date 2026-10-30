#!/usr/bin/env bash
# 把已编译的二进制打成发布压缩包。
#
# 用法：package-bins.sh <包名> <版本> <目标三元组> <输出目录> <二进制...>
# 例如：package-bins.sh workroam 0.1.0 x86_64-unknown-linux-gnu dist workroam workroamd
#
# 二进制从 ${CARGO_TARGET_DIR:-target}/<目标>/release/ 读取。
# Windows 目标输出 .zip，其余输出 .tar.gz。压缩包内是一个同名目录，
# 包含二进制、LICENSE、README 与 CHANGELOG。
set -euo pipefail

if [ "$#" -lt 5 ]; then
  echo "用法：$0 <包名> <版本> <目标三元组> <输出目录> <二进制...>" >&2
  exit 2
fi

name="$1"; version="$2"; target="$3"; out="$4"; shift 4
root="$(cd "$(dirname "$0")/../.." && pwd)"
bin_dir="${CARGO_TARGET_DIR:-$root/target}/$target/release"

case "$target" in
  *windows*) exe=".exe"; ext="zip" ;;
  *) exe=""; ext="tar.gz" ;;
esac

stem="$name-$version-$target"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/$stem" "$out"

for bin in "$@"; do
  src="$bin_dir/$bin$exe"
  if [ ! -f "$src" ]; then
    echo "找不到二进制：$src" >&2
    exit 1
  fi
  cp "$src" "$work/$stem/"
  case "$target" in *windows*) ;; *) chmod 0755 "$work/$stem/$bin" ;; esac
done

for doc in LICENSE README.md README.zh-CN.md CHANGELOG.md CHANGELOG.zh-CN.md; do
  if [ -f "$root/$doc" ]; then cp "$root/$doc" "$work/$stem/"; fi
done

out_abs="$(cd "$out" && pwd)"
archive="$out_abs/$stem.$ext"
rm -f "$archive"
(
  cd "$work"
  if [ "$ext" = "zip" ]; then
    if command -v 7z >/dev/null 2>&1; then
      7z a -tzip -bso0 -bsp0 "$archive" "$stem" >/dev/null
    else
      zip -qr "$archive" "$stem"
    fi
  else
    tar -czf "$archive" "$stem"
  fi
)
echo "$archive"
