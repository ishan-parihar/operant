#!/usr/bin/env bash
# scripts/tests/genome_tombstone_lint_test.sh — the root tombstone's contract.
#
# The defect this pins: `genome archive` moves a file into docs/archive/ and
# writes a pointer stub at the source's ORIGINAL path so the old path stays
# findable. When the source lived at the repo ROOT, that pointer is itself an
# untracked .md at the root, and lint_root_placement flagged it E306 "no role
# points here" — a false positive on the tool's own artefact. There was no way
# to silence it except committing the pointer into docs/archive/, i.e.
# archiving the pointer to the archive. Observed 2026-10-30: archiving
# PLAN_SESSION_PICKER_UPGRADE.md left a permanent E306 on its own tombstone.
#
# Asserted here, against a toy tree in /tmp (the real repo is only READ):
#   1. a root tombstone pointer is NOT reported misplaced (E306)
#   2. an ordinary root .md with no role STILL is — the check was weakened
#      only for tombstones, not disabled
#   3. is_tombstone classifies pointers, junk and missing paths correctly
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
GENOME="$REPO/scripts/genome"
TOY="/tmp/genome-tombstone-toy.$$"
trap 'rm -rf "$TOY"' EXIT

fail() { echo "FAIL: $*"; exit 1; }

# ── a toy tree: minimal genome.toml, the tombstone, and a junk doc ─────────
mkdir -p "$TOY/docs/plans" "$TOY/docs/archive" "$TOY/docs/audit"
cat > "$TOY/genome.toml" <<'TOML'
[paths]
agents = "AGENTS.md"
state = "docs/STATE.md"
archive = "docs/archive"
plans = "docs/plans"
TOML

cat > "$TOY/PLAN_SESSION_PICKER_UPGRADE.md" <<'MD'
# PLAN_SESSION_PICKER_UPGRADE (superseded)

<!-- kb: tombstone target="docs/archive/PLAN_SESSION_PICKER_UPGRADE.md" -->

Moved to `docs/archive/PLAN_SESSION_PICKER_UPGRADE.md` on 2026-10-30.

**Reason:** superseded by the new plan.
MD

printf '# temporary junk doc\n\nno role points here\n' > "$TOY/SCRATCH-ROLE-TEST.md"

# misplaced_paths enumerates via `git ls-files -co`, which is EMPTY outside a
# git repo — without this the lint silently checks nothing and every assertion
# below would pass vacuously.
git -C "$TOY" init -q .

lint_toy() { (cd "$TOY" && python3 "$GENOME" lint 2>&1 || true); }

# ── 1 + 2: the tombstone is exempt, the junk doc is not ────────────────────
OUT="$(lint_toy)"
echo "$OUT" | grep -q "PLAN_SESSION_PICKER_UPGRADE.md" \
    && fail "1: the root tombstone pointer was reported misplaced (E306)" \
    || echo "ok 1: a root tombstone pointer is not misplaced"
echo "$OUT" | grep -q "SCRATCH-ROLE-TEST.md" \
    && echo "ok 2: an ordinary root .md with no role is still reported" \
    || fail "2: the root-placement check no longer catches a real misplaced doc"

# ── 3: is_tombstone is honest about paths it cannot read ───────────────────
python3 - "$TOY" "$GENOME" <<'PY' || exit 1
import importlib.machinery, importlib.util, sys
from pathlib import Path
root = Path(sys.argv[1])
genome_src = Path(sys.argv[2])
loader = importlib.machinery.SourceFileLoader("genome", str(genome_src))
spec = importlib.util.spec_from_loader("genome", loader)
mod = importlib.util.module_from_spec(spec)
loader.exec_module(mod)
assert mod.is_tombstone(root / "PLAN_SESSION_PICKER_UPGRADE.md"), \
    "3a: a pointer carrying the tombstone comment IS a tombstone"
assert not mod.is_tombstone(root / "SCRATCH-ROLE-TEST.md"), \
    "3b: an ordinary doc is NOT a tombstone"
assert not mod.is_tombstone(root / "does-not-exist.md"), \
    "3c: a missing path is NOT a tombstone (no crash, no false positive)"
print("ok 3: is_tombstone classifies pointers, junk and missing paths")
PY

echo "PASS genome_tombstone_lint_test"
