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

remote_main_output="$(GIT_TERMINAL_PROMPT=0 "$git_bin" ls-remote --exit-code origin refs/heads/main 2>/dev/null)" || fail "checkout is not aligned with origin/main"
remote_main_sha="$($python_bin - "$remote_main_output" <<'PY'
import re
import sys

lines = [line for line in sys.argv[1].splitlines() if line.strip()]
if len(lines) != 1:
    raise SystemExit('unexpected origin/main ref listing')
sha, ref = lines[0].split()
if not re.fullmatch(r'[0-9a-f]{40}', sha):
    raise SystemExit('unexpected origin/main sha')
if ref != 'refs/heads/main':
    raise SystemExit('unexpected origin/main ref')
print(sha)
PY
)" || fail "checkout is not aligned with origin/main"
[[ "$($git_bin rev-parse HEAD)" == "$remote_main_sha" ]] || fail "checkout is not aligned with origin/main"

clever_config="$root/.clever.json"
[[ -f "$clever_config" ]] || fail "missing .clever.json"

cc_staging_remote="$("$git_bin" remote get-url cc-staging 2>/dev/null)" || fail "missing cc-staging remote"
if ! "$python_bin" - "$clever_config" "$cc_staging_remote" <<'PY'
from pathlib import Path
import json
import re
import sys
from urllib.parse import urlsplit, urlunsplit

REQUIRED_APP_KEYS = {'app_id', 'org_id', 'deploy_url', 'git_ssh_url', 'name', 'alias'}


def fail(message):
    raise SystemExit(message)


def normalize_git_url(raw):
    if not isinstance(raw, str):
        fail('invalid cc-staging remote URL')
    raw = raw.strip()
    if not raw:
        fail('invalid cc-staging remote URL')
    if '://' in raw:
        split = urlsplit(raw)
        if split.scheme not in {'https', 'ssh', 'git+ssh'}:
            fail('invalid cc-staging remote URL')
        if not split.netloc or not split.path or split.query or split.fragment:
            fail('invalid cc-staging remote URL')
        if split.scheme == 'https' and (split.username is not None or split.password is not None):
            fail('invalid cc-staging remote URL')
        path = split.path.rstrip('/')
        if not path:
            fail('invalid cc-staging remote URL')
        if path.endswith('.git'):
            path = path[:-4]
        scheme = 'ssh' if split.scheme in {'ssh', 'git+ssh'} else 'https'
        return urlunsplit((scheme, split.netloc, path, '', ''))
    match = re.fullmatch(r'(?:(?P<user>[^@/]+)@)?(?P<host>[^:/]+):(?P<path>.+)', raw)
    if not match:
        fail('invalid cc-staging remote URL')
    path = match.group('path').rstrip('/')
    if not path:
        fail('invalid cc-staging remote URL')
    if path.endswith('.git'):
        path = path[:-4]
    user = match.group('user')
    host = match.group('host')
    if not host:
        fail('invalid cc-staging remote URL')
    netloc = f'{user}@{host}' if user else host
    return f'ssh://{netloc}/{path}'


payload = json.loads(Path(sys.argv[1]).read_text(encoding='utf-8'))
if not isinstance(payload, dict) or set(payload) - {'apps', 'default'} or 'apps' not in payload:
    fail('invalid .clever.json')
apps = payload.get('apps')
if not isinstance(apps, list):
    fail('invalid .clever.json')
app_ids = set()
staging = []
for app in apps:
    if not isinstance(app, dict) or set(app) != REQUIRED_APP_KEYS:
        fail('invalid .clever.json')
    for key in REQUIRED_APP_KEYS:
        if not isinstance(app[key], str) or not app[key]:
            fail('invalid .clever.json')
    app_ids.add(app['app_id'])
    if app['alias'] == 'staging':
        staging.append(app)
if len(staging) != 1:
    fail('expected exactly one staging alias in .clever.json')
if 'default' in payload:
    default = payload['default']
    if not isinstance(default, str) or not default:
        fail('invalid .clever.json')
    if default not in app_ids:
        fail('invalid .clever.json')
actual = normalize_git_url(sys.argv[2])
expected = {
    normalize_git_url(staging[0]['deploy_url']),
    normalize_git_url(staging[0]['git_ssh_url']),
}
if actual not in expected:
    fail('cc-staging remote does not target staging app')
PY
then
  fail "cc-staging remote does not target staging app"
fi

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
