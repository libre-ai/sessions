#!/usr/bin/env bash
set -euo pipefail

sessions_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
libre_root="${LIBRE_AI_ROOT:-$(cd "$sessions_root/.." && pwd)}"
proof_kit="$libre_root/proof-kit"
agent_factory="$libre_root/agent-factory"
output="$(python3 -c 'import pathlib, sys; print(pathlib.Path(sys.argv[1]).resolve())' "${1:-$sessions_root/target/stack-proof}")"
tmp_root="$(python3 -c 'import os, pathlib; print(pathlib.Path(os.environ.get("TMPDIR", "/tmp")).resolve())')"
case "$output" in
  "$sessions_root"/target/*|"$tmp_root"/sessions-stack-*) ;;
  *) echo "refusing destructive output path outside sessions/target or ${tmp_root}/sessions-stack-*" >&2; exit 1 ;;
esac
auth_dir="$output/.authorization"

for path in "$proof_kit/inspect/Cargo.toml" "$proof_kit/db-inspect/Cargo.toml" "$agent_factory/Cargo.toml"; do
  [[ -f "$path" ]] || { echo "missing sibling repository contract: $path" >&2; exit 1; }
done

rm -rf "$output"
mkdir -p "$output"
trap 'rm -rf "$auth_dir"' EXIT

cd "$libre_root"
cargo run --quiet --manifest-path proof-kit/inspect/Cargo.toml -- \
  portal inspect sessions/crates/ui --evidence > "$output/proof-kit-ui.json"

cargo run --quiet --manifest-path proof-kit/db-inspect/Cargo.toml -- run \
  --manifest sessions/docs/db/jobs-manifest.json \
  --schema-dump sessions/crates/server/migrations/0001_jobs_and_outbox.sql \
  --inspection-at 2026-07-13T00:00:00Z \
  --profile protected_branch \
  --report-json "$output/db-inspection.json"

cd "$sessions_root"
./scripts/build-owner-app.sh
cargo build --release --bin presto-server
ARTIFACT_VERSION=0.0.0 cargo run --quiet --bin emit-artifact-manifest -- \
  target/release/presto-server --json-out "$output/artifact-manifest.json"

cd "$agent_factory"
cargo run --quiet -p bolt-cos-matic-cli --example issue_handoff_authorization_fixture -- \
  "$sessions_root/docs/handoff/lm-slice-handoff.json" "$auth_dir"

cd "$libre_root"
cargo run --quiet --manifest-path agent-factory/Cargo.toml --bin bolt-cosmatic -- \
  handoff plan sessions/docs/handoff/lm-slice-handoff.json \
  --dry-run --json \
  --evidence-report "$output/proof-kit-ui.json" \
  --db-inspection-report "$output/db-inspection.json" \
  --biscuit-token-file "$auth_dir/token.txt" \
  --biscuit-keyset "$auth_dir/keyset.json" \
  --biscuit-revocations "$auth_dir/revocations.json" \
  --biscuit-replay-directory "$auth_dir/replay" \
  > "$output/agent-factory-dry-run.json"

python3 - "$output" <<'PY'
from pathlib import Path
import json, sys
root = Path(sys.argv[1])
ui = json.loads((root / 'proof-kit-ui.json').read_text())
db = json.loads((root / 'db-inspection.json').read_text())
plan = json.loads((root / 'agent-factory-dry-run.json').read_text())
manifest = json.loads((root / 'artifact-manifest.json').read_text())
assert ui['status'] == 'passed'
assert db['data']['status'] == 'passed' and not db['data']['summary']['gate_blocked']
assert not plan['report']['findings']
assert all(gate['status'] in ('pass', 'passed') for gate in plan['gates'])
assert manifest['artifact']['hash'].startswith('sha256:')
print(f"sessions stack proof PASS: {len(ui['checks'])} UI checks, {len(plan['gates'])} Agent Factory gates")
PY

rm -rf "$auth_dir"
trap - EXIT
if command -v sha256sum >/dev/null; then
  sha256sum "$output"/*.json
else
  shasum -a 256 "$output"/*.json
fi
