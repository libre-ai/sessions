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

source_dir="$root/target/dx/rumble-lm-join/release/web/public"
rm -rf "$source_dir"

(
  cd "$root/crates/join"
  export CARGO_INCREMENTAL=0
  export SOURCE_DATE_EPOCH=0
  export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }--remap-path-prefix=$root=. --remap-path-prefix=$HOME=<home>"
  "$cli" build --release --web --locked
)

destination="$root/crates/server/static/join-app"
rm -rf "$destination"
mkdir -p "$destination"
cp -R "$source_dir"/. "$destination"/
python3 "$root/scripts/finalize-join-app.py" "$destination"
"$root/scripts/verify-join-app.sh" "$destination"

echo "join bundle copied to crates/server/static/join-app"
