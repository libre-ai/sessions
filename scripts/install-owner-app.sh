#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
package_dir="${1:?usage: install-owner-app.sh PACKAGE_DIR [DESTINATION]}"
destination="${2:-$root/crates/server/static/owner-app}"

if [[ ! -f "$package_dir/SHA256SUMS" || ! -d "$package_dir/owner-app" ]]; then
  echo "invalid owner bundle package: $package_dir" >&2
  exit 1
fi
if find "$package_dir/owner-app" -type l -print -quit | grep -q .; then
  echo "owner bundle package contains a symlink" >&2
  exit 1
fi

expected="$(mktemp)"
actual="$(mktemp)"
trap 'rm -f "$expected" "$actual"' EXIT
sed -nE 's/^[0-9a-f]{64}  (owner-app\/.*)$/\1/p' "$package_dir/SHA256SUMS" \
  | LC_ALL=C sort > "$expected"
(
  cd "$package_dir"
  find owner-app -type f -print | LC_ALL=C sort > "$actual"
)
if ! cmp -s "$expected" "$actual"; then
  echo "owner bundle file list does not match SHA256SUMS" >&2
  diff -u "$expected" "$actual" >&2 || true
  exit 1
fi

(
  cd "$package_dir"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum --check --strict SHA256SUMS
  else
    shasum -a 256 --check SHA256SUMS
  fi
)
"$root/scripts/verify-owner-app.sh" "$package_dir/owner-app"

rm -rf "$destination"
mkdir -p "$(dirname "$destination")"
cp -R "$package_dir/owner-app" "$destination"
echo "verified owner bundle installed at $destination"
