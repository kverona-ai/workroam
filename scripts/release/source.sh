#!/usr/bin/env bash
# 打源码包。
#
# 用法：source.sh <版本> <git 引用> <输出目录>
#
# 产物：
#   workroam-<版本>-source.tar.gz / .zip   该引用的完整源码（git archive）
#   workroam-<版本>-source-vendored.tar.gz 另附 vendor/ 与 .cargo/config.toml，
#                                          可用 cargo build --release --offline 离线构建
#                                          （仓库没有 Cargo.toml 时不生成）
set -euo pipefail

if [ "$#" -ne 3 ]; then
  echo "用法：$0 <版本> <git 引用> <输出目录>" >&2
  exit 2
fi

version="$1"; ref="$2"; out="$3"
root="$(cd "$(dirname "$0")/../.." && pwd)"
mkdir -p "$out"
out="$(cd "$out" && pwd)"
prefix="workroam-$version"

git -C "$root" archive --format=tar.gz --prefix="$prefix/" -o "$out/$prefix-source.tar.gz" "$ref"
git -C "$root" archive --format=zip --prefix="$prefix/" -o "$out/$prefix-source.zip" "$ref"
echo "$out/$prefix-source.tar.gz"
echo "$out/$prefix-source.zip"

if ! git -C "$root" cat-file -e "$ref:Cargo.toml" 2>/dev/null; then
  echo "该引用没有 Cargo.toml，跳过附带依赖的源码包" >&2
  exit 0
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
git -C "$root" archive --format=tar --prefix="$prefix/" "$ref" | tar -x -C "$work"
(
  cd "$work/$prefix"
  mkdir -p .cargo
  # 只把 vendor 生成的 source 替换段追加进去，保留仓库已有的 .cargo/config.toml
  extra=()
  # 桌面应用不在 workspace 里时，把它的依赖也放进 vendor/
  if [ -f apps/desktop/src-tauri/Cargo.toml ] && ! cargo metadata --format-version 1 --no-deps 2>/dev/null \
      | grep -q '"manifest_path":"[^"]*apps[/\\]desktop[/\\]src-tauri[/\\]Cargo.toml"'; then
    extra=(--sync apps/desktop/src-tauri/Cargo.toml)
  fi
  cargo vendor --locked --versioned-dirs "${extra[@]}" vendor >> .cargo/config.toml
)
tar -czf "$out/$prefix-source-vendored.tar.gz" -C "$work" "$prefix"
echo "$out/$prefix-source-vendored.tar.gz"
