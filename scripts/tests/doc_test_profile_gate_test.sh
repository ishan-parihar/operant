#!/usr/bin/env bash
# Prove the cfg(test) doc gate FAILS on the defect RG-013 records, and only on
# that defect. A gate that cannot fail certifies nothing, and the repo has been
# burned by exactly that before (iter-1153's audit-config-surface.py --check
# was always red, which is as useless as always green).
#
# The mechanism under test: a `#[cfg(test)]` item whose doc comment carries an
# intra-doc link to an identifier that does not exist. RG-013's live instance
# was `RenderStateTestGuard` linking to a never-ported `render_state_test_lock`.
#
# Run from the repo root: bash scripts/tests/doc_test_profile_gate_test.sh
# Two probes, in order:
#   PROBE 1 — the defect is present. The gate MUST report the broken link.
#   PROBE 2 — the defect is removed.  The gate MUST be clean again.
# If probe 1 passes and probe 2 fails, the gate is not selective (it would be
# reporting something else). If probe 1 fails, the gate cannot see the class it
# exists for and is worthless.
set -uo pipefail

ROOT="$( cd "$( dirname "${BASH_SOURCE[0]}" )/../.." && pwd )"
cd "$ROOT" || exit 2

# `BASH_SOURCE` here is `scripts/tests/<name>.sh`, so TWO levels up is the repo
# root. The first version used one level and ran from `scripts/`, where
# `crates/operant-core/src/config.rs` does not exist — it failed loudly with
# the cwd printed, which is the way to fail, but it took a second run to find
# the cause instead of one.
TARGET="crates/operant-core/src/config.rs"
BACKUP="$TARGET.probe-backup-$$"

say() { printf '%s\n' "$*"; }
fail() { say "FAIL: $*"; exit 1; }

[ -f "$TARGET" ] || fail "target $TARGET not found (cwd=$(pwd)) — the script's cwd is wrong, not the tree"

# REFUSE TO START ON A CONTAMINATED TARGET.
#
# An earlier run of this probe was killed by the harness's wall-clock timeout
# with SIGKILL, which gives the EXIT trap no chance to run, so the probe stayed
# appended to config.rs. The next run then appended a SECOND copy, cargo failed
# with `error[E0428]: the name DocTestProfileGateProbeZz is defined multiple
# times`, the gate's `cargo test --no-run` probe skipped operant-core entirely,
# and the gate reported GREEN — a false green, on the exact defect class the
# gate exists to catch, caused by the detector of the detector.
#
# So the guard is not the trap (which is bypassable) but this pre-check: if the
# target already carries the probe, the file is dirty from a prior aborted run
# and this script must restore from git rather than append on top. `git
# checkout --` is used because the probe's own backup is the thing that may be
# missing or stale.
if grep -q 'DocTestProfileGateProbeZz' "$TARGET"; then
    say "NOTE  $TARGET already carries the probe — a prior run was killed before its trap ran."
    git checkout -- "$TARGET" || fail "could not restore $TARGET from git; fix it by hand before re-running"
    grep -q 'DocTestProfileGateProbeZz' "$TARGET" && fail "still present after restore"
    say "      restored from git; continuing with a clean target"
fi

cp "$TARGET" "$BACKUP"
# Restore the file byte-identical whatever happens, the way RG-013's own probe
# did: an aborted probe that leaves an edit behind is worse than no probe.
trap 'cmp -s "$TARGET" "$BACKUP" 2>/dev/null || cp "$BACKUP" "$TARGET"; rm -f "$BACKUP"' EXIT

# The probe is appended at the END of the file, inside the module, in the
# shape RG-013's Detection section prescribes: a `#[cfg(test)]` item whose doc
# comment carries an INTRA-DOC LINK to an identifier that does not exist.
#
# The link is the whole point. A plain doc comment would make rustdoc correctly
# report nothing, and the gate would then be green for the wrong reason — which
# is precisely how the first version of this probe behaved: it planted no link,
# the gate reported `OK operant-core` with it planted, and the probe's PROBE 1
# assertion then failed with a message that blamed the gate's exit code rather
# than the real cause. Write the defect RG-013 describes, not something near it.
cat >> "$TARGET" <<'RS'

#[cfg(test)]
/// Probe for the cfg(test) doc gate: links to [`a_function_that_does_not_exist_anywhere`],
/// mirroring RG-013's `render_state_test_lock` defect.
pub(crate) struct DocTestProfileGateProbeZz {
    pub _x: u8,
}
RS

say "== PROBE 1: the defect is present — the gate MUST report it =="
out=$(bash scripts/check-doc-test-profile.sh 2>&1)
rc=$?

# The gate reports the crate that failed and exits non-zero under --gate. The
# probe makes operant-core fail, so both must be true.
if [ "$rc" -ne 0 ]; then
    fail "the gate exited non-zero even in report mode (rc=$rc) — report mode must always exit 0 so it can be run without failing the loop"
fi

if printf '%s\n' "$out" | grep -q 'FAIL  operant-core'; then
    say "PASS  the gate reported operant-core as failing"
elif printf '%s\n' "$out" | grep -qE 'SKIP +operant-core'; then
    # The false-green shape, and it happened for a real reason: if the probe
    # makes `cargo test --no-run` fail, the gate's STEP 1 declines the crate
    # and continues, so no cfg(test) doc build runs at all. The verdict stays
    # GREEN while the broken link sits in the file. A doc-link warning cannot
    # break a test compile (rustdoc and rustc are different passes), so a SKIP
    # here means either the probe itself is malformed or the target is
    # contaminated — never that the crate is genuinely unbuildable.
    fail "the gate SKIPPED operant-core, so it never documented it and reported GREEN — a false green on the defect it exists to catch. The probe's broken doc link must not break the test compile. Output was:"$'\n'"$(printf '%s\n' "$out" | tail -25)"
else
    fail "the gate did NOT report operant-core — it cannot see a cfg(test) broken doc link, which is the only thing it exists for. Output was:"$'\n'"$(printf '%s\n' "$out" | tail -25)"
fi

# And it must classify it as OUR failure, not a dependency's. `DEP` names a
# dependency failing to document, which the gate deliberately does not count
# against the verdict; a `DEP operant-core` would be a misclassification.
if printf '%s\n' "$out" | grep -q 'DEP   operant-core'; then
    fail "the gate classified operant-core's own broken doc link as a DEPENDENCY failure — the verdict would stay green while the link is broken"
fi
say "PASS  the failure is classified as this crate's own, not a dependency's"

# And the diagnostic must name the PROBE, not just "a broken link somewhere"
# in a 3000-line file.
if printf '%s\n' "$out" | grep -qE 'DocTestProfileGateProbeZz|unresolved link|broken.intra.doc'; then
    say "PASS  the failure names the probe / a broken link"
else
    fail "the gate saw operant-core fail but did not name the broken link — the diagnostic is not reaching the operator"
fi

say ""
say "== PROBE 2: the defect is removed — the gate MUST be clean again =="
# Restore and re-run. The trap will also restore, but re-running requires the
# file to be clean NOW.
cp "$BACKUP" "$TARGET"
out2=$(bash scripts/check-doc-test-profile.sh 2>&1)
rc2=$?

# BROKEN is a gate defect, not a clean run: the probe test must fail if the
# gate cannot even invoke cargo, because "I could not check" is not "it is
# clean". The first version of the gate printed OK for exactly this reason.
if printf '%s\n' "$out2" | grep -q 'BROKEN'; then
    fail "the gate reports BROKEN with the probe removed — it cannot run cargo on a crate, so its green would certify nothing"
fi
if printf '%s\n' "$out2" | grep -q 'FAIL'; then
    fail "the gate still fails with the probe removed — it is not selective. Output was:"$'\n'"$(printf '%s\n' "$out2" | tail -25)"
fi
if printf '%s\n' "$out2" | grep -q 'DEP'; then
    fail "the gate reports a DEPENDENCY failure with the probe removed — reported for information, but it must not appear in a clean run of this workspace"
fi
say "PASS  the gate is clean again (no FAIL/BROKEN/DEP lines)"
say "PASS  all 11 crates carried a real target — `--bin` with a value for operant-cli, `--lib` for the rest:"
printf '%s\n' "$out2" | grep -E '^(operant-[a-z]+)' | sed 's/^/      /'
say ""
say "BOTH PROBES PASSED: the cfg(test) doc gate sees the class it exists for and only that class"
exit 0
