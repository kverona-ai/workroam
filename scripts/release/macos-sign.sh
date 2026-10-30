#!/usr/bin/env bash
# macOS 代码签名与公证（发布流水线用，只在 macOS 上运行）。
#
# 用法：
#   macos-sign.sh binaries <文件...>   用 Developer ID 签名命令行二进制，并打包提交公证
#   macos-sign.sh dmg <文件>           公证 Tauri 已签名的 .dmg，并装订（staple）公证票据
#   macos-sign.sh verify-app <.app>    检查 .app 的签名、公证票据与 Gatekeeper 评估
#
# 环境变量（来自 GitHub Secrets；没有 APPLE_CERTIFICATE 时什么也不做）：
#   APPLE_CERTIFICATE           Developer ID Application 证书（.p12 的 base64）
#   APPLE_CERTIFICATE_PASSWORD  .p12 的密码
#   APPLE_SIGNING_IDENTITY      如 "Developer ID Application: Company (TEAMID)"
#   公证凭据二选一：
#     APPLE_API_KEY、APPLE_API_ISSUER、APPLE_API_KEY_PATH（App Store Connect API 密钥 .p8 的路径）
#     APPLE_ID、APPLE_PASSWORD（App 专用密码）、APPLE_TEAM_ID
#
# 兼容 macOS 自带的 bash 3.2。
set -euo pipefail

mode="${1:-}"
[ "$#" -gt 0 ] && shift

case "$mode" in
  binaries | dmg | verify-app) ;;
  *)
    echo "用法：$0 binaries <文件...> | dmg <文件> | verify-app <.app>" >&2
    exit 2
    ;;
esac

if [ -z "${APPLE_CERTIFICATE:-}" ]; then
  echo "未配置 APPLE_CERTIFICATE，跳过 macOS 签名（$mode）"
  exit 0
fi
if [ -z "${APPLE_SIGNING_IDENTITY:-}" ]; then
  echo "::error::配置了 APPLE_CERTIFICATE，但没有配置 APPLE_SIGNING_IDENTITY" >&2
  exit 1
fi

tmp="$(mktemp -d)"
keychain=""
cleanup() {
  if [ -n "$keychain" ]; then security delete-keychain "$keychain" 2>/dev/null || true; fi
  rm -rf "$tmp"
}
trap cleanup EXIT

notary=()
if [ -n "${APPLE_API_KEY:-}" ] && [ -n "${APPLE_API_ISSUER:-}" ] && [ -n "${APPLE_API_KEY_PATH:-}" ]; then
  notary=(--key "$APPLE_API_KEY_PATH" --key-id "$APPLE_API_KEY" --issuer "$APPLE_API_ISSUER")
elif [ -n "${APPLE_ID:-}" ] && [ -n "${APPLE_PASSWORD:-}" ] && [ -n "${APPLE_TEAM_ID:-}" ]; then
  notary=(--apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" --team-id "$APPLE_TEAM_ID")
fi

has_notary() { [ "${#notary[@]}" -gt 0 ]; }

notarize() { # notarize <zip|dmg>
  if ! has_notary; then
    echo "::warning::没有配置公证凭据（APPLE_API_* 或 APPLE_ID/APPLE_PASSWORD/APPLE_TEAM_ID），$(basename "$1") 只签名不公证"
    return 1
  fi
  echo "提交公证：$(basename "$1")"
  local result="$tmp/notary.json" status id
  xcrun notarytool submit "$1" "${notary[@]}" --wait --timeout 2h --output-format json > "$result" || true
  status="$(jq -r '.status // empty' "$result" 2>/dev/null || true)"
  id="$(jq -r '.id // empty' "$result" 2>/dev/null || true)"
  echo "公证结果：${status:-未知}（${id:-无 ID}）"
  if [ "$status" != Accepted ]; then
    cat "$result" >&2 || true
    if [ -n "$id" ]; then xcrun notarytool log "$id" "${notary[@]}" >&2 || true; fi
    echo "::error::$(basename "$1") 公证失败：${status:-提交失败}" >&2
    exit 1
  fi
}

import_certificate() {
  keychain="$tmp/workroam-signing.keychain-db"
  local pass
  pass="$(openssl rand -hex 16)"
  printf '%s' "$APPLE_CERTIFICATE" | base64 --decode > "$tmp/cert.p12"
  security create-keychain -p "$pass" "$keychain"
  security set-keychain-settings -lut 21600 "$keychain"
  security unlock-keychain -p "$pass" "$keychain"
  security import "$tmp/cert.p12" -k "$keychain" -P "${APPLE_CERTIFICATE_PASSWORD:-}" -T /usr/bin/codesign >/dev/null
  security set-key-partition-list -S apple-tool:,apple: -s -k "$pass" "$keychain" >/dev/null
  # 把临时钥匙串加到搜索列表最前面，codesign 才能找到证书
  # shellcheck disable=SC2046
  security list-keychains -d user -s "$keychain" $(security list-keychains -d user | tr -d '"')
  rm -f "$tmp/cert.p12"
}

case "$mode" in
  binaries)
    [ "$#" -gt 0 ] || { echo "binaries 需要至少一个文件" >&2; exit 2; }
    import_certificate
    for f in "$@"; do
      codesign --force --options runtime --timestamp --keychain "$keychain" \
        --sign "$APPLE_SIGNING_IDENTITY" "$f"
      codesign --verify --strict --verbose=2 "$f"
    done
    # 单个可执行文件不能装订票据，公证结果由 Gatekeeper 在线查询
    mkdir -p "$tmp/payload"
    cp "$@" "$tmp/payload/"
    (cd "$tmp" && ditto -c -k --keepParent payload payload.zip)
    notarize "$tmp/payload.zip" || true
    ;;
  dmg)
    [ "$#" -eq 1 ] || { echo "dmg 需要一个 .dmg 文件" >&2; exit 2; }
    codesign --verify --verbose=2 "$1"
    if notarize "$1"; then
      xcrun stapler staple "$1"
      xcrun stapler validate "$1"
    fi
    ;;
  verify-app)
    [ "$#" -eq 1 ] || { echo "verify-app 需要一个 .app" >&2; exit 2; }
    codesign --verify --deep --strict --verbose=2 "$1"
    codesign --display --verbose=2 "$1" 2>&1 | grep -E '^(Authority|TeamIdentifier|Runtime)' || true
    if has_notary; then
      xcrun stapler validate "$1"
      spctl --assess --type execute --verbose=2 "$1"
    fi
    ;;
esac
