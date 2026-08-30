#!/usr/bin/env bash
# scripts/check-pk-sidecar.sh
#
# Plan 015 / Phase 0 — pk-sidecar scaffold gate.
#
# Asserts the Phase 0 contract from the 015 plan:
#   1. The scaffold file exists at the canonical path.
#   2. It is executable.
#   3. It answers a JSON `ping` with the expected shape.
#
# This script is meant to be the first gate a Phase 1 PR must pass
# before any supervisor code lands. It is intentionally cheap: no
# python venv, no tokio, no operant binary. The Phase 1 supervisor
# will get a richer script that runs the same checks and adds the
# SessionKernel + NDJSON server.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCAFFOLD="$REPO_ROOT/pk-sidecar/main.py"

if [[ ! -f "$SCAFFOLD" ]]; then
    echo "FAIL: scaffold missing at $SCAFFOLD (015 Phase 0)" >&2
    exit 1
fi

if [[ ! -x "$SCAFFOLD" ]]; then
    echo "FAIL: scaffold not executable: $SCAFFOLD" >&2
    exit 1
fi

# Drive a ping and assert the response shape.
PING_REQUEST='{"op": "ping"}'
RESPONSE="$(printf '%s\n' "$PING_REQUEST" | "$SCAFFOLD" 2>/dev/null || true)"

if [[ -z "$RESPONSE" ]]; then
    echo "FAIL: scaffold produced no response to ping" >&2
    exit 1
fi

# `phase` must be 0, `ok` must be true, and a pid must be present.
# Use python3 for the parse so we don't pull in jq.
PARSE_OK="$(printf '%s' "$RESPONSE" | python3 -c '
import json, sys
try:
    payload = json.loads(sys.stdin.read())
except Exception as exc:
    print(f"PARSE_ERROR: {exc}")
    sys.exit(2)

if not payload.get("ok"):
    print(f"NOT_OK: {payload}")
    sys.exit(1)
if payload.get("phase") != 0:
    print("WRONG_PHASE: " + repr(payload.get("phase")))
    sys.exit(1)
if "pid" not in payload:
    print("NO_PID")
    sys.exit(1)
print("OK")
')"

if [[ "$PARSE_OK" != "OK" ]]; then
    echo "FAIL: scaffold ping response was: $RESPONSE" >&2
    echo "      parse: $PARSE_OK" >&2
    exit 1
fi

echo "OK: pk-sidecar scaffold present, executable, and answers ping"
