#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cli="${DIOXUS_CLI:-}"

if [[ -z "$cli" && -x "${HOME}/.cargo/bin/dx" ]]; then
  cli="${HOME}/.cargo/bin/dx"
elif [[ -z "$cli" ]]; then
  cli="$(command -v dx || true)"
fi

if [[ -z "$cli" ]] || ! "$cli" --version 2>/dev/null | grep -q '^dioxus 0\.7\.9 '; then
  echo "dioxus-cli 0.7.9 is required (cargo install dioxus-cli --version 0.7.9 --locked)" >&2
  exit 1
fi

source_dir="$root/target/dx/rumble-lm-app/release/web/public"
rm -rf "$source_dir"

(
  cd "$root/crates/app"
  "$cli" build --release --web --locked
)

destination="$root/crates/server/static/owner-app"
rm -rf "$destination"
mkdir -p "$destination"
cp -R "$source_dir"/. "$destination"/
"$root/scripts/verify-owner-app.sh" "$destination"

echo "owner bundle copied to crates/server/static/owner-app"
