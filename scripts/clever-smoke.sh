#!/usr/bin/env bash
# Post-deploy smoke test for a Presto-Matic deployment.
#
# Usage: scripts/clever-smoke.sh https://your-app.cleverapps.io
#
# Checks /health and POST /sessions. The POST creates a short-lived session
# envelope in memory only for the duration of the request; the script never
# prints JSON, host tokens, join tokens or URL fragments.
set -euo pipefail

base_input="${1:?usage: clever-smoke.sh <base-url>}"

curl_bin="${CURL_BIN:-curl}"
python_bin="${PYTHON_BIN:-python3}"

validate_base_url() {
  local url="$1"
  if ! "$python_bin" - "$url" <<'PY'
from urllib.parse import urlsplit
import sys

parts = urlsplit(sys.argv[1])
try:
    if parts.scheme not in {'http', 'https'}:
        raise ValueError
    if not parts.hostname:
        raise ValueError
    if parts.username is not None or parts.password is not None:
        raise ValueError
    if parts.query or parts.fragment:
        raise ValueError
    if parts.path not in ('', '/'):
        raise ValueError
    _ = parts.port
    if parts.scheme == 'http' and parts.hostname not in {'localhost', '127.0.0.1', '::1'}:
        raise ValueError
except ValueError:
    raise SystemExit(1)
PY
  then
    echo "smoke failed: invalid base URL" >&2
    exit 1
  fi
}

validate_base_url "$base_input"
base="${base_input%/}"

request_json() {
  local method="$1"
  local url="$2"
  local output="$3"
  if [[ "$method" == GET ]]; then
    "$curl_bin" --silent --fail --max-time 10 --max-filesize 1048576 \
      --output "$output" "$url"
  else
    "$curl_bin" --silent --fail --max-time 10 --max-filesize 1048576 \
      --request POST --data-binary '' --output "$output" "$url"
  fi
}

validate_health() {
  local body="$1"
  "$python_bin" - "$body" <<'PY'
from pathlib import Path
import sys
body = Path(sys.argv[1]).read_text(encoding='utf-8').strip()
if body != 'ok':
    raise SystemExit('health check failed')
PY
}

validate_session() {
  local body="$1"
  "$python_bin" - "$body" <<'PY'
from pathlib import Path
import json
import sys
from urllib.parse import urlsplit

MAX_SESSION_ID_LENGTH = 128
MAX_TOKEN_LENGTH = 512


def fail(message):
    raise SystemExit(message)


def is_safe_session_id(value):
    return (
        isinstance(value, str)
        and 0 < len(value) <= MAX_SESSION_ID_LENGTH
        and all(0x21 <= ord(ch) <= 0x7e and ch not in '/?#&%+' for ch in value)
    )


def is_safe_token(value):
    return (
        isinstance(value, str)
        and 0 < len(value) <= MAX_TOKEN_LENGTH
        and all(0x21 <= ord(ch) <= 0x7e and ch not in '&#' for ch in value)
    )


def validate_legacy_join_url(session_id, value):
    if not is_safe_session_id(session_id) or not isinstance(value, str):
        fail('unexpected join_url')
    split = urlsplit(value)
    if split.scheme or split.netloc or split.fragment or split.path != '/':
        fail('unexpected join_url')
    if split.query != f's={session_id}':
        fail('unexpected join_url')


def validate_secure_join_url(session_id, value):
    if not is_safe_session_id(session_id) or not isinstance(value, str):
        fail('unexpected secure_join_url')
    split = urlsplit(value)
    if split.scheme or split.netloc or split.query:
        fail('unexpected secure_join_url')
    if split.path != f'/join/{session_id}':
        fail('unexpected secure_join_url')
    if not split.fragment.startswith('token='):
        fail('unexpected secure_join_url')
    token = split.fragment[len('token='):]
    if not is_safe_token(token):
        fail('unexpected secure_join_url')


body = json.loads(Path(sys.argv[1]).read_text(encoding='utf-8'))
if not isinstance(body, dict):
    fail('missing JSON envelope')
data = body.get('data')
if not isinstance(data, dict):
    fail('missing data envelope')
session_id = data.get('session_id')
if not is_safe_session_id(session_id):
    fail('missing session_id')
host_token = data.get('host_token')
if not is_safe_token(host_token):
    fail('missing host_token')
validate_legacy_join_url(session_id, data.get('join_url'))
validate_secure_join_url(session_id, data.get('secure_join_url'))
PY
}

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

health_body="$tmp/health.txt"
session_body="$tmp/session.json"

printf '→ health\n'
request_json GET "$base/health" "$health_body" || { echo "smoke failed: health request" >&2; exit 1; }
validate_health "$health_body" || { echo "smoke failed: health payload" >&2; exit 1; }
printf 'ok\n'

printf '→ session mint\n'
request_json POST "$base/sessions" "$session_body" || { echo "smoke failed: session request" >&2; exit 1; }
validate_session "$session_body" || { echo "smoke failed: session payload" >&2; exit 1; }
printf 'ok\n'

printf 'smoke passed\n'
