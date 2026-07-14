#!/usr/bin/env bash
set -euo pipefail

root="${ROOT:-$(git rev-parse --show-toplevel)}"
cd "$root"

git_bin="${GIT_BIN:-git}"
clever_bin="${CLEVER_BIN:-clever}"
python_bin="${PYTHON_BIN:-python3}"

fail() {
  echo "preflight failed: $1" >&2
  exit 1
}

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

[[ -z "$($git_bin status --porcelain)" ]] || fail "checkout is dirty"
[[ "$($git_bin branch --show-current)" == "main" ]] || fail "checkout must be on main"
[[ "$($git_bin rev-parse HEAD)" == "$($git_bin rev-parse origin/main)" ]] || fail "checkout is not aligned with origin/main"

clever_files=()
while IFS= read -r path; do
  clever_files+=("$path")
done < <(find "$root" -name .clever.json -type f | LC_ALL=C sort)
[[ ${#clever_files[@]} -eq 1 ]] || fail "expected exactly one .clever.json"
clever_config="${clever_files[0]}"

alias_matches="$("$python_bin" - "$clever_config" <<'PY'
from pathlib import Path
import json
import sys


def walk(value):
    if isinstance(value, dict):
        for key, nested in value.items():
            if key == 'alias' and nested == 'staging':
                yield 1
            yield from walk(nested)
    elif isinstance(value, list):
        for item in value:
            yield from walk(item)

config = json.loads(Path(sys.argv[1]).read_text(encoding='utf-8'))
print(sum(walk(config)))
PY
)"
[[ "$alias_matches" == "1" ]] || fail "expected exactly one staging alias in .clever.json"

"$clever_bin" --version >/dev/null || fail "clever CLI unavailable"
"$clever_bin" status -a staging >/dev/null || fail "staging status failed"

clever_env_json="$tmp/clever-env.json"
"$clever_bin" env -a staging -F json > "$clever_env_json"

"$python_bin" - "$clever_env_json" <<'PY'
import json
import re
import sys
from pathlib import Path

raw = Path(sys.argv[1]).read_text(encoding='utf-8')
try:
    payload = json.loads(raw)
except json.JSONDecodeError as exc:
    raise SystemExit(f'invalid clever env JSON: {exc}')

if isinstance(payload, dict):
    items = payload.items()
elif isinstance(payload, list):
    items = []
    for item in payload:
        if not isinstance(item, dict):
            raise SystemExit('invalid clever env entry')
        name = item.get('name') or item.get('key')
        if not isinstance(name, str):
            raise SystemExit('invalid clever env entry name')
        items.append((name, item.get('value')))
else:
    raise SystemExit('unsupported clever env JSON shape')

env = {name: value for name, value in items if isinstance(name, str)}

required = [
    'OWNER_AUTH_SINGLE_INSTANCE',
    'OIDC_ISSUER',
    'OIDC_CLIENT_ID',
    'OIDC_REDIRECT_URI',
    'BISCUIT_PRIVATE_KEY',
    'INGEST_TOKEN',
]
for name in required:
    if name not in env or not isinstance(env[name], str) or not env[name]:
        raise SystemExit(f'missing required env: {name}')

forbidden = [
    'DATABASE_URL',
    'REDIS_URL',
    'CLEVER_AI_ENABLED',
    'CLEVER_AI_BASE_URL',
    'CLEVER_AI_API_KEY',
    'CLEVER_AI_CONTRACT_REF',
    'CLEVER_AI_EMBED_MODEL',
    'CLEVER_AI_CHAT_MODEL',
    'LOCAL_AI_ENABLED',
    'LOCAL_AI_BASE_URL',
    'LOCAL_AI_API_KEY',
    'LOCAL_AI_EMBED_MODEL',
    'LOCAL_AI_CHAT_MODEL',
]
for name in forbidden:
    if name in env:
        raise SystemExit(f'forbidden env present: {name}')

if env['OWNER_AUTH_SINGLE_INSTANCE'] != '1':
    raise SystemExit('OWNER_AUTH_SINGLE_INSTANCE must be 1')

if not re.fullmatch(r'[0-9a-fA-F]{64}', env['BISCUIT_PRIVATE_KEY']):
    raise SystemExit('invalid BISCUIT_PRIVATE_KEY format')

ingest = env['INGEST_TOKEN']
if not (32 <= len(ingest) <= 512) or any(ord(ch) < 0x21 or ord(ch) > 0x7e for ch in ingest):
    raise SystemExit('invalid INGEST_TOKEN format')

if not env['OIDC_ISSUER'].startswith('https://'):
    raise SystemExit('OIDC_ISSUER must be https')
redirect = env['OIDC_REDIRECT_URI']
if not redirect.startswith('https://') or not redirect.endswith('/auth/callback'):
    raise SystemExit('OIDC_REDIRECT_URI must be an https callback')
PY

printf 'staging preflight passed\n'
