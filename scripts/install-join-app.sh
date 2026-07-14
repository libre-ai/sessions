#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
package_dir="${1:-$root/target/join-app-bundle}"
source_dir="$package_dir/join-app"

if [[ ! -d "$source_dir" ]]; then
  echo "join app bundle not found at $source_dir" >&2
  exit 1
fi

"$root/scripts/verify-join-app.sh" "$source_dir"

destination="$root/crates/server/static/join-app"
rm -rf "$destination"
mkdir -p "$destination"
cp -R "$source_dir"/. "$destination"/

echo "join bundle installed at $destination"
