#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
python_bin="${PYTHON_BIN:-python3}"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

mkdir -p "$tmp/bin"

cat >"$tmp/bin/clever" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"$CALL_LOG"
case "$*" in
  'status -a staging -F json')
    cat "$STATUS_FIXTURE"
    ;;
  'env -a staging -F json')
    cat "$ENV_FIXTURE"
    ;;
  *)
    echo "unexpected clever call: $*" >&2
    exit 64
    ;;
esac
EOF
chmod +x "$tmp/bin/clever"

repo="$tmp/repo"
origin="$tmp/origin.git"
mkdir -p "$repo"
git init --bare "$origin" >/dev/null
git clone "$origin" "$repo" >/dev/null
cd "$repo"
git config user.name "Test User"
git config user.email "test@example.com"
git branch -M main

fixture="$root/scripts/fixtures/clever-tools-staging.json"

fixture_field() {
  "$python_bin" - "$fixture" "$1" "$2" <<'PY'
import json
import sys
from pathlib import Path

payload = json.loads(Path(sys.argv[1]).read_text(encoding='utf-8'))
for app in payload.get('apps', []):
    if app.get('alias') == sys.argv[2]:
        value = app.get(sys.argv[3])
        if not isinstance(value, str) or not value:
            raise SystemExit('missing staging fixture field')
        print(value)
        raise SystemExit(0)
raise SystemExit('missing staging fixture')
PY
}

ssh_like_to_ssh() {
  "$python_bin" - "$1" <<'PY'
from urllib.parse import urlsplit
import sys

split = urlsplit(sys.argv[1])
if split.scheme not in {'ssh', 'git+ssh'} or not split.hostname or not split.path:
    raise SystemExit('expected ssh-like url')
user = f"{split.username}@" if split.username else ''
port = f":{split.port}" if split.port is not None else ''
print(f"ssh://{user}{split.hostname}{port}{split.path}")
PY
}

scp_from_ssh_like() {
  "$python_bin" - "$1" <<'PY'
from urllib.parse import urlsplit
import sys

split = urlsplit(sys.argv[1])
if split.scheme not in {'ssh', 'git+ssh'} or not split.hostname or not split.path:
    raise SystemExit('expected ssh-like url')
user = f"{split.username}@" if split.username else ''
path = split.path.lstrip('/')
print(f"{user}{split.hostname}:{path}")
PY
}

setup_repo_with_config() (
  local origin_path="$1"
  local repo_path="$2"
  local config_path="$3"
  local remote_url="$4"
  git init --bare "$origin_path" >/dev/null
  git clone "$origin_path" "$repo_path" >/dev/null
  cd "$repo_path"
  git config user.name "Test User"
  git config user.email "test@example.com"
  git branch -M main
  cp "$config_path" .clever.json
  printf 'main\n' > tracked.txt
  git add .clever.json tracked.txt
  git commit -m "init" >/dev/null
  git remote add cc-staging "$remote_url"
  git push -u origin main >/dev/null
)

run_preflight_in_root() {
  local repo_root="$1"
  local log_file="$2"
  local status_fixture="$3"
  local env_fixture="$4"
  local stdout_file="$5"
  local stderr_file="$6"
  : >"$log_file"
  CALL_LOG="$log_file" STATUS_FIXTURE="$status_fixture" ENV_FIXTURE="$env_fixture" \
    ROOT="$repo_root" GIT_BIN=git CLEVER_BIN="$tmp/bin/clever" PYTHON_BIN=python3 PATH="$tmp/bin:$PATH" \
    "$root/scripts/clever-staging-preflight.sh" >"$stdout_file" 2>"$stderr_file"
}

staging_deploy_url="$(fixture_field staging deploy_url)"
staging_git_ssh_url="$(fixture_field staging git_ssh_url)"
production_deploy_url="$(fixture_field production deploy_url)"
staging_git_ssh_ssh="$(ssh_like_to_ssh "$staging_git_ssh_url")"
staging_git_ssh_scp="$(scp_from_ssh_like "$staging_git_ssh_url")"
staging_deploy_url_no_slash="${staging_deploy_url%/}"
production_deploy_url_no_slash="${production_deploy_url%/}"

cp "$root/scripts/fixtures/clever-tools-staging.json" .clever.json
printf 'main\n' > tracked.txt
git add .clever.json tracked.txt
git commit -m "init" >/dev/null
git remote add cc-staging "$staging_deploy_url_no_slash"
git push -u origin main >/dev/null

status_ok="$tmp/status-ok.json"
cat >"$status_ok" <<'JSON'
{"scalability":{"horizontal":{"min":1,"max":1}}}
JSON

status_scale_gt1="$tmp/status-scale-gt1.json"
cat >"$status_scale_gt1" <<'JSON'
{"scalability":{"horizontal":{"min":1,"max":2}}}
JSON

env_success="$tmp/env-success.json"
cat >"$env_success" <<'JSON'
{
  "env": [
    {"name": "CC_RUST_BIN", "value": "presto-server"},
    {"name": "CC_CACHE_DEPENDENCIES", "value": "true"},
    {"name": "CC_PRE_BUILD_HOOK", "value": "./scripts/clever-pre-build.sh"},
    {"name": "OWNER_AUTH_SINGLE_INSTANCE", "value": "1"},
    {"name": "OIDC_ISSUER", "value": "https://issuer.example"},
    {"name": "OIDC_CLIENT_ID", "value": "client-123"},
    {"name": "OIDC_REDIRECT_URI", "value": "https://staging.example/auth/callback"},
    {"name": "BISCUIT_PRIVATE_KEY", "value": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"},
    {"name": "INGEST_TOKEN", "value": "abcdefghijklmnopqrstuvwxyz123456"},
    {"name": "SHARED_SOURCE_FLAG", "value": "https://issuer.example"}
  ],
  "fromAddons": [
    {
      "addonId": "addon-staging-cache",
      "addonName": "staging-cache",
      "env": [
        {"name": "SHARED_SOURCE_FLAG", "value": "https://issuer.example"},
        {"name": "ADDON_ONLY_FLAG", "value": "client-123"}
      ]
    }
  ],
  "fromDependencies": [
    {
      "addonId": "dep-api",
      "addonName": "api",
      "env": [
        {"name": "SHARED_SOURCE_FLAG", "value": "https://issuer.example"},
        {"name": "DEPENDENCY_ONLY_FLAG", "value": "0123456789abcdef"}
      ]
    }
  ]
}
JSON

env_direct_list="$tmp/env-direct-list.json"
cat >"$env_direct_list" <<'JSON'
[
  {"name": "CC_RUST_BIN", "value": "presto-server"},
  {"name": "CC_CACHE_DEPENDENCIES", "value": "true"},
  {"name": "CC_PRE_BUILD_HOOK", "value": "./scripts/clever-pre-build.sh"},
  {"name": "OWNER_AUTH_SINGLE_INSTANCE", "value": "1"},
  {"name": "OIDC_ISSUER", "value": "https://issuer.example"},
  {"name": "OIDC_CLIENT_ID", "value": "client-123"},
  {"name": "OIDC_REDIRECT_URI", "value": "https://staging.example/auth/callback"},
  {"name": "BISCUIT_PRIVATE_KEY", "value": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"},
  {"name": "INGEST_TOKEN", "value": "abcdefghijklmnopqrstuvwxyz123456"}
]
JSON

env_direct_list_collision="$tmp/env-direct-list-collision.json"
cat >"$env_direct_list_collision" <<'JSON'
[
  {"name": "CC_RUST_BIN", "value": "presto-server"},
  {"name": "CC_CACHE_DEPENDENCIES", "value": "true"},
  {"name": "CC_PRE_BUILD_HOOK", "value": "./scripts/clever-pre-build.sh"},
  {"name": "OWNER_AUTH_SINGLE_INSTANCE", "value": "1"},
  {"name": "OIDC_ISSUER", "value": "https://issuer.example"},
  {"name": "OIDC_CLIENT_ID", "value": "client-123"},
  {"name": "OIDC_REDIRECT_URI", "value": "https://staging.example/auth/callback"},
  {"name": "BISCUIT_PRIVATE_KEY", "value": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"},
  {"name": "INGEST_TOKEN", "value": "abcdefghijklmnopqrstuvwxyz123456"},
  {"name": "AMBIGUOUS_FLAG", "value": "https://issuer.example"},
  {"name": "AMBIGUOUS_FLAG", "value": "client-123"}
]
JSON

env_flat_dict="$tmp/env-flat-dict.json"
cat >"$env_flat_dict" <<'JSON'
{
  "CC_RUST_BIN": "presto-server",
  "CC_CACHE_DEPENDENCIES": "true",
  "CC_PRE_BUILD_HOOK": "./scripts/clever-pre-build.sh",
  "OWNER_AUTH_SINGLE_INSTANCE": "1",
  "OIDC_ISSUER": "https://issuer.example",
  "OIDC_CLIENT_ID": "client-123",
  "OIDC_REDIRECT_URI": "https://staging.example/auth/callback",
  "BISCUIT_PRIVATE_KEY": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
  "INGEST_TOKEN": "abcdefghijklmnopqrstuvwxyz123456"
}
JSON

env_missing_prebuild="$tmp/env-missing-prebuild.json"
cat >"$env_missing_prebuild" <<'JSON'
{
  "CC_RUST_BIN": "presto-server",
  "CC_CACHE_DEPENDENCIES": "true",
  "OWNER_AUTH_SINGLE_INSTANCE": "1",
  "OIDC_ISSUER": "https://issuer.example",
  "OIDC_CLIENT_ID": "client-123",
  "OIDC_REDIRECT_URI": "https://staging.example/auth/callback",
  "BISCUIT_PRIVATE_KEY": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
  "INGEST_TOKEN": "abcdefghijklmnopqrstuvwxyz123456"
}
JSON

env_wrong_cache="$tmp/env-wrong-cache.json"
cat >"$env_wrong_cache" <<'JSON'
{
  "CC_RUST_BIN": "presto-server",
  "CC_CACHE_DEPENDENCIES": "false",
  "CC_PRE_BUILD_HOOK": "./scripts/clever-pre-build.sh",
  "OWNER_AUTH_SINGLE_INSTANCE": "1",
  "OIDC_ISSUER": "https://issuer.example",
  "OIDC_CLIENT_ID": "client-123",
  "OIDC_REDIRECT_URI": "https://staging.example/auth/callback",
  "BISCUIT_PRIVATE_KEY": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
  "INGEST_TOKEN": "abcdefghijklmnopqrstuvwxyz123456"
}
JSON

env_forbidden_addon="$tmp/env-forbidden-addon.json"
cat >"$env_forbidden_addon" <<'JSON'
{
  "env": [
    {"name": "CC_RUST_BIN", "value": "presto-server"},
    {"name": "CC_CACHE_DEPENDENCIES", "value": "true"},
    {"name": "CC_PRE_BUILD_HOOK", "value": "./scripts/clever-pre-build.sh"},
    {"name": "OWNER_AUTH_SINGLE_INSTANCE", "value": "1"},
    {"name": "OIDC_ISSUER", "value": "https://issuer.example"},
    {"name": "OIDC_CLIENT_ID", "value": "client-123"},
    {"name": "OIDC_REDIRECT_URI", "value": "https://staging.example/auth/callback"},
    {"name": "BISCUIT_PRIVATE_KEY", "value": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"},
    {"name": "INGEST_TOKEN", "value": "abcdefghijklmnopqrstuvwxyz123456"}
  ],
  "fromAddons": [
    {
      "addonId": "addon-staging-cache",
      "addonName": "staging-cache",
      "env": [
        {"name": "DATABASE_URL", "value": "postgres://issuer.example@localhost/app"}
      ]
    }
  ],
  "fromDependencies": []
}
JSON

env_forbidden_dependency="$tmp/env-forbidden-dependency.json"
cat >"$env_forbidden_dependency" <<'JSON'
{
  "env": [
    {"name": "CC_RUST_BIN", "value": "presto-server"},
    {"name": "CC_CACHE_DEPENDENCIES", "value": "true"},
    {"name": "CC_PRE_BUILD_HOOK", "value": "./scripts/clever-pre-build.sh"},
    {"name": "OWNER_AUTH_SINGLE_INSTANCE", "value": "1"},
    {"name": "OIDC_ISSUER", "value": "https://issuer.example"},
    {"name": "OIDC_CLIENT_ID", "value": "client-123"},
    {"name": "OIDC_REDIRECT_URI", "value": "https://staging.example/auth/callback"},
    {"name": "BISCUIT_PRIVATE_KEY", "value": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"},
    {"name": "INGEST_TOKEN", "value": "abcdefghijklmnopqrstuvwxyz123456"}
  ],
  "fromAddons": [],
  "fromDependencies": [
    {
      "addonId": "dep-redis",
      "addonName": "redis",
      "env": [
        {"name": "REDIS_URL", "value": "redis://client-123@localhost/0"}
      ]
    }
  ]
}
JSON

env_collision="$tmp/env-collision.json"
cat >"$env_collision" <<'JSON'
{
  "env": [
    {"name": "CC_RUST_BIN", "value": "presto-server"},
    {"name": "CC_CACHE_DEPENDENCIES", "value": "true"},
    {"name": "CC_PRE_BUILD_HOOK", "value": "./scripts/clever-pre-build.sh"},
    {"name": "OWNER_AUTH_SINGLE_INSTANCE", "value": "1"},
    {"name": "OIDC_ISSUER", "value": "https://issuer.example"},
    {"name": "OIDC_CLIENT_ID", "value": "client-123"},
    {"name": "OIDC_REDIRECT_URI", "value": "https://staging.example/auth/callback"},
    {"name": "BISCUIT_PRIVATE_KEY", "value": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"},
    {"name": "INGEST_TOKEN", "value": "abcdefghijklmnopqrstuvwxyz123456"},
    {"name": "AMBIGUOUS_FLAG", "value": "https://issuer.example"}
  ],
  "fromAddons": [
    {
      "addonId": "addon-shared",
      "addonName": "shared",
      "env": [
        {"name": "AMBIGUOUS_FLAG", "value": "https://issuer.example"}
      ]
    }
  ],
  "fromDependencies": [
    {
      "addonId": "dep-shared",
      "addonName": "shared-dep",
      "env": [
        {"name": "AMBIGUOUS_FLAG", "value": "client-123"}
      ]
    }
  ]
}
JSON

env_invalid_shape="$tmp/env-invalid-shape.json"
cat >"$env_invalid_shape" <<'JSON'
{
  "env": "oops",
  "fromAddons": [],
  "fromDependencies": []
}
JSON

env_invalid_member="$tmp/env-invalid-member.json"
cat >"$env_invalid_member" <<'JSON'
{
  "env": [
    {"name": "CC_RUST_BIN", "value": "presto-server"},
    {"name": "CC_CACHE_DEPENDENCIES", "value": "true"},
    {"name": "CC_PRE_BUILD_HOOK", "value": "./scripts/clever-pre-build.sh"},
    {"name": "OWNER_AUTH_SINGLE_INSTANCE", "value": "1"},
    {"name": "OIDC_ISSUER", "value": "https://issuer.example"},
    {"name": "OIDC_CLIENT_ID", "value": "client-123"},
    {"name": "OIDC_REDIRECT_URI", "value": "https://staging.example/auth/callback"},
    {"name": "BISCUIT_PRIVATE_KEY", "value": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"},
    {"name": "INGEST_TOKEN", "value": "abcdefghijklmnopqrstuvwxyz123456"}
  ],
  "fromAddons": [],
  "fromDependencies": [
    {
      "addonId": "dep-invalid",
      "addonName": "invalid",
      "env": [
        {"name": "BROKEN", "value": 42}
      ]
    }
  ]
}
JSON

env_invalid_oidc_issuer="$tmp/env-invalid-oidc-issuer.json"
cat >"$env_invalid_oidc_issuer" <<'JSON'
{
  "env": [
    {"name": "CC_RUST_BIN", "value": "presto-server"},
    {"name": "CC_CACHE_DEPENDENCIES", "value": "true"},
    {"name": "CC_PRE_BUILD_HOOK", "value": "./scripts/clever-pre-build.sh"},
    {"name": "OWNER_AUTH_SINGLE_INSTANCE", "value": "1"},
    {"name": "OIDC_ISSUER", "value": "https://issuer.example?realm=staging"},
    {"name": "OIDC_CLIENT_ID", "value": "client-123"},
    {"name": "OIDC_REDIRECT_URI", "value": "https://staging.example/auth/callback"},
    {"name": "BISCUIT_PRIVATE_KEY", "value": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"},
    {"name": "INGEST_TOKEN", "value": "abcdefghijklmnopqrstuvwxyz123456"}
  ],
  "fromAddons": [],
  "fromDependencies": []
}
JSON

env_invalid_oidc_redirect="$tmp/env-invalid-oidc-redirect.json"
cat >"$env_invalid_oidc_redirect" <<'JSON'
{
  "env": [
    {"name": "CC_RUST_BIN", "value": "presto-server"},
    {"name": "CC_CACHE_DEPENDENCIES", "value": "true"},
    {"name": "CC_PRE_BUILD_HOOK", "value": "./scripts/clever-pre-build.sh"},
    {"name": "OWNER_AUTH_SINGLE_INSTANCE", "value": "1"},
    {"name": "OIDC_ISSUER", "value": "https://issuer.example"},
    {"name": "OIDC_CLIENT_ID", "value": "client-123"},
    {"name": "OIDC_REDIRECT_URI", "value": "https://staging.example/not-callback"},
    {"name": "BISCUIT_PRIVATE_KEY", "value": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"},
    {"name": "INGEST_TOKEN", "value": "abcdefghijklmnopqrstuvwxyz123456"}
  ],
  "fromAddons": [],
  "fromDependencies": []
}
JSON

env_invalid_oidc_client_id="$tmp/env-invalid-oidc-client-id.json"
cat >"$env_invalid_oidc_client_id" <<'JSON'
{
  "env": [
    {"name": "CC_RUST_BIN", "value": "presto-server"},
    {"name": "CC_CACHE_DEPENDENCIES", "value": "true"},
    {"name": "CC_PRE_BUILD_HOOK", "value": "./scripts/clever-pre-build.sh"},
    {"name": "OWNER_AUTH_SINGLE_INSTANCE", "value": "1"},
    {"name": "OIDC_ISSUER", "value": "https://issuer.example"},
    {"name": "OIDC_CLIENT_ID", "value": "client-123 "},
    {"name": "OIDC_REDIRECT_URI", "value": "https://staging.example/auth/callback"},
    {"name": "BISCUIT_PRIVATE_KEY", "value": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"},
    {"name": "INGEST_TOKEN", "value": "abcdefghijklmnopqrstuvwxyz123456"}
  ],
  "fromAddons": [],
  "fromDependencies": []
}
JSON

secret_regex='issuer\.example|client-123|0123456789abcdef|abcdefghijklmnopqrstuvwxyz123456'

run_preflight() {
  local log_file="$1"
  local status_fixture="$2"
  local env_fixture="$3"
  local stdout_file="$4"
  local stderr_file="$5"
  : >"$log_file"
  CALL_LOG="$log_file" STATUS_FIXTURE="$status_fixture" ENV_FIXTURE="$env_fixture" \
    GIT_BIN=git CLEVER_BIN="$tmp/bin/clever" PYTHON_BIN=python3 PATH="$tmp/bin:$PATH" \
    "$root/scripts/clever-staging-preflight.sh" >"$stdout_file" 2>"$stderr_file"
}

assert_secret_free() {
  local label="$1"
  local stdout_file="$2"
  local stderr_file="$3"
  if grep -qE "$secret_regex" "$stdout_file" "$stderr_file"; then
    echo "preflight leaked sensitive values on $label" >&2
    cat "$stdout_file" >&2
    cat "$stderr_file" >&2
    exit 1
  fi
}

assert_call_log() {
  local label="$1"
  local log_file="$2"
  local expected="$3"
  if [[ "$(cat "$log_file")" != "$expected" ]]; then
    printf 'unexpected clever calls for %s:\n%s\n' "$label" "$(cat "$log_file")" >&2
    exit 1
  fi
}

expect_success() {
  local label="$1"
  local status_fixture="$2"
  local env_fixture="$3"
  local expected_log="$4"
  local stdout_file="$tmp/$label.out"
  local stderr_file="$tmp/$label.err"
  local log_file="$tmp/$label.log"
  run_preflight "$log_file" "$status_fixture" "$env_fixture" "$stdout_file" "$stderr_file"
  assert_secret_free "$label" "$stdout_file" "$stderr_file"
  assert_call_log "$label" "$log_file" "$expected_log"
}

expect_failure() {
  local label="$1"
  local status_fixture="$2"
  local env_fixture="$3"
  local expected_log="$4"
  local stdout_file="$tmp/$label.out"
  local stderr_file="$tmp/$label.err"
  local log_file="$tmp/$label.log"
  if run_preflight "$log_file" "$status_fixture" "$env_fixture" "$stdout_file" "$stderr_file"; then
    echo "preflight accepted $label" >&2
    exit 1
  fi
  assert_secret_free "$label" "$stdout_file" "$stderr_file"
  assert_call_log "$label" "$log_file" "$expected_log"
}

expect_success "canonical-success" "$status_ok" "$env_success" $'status -a staging -F json\nenv -a staging -F json'
expect_success "direct-list-compat" "$status_ok" "$env_direct_list" $'status -a staging -F json\nenv -a staging -F json'
expect_success "flat-dict-compat" "$status_ok" "$env_flat_dict" $'status -a staging -F json\nenv -a staging -F json'

expect_failure "direct-list-ambiguous" "$status_ok" "$env_direct_list_collision" $'status -a staging -F json\nenv -a staging -F json'
expect_failure "scale-gt1" "$status_scale_gt1" "$env_success" 'status -a staging -F json'
expect_failure "missing-cc-pre-build-hook" "$status_ok" "$env_missing_prebuild" $'status -a staging -F json\nenv -a staging -F json'
expect_failure "wrong-cc-cache-dependencies" "$status_ok" "$env_wrong_cache" $'status -a staging -F json\nenv -a staging -F json'
expect_failure "hidden-database-url-in-addon" "$status_ok" "$env_forbidden_addon" $'status -a staging -F json\nenv -a staging -F json'
expect_failure "hidden-redis-url-in-dependency" "$status_ok" "$env_forbidden_dependency" $'status -a staging -F json\nenv -a staging -F json'
expect_failure "ambiguous-collision" "$status_ok" "$env_collision" $'status -a staging -F json\nenv -a staging -F json'
expect_failure "invalid-env-shape" "$status_ok" "$env_invalid_shape" $'status -a staging -F json\nenv -a staging -F json'
expect_failure "invalid-env-member" "$status_ok" "$env_invalid_member" $'status -a staging -F json\nenv -a staging -F json'
expect_failure "invalid-oidc-issuer" "$status_ok" "$env_invalid_oidc_issuer" $'status -a staging -F json\nenv -a staging -F json'
expect_failure "invalid-oidc-redirect" "$status_ok" "$env_invalid_oidc_redirect" $'status -a staging -F json\nenv -a staging -F json'
expect_failure "invalid-oidc-client-id" "$status_ok" "$env_invalid_oidc_client_id" $'status -a staging -F json\nenv -a staging -F json'

git remote set-url cc-staging "$staging_git_ssh_url"
expect_success "git+ssh-compat" "$status_ok" "$env_success" $'status -a staging -F json\nenv -a staging -F json'

git remote set-url cc-staging "$staging_git_ssh_ssh"
expect_success "ssh-url-compat" "$status_ok" "$env_success" $'status -a staging -F json\nenv -a staging -F json'

git remote set-url cc-staging "$staging_git_ssh_scp"
expect_success "scp-compat" "$status_ok" "$env_success" $'status -a staging -F json\nenv -a staging -F json'

git remote set-url cc-staging "$production_deploy_url_no_slash"
expect_failure "remote-mispointed" "$status_ok" "$env_success" ''
git remote set-url cc-staging "$staging_deploy_url_no_slash"

form_valid_origin="$tmp/form-valid-origin.git"
form_valid_repo="$tmp/form-valid-repo"
setup_repo_with_config "$form_valid_origin" "$form_valid_repo" "$fixture" "$staging_deploy_url_no_slash"
form_valid_stdout="$tmp/form-valid.out"
form_valid_stderr="$tmp/form-valid.err"
form_valid_log="$tmp/form-valid.log"
run_preflight_in_root "$form_valid_repo" "$form_valid_log" "$status_ok" "$env_success" "$form_valid_stdout" "$form_valid_stderr"
assert_secret_free "clever-json-default" "$form_valid_stdout" "$form_valid_stderr"
assert_call_log "clever-json-default" "$form_valid_log" $'status -a staging -F json\nenv -a staging -F json'

form_bad_default="$tmp/clever-tools-bad-default.json"
"$python_bin" - "$fixture" "$form_bad_default" <<'PY'
import json
import sys
from pathlib import Path

payload = json.loads(Path(sys.argv[1]).read_text(encoding='utf-8'))
payload['default'] = 42
Path(sys.argv[2]).write_text(json.dumps(payload, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
PY
form_bad_default_origin="$tmp/form-bad-default-origin.git"
form_bad_default_repo="$tmp/form-bad-default-repo"
setup_repo_with_config "$form_bad_default_origin" "$form_bad_default_repo" "$form_bad_default" "$staging_deploy_url_no_slash"
form_bad_default_stdout="$tmp/form-bad-default.out"
form_bad_default_stderr="$tmp/form-bad-default.err"
form_bad_default_log="$tmp/form-bad-default.log"
if run_preflight_in_root "$form_bad_default_repo" "$form_bad_default_log" "$status_ok" "$env_success" "$form_bad_default_stdout" "$form_bad_default_stderr"; then
  echo "preflight accepted a non-string .clever.json default" >&2
  exit 1
fi
assert_secret_free "clever-json-default-type" "$form_bad_default_stdout" "$form_bad_default_stderr"
assert_call_log "clever-json-default-type" "$form_bad_default_log" ''

form_extra_key="$tmp/clever-tools-extra-key.json"
"$python_bin" - "$fixture" "$form_extra_key" <<'PY'
import json
import sys
from pathlib import Path

payload = json.loads(Path(sys.argv[1]).read_text(encoding='utf-8'))
payload['unexpected'] = 'value'
Path(sys.argv[2]).write_text(json.dumps(payload, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
PY
form_extra_key_origin="$tmp/form-extra-key-origin.git"
form_extra_key_repo="$tmp/form-extra-key-repo"
setup_repo_with_config "$form_extra_key_origin" "$form_extra_key_repo" "$form_extra_key" "$staging_deploy_url_no_slash"
form_extra_key_stdout="$tmp/form-extra-key.out"
form_extra_key_stderr="$tmp/form-extra-key.err"
form_extra_key_log="$tmp/form-extra-key.log"
if run_preflight_in_root "$form_extra_key_repo" "$form_extra_key_log" "$status_ok" "$env_success" "$form_extra_key_stdout" "$form_extra_key_stderr"; then
  echo "preflight accepted an unexpected .clever.json key" >&2
  exit 1
fi
assert_secret_free "clever-json-extra-key" "$form_extra_key_stdout" "$form_extra_key_stderr"
assert_call_log "clever-json-extra-key" "$form_extra_key_log" ''

# Dirty checkout must fail before clever is queried.
echo dirty >> tracked.txt
if CALL_LOG="$tmp/dirty.log" STATUS_FIXTURE="$status_ok" ENV_FIXTURE="$env_success" \
  GIT_BIN=git CLEVER_BIN="$tmp/bin/clever" PYTHON_BIN=python3 PATH="$tmp/bin:$PATH" \
  "$root/scripts/clever-staging-preflight.sh" >/dev/null 2>&1; then
  echo "preflight accepted a dirty checkout" >&2
  exit 1
fi
if [[ -s "$tmp/dirty.log" ]]; then
  echo "preflight contacted clever on dirty checkout" >&2
  cat "$tmp/dirty.log" >&2
  exit 1
fi
git checkout -- tracked.txt >/dev/null

# Non-main branch must fail before clever is queried.
git checkout -b feature >/dev/null
if CALL_LOG="$tmp/feature.log" STATUS_FIXTURE="$status_ok" ENV_FIXTURE="$env_success" \
  GIT_BIN=git CLEVER_BIN="$tmp/bin/clever" PYTHON_BIN=python3 PATH="$tmp/bin:$PATH" \
  "$root/scripts/clever-staging-preflight.sh" >/dev/null 2>&1; then
  echo "preflight accepted a non-main branch" >&2
  exit 1
fi
if [[ -s "$tmp/feature.log" ]]; then
  echo "preflight contacted clever on non-main branch" >&2
  cat "$tmp/feature.log" >&2
  exit 1
fi
git checkout main >/dev/null

# Missing remote must fail before clever is queried.
git remote remove cc-staging
if CALL_LOG="$tmp/remote.log" STATUS_FIXTURE="$status_ok" ENV_FIXTURE="$env_success" \
  GIT_BIN=git CLEVER_BIN="$tmp/bin/clever" PYTHON_BIN=python3 PATH="$tmp/bin:$PATH" \
  "$root/scripts/clever-staging-preflight.sh" >/dev/null 2>&1; then
  echo "preflight accepted a missing cc-staging remote" >&2
  exit 1
fi
if [[ -s "$tmp/remote.log" ]]; then
  echo "preflight contacted clever without cc-staging remote" >&2
  cat "$tmp/remote.log" >&2
  exit 1
fi
git remote add cc-staging "$staging_deploy_url_no_slash"

# Divergence from origin/main on the bare origin must fail before clever is queried.
second_clone="$tmp/second-clone"
git clone -b main "$origin" "$second_clone" >/dev/null
(
  cd "$second_clone"
  git config user.name "Test User"
  git config user.email "test@example.com"
  printf 'remote drift\n' >> tracked.txt
  git commit -am "remote drift" >/dev/null
  git push origin main >/dev/null
)
if CALL_LOG="$tmp/drift.log" STATUS_FIXTURE="$status_ok" ENV_FIXTURE="$env_success" \
  GIT_BIN=git CLEVER_BIN="$tmp/bin/clever" PYTHON_BIN=python3 PATH="$tmp/bin:$PATH" \
  "$root/scripts/clever-staging-preflight.sh" >/dev/null 2>&1; then
  echo "preflight accepted a checkout not aligned with origin/main" >&2
  exit 1
fi
if [[ -s "$tmp/drift.log" ]]; then
  echo "preflight contacted clever on diverged checkout" >&2
  cat "$tmp/drift.log" >&2
  exit 1
fi

echo "clever staging preflight topology and redaction verified"
