#!/usr/bin/env bash
# scripts/tests/command_sweep_test.sh — the TUI command sweep's own contract.
#
# The defect this pins: the sweep read ANY non-zero exit as "this command fell
# through to the model", so a simulator that could not START — no writable
# HOME, a missing shared library, a crash before the first frame — was reported
# as 98 broken commands. Measured 2026-11-05 in a sandbox whose HOME was
# read-only: `consumed: 0/98` and all 98 listed under "BROKEN". That is the
# RT-162 shape (a gate whose failures are all one cause reports that cause once
# per command), and the next agent would have read "98 broken" as 98 defects in
# working commands.
#
# Asserted here, against FAKE simulators (no real binary is driven):
#   1. a simulator that cannot start is reported as a SIMULATOR ERROR, and NOT
#      as a fall-through
#   2. a simulator that reports the assertion failure IS reported as a
#      fall-through (the defect class the sweep exists to find)
#   3. both classes exit non-zero: a gate that cannot verify must not pass
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SWEEP="$REPO_ROOT/crates/operant-cli/tests/tui_scenarios/command_sweep.py"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

fail() { echo "FAIL: $*"; exit 1; }

# ── the two fake simulators ─────────────────────────────────────────────────
cat > "$TMP/cannot-start" <<'EOF'
#!/bin/sh
echo "boom: the simulator could not start" >&2
exit 1
EOF
cat > "$TMP/fell-through" <<'EOF'
#!/bin/sh
echo "Error: Assertion failed: messages == 0 (actual: 1)" >&2
exit 1
EOF
chmod +x "$TMP/cannot-start" "$TMP/fell-through"

# ── 1. a simulator that cannot start is an ERROR, not a fall-through ───────
out="$(python3 "$SWEEP" --binary "$TMP/cannot-start" 2>&1)" && rc=0 || rc=$?
echo "$out" | grep -q "SIMULATOR ERROR" \
    || fail "a simulator that cannot start must be reported as a SIMULATOR ERROR"
echo "$out" | grep -q "BROKEN (fall through to the model as prompt text)" \
    && fail "a simulator that cannot start must NOT be reported as a fall-through"
[ "$rc" -ne 0 ] || fail "a sweep that could not verify anything must exit non-zero"

# ── 2. a real fall-through is reported as a fall-through ───────────────────
out="$(python3 "$SWEEP" --binary "$TMP/fell-through" 2>&1)" && rc=0 || rc=$?
echo "$out" | grep -q "BROKEN (fall through to the model as prompt text)" \
    || fail "a command that reached the model must be reported under BROKEN"
echo "$out" | grep -q "SIMULATOR ERROR" \
    && fail "a fall-through is not a simulator error"
[ "$rc" -ne 0 ] || fail "a sweep that found a fall-through must exit non-zero"

echo "PASS: the command sweep separates a fall-through from a simulator error"
