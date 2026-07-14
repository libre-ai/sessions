#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

log="$tmp/curl.log"
mkdir -p "$tmp/bin"

cat >"$tmp/bin/curl" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"$CALL_LOG"
out="${@: -2:1}"
url="${@: -1}"
if [[ ${FAIL_CURL:-0} == 1 ]]; then
  echo 'curl: (22) The requested URL returned error: 503' >&2
  exit 22
fi
case "$url" in
  */health) printf 'ok' > "$out" ;;
  */sessions)
    cat > "$out" <<'JSON'
{"data":{"session_id":"SESSION123456","host_token":"HOSTTOKEN1234567890","join_url":"/?s=SESSION123456","secure_join_url":"/join/SESSION123456#token=JOINTOKEN1234567890"}}
JSON
    ;;
  *)
    echo "unexpected url: $url" >&2
    exit 64
    ;;
esac
EOF
chmod +x "$tmp/bin/curl"

success_stdout="$tmp/success.out"
success_stderr="$tmp/success.err"
CALL_LOG="$log" CURL_BIN="$tmp/bin/curl" PYTHON_BIN=python3 \
  "$root/scripts/clever-smoke.sh" https://example.test >"$success_stdout" 2>"$success_stderr"
if grep -qE 'HOSTTOKEN|JOINTOKEN|SESSION123|#token=|\{"data"' "$success_stdout" "$success_stderr"; then
  echo "smoke leaked sensitive data on success" >&2
  cat "$success_stdout" >&2
  cat "$success_stderr" >&2
  exit 1
fi
if [[ $(wc -l < "$log") -ne 2 ]]; then
  echo "smoke did not perform the two expected requests" >&2
  cat "$log" >&2
  exit 1
fi

loopback_stdout="$tmp/loopback.out"
loopback_stderr="$tmp/loopback.err"
CALL_LOG="$tmp/loopback.log" CURL_BIN="$tmp/bin/curl" PYTHON_BIN=python3 \
  "$root/scripts/clever-smoke.sh" http://127.0.0.1:3000 >"$loopback_stdout" 2>"$loopback_stderr"
if grep -qE 'HOSTTOKEN|JOINTOKEN|SESSION123|#token=|\{"data"' "$loopback_stdout" "$loopback_stderr"; then
  echo "smoke leaked sensitive data on loopback success" >&2
  cat "$loopback_stdout" >&2
  cat "$loopback_stderr" >&2
  exit 1
fi

nonloopback_stdout="$tmp/nonloopback.out"
nonloopback_stderr="$tmp/nonloopback.err"
: > "$tmp/nonloopback.log"
if CALL_LOG="$tmp/nonloopback.log" CURL_BIN="$tmp/bin/curl" PYTHON_BIN=python3 \
  "$root/scripts/clever-smoke.sh" http://example.test >"$nonloopback_stdout" 2>"$nonloopback_stderr"; then
  echo "smoke unexpectedly accepted non-loopback http" >&2
  exit 1
fi
if [[ -s "$tmp/nonloopback.log" ]]; then
  echo "smoke contacted curl before rejecting non-loopback http" >&2
  cat "$tmp/nonloopback.log" >&2
  exit 1
fi
if grep -qE 'HOSTTOKEN|JOINTOKEN|SESSION123|#token=|\{"data"' "$nonloopback_stdout" "$nonloopback_stderr"; then
  echo "smoke leaked sensitive data on rejection" >&2
  cat "$nonloopback_stdout" >&2
  cat "$nonloopback_stderr" >&2
  exit 1
fi

echo "clever smoke redaction and URL policy verified"
