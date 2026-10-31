#!/usr/bin/env bash
# macOS 代码签名与公证（发布流水线用，只在 macOS 上运行）。
#
# 用法：
#   macos-sign.sh binaries <文件...>   用 Developer ID 签名命令行二进制，并打包提交公证
#   macos-sign.sh app <.app>           公证 Tauri 已签名的 .app，并装订（staple）公证票据
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
#   NOTARY_TIMEOUT_SECONDS      命令行与 .app 公证最长等多久，默认 14400（4 小时），超时即失败
#   NOTARY_DMG_TIMEOUT_SECONDS  .dmg 公证最长等多久，默认 2700（45 分钟），超时只警告、不装订
#
# 兼容 macOS 自带的 bash 3.2。
set -euo pipefail

mode="${1:-}"
[ "$#" -gt 0 ] && shift

case "$mode" in
  binaries | app | dmg | verify-app) ;;
  *)
    echo "用法：$0 binaries <文件...> | app <.app> | dmg <文件> | verify-app <.app>" >&2
    exit 2
    ;;
esac

if [ -z "${APPLE_CERTIFICATE:-}" ]; then
  echo "未配置 APPLE_CERTIFICATE，跳过 macOS 签名（${mode}）"
  exit 0
fi
if [ -z "${APPLE_SIGNING_IDENTITY:-}" ]; then
  echo "::error::配置了 APPLE_CERTIFICATE，但没有配置 APPLE_SIGNING_IDENTITY" >&2
  exit 1
fi

NOTARY_TIMEOUT_SECONDS="${NOTARY_TIMEOUT_SECONDS:-14400}"
NOTARY_DMG_TIMEOUT_SECONDS="${NOTARY_DMG_TIMEOUT_SECONDS:-2700}"
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

notarize() { # notarize <zip|dmg> [等待秒数] [soft]
  # 返回 0 = 已通过；1 = 没有公证凭据；2 = soft 模式下等待超时（已提交，Apple 稍后会处理完）
  if ! has_notary; then
    echo "::warning::没有配置公证凭据（APPLE_API_* 或 APPLE_ID/APPLE_PASSWORD/APPLE_TEAM_ID），$(basename "$1") 只签名不公证"
    return 1
  fi
  local name submit="$tmp/notary-submit.json" result="$tmp/notary.json" status="" id="" attempt deadline
  local wait_seconds="${2:-$NOTARY_TIMEOUT_SECONDS}" soft="${3:-}"
  name="$(basename "$1")"

  # 先上传拿到提交 ID，再单独等待结果。Apple 处理可能要几十分钟，
  # 等待期间网络断一下不能让整个任务失败，所以等待失败时用同一个 ID 继续等。
  for attempt in 1 2 3; do
    echo "提交公证：${name}（第 ${attempt} 次）"
    if xcrun notarytool submit "$1" "${notary[@]}" --output-format json > "$submit"; then
      id="$(jq -r '.id // empty' "$submit" 2>/dev/null || true)"
      [ -n "$id" ] && break
    fi
    cat "$submit" >&2 || true
    sleep 30
  done
  if [ -z "$id" ]; then
    echo "::error::${name} 提交公证失败" >&2
    exit 1
  fi
  echo "提交 ID：${id}，等待 Apple 处理"

  deadline=$(($(date +%s) + wait_seconds))
  while :; do
    xcrun notarytool wait "$id" "${notary[@]}" --timeout 20m --output-format json > "$result" 2>"$tmp/notary-wait.err" || true
    status="$(jq -r '.status // empty' "$result" 2>/dev/null || true)"
    case "$status" in
      Accepted | Invalid | Rejected) break ;;
    esac
    if [ "$(date +%s)" -ge "$deadline" ]; then
      if [ "$soft" = soft ]; then
        echo "::warning::${name} 等了 $((wait_seconds / 60)) 分钟 Apple 还没处理完（${id}），先不装订票据。Apple 处理完后 Gatekeeper 会在线查到公证结果"
        return 2
      fi
      echo "::error::${name} 公证等待超时（${id}）" >&2
      cat "$tmp/notary-wait.err" >&2 || true
      exit 1
    fi
    if [ -z "$status" ] && grep -qs 'Timeout' "$result" "$tmp/notary-wait.err"; then
      status="Apple 仍在处理"
    fi
    echo "仍在等待公证结果（${status:-暂时连不上 Apple}），30 秒后继续"
    sleep 30
  done

  echo "公证结果：${status}（${id}）"
  if [ "$status" != Accepted ]; then
    cat "$result" >&2 || true
    xcrun notarytool log "$id" "${notary[@]}" >&2 || true
    echo "::error::${name} 公证失败：${status}" >&2
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
  app)
    [ "$#" -eq 1 ] || { echo "app 需要一个 .app" >&2; exit 2; }
    codesign --verify --deep --strict --verbose=2 "$1"
    (cd "$(dirname "$1")" && ditto -c -k --keepParent "$(basename "$1")" "$tmp/app.zip")
    if notarize "$tmp/app.zip"; then
      xcrun stapler staple "$1"
      xcrun stapler validate "$1"
    fi
    ;;
  dmg)
    [ "$#" -eq 1 ] || { echo "dmg 需要一个 .dmg 文件" >&2; exit 2; }
    codesign --verify --verbose=2 "$1"
    # 里面的 .app 已经公证并装订了票据，.dmg 的公证只是锦上添花：
    # Apple 偶尔要排队一两个小时，等不到就带着签名发布，不让整个发布失败
    if notarize "$1" "$NOTARY_DMG_TIMEOUT_SECONDS" soft; then
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
