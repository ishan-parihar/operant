#!/usr/bin/env bash
# scripts/tests/genome_claim_paths_test.sh — a claim is a promise about FILES.
#
# The defect this pins (2026-11-14 swarm audit, finding 24): `genome claim` resolved
# the item through a workflow board and died with "no workflow board holds `X`" for
# any slice that was not already a board row. The coordination step was therefore
# unavailable to exactly the work that had not been planned onto a board: an agent
# either edited unclaimed files or burned a turn inventing a board row. `claim` now
# accepts --paths and records the promise against the files it covers, and refuses a
# claim that overlaps a peer's.
#
# Asserted here, against a toy tree (the real repo is only READ):
#   1. a path claim writes a ledger row naming the agent and the paths
#   2. a peer's OVERLAPPING scope is refused (a parent dir covers the file)
#   3. a peer's DISJOINT scope is accepted
#   4. --release drops the claimer's own row and nothing else
#   5. no board row and no --paths still fails, and says what to do instead
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
GENOME_SRC="$REPO/scripts/genome"
TOY="/tmp/genome-claim-paths.$$"  # /tmp, not $TMPDIR: a read-only TMPDIR is a real host state
trap 'rm -rf "$TOY"' EXIT

fail() { echo "FAIL: $*"; exit 1; }

command -v python3 >/dev/null || fail "python3 is required"

mkdir -p "$TOY/agents/workflows" "$TOY/docs"
cp "$GENOME_SRC" "$TOY/genome"
chmod +x "$TOY/genome"
: > "$TOY/docs/STATE.md"
cat > "$TOY/genome.toml" <<'TOML'
[genome]
modules = ["core", "agents"]

[core]
paths = { state = "docs/STATE.md", worklog = "docs/WORKLOG", workflows = "agents/workflows" }
checklist = []
TOML

# `coord_root` re-roots coordination state onto the MAIN worktree; the toy is a
# plain directory, so the ledger lands beside the boards it coordinates with.
# Resolved by SEARCHING rather than assuming a directory: the ledger path is
# derived from the workflows role, and a test that hardcodes `docs/kb` pins the
# wrong thing.
ledger_path() {
  local rel
  rel="$( cd "$TOY" && find . -name claims.md -not -path './.git/*' | head -1 )"
  [ -n "$rel" ] || return 1
  printf '%s/%s\n' "$TOY" "${rel#./}"
}

# 1. a path claim is recorded
out="$(cd "$TOY" && GENOME_AGENT_ID=agent-a ./genome claim slice-one --paths crates/a/src/one.rs 2>&1)" \
  || fail "a path claim must succeed: $out"
LED="$(ledger_path)" || fail "a path claim must create the ledger"
grep -q 'agent-a' "$LED" || fail "the ledger must name the claimer: $LED"
grep -q 'slice-one' "$LED" || fail "the ledger row must carry the item: $LED"
grep -q 'crates/a/src/one.rs' "$LED" || fail "the ledger row must carry the path: $LED"

# 2. a peer's overlapping claim (a parent directory) is refused
#    Captured first, then matched: `set -o pipefail` makes `( ... ) | grep` fail
#    whenever the command itself exits non-zero, and `die` exits 2 — a correct
#    refusal would read as a failed assertion.
out="$(cd "$TOY" && GENOME_AGENT_ID=agent-b ./genome claim slice-two --paths crates/a/src/ 2>&1)" || true
echo "$out" | grep -q 'is claimed by agent-a' || fail "an overlapping scope must be refused: $out"

# 3. a peer's disjoint claim is accepted
out="$(cd "$TOY" && GENOME_AGENT_ID=agent-b ./genome claim slice-two --paths crates/b/src/two.rs 2>&1)" \
  || fail "a disjoint claim must succeed: $out"
echo "$out" | grep -q 'agent-b' || fail "the acceptance must name the claimer: $out"

# 4. release drops only the claimer's own row
(cd "$TOY" && GENOME_AGENT_ID=agent-a ./genome claim slice-one --release >/dev/null 2>&1) \
  || fail "--release must succeed for the claimer"
LED="$(ledger_path)" || fail "the ledger must still exist after a release"
grep -q 'slice-two' "$LED" || fail "a peer's claim must survive a release: $LED"
if grep -q 'slice-one' "$LED"; then
  fail "the released claim must be gone: $LED"
fi

# 5. no board row and no --paths still fails, and names the alternative
out="$(cd "$TOY" && GENOME_AGENT_ID=agent-a ./genome claim slice-three 2>&1)" || true
echo "$out" | grep -q -- '--paths' || fail "an unboarded item with no --paths must fail and name --paths: $out"

echo "ok: a claim can be made against the files a slice will touch"