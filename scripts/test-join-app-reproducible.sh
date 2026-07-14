#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

hash_bundle() {
  local output="$1"
  (
    cd "$root/crates/server/static/join-app"
    find . -type f -print | LC_ALL=C sort | while IFS= read -r file; do
      if command -v sha256sum >/dev/null 2>&1; then sha256sum "$file"; else shasum -a 256 "$file"; fi
    done > "$output"
  )
}

"$root/scripts/build-join-app.sh"
hash_bundle "$tmp/first"
"$root/scripts/build-join-app.sh"
hash_bundle "$tmp/second"
cmp "$tmp/first" "$tmp/second"
echo "join bundle repeatability verified across two dx builds in this producer environment"
