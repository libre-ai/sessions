#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

log="$tmp/calls.log"
mkdir -p "$tmp/bin"

cat >"$tmp/bin/rustup" <<'EOF'
#!/usr/bin/env bash
printf 'rustup %s\n' "$*" >>"$CALL_LOG"
EOF

cat >"$tmp/bin/cargo" <<'EOF'
#!/usr/bin/env bash
printf 'cargo %s\n' "$*" >>"$CALL_LOG"
EOF

cat >"$tmp/bin/build-owner-app.sh" <<'EOF'
#!/usr/bin/env bash
printf 'owner\n' >>"$CALL_LOG"
EOF

cat >"$tmp/bin/build-join-app.sh" <<'EOF'
#!/usr/bin/env bash
printf 'join\n' >>"$CALL_LOG"
EOF

cat >"$tmp/bin/dx" <<'EOF'
#!/usr/bin/env bash
if [[ ${1:-} == --version ]]; then
  echo 'dioxus 0.7.9 fake'
  exit 0
fi
printf 'dx %s\n' "$*" >>"$CALL_LOG"
EOF

chmod +x "$tmp/bin"/*
CALL_LOG="$log" PATH="$tmp/bin:$PATH" \
CARGO_BIN="$tmp/bin/cargo" \
RUSTUP_BIN="$tmp/bin/rustup" \
BUILD_OWNER_APP="$tmp/bin/build-owner-app.sh" \
BUILD_JOIN_APP="$tmp/bin/build-join-app.sh" \
DIOXUS_CLI="$tmp/bin/dx" \
  "$root/scripts/clever-pre-build.sh"

expected=$'rustup target add wasm32-unknown-unknown\ncargo install dioxus-cli --version 0.7.9 --locked\nowner\njoin'
actual="$(cat "$log")"
if [[ "$actual" != "$expected" ]]; then
  printf 'unexpected call order:\n%s\n' "$actual" >&2
  exit 1
fi

fail_log="$tmp/fail.log"
cat >"$tmp/bin/cargo-fail" <<'EOF'
#!/usr/bin/env bash
printf 'cargo-fail %s\n' "$*" >>"$CALL_LOG"
exit 42
EOF
chmod +x "$tmp/bin/cargo-fail"
if CALL_LOG="$fail_log" PATH="$tmp/bin:$PATH" \
  CARGO_BIN="$tmp/bin/cargo-fail" \
  RUSTUP_BIN="$tmp/bin/rustup" \
  BUILD_OWNER_APP="$tmp/bin/build-owner-app.sh" \
  BUILD_JOIN_APP="$tmp/bin/build-join-app.sh" \
  DIOXUS_CLI="$tmp/bin/dx" \
  "$root/scripts/clever-pre-build.sh" >/dev/null 2>&1; then
  echo "pre-build unexpectedly succeeded with failing cargo" >&2
  exit 1
fi
if grep -qE '^(owner|join)$' "$fail_log"; then
  echo "pre-build continued after failing cargo" >&2
  cat "$fail_log" >&2
  exit 1
fi

echo "clever pre-build order and fail-closed wiring verified"
