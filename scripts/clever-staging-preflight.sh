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

[[ -z "$("$git_bin" status --porcelain)" ]] || fail "checkout is dirty"
[[ "$($git_bin branch --show-current)" == "main" ]] || fail "checkout must be on main"
[[ "$($git_bin rev-parse HEAD)" == "$($git_bin rev-parse origin/main)" ]] || fail "checkout is not aligned with origin/main"
if ! "$git_bin" remote get-url cc-staging >/dev/null 2>&1; then
  fail "missing cc-staging remote"
fi

clever_files=()
while IFS= read -r path; do
  clever_files+=("$path")
done < <(find "$root" -name .clever.json -type f | LC_ALL=C sort)
[[ ${#clever_files[@]} -eq 1 ]] || fail "expected exactly one .clever.json"
clever_config="${clever_files[0]}"

alias_matches="$($python_bin - "$clever_config" <<'PY'
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

clever_status_json="$tmp/clever-status.json"
if ! "$clever_bin" status -a staging -F json > "$clever_status_json"; then
  fail "staging status failed"
fi

"$python_bin" - "$clever_status_json" <<'PY'
import json
import sys
from pathlib import Path

status_raw = Path(sys.argv[1]).read_text(encoding='utf-8')
try:
    status = json.loads(status_raw)
except json.JSONDecodeError as exc:
    raise SystemExit(f'invalid clever status JSON: {exc}')

if not isinstance(status, dict):
    raise SystemExit('unsupported clever status JSON shape')

scalability = status.get('scalability')
if not isinstance(scalability, dict):
    raise SystemExit('missing scalability block')

horizontal = scalability.get('horizontal')
if not isinstance(horizontal, dict):
    raise SystemExit('missing scalability.horizontal block')

if horizontal.get('min') != 1 or horizontal.get('max') != 1:
    raise SystemExit('staging must be exactly one instance')
PY

clever_env_json="$tmp/clever-env.json"
if ! "$clever_bin" env -a staging -F json > "$clever_env_json"; then
  fail "staging env failed"
fi

"$python_bin" - "$clever_env_json" <<'PY'
import json
import re
import sys
import unicodedata
from pathlib import Path
from urllib.parse import urlsplit

MAX_CLIENT_ID_LENGTH = 256

FORBIDDEN = {
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
}
REQUIRED_EXACT = {
    'CC_RUST_BIN': 'presto-server',
    'CC_CACHE_DEPENDENCIES': 'true',
    'CC_PRE_BUILD_HOOK': './scripts/clever-pre-build.sh',
    'OWNER_AUTH_SINGLE_INSTANCE': '1',
}
REQUIRED_PRESENT = [
    'OIDC_ISSUER',
    'OIDC_CLIENT_ID',
    'OIDC_REDIRECT_URI',
    'BISCUIT_PRIVATE_KEY',
    'INGEST_TOKEN',
]


def fail(message):
    raise SystemExit(message)


def load_payload(path):
    raw = Path(path).read_text(encoding='utf-8')
    try:
        return json.loads(raw)
    except json.JSONDecodeError as exc:
        fail(f'invalid clever env JSON: {exc.msg}')


def parse_entry(entry):
    if not isinstance(entry, dict):
        fail('invalid clever env entry')
    name = entry.get('name')
    key = entry.get('key')
    if isinstance(name, str) and isinstance(key, str) and name != key:
        fail('invalid clever env entry')
    if isinstance(name, str):
        actual_name = name
    elif isinstance(key, str):
        actual_name = key
    else:
        fail('invalid clever env entry')
    value = entry.get('value')
    if not isinstance(value, str):
        fail('invalid clever env entry')
    return actual_name, value


def add_env(env, name, value):
    existing = env.get(name)
    if existing is None:
        env[name] = value
    elif existing != value:
        fail(f'ambiguous clever env collision for {name}')


def parse_source(source, source_kind):
    if not isinstance(source, dict):
        fail(f'invalid clever {source_kind}')
    addon_id = source.get('addonId')
    addon_name = source.get('addonName')
    if not isinstance(addon_id, str) or not isinstance(addon_name, str):
        fail(f'invalid clever {source_kind}')
    entries = source.get('env')
    if not isinstance(entries, list):
        fail(f'invalid clever {source_kind}')
    return [parse_entry(entry) for entry in entries]


def parse_payload(payload):
    env = {}
    if isinstance(payload, list):
        for entry in payload:
            add_env(env, *parse_entry(entry))
        return env
    if not isinstance(payload, dict):
        fail('unsupported clever env JSON shape')

    canonical_keys = {'env', 'fromAddons', 'fromDependencies'}
    payload_keys = set(payload)
    if payload_keys <= canonical_keys and 'env' in payload:
        entries = payload.get('env')
        if not isinstance(entries, list):
            fail('invalid clever env JSON shape')
        for entry in entries:
            add_env(env, *parse_entry(entry))

        from_addons = payload.get('fromAddons', [])
        if not isinstance(from_addons, list):
            fail('invalid clever env JSON shape')
        for source in from_addons:
            for name, value in parse_source(source, 'addon source'):
                add_env(env, name, value)

        from_dependencies = payload.get('fromDependencies', [])
        if not isinstance(from_dependencies, list):
            fail('invalid clever env JSON shape')
        for source in from_dependencies:
            for name, value in parse_source(source, 'dependency source'):
                add_env(env, name, value)
        return env

    if payload_keys.isdisjoint(canonical_keys) and all(isinstance(value, str) for value in payload.values()):
        for name, value in payload.items():
            add_env(env, name, value)
        return env

    fail('unsupported clever env JSON shape')


def validate_url(name, value, required_path=None):
    split = urlsplit(value)
    if split.scheme != 'https':
        fail(f'invalid {name}')
    if not split.hostname:
        fail(f'invalid {name}')
    if split.username is not None or split.password is not None:
        fail(f'invalid {name}')
    try:
        port = split.port
    except ValueError:
        fail(f'invalid {name}')
    if port is not None and not (1 <= port <= 65535):
        fail(f'invalid {name}')
    if split.query or split.fragment:
        fail(f'invalid {name}')
    if required_path is not None and split.path != required_path:
        fail(f'invalid {name}')


def validate_client_id(value):
    if not (0 < len(value) <= MAX_CLIENT_ID_LENGTH):
        fail('invalid OIDC_CLIENT_ID')
    if any(ch.isspace() or unicodedata.category(ch).startswith('C') for ch in value):
        fail('invalid OIDC_CLIENT_ID')


raw_payload = load_payload(sys.argv[1])
env = parse_payload(raw_payload)

for name, expected in REQUIRED_EXACT.items():
    if env.get(name) != expected:
        fail(f'bad required env: {name}')

for name in REQUIRED_PRESENT:
    if name not in env or not isinstance(env[name], str) or not env[name]:
        fail(f'missing required env: {name}')

for name in FORBIDDEN:
    if name in env:
        fail(f'forbidden env present: {name}')

if not re.fullmatch(r'[0-9a-fA-F]{64}', env['BISCUIT_PRIVATE_KEY']):
    fail('invalid BISCUIT_PRIVATE_KEY format')

ingest = env['INGEST_TOKEN']
if not (32 <= len(ingest) <= 512) or any(ord(ch) < 0x21 or ord(ch) > 0x7e for ch in ingest):
    fail('invalid INGEST_TOKEN format')

validate_url('OIDC_ISSUER', env['OIDC_ISSUER'])
validate_url('OIDC_REDIRECT_URI', env['OIDC_REDIRECT_URI'], required_path='/auth/callback')
validate_client_id(env['OIDC_CLIENT_ID'])
PY

printf 'staging preflight passed\n'
