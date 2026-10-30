#!/usr/bin/env bash
# 用假的发布产物在本机走一遍 install-cli.sh 与 install-server.sh：
# 打包、生成 SHA256SUMS、从 file:// 地址下载、校验并安装，最后检查安装结果。
# 另外检查校验和不一致时安装会失败。只在 Linux / macOS 上运行。
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
version=9.9.9

case "$(uname -m)" in x86_64|amd64) arch=x86_64 ;; *) arch=aarch64 ;; esac
case "$(uname -s)" in
  Darwin) cli_target="$arch-apple-darwin"; server_target="$arch-apple-darwin" ;;
  *) cli_target="$arch-unknown-linux-gnu"; server_target="$arch-unknown-linux-musl" ;;
esac

fake_bins() {
  # fake_bins <目标> <二进制...>
  local dir="$work/target/$1/release"; shift
  mkdir -p "$dir"
  for b in "$@"; do
    printf '#!/bin/sh\necho "%s %s"\n' "$b" "$version" > "$dir/$b"
    chmod +x "$dir/$b"
  done
}

fake_bins "$cli_target" workroam workroamd
fake_bins "$server_target" workroam-server
rel="$work/releases/v$version"
mkdir -p "$rel"
export CARGO_TARGET_DIR="$work/target"
"$root/scripts/release/package-bins.sh" workroam "$version" "$cli_target" "$rel" workroam workroamd >/dev/null
"$root/scripts/release/package-bins.sh" workroam-server "$version" "$server_target" "$rel" workroam-server >/dev/null
"$root/scripts/release/install-scripts.sh" "$work/scripts" >/dev/null
(cd "$rel" && if command -v sha256sum >/dev/null; then sha256sum ./*.tar.gz; else shasum -a 256 ./*.tar.gz; fi | sed 's# \./# #' > SHA256SUMS)

export WORKROAM_DOWNLOAD_URL="file://$work/releases"
sh "$work/scripts/install-cli.sh" --version "v$version" --dir "$work/bin"
sh "$work/scripts/install-server.sh" --version "$version" --dir "$work/bin"

for b in workroam workroamd workroam-server; do
  out="$("$work/bin/$b")"
  [ "$out" = "$b $version" ] || { echo "安装结果不对：$b 输出 $out" >&2; exit 1; }
done

# 校验和被篡改时必须失败，且不能覆盖已安装的文件
# 换成一个不可能的哈希；只改首字符时，真实哈希恰好以 0 开头就等于没改（1/16 概率）
awk '{ $1 = "0000000000000000000000000000000000000000000000000000000000000000"; print }' \
  "$rel/SHA256SUMS" > "$rel/SHA256SUMS.new" && mv "$rel/SHA256SUMS.new" "$rel/SHA256SUMS"
if sh "$work/scripts/install-cli.sh" --version "$version" --dir "$work/bin2" 2>/dev/null; then
  echo "校验和不一致时安装没有失败" >&2
  exit 1
fi
[ ! -e "$work/bin2/workroam" ] || { echo "校验失败后仍然安装了文件" >&2; exit 1; }

echo "安装脚本测试通过"
