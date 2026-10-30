#!/usr/bin/env bash
# 用发布用的 GPG 密钥给 SHA256SUMS 签名，并附上公钥。
#
# 用法：gpg-sign.sh <产物目录>
#
# 环境变量（来自 GitHub Secrets；没有 GPG_PRIVATE_KEY 时什么也不做）：
#   GPG_PRIVATE_KEY  ASCII 格式的私钥（gpg --armor --export-secret-keys）
#   GPG_PASSPHRASE   私钥密码，没有密码时留空
#
# 产出：
#   SHA256SUMS.asc            SHA256SUMS 的分离签名
#   workroam-release-key.asc  对应的公钥，方便用户导入
set -euo pipefail

if [ "$#" -ne 1 ]; then
  echo "用法：$0 <产物目录>" >&2
  exit 2
fi
dist="$1"

if [ -z "${GPG_PRIVATE_KEY:-}" ]; then
  echo "未配置 GPG_PRIVATE_KEY，SHA256SUMS 不做 GPG 签名"
  exit 0
fi
[ -f "$dist/SHA256SUMS" ] || { echo "找不到 $dist/SHA256SUMS" >&2; exit 1; }

GNUPGHOME="$(mktemp -d)"
export GNUPGHOME
trap 'gpgconf --kill gpg-agent 2>/dev/null || true; rm -rf "$GNUPGHOME"' EXIT

printf '%s\n' "$GPG_PRIVATE_KEY" | gpg --batch --quiet --import
fpr="$(gpg --batch --list-secret-keys --with-colons | awk -F: '/^fpr:/ { print $10; exit }')"
[ -n "$fpr" ] || { echo "::error::GPG_PRIVATE_KEY 里没有私钥" >&2; exit 1; }

printf '%s' "${GPG_PASSPHRASE:-}" | gpg --batch --yes --pinentry-mode loopback --passphrase-fd 0 \
  --local-user "$fpr" --armor --detach-sign --output "$dist/SHA256SUMS.asc" "$dist/SHA256SUMS"
gpg --batch --armor --export "$fpr" > "$dist/workroam-release-key.asc"
gpg --batch --verify "$dist/SHA256SUMS.asc" "$dist/SHA256SUMS"
echo "已用 GPG 密钥 $fpr 签名 SHA256SUMS"
