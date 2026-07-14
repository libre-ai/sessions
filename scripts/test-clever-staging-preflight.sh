#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
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

cat > .clever.json <<'JSON'
{"services":[{"alias":"staging","id":"svc_staging"}]}
JSON
printf 'main\n' > tracked.txt
git add .clever.json tracked.txt
git commit -m "init" >/dev/null
git remote add cc-staging "$origin"
git push -u origin main >/dev/null

status_ok="$tmp/status-ok.json"
cat >"$status_ok" <<'JSON'
{"scalability":{"horizontal":{"min":1,"max":1}}}
JSON

status_scale_gt1="$tmp/status-scale-gt1.json"
cat >"$status_scale_gt1" <<'JSON'
{"scalability":{"horizontal":{"min":1,"max":2}}}
JSON

env_ok="$tmp/env-ok.json"
cat >"$env_ok" <<'JSON'
{"CC_RUST_BIN":"presto-server","CC_CACHE_DEPENDENCIES":"true","CC_PRE_BUILD_HOOK":"./scripts/clever-pre-build.sh","OWNER_AUTH_SINGLE_INSTANCE":"1","OIDC_ISSUER":"https://issuer.example","OIDC_CLIENT_ID":"client-123","OIDC_REDIRECT_URI":"https://staging.example/auth/callback","BISCUIT_PRIVATE_KEY":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef","INGEST_TOKEN":"abcdefghijklmnopqrstuvwxyz123456"}
JSON

env_missing_prebuild="$tmp/env-missing-prebuild.json"
cat >"$env_missing_prebuild" <<'JSON'
{"CC_RUST_BIN":"presto-server","CC_CACHE_DEPENDENCIES":"true","OWNER_AUTH_SINGLE_INSTANCE":"1","OIDC_ISSUER":"https://issuer.example","OIDC_CLIENT_ID":"client-123","OIDC_REDIRECT_URI":"https://staging.example/auth/callback","BISCUIT_PRIVATE_KEY":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef","INGEST_TOKEN":"abcdefghijklmnopqrstuvwxyz123456"}
JSON

env_wrong_cache="$tmp/env-wrong-cache.json"
cat >"$env_wrong_cache" <<'JSON'
{"CC_RUST_BIN":"presto-server","CC_CACHE_DEPENDENCIES":"false","CC_PRE_BUILD_HOOK":"./scripts/clever-pre-build.sh","OWNER_AUTH_SINGLE_INSTANCE":"1","OIDC_ISSUER":"https://issuer.example","OIDC_CLIENT_ID":"client-123","OIDC_REDIRECT_URI":"https://staging.example/auth/callback","BISCUIT_PRIVATE_KEY":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef","INGEST_TOKEN":"abcdefghijklmnopqrstuvwxyz123456"}
JSON

run_preflight() {
  local log_file="$1"
  local status_fixture="$2"
  local env_fixture="$3"
  local stdout_file="$4"
  local stderr_file="$5"
  CALL_LOG="$log_file" STATUS_FIXTURE="$status_fixture" ENV_FIXTURE="$env_fixture" \
    GIT_BIN=git CLEVER_BIN="$tmp/bin/clever" PYTHON_BIN=python3 PATH="$tmp/bin:$PATH" \
    "$root/scripts/clever-staging-preflight.sh" >"$stdout_file" 2>"$stderr_file"
}

success_stdout="$tmp/success.out"
success_stderr="$tmp/success.err"
success_log="$tmp/success.log"
run_preflight "$success_log" "$status_ok" "$env_ok" "$success_stdout" "$success_stderr"
if grep -qE 'issuer\.example|client-123|0123456789abcdef|abcdefghijklmnopqrstuvwxyz123456' "$success_stdout" "$success_stderr"; then
  echo "preflight leaked sensitive values on success" >&2
  cat "$success_stdout" >&2
  cat "$success_stderr" >&2
  exit 1
fi
if [[ "$(cat "$success_log")" != $'status -a staging -F json\nenv -a staging -F json' ]]; then
  printf 'unexpected clever call order on success:\n%s\n' "$(cat "$success_log")" >&2
  exit 1
fi

scale_stdout="$tmp/scale.out"
scale_stderr="$tmp/scale.err"
scale_log="$tmp/scale.log"
if CALL_LOG="$scale_log" STATUS_FIXTURE="$status_scale_gt1" ENV_FIXTURE="$env_ok" \
  GIT_BIN=git CLEVER_BIN="$tmp/bin/clever" PYTHON_BIN=python3 PATH="$tmp/bin:$PATH" \
  "$root/scripts/clever-staging-preflight.sh" >"$scale_stdout" 2>"$scale_stderr"; then
  echo "preflight accepted scale > 1" >&2
  exit 1
fi
if [[ "$(cat "$scale_log")" != 'status -a staging -F json' ]]; then
  printf 'unexpected clever calls for scale refusal:\n%s\n' "$(cat "$scale_log")" >&2
  exit 1
fi
if grep -qE 'issuer\.example|client-123|0123456789abcdef|abcdefghijklmnopqrstuvwxyz123456' "$scale_stdout" "$scale_stderr"; then
  echo "preflight leaked sensitive values on scale refusal" >&2
  cat "$scale_stdout" >&2
  cat "$scale_stderr" >&2
  exit 1
fi

env_missing_stdout="$tmp/env-missing.out"
env_missing_stderr="$tmp/env-missing.err"
env_missing_log="$tmp/env-missing.log"
if CALL_LOG="$env_missing_log" STATUS_FIXTURE="$status_ok" ENV_FIXTURE="$env_missing_prebuild" \
  GIT_BIN=git CLEVER_BIN="$tmp/bin/clever" PYTHON_BIN=python3 PATH="$tmp/bin:$PATH" \
  "$root/scripts/clever-staging-preflight.sh" >"$env_missing_stdout" 2>"$env_missing_stderr"; then
  echo "preflight accepted missing CC_PRE_BUILD_HOOK" >&2
  exit 1
fi
if [[ "$(cat "$env_missing_log")" != $'status -a staging -F json\nenv -a staging -F json' ]]; then
  printf 'unexpected clever calls for env-missing refusal:\n%s\n' "$(cat "$env_missing_log")" >&2
  exit 1
fi

env_wrong_stdout="$tmp/env-wrong.out"
env_wrong_stderr="$tmp/env-wrong.err"
env_wrong_log="$tmp/env-wrong.log"
if CALL_LOG="$env_wrong_log" STATUS_FIXTURE="$status_ok" ENV_FIXTURE="$env_wrong_cache" \
  GIT_BIN=git CLEVER_BIN="$tmp/bin/clever" PYTHON_BIN=python3 PATH="$tmp/bin:$PATH" \
  "$root/scripts/clever-staging-preflight.sh" >"$env_wrong_stdout" 2>"$env_wrong_stderr"; then
  echo "preflight accepted wrong CC_CACHE_DEPENDENCIES value" >&2
  exit 1
fi
if [[ "$(cat "$env_wrong_log")" != $'status -a staging -F json\nenv -a staging -F json' ]]; then
  printf 'unexpected clever calls for env-value refusal:\n%s\n' "$(cat "$env_wrong_log")" >&2
  exit 1
fi

# Missing remote must fail before clever is queried.
git remote remove cc-staging
if CALL_LOG="$tmp/remote.log" STATUS_FIXTURE="$status_ok" ENV_FIXTURE="$env_ok" \
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

# Dirty checkout must fail before clever is queried.
echo dirty >> tracked.txt
if CALL_LOG="$tmp/dirty.log" STATUS_FIXTURE="$status_ok" ENV_FIXTURE="$env_ok" \
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
if CALL_LOG="$tmp/feature.log" STATUS_FIXTURE="$status_ok" ENV_FIXTURE="$env_ok" \
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

# Divergence from origin/main must fail.
echo drift >> tracked.txt
git commit -am "drift" >/dev/null
if CALL_LOG="$tmp/drift.log" STATUS_FIXTURE="$status_ok" ENV_FIXTURE="$env_ok" \
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
