#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source_dir="${1:-$root/crates/server/static/join-app}"
package_dir="${2:-$root/target/join-app-bundle}"

"$root/scripts/verify-join-app.sh" "$source_dir"
rm -rf "$package_dir"
mkdir -p "$package_dir"
cp -R "$source_dir" "$package_dir/join-app"

(
  cd "$package_dir"
  find join-app -type f -print \
    | LC_ALL=C sort \
    | while IFS= read -r file; do
        if command -v sha256sum >/dev/null 2>&1; then
          sha256sum "$file"
        else
          shasum -a 256 "$file"
        fi
      done > SHA256SUMS
)

echo "join bundle packaged at $package_dir"
