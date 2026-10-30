#!/usr/bin/env bash
# 检查目录下所有 .exe 与 .msi 都带有有效的 Authenticode 签名（在 Linux 上用 osslsigncode）。
#
# 用法：windows-verify.sh <目录...>
set -euo pipefail

if [ "$#" -eq 0 ]; then
  echo "用法：$0 <目录...>" >&2
  exit 2
fi

if ! command -v osslsigncode >/dev/null 2>&1; then
  sudo apt-get update -qq
  sudo apt-get install -y -qq --no-install-recommends osslsigncode >/dev/null
fi

log="$(mktemp)"
trap 'rm -f "$log"' EXIT
count=0; failed=0
while IFS= read -r -d '' f; do
  count=$((count + 1))
  if osslsigncode verify -in "$f" > "$log" 2>&1; then
    signer="$(grep -m1 -E '^[[:space:]]*Subject:' "$log" | sed 's/^[[:space:]]*Subject:[[:space:]]*//' || true)"
    echo "✓ ${f} ${signer:+（${signer}）}"
  else
    failed=$((failed + 1))
    echo "::error::$f 没有有效的签名"
    cat "$log"
  fi
done < <(find "$@" -type f \( -iname '*.exe' -o -iname '*.msi' \) -print0 | sort -z)

if [ "$count" -eq 0 ]; then
  echo "::error::在 $* 下没有找到 .exe 或 .msi" >&2
  exit 1
fi
echo "检查了 $count 个文件，$failed 个没有有效签名"
[ "$failed" -eq 0 ]
