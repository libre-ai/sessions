#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

rustup target add wasm32-unknown-unknown
cargo install dioxus-cli --version 0.7.9 --locked
./scripts/build-owner-app.sh
./scripts/test-owner-app-package.sh
