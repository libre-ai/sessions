#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

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
git push -u origin main >/dev/null

clever_env="$tmp/clever-env.json"
cat >"$clever_env" <<'JSON'
{"OWNER_AUTH_SINGLE_INSTANCE":"1","OIDC_ISSUER":"https://issuer.example","OIDC_CLIENT_ID":"client-123","OIDC_REDIRECT_URI":"https://staging.example/auth/callback","BISCUIT_PRIVATE_KEY":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef","INGEST_TOKEN":"abcdefghijklmnopqrstuvwxyz123456"}
JSON

mkdir -p "$tmp/bin"
cat >"$tmp/bin/clever" <<'EOF'
#!/usr/bin/env bash
case "$1" in
  --version)
    echo 'clever version 1.0.0'
    ;;
  status)
    echo 'staging: UP'
    ;;
  env)
    cat "$CLEVER_ENV_FIXTURE"
    ;;
  *)
    echo "unexpected clever call: $*" >&2
    exit 64
    ;;
esac
EOF
chmod +x "$tmp/bin/clever"

success_out="$tmp/success.out"
success_err="$tmp/success.err"
GIT_BIN=git CLEVER_BIN="$tmp/bin/clever" PYTHON_BIN=python3 \
  CLEVER_ENV_FIXTURE="$clever_env" PATH="$tmp/bin:$PATH" \
  "$root/scripts/clever-staging-preflight.sh" >"$success_out" 2>"$success_err"
if grep -qE 'issuer\.example|client-123|0123456789abcdef|abcdefghijklmnopqrstuvwxyz123456' "$success_out" "$success_err"; then
  echo "preflight leaked sensitive values on success" >&2
  cat "$success_out" >&2
  cat "$success_err" >&2
  exit 1
fi

# Dirty checkout must fail before clever is queried.
echo dirty >> tracked.txt
if GIT_BIN=git CLEVER_BIN="$tmp/bin/clever" PYTHON_BIN=python3 \
  CLEVER_ENV_FIXTURE="$clever_env" PATH="$tmp/bin:$PATH" \
  "$root/scripts/clever-staging-preflight.sh" >/dev/null 2>&1; then
  echo "preflight accepted a dirty checkout" >&2
  exit 1
fi
git checkout -- tracked.txt >/dev/null

# Non-main branch must fail before clever is queried.
git checkout -b feature >/dev/null
if GIT_BIN=git CLEVER_BIN="$tmp/bin/clever" PYTHON_BIN=python3 \
  CLEVER_ENV_FIXTURE="$clever_env" PATH="$tmp/bin:$PATH" \
  "$root/scripts/clever-staging-preflight.sh" >/dev/null 2>&1; then
  echo "preflight accepted a non-main branch" >&2
  exit 1
fi
git checkout main >/dev/null

# Divergence from origin/main must fail.
echo drift >> tracked.txt
git commit -am "drift" >/dev/null
if GIT_BIN=git CLEVER_BIN="$tmp/bin/clever" PYTHON_BIN=python3 \
  CLEVER_ENV_FIXTURE="$clever_env" PATH="$tmp/bin:$PATH" \
  "$root/scripts/clever-staging-preflight.sh" >/dev/null 2>&1; then
  echo "preflight accepted a checkout not aligned with origin/main" >&2
  exit 1
fi

echo "clever staging preflight topology and redaction verified"
