#!/usr/bin/env bash
# scripts/tests/audit_ledger_index_test.sh — the audit ledger's own gate.
#
# The defect this pins: the gate compared the index table and the checkboxes
# as SETS. A set has no duplicates, so the gate could not see an id used by
# two rows — and two agents on the shared branch had picked the same id EIGHT
# times in this ledger's life (five in the index table, eight among the
# detail sections), each collision recorded only where a reader happened to
# look. A lookup table whose ids are ambiguous is not a lookup table.
#
# Asserted here, against COPIES of the real ledger (the real one is never
# touched):
#   1. the published ledger passes
#   2. a DUPLICATE index id fails, and names the id
#   3. a DUPLICATE detail-section id fails, and names the id
#   4. an index row with NO detail section fails
#   5. the RG-016 checks still fire (an open checkbox with no index row)
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
GATE="$REPO_ROOT/scripts/audit-ledger-index.py"
LEDGER="$REPO_ROOT/docs/audit/2026-06-19-BUGS.md"
# A sandbox may point TMPDIR at a read-only area, so pick a writable base.
BASE="${TMPDIR:-/tmp}"
[ -d "$BASE" ] && [ -w "$BASE" ] || BASE=/tmp
TMP="$(mktemp -d -p "$BASE")"
trap 'rm -rf "$TMP"' EXIT

fail() { echo "FAIL: $*"; exit 1; }

# ── 1. the published ledger passes ──────────────────────────────────────────
python3 "$GATE" "$LEDGER" > "$TMP/clean.out" 2>&1 \
    || fail "the published ledger must pass the gate: $(cat "$TMP/clean.out")"
grep -q "the id space is unique" "$TMP/clean.out" \
    || fail "the gate must report the id-space check it now makes"

# ── 2. a DUPLICATE index id fails, and names the id ────────────────────────
# The first index row is duplicated under a second id, so the set difference
# cannot see it — only the uniqueness check can.
FIRST_ROW="$(grep -m1 '^| RT-' "$LEDGER")"
cp "$LEDGER" "$TMP/dupe-index.md"
python3 - "$TMP/dupe-index.md" "$FIRST_ROW" <<'EOF'
import pathlib, sys
path, row = pathlib.Path(sys.argv[1]), sys.argv[2]
text = path.read_text()
# Rename the FIRST index row's id to an id that already exists further down
# (RT-245 is published and unique), so two rows share it.
text = text.replace(row, row.replace("| RT-245 |", "| RT-245 |", 1), 1)
lines = text.splitlines()
for i, line in enumerate(lines):
    if line.startswith("| RT-245 |"):
        lines[i] = line.replace("| RT-245 |", "| RT-243 |", 1)
        break
path.write_text("\n".join(lines) + "\n")
EOF
out="$(python3 "$GATE" "$TMP/dupe-index.md" 2>&1)" && rc=0 || rc=$?
[ "$rc" -ne 0 ] || fail "a duplicate index id must fail the gate"
echo "$out" | grep -q "TWO index rows" \
    || fail "the gate must report a duplicate index id: $out"
echo "$out" | grep -q "RT-243" \
    || fail "the gate must NAME the duplicate id: $out"

# ── 3. a DUPLICATE detail-section id fails, and names the id ───────────────
cp "$LEDGER" "$TMP/dupe-detail.md"
python3 - "$TMP/dupe-detail.md" "$GATE" <<'EOF'
import importlib.util, pathlib, re, sys
path = pathlib.Path(sys.argv[1])
text = path.read_text()
# Duplicate one DETAIL heading's id: the second occurrence of an id becomes a
# copy of the first, so two sections share it.
lines = text.splitlines()
headings = [
    (i, re.match(r"^- \[x\] \*\*(RT-\d+)", line).group(1))
    for i, line in enumerate(lines)
    if re.match(r"^- \[x\] \*\*RT-\d+", line)
]
counts = {}
for _, ident in headings:
    counts[ident] = counts.get(ident, 0) + 1
# The gate's own published-exception list: a victim whose id is already
# exempt would have its new duplicate absorbed, and the test would prove
# nothing.
spec = importlib.util.spec_from_file_location("gate", sys.argv[2])
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)
exempt = gate.DUPLICATE_IDS
# A victim whose id is currently UNIQUE and NOT exempt.
victims = [
    (i, ident)
    for i, ident in headings
    if counts[ident] == 1 and ident not in exempt
]
if len(victims) < 2:
    raise SystemExit("the ledger needs two unique detail sections for this mutation")
first, victim = victims[0]
second = victims[1][0]
# Point the SECOND section at the FIRST section's id.
lines[second] = re.sub(r"^- \[x\] \*\*RT-\d+", f"- [x] **{victim}", lines[second])
path.write_text("\n".join(lines) + "\n")
EOF
out="$(python3 "$GATE" "$TMP/dupe-detail.md" 2>&1)" && rc=0 || rc=$?
[ "$rc" -ne 0 ] || fail "a duplicate detail-section id must fail the gate"
echo "$out" | grep -q "TWO detail sections" \
    || fail "the gate must report a duplicate detail-section id: $out"

# ── 4. an index row with NO detail section fails ───────────────────────────
cp "$LEDGER" "$TMP/no-detail.md"
python3 - "$TMP/no-detail.md" <<'EOF'
import pathlib, re, sys
path = pathlib.Path(sys.argv[1])
text = path.read_text()
# Remove the ONLY detail section of an id that IS in the index table, so the
# index row is left without the finding it claims. (An id with two sections
# would still have one left.)
index_ids = set(re.findall(r"^\|\s*(RT-\d+)\s*\|", text, re.M))
detail_ids = re.findall(r"^- \[[ x]\] \*\*(RT-\d+)", text, re.M)
counts = {}
for ident in detail_ids:
    counts[ident] = counts.get(ident, 0) + 1
lines = text.splitlines()
removed = False
for i, line in enumerate(lines):
    m = re.match(r"^- \[[ x]\] \*\*(RT-\d+)", line)
    if m and m.group(1) in index_ids and counts[m.group(1)] == 1 and not removed:
        del lines[i]
        removed = True
        break
assert removed, "no single-appearance detail section to remove"
path.write_text("\n".join(lines) + "\n")
EOF
out="$(python3 "$GATE" "$TMP/no-detail.md" 2>&1)" && rc=0 || rc=$?
[ "$rc" -ne 0 ] || fail "an index row with no detail section must fail the gate"
echo "$out" | grep -q "NO detail section" \
    || fail "the gate must report an index row with no detail section: $out"

# ── 5. the RG-016 checks still fire ────────────────────────────────────────
cp "$LEDGER" "$TMP/open-row.md"
python3 - "$TMP/open-row.md" <<'EOF'
import pathlib, sys
path = pathlib.Path(sys.argv[1])
text = path.read_text()
# Turn a CLOSED index row into a non-CLOSED one: the checkbox says closed,
# the table says open, and RG-016 must report the disagreement.
lines = text.splitlines()
for i, line in enumerate(lines):
    if line.startswith("| RT-245 |") and "CLOSED" in line:
        lines[i] = line.replace("**CLOSED", "**OPEN", 1)
        break
path.write_text("\n".join(lines) + "\n")
EOF
out="$(python3 "$GATE" "$TMP/open-row.md" 2>&1)" && rc=0 || rc=$?
[ "$rc" -ne 0 ] || fail "an index row that disagrees with its checkbox must fail the gate"
echo "$out" | grep -q "not CLOSED but closed in the checkboxes" \
    || fail "the RG-016 check must still fire: $out"

echo "PASS: the ledger gate catches a duplicate id, a detail-less row, and a state disagreement"
