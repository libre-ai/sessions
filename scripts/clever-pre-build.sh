#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

cargo_bin="${CARGO_BIN:-cargo}"
rustup_bin="${RUSTUP_BIN:-rustup}"
build_owner="${BUILD_OWNER_APP:-$root/scripts/build-owner-app.sh}"
build_join="${BUILD_JOIN_APP:-$root/scripts/build-join-app.sh}"

"$rustup_bin" target add wasm32-unknown-unknown
"$cargo_bin" install dioxus-cli --version 0.7.9 --locked
"$build_owner"
"$build_join"
