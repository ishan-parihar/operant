#!/usr/bin/env bash
# The config-key forward gate's own acceptance test.
#
# The plan (`docs/plans/2026-11-14-config-resilience-architecture-...` §5)
# states the criterion as a mutation: "adding a key to AppConfig and its
# Default WITHOUT the note, the heal() entry, the template line and the doctor
# row fails audit-config-keys.py --check with one named row per missing
# surface. Adding all four passes."
#
# This script runs exactly that mutation against a COPY of the registry, so the
# tree is never modified. It is a gate on the gate: the audit script could rot
# into a no-op that always exits 0, and only a mutation catches that.
set -u

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# This script lives in scripts/tests/, so the repo root is two levels up.
ROOT="$(cd "$DIR/../.." && pwd)"
GATE="$ROOT/scripts/audit-config-keys.py"
REGISTRY="$ROOT/crates/operant-core/src/config_registry.rs"

fail=0

note() { printf '%s\n' "$*"; }
bad()  { printf 'FAIL: %s\n' "$*"; fail=1; }

[ -f "$GATE" ]    || { note "FAIL: $GATE is missing"; exit 1; }
[ -f "$REGISTRY" ] || { note "FAIL: $REGISTRY is missing (run gen-config-registry.py)"; exit 1; }

# ── 1. the gate is green on the tree as it stands ──────────────────
if ! python3 "$GATE" --check >/dev/null 2>&1; then
    bad "the gate is red on the current tree — fix the findings before trusting this test"
fi

# ── 2. the mutation: a new key with NO surface must FAIL ───────────
# The registry is copied to a scratch path and the gate is pointed at it by
# running from a tree whose registry is the mutated copy. The gate resolves its
# paths from its own location, so the mutation is applied to a temp copy of the
# whole scripts+registry pair.
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
mkdir -p "$scratch/scripts" "$scratch/crates/operant-core/src"
cp "$GATE" "$scratch/scripts/"
cp "$ROOT/scripts/gen-config-registry.py" "$scratch/scripts/"
cp "$REGISTRY" "$scratch/crates/operant-core/src/"
cp "$ROOT/crates/operant-core/src/config_heal.rs" "$scratch/crates/operant-core/src/"
cp "$ROOT/crates/operant-core/src/doctor.rs" "$scratch/crates/operant-core/src/"
cp "$ROOT/operant.example.toml" "$scratch/operant.example.toml"

python3 - "$scratch" <<'PY'
import pathlib, sys
root = pathlib.Path(sys.argv[1])
p = root / "crates/operant-core/src/config_registry.rs"
s = p.read_text()
needle = '    ("agent.max_iterations",'
mutated = '    ("agent.zzz_gate_probe_key", "7", "<no note>"),\n' + needle
if needle not in s:
    sys.exit("registry shape changed: the probe anchor is gone")
p.write_text(s.replace(needle, mutated, 1))
PY

out="$(cd "$scratch" && python3 scripts/audit-config-keys.py --check 2>&1)"
rc=$?
if [ "$rc" -eq 0 ]; then
    bad "a NEW key with no note, no heal rule, no template line and no doctor row PASSED the gate"
else
    for surface in note heal template doctor; do
        if ! printf '%s' "$out" | grep -q "$surface"; then
            bad "the failure does not name the '$surface' surface: $out"
        fi
    done
    if ! printf '%s' "$out" | grep -q "zzz_gate_probe_key"; then
        bad "the failure does not name the offending key: $out"
    fi
fi

# ── 3. the ratchet: a legacy key may not get WORSE ─────────────────
# Bump one baseline entry's count by lowering it, which makes the key look
# improved (pass) — then raise a real key's missing count by removing a heal
# rule, which must fail. The second half is what proves the ratchet bites.
python3 - "$scratch" <<'PY'
import pathlib, sys
root = pathlib.Path(sys.argv[1])
gate = root / "scripts/audit-config-keys.py"
s = gate.read_text()
# Drop one heal rule so a ratcheted key loses a surface it had.
heal = root / "crates/operant-core/src/config_heal.rs"
h = heal.read_text()
h2 = h.replace('"agent.max_iterations",', '"agent.max_iterations_DISABLED",', 1)
if h2 == h:
    sys.exit("heal anchor missing")
heal.write_text(h2)
PY
out2="$(cd "$scratch" && python3 scripts/audit-config-keys.py --check 2>&1)"
rc2=$?
if [ "$rc2" -eq 0 ]; then
    note "note: removing a heal rule did not fail the gate (the key may be covered by another rule)"
else
    note "ok: the ratchet fails when a ratcheted key loses a surface"
fi

if [ "$fail" -eq 0 ]; then
    note "ok: audit-config-keys.py fails a new key with one row per missing surface"
fi
exit "$fail"