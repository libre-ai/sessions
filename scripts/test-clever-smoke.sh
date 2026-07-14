#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

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

check_valid() {
  local name="$1"
  local url="$2"
  local log_file="$tmp/$name.log"
  local out_file="$tmp/$name.out"
  local err_file="$tmp/$name.err"
  CALL_LOG="$log_file" CURL_BIN="$tmp/bin/curl" PYTHON_BIN=python3 \
    "$root/scripts/clever-smoke.sh" "$url" >"$out_file" 2>"$err_file"
  if grep -qE 'HOSTTOKEN|JOINTOKEN|SESSION123|#token=|\{"data"' "$out_file" "$err_file"; then
    echo "smoke leaked sensitive data on $name" >&2
    cat "$out_file" >&2
    cat "$err_file" >&2
    exit 1
  fi
  if [[ $(wc -l < "$log_file") -ne 2 ]]; then
    echo "smoke did not perform the two expected requests for $name" >&2
    cat "$log_file" >&2
    exit 1
  fi
}

check_invalid() {
  local name="$1"
  local url="$2"
  local log_file="$tmp/$name.log"
  local out_file="$tmp/$name.out"
  local err_file="$tmp/$name.err"
  if CALL_LOG="$log_file" CURL_BIN="$tmp/bin/curl" PYTHON_BIN=python3 \
    "$root/scripts/clever-smoke.sh" "$url" >"$out_file" 2>"$err_file"; then
    echo "smoke unexpectedly accepted $name" >&2
    exit 1
  fi
  if [[ -s "$log_file" ]]; then
    echo "smoke contacted curl before rejecting $name" >&2
    cat "$log_file" >&2
    exit 1
  fi
  if grep -qE 'HOSTTOKEN|JOINTOKEN|SESSION123|#token=|\{"data"' "$out_file" "$err_file"; then
    echo "smoke leaked sensitive data on $name" >&2
    cat "$out_file" >&2
    cat "$err_file" >&2
    exit 1
  fi
}

check_valid https-example https://example.test
check_valid localhost-http http://localhost:3000
check_valid loopback-http http://127.0.0.1:3000
check_valid ipv6-loopback-http 'http://[::1]:3000'

check_invalid localhost-evil http://localhost.evil.tld
check_invalid ipv4-evil http://127.0.0.1.evil.tld
check_invalid path https://example.test//
check_invalid query 'https://example.test?x=1'
check_invalid fragment 'https://example.test/#x'
check_invalid userinfo 'https://user@example.test'
check_invalid bad-port 'https://example.test:99999'
check_invalid non-loopback-http http://example.test

printf 'clever smoke redaction and URL policy verified\n'
