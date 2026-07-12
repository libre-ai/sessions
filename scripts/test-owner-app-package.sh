#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source_dir="${1:-$root/crates/server/static/owner-app}"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

"$root/scripts/package-owner-app.sh" "$source_dir" "$tmp/package"
"$root/scripts/package-owner-app.sh" "$source_dir" "$tmp/package-again" >/dev/null
cmp "$tmp/package/SHA256SUMS" "$tmp/package-again/SHA256SUMS"
cp -R "$tmp/package" "$tmp/tampered"
asset="$(find "$tmp/tampered/owner-app/assets" -type f -name '*.js' -print -quit)"
printf '\n// tampered\n' >> "$asset"
if "$root/scripts/install-owner-app.sh" "$tmp/tampered" "$tmp/rejected" >/dev/null 2>&1; then
  echo "tampered owner bundle was accepted" >&2
  exit 1
fi

cp -R "$tmp/package" "$tmp/extra-file"
printf 'unexpected' > "$tmp/extra-file/owner-app/unexpected.txt"
if "$root/scripts/install-owner-app.sh" "$tmp/extra-file" "$tmp/rejected-extra" >/dev/null 2>&1; then
  echo "owner bundle with an unattested file was accepted" >&2
  exit 1
fi

"$root/scripts/install-owner-app.sh" "$tmp/package" "$tmp/installed" >/dev/null
diff -qr "$source_dir" "$tmp/installed"
echo "owner bundle package gate verified: reproducible sums, tampering/extra files rejected, exact bundle installed"
