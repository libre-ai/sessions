#!/usr/bin/env bash
# Post-deploy smoke test for a Presto-Matic deployment.
#
# Usage: scripts/clever-smoke.sh https://your-app.cleverapps.io
#
# Checks /health and that POST /sessions returns a host token — which exercises
# the session-store write (Postgres in prod) and the Biscuit mint end to end.
set -euo pipefail

BASE="${1:?usage: clever-smoke.sh <base-url>}"
BASE="${BASE%/}"

echo "→ GET ${BASE}/health"
health=$(curl -fsS --max-time 10 "${BASE}/health")
[ "$health" = "ok" ] || { echo "FAIL: /health returned '${health}'"; exit 1; }
echo "  ok"

echo "→ POST ${BASE}/sessions"
resp=$(curl -fsS --max-time 10 -X POST "${BASE}/sessions")
echo "  ${resp}"
echo "$resp" | grep -q '"host_token"' || { echo "FAIL: /sessions returned no host token"; exit 1; }
echo "$resp" | grep -q '"session_id"' || { echo "FAIL: /sessions returned no session id"; exit 1; }
echo "  ok: session created with a host token"

echo "✅ smoke test passed"
