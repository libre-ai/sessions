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
body = json.loads(Path(sys.argv[1]).read_text(encoding='utf-8'))
if not isinstance(body, dict):
    raise SystemExit('missing JSON envelope')
data = body.get('data')
if not isinstance(data, dict):
    raise SystemExit('missing data envelope')
for field in ('session_id', 'host_token'):
    value = data.get(field)
    if not isinstance(value, str) or not value:
        raise SystemExit(f'missing {field}')
if not isinstance(data.get('join_url'), str) or not isinstance(data.get('secure_join_url'), str):
    raise SystemExit('missing join URLs')
if not data['join_url'].startswith('/?s='):
    raise SystemExit('unexpected join_url')
if not data['secure_join_url'].startswith('/join/') or '#token=' not in data['secure_join_url']:
    raise SystemExit('unexpected secure_join_url')
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
