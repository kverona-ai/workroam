#!/usr/bin/env bash
# 生成发布用的安装脚本：install-cli.sh、install-server.sh、install-cli.ps1、install-server.ps1。
#
# 用法：install-scripts.sh <输出目录>
set -euo pipefail

out="$1"
root="$(cd "$(dirname "$0")/../.." && pwd)"
mkdir -p "$out"

cp "$root/scripts/install.sh" "$out/install-cli.sh"
sed 's/WORKROAM_COMPONENT:-cli}/WORKROAM_COMPONENT:-server}/' "$root/scripts/install.sh" > "$out/install-server.sh"
grep -q 'WORKROAM_COMPONENT:-server' "$out/install-server.sh" || { echo "生成 install-server.sh 失败" >&2; exit 1; }

cp "$root/scripts/install.ps1" "$out/install-cli.ps1"
sed "s/else { 'cli' }/else { 'server' }/" "$root/scripts/install.ps1" > "$out/install-server.ps1"
grep -q "else { 'server' }" "$out/install-server.ps1" || { echo "生成 install-server.ps1 失败" >&2; exit 1; }

chmod 0755 "$out/install-cli.sh" "$out/install-server.sh"
# 存储一键部署脚本（DEP-01）存在时一起发布
if [ -f "$root/deploy/install.sh" ]; then install -m 0755 "$root/deploy/install.sh" "$out/install.sh"; fi
ls "$out"/install*
