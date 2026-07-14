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
    if [[ -z ${SESSION_FIXTURE:-} ]]; then
      echo 'missing SESSION_FIXTURE' >&2
      exit 64
    fi
    cat "$SESSION_FIXTURE" > "$out"
    ;;
  *)
    echo "unexpected url: $url" >&2
    exit 64
    ;;
esac
EOF
chmod +x "$tmp/bin/curl"

make_fixture() {
  local path="$1"
  cat >"$path" <<'JSON'
{"data":{"session_id":"SESSION123456","host_token":"HOSTTOKEN1234567890","join_url":"/?s=SESSION123456","secure_join_url":"/join/SESSION123456#token=JOINTOKEN1234567890"}}
JSON
}

make_fixture_legacy_next() {
  local path="$1"
  cat >"$path" <<'JSON'
{"data":{"session_id":"SESSION123456","host_token":"HOSTTOKEN1234567890","join_url":"/?s=SESSION123456&next=/","secure_join_url":"/join/SESSION123456#token=JOINTOKEN1234567890"}}
JSON
}

make_fixture_legacy_wrong_session() {
  local path="$1"
  cat >"$path" <<'JSON'
{"data":{"session_id":"SESSION123456","host_token":"HOSTTOKEN1234567890","join_url":"/?s=WRONGSESSION","secure_join_url":"/join/SESSION123456#token=JOINTOKEN1234567890"}}
JSON
}

make_fixture_legacy_bad_path() {
  local path="$1"
  cat >"$path" <<'JSON'
{"data":{"session_id":"SESSION123456","host_token":"HOSTTOKEN1234567890","join_url":"/join?s=SESSION123456","secure_join_url":"/join/SESSION123456#token=JOINTOKEN1234567890"}}
JSON
}

make_fixture_secure_bad_path() {
  local path="$1"
  cat >"$path" <<'JSON'
{"data":{"session_id":"SESSION123456","host_token":"HOSTTOKEN1234567890","join_url":"/?s=SESSION123456","secure_join_url":"/join/SESSION123456/extra#token=JOINTOKEN1234567890"}}
JSON
}

make_fixture_secure_bad_query() {
  local path="$1"
  cat >"$path" <<'JSON'
{"data":{"session_id":"SESSION123456","host_token":"HOSTTOKEN1234567890","join_url":"/?s=SESSION123456","secure_join_url":"/join/SESSION123456?next=/#token=JOINTOKEN1234567890"}}
JSON
}

make_fixture_secure_bad_fragment() {
  local path="$1"
  cat >"$path" <<'JSON'
{"data":{"session_id":"SESSION123456","host_token":"HOSTTOKEN1234567890","join_url":"/?s=SESSION123456","secure_join_url":"/join/SESSION123456#token=JOINTOKEN1234567890&next=/"}}
JSON
}

make_fixture_absolute_urls() {
  local path="$1"
  cat >"$path" <<'JSON'
{"data":{"session_id":"SESSION123456","host_token":"HOSTTOKEN1234567890","join_url":"https://example.test/?s=SESSION123456","secure_join_url":"https://example.test/join/SESSION123456#token=JOINTOKEN1234567890"}}
JSON
}

valid_fixture="$tmp/session-valid.json"
next_fixture="$tmp/session-next.json"
wrong_session_fixture="$tmp/session-wrong-session.json"
bad_path_fixture="$tmp/session-bad-path.json"
secure_bad_path_fixture="$tmp/session-secure-bad-path.json"
secure_bad_query_fixture="$tmp/session-secure-bad-query.json"
secure_bad_fragment_fixture="$tmp/session-secure-bad-fragment.json"
absolute_fixture="$tmp/session-absolute.json"
make_fixture "$valid_fixture"
make_fixture_legacy_next "$next_fixture"
make_fixture_legacy_wrong_session "$wrong_session_fixture"
make_fixture_legacy_bad_path "$bad_path_fixture"
make_fixture_secure_bad_path "$secure_bad_path_fixture"
make_fixture_secure_bad_query "$secure_bad_query_fixture"
make_fixture_secure_bad_fragment "$secure_bad_fragment_fixture"
make_fixture_absolute_urls "$absolute_fixture"

secret_regex='SESSION123456|HOSTTOKEN1234567890|JOINTOKEN1234567890|next='

check_valid() {
  local name="$1"
  local url="$2"
  local log_file="$tmp/$name.log"
  local out_file="$tmp/$name.out"
  local err_file="$tmp/$name.err"
  CALL_LOG="$log_file" SESSION_FIXTURE="$valid_fixture" CURL_BIN="$tmp/bin/curl" PYTHON_BIN=python3 \
    "$root/scripts/clever-smoke.sh" "$url" >"$out_file" 2>"$err_file"
  if grep -qE "$secret_regex" "$out_file" "$err_file"; then
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
  local fixture="$2"
  local url="$3"
  local log_file="$tmp/$name.log"
  local out_file="$tmp/$name.out"
  local err_file="$tmp/$name.err"
  if CALL_LOG="$log_file" SESSION_FIXTURE="$fixture" CURL_BIN="$tmp/bin/curl" PYTHON_BIN=python3 \
    "$root/scripts/clever-smoke.sh" "$url" >"$out_file" 2>"$err_file"; then
    echo "smoke unexpectedly accepted $name" >&2
    exit 1
  fi
  if [[ $(wc -l < "$log_file") -ne 2 ]]; then
    echo "smoke did not perform the two expected requests for $name" >&2
    cat "$log_file" >&2
    exit 1
  fi
  if grep -qE "$secret_regex" "$out_file" "$err_file"; then
    echo "smoke leaked sensitive data on $name" >&2
    cat "$out_file" >&2
    cat "$err_file" >&2
    exit 1
  fi
}

check_invalid_base_url() {
  local name="$1"
  local url="$2"
  local log_file="$tmp/$name.log"
  local out_file="$tmp/$name.out"
  local err_file="$tmp/$name.err"
  : >"$log_file"
  if CALL_LOG="$log_file" SESSION_FIXTURE="$valid_fixture" CURL_BIN="$tmp/bin/curl" PYTHON_BIN=python3 \
    "$root/scripts/clever-smoke.sh" "$url" >"$out_file" 2>"$err_file"; then
    echo "smoke unexpectedly accepted $name" >&2
    exit 1
  fi
  if [[ -s "$log_file" ]]; then
    echo "smoke contacted curl before rejecting $name" >&2
    cat "$log_file" >&2
    exit 1
  fi
  if grep -qE "$secret_regex" "$out_file" "$err_file"; then
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

check_invalid legacy-next "$next_fixture" https://example.test
check_invalid legacy-wrong-session "$wrong_session_fixture" https://example.test
check_invalid legacy-bad-path "$bad_path_fixture" https://example.test
check_invalid secure-bad-path "$secure_bad_path_fixture" https://example.test
check_invalid secure-bad-query "$secure_bad_query_fixture" https://example.test
check_invalid secure-bad-fragment "$secure_bad_fragment_fixture" https://example.test
check_invalid absolute-urls "$absolute_fixture" https://example.test

check_invalid_base_url localhost-evil http://localhost.evil.tld
check_invalid_base_url ipv4-evil http://127.0.0.1.evil.tld
check_invalid_base_url path https://example.test//
check_invalid_base_url query 'https://example.test?x=1'
check_invalid_base_url fragment 'https://example.test/#x'
check_invalid_base_url userinfo 'https://user@example.test'
check_invalid_base_url bad-port 'https://example.test:99999'
check_invalid_base_url non-loopback-http http://example.test

printf 'clever smoke redaction and URL policy verified\n'
