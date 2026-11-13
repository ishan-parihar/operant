#!/usr/bin/env bash
# scripts/tests/genome_claim_holder_test.sh — the claim/done holder gate.
#
# The defect this pins: `claim_holder` parsed the holder of a claim with the
# charset `[A-Za-z0-9_.-]`, but `agent_identity` DISAMBIGUATES a checkout from
# the shared host identity by appending `@<checkout>`. The parse therefore
# stopped at the `@`: the holder of a claim read as the bare agent name while
# the identity was the suffixed one, so `genome done` refused the claimer's
# OWN claim as somebody else's. Every checkout-disambiguated agent — every
# worktree agent in this fleet — could claim an item and never finish it, and
# the item sat in `in-progress` forever (observed 2026-11-12, iter-2198).
#
# Asserted here:
#   1. a checkout-disambiguated holder is read WHOLE
#   2. a bare holder is still read (the old case must not regress)
#   3. an unclaimed cell has no holder
#   4. the round trip holds: the parsed holder EQUALS the identity the
#      claimer was given
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
GENOME="$REPO_ROOT/scripts/genome"

fail() { echo "FAIL: $*"; exit 1; }

python3 - "$GENOME" <<'EOF'
import importlib.machinery
import importlib.util
import sys

path = sys.argv[1]
loader = importlib.machinery.SourceFileLoader("genome_under_test", path)
spec = importlib.util.spec_from_loader("genome_under_test", loader)
genome = importlib.util.module_from_spec(spec)
loader.exec_module(genome)

def check(actual, expected, what):
    if actual != expected:
        raise SystemExit(
            f"FAIL: {what}: expected {expected!r}, got {actual!r}"
        )

# 1. the checkout-disambiguated holder is read WHOLE
check(
    genome.claim_holder("claimed-by `agent-omarchy@tui-input-wt` since 2026-11-12T12:00"),
    "agent-omarchy@tui-input-wt",
    "a checkout-disambiguated holder must be read whole",
)

# 2. the bare holder still reads
check(
    genome.claim_holder("claimed-by `agent-omarchy` since 2026-11-12T12:00"),
    "agent-omarchy",
    "a bare holder must still read",
)

# 3. an unclaimed cell has no holder
check(genome.claim_holder("—"), None, "an unclaimed cell must have no holder")
check(genome.claim_holder(""), None, "an empty cell must have no holder")

# 4. the round trip: what the claim writes is what the done check reads
aid = "agent-omarchy@tui-input-wt"
check(
    genome.claim_holder(genome.claim_text(aid)),
    aid,
    "the holder the claim writes must be the holder the done check reads",
)
EOF

echo "OK: the claim holder reads a checkout-disambiguated agent id whole"
