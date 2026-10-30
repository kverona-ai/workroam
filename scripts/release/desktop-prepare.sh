#!/usr/bin/env bash
# 构建桌面应用之前的准备工作。
#
# 用法：desktop-prepare.sh [目标三元组] [build|placeholder]
#
# 1. 安装前端依赖并执行 build 脚本，生成 tauri.conf.json 里 frontendDist 指向的目录；
# 2. 准备 bundle.externalBin 列出的随附二进制（例如 binaries/workroamd），
#    按 Tauri 的约定放到 src-tauri/<路径>-<目标三元组>[.exe]：
#    build 模式用 cargo 编译真实的二进制，placeholder 模式只放空文件，
#    够 cargo clippy / cargo test 通过 tauri-build 的检查。
set -euo pipefail

target="${1:-}"
mode="${2:-placeholder}"
root="$(cd "$(dirname "$0")/../.." && pwd)"
app="$root/apps/desktop"
conf="$app/src-tauri/tauri.conf.json"

[ -n "$target" ] || target="$(rustc -vV | sed -n 's/^host: //p')"

if [ -f "$app/package.json" ]; then
  (
    cd "$app"
    if [ -f pnpm-lock.yaml ]; then
      corepack enable && pnpm install --frozen-lockfile && pnpm run --if-present build
    elif [ -f yarn.lock ]; then
      corepack enable && yarn install --immutable && yarn run build
    elif [ -f package-lock.json ]; then
      npm ci && npm run --if-present build
    else
      npm install --no-audit --no-fund && npm run --if-present build
    fi
  )
fi

[ -f "$conf" ] || exit 0

case "$target" in *windows*) exe=".exe" ;; *) exe="" ;; esac

# Windows 上的 jq 输出 CRLF，去掉 \r，否则文件名会带上回车
externals="$(jq -r '.bundle.externalBin // [] | .[]' "$conf" | tr -d '\r')"
[ -n "$externals" ] || exit 0

bins=()
for ext in $externals; do bins+=(--bin "$(basename "$ext")"); done

if [ "$mode" = build ]; then
  cargo build --release --locked --target "$target" "${bins[@]}"
fi

for ext in $externals; do
  name="$(basename "$ext")"
  dst="$app/src-tauri/$ext-$target$exe"
  mkdir -p "$(dirname "$dst")"
  if [ "$mode" = build ]; then
    cp "${CARGO_TARGET_DIR:-$root/target}/$target/release/$name$exe" "$dst"
  elif [ ! -f "$dst" ]; then
    : > "$dst"
  fi
  chmod +x "$dst" 2>/dev/null || true
  echo "随附二进制：$dst"
done
