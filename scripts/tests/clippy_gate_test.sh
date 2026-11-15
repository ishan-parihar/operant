#!/usr/bin/env bash
# scripts/tests/clippy_gate_test.sh — fake-cargo test for the gate's
# documented "Test targets are exempted" policy (iter-2444, RT-585).
#
# WHAT WAS BROKEN: AGENTS.md:809 promises test targets are exempt from
# -D clippy::unwrap_used / -D clippy::expect_used. Only the in-test-module half
# was true — `#![cfg_attr(test, allow(...))]` in lib.rs/main.rs reaches a
# `#[cfg(test)] mod tests`, NOT an integration target under `crates/*/tests/`,
# which is a SEPARATE crate. Those files assert with `.expect(..)` once per line
# and carried no suppression, so the gate denied 19 sites in
# crates/operant-core/tests/evidence_seams.rs alone and could not go green.
#
# The fix has two halves, and this test pins BOTH:
#   1. the lib/bin pass is the authority on "did it compile", so a denied
#      TEST target must not abort the run before its diagnostics are counted
#   2. the two denied lints on crates/*/tests/ are exempted in the jq filter,
#      but a test target still fails on any OTHER class (a real compile error,
#      an unwrap in a non-test helper, another lint)
#
# Fake cargo emits scripted JSON so nothing real is compiled and no network is
# touched.
set -euo pipefail

SCRIPTS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

mkdir -p "$TMP/bin" "$TMP/home"
cat > "$TMP/bin/cargo" <<EOF
#!/usr/bin/env bash
# Record the args, then emit whatever JSON the scenario file holds.
echo "\$*" >> "$TMP/log"
if [ "\$1" = "clippy" ]; then
    cat "$TMP/scenario.jsonl"
fi
exit 0
EOF
chmod +x "$TMP/bin/cargo"
printf 'fake-cargo 0.0.0\n' > "$TMP/bin/rustc"
cp "$TMP/bin/cargo" "$TMP/bin/rustc"

# A gate run in a scratch HOME: the allowlist it compares against is empty, so
# every real diagnostic shows up as "new" — exactly what we want to assert on.
mkdir -p "$TMP/gate/.ci"
: > "$TMP/gate/.ci/clippy-allowlist.txt"

run_gate() {
    # The gate derives ROOT_DIR from its own path and sources
    # scripts/cargo-bin.sh, so the scratch copy needs both next to it.
    mkdir -p "$TMP/gate/scripts"
    cp "$SCRIPTS_DIR/clippy-warning-gate.sh" "$TMP/gate/scripts/gate.sh"
    cp "$SCRIPTS_DIR/cargo-bin.sh" "$TMP/gate/scripts/cargo-bin.sh"
    HOME="$TMP/home" PATH="$TMP/bin:/usr/bin:/bin" \
        bash "$TMP/gate/scripts/gate.sh" 2>&1
}

# cargo's JSON shape, as the gate's jq reads it: a compiler-message record with
# a target name, a primary span, and a lint code.
diag() { # diag <target> <file> <lint> <level>
    printf '{"reason":"compiler-message","target":{"name":"%s"},"message":{"level":"%s","message":"%s","code":{"code":"%s"},"spans":[{"is_primary":true,"file_name":"%s"}]}}\n' \
        "$1" "$4" "$3" "$3" "$2"
}
ok_record() { printf '{"reason":"build-finished","success":true}\n'; }

# --- 1. a denied lint in crates/*/tests/ is EXEMPT ---------------------------
{
    ok_record
    diag lib crates/operant-core/tests/evidence_seams.rs clippy::expect_used error
} > "$TMP/scenario.jsonl"
out="$(run_gate || true)"
echo "$out" | grep -q 'clippy-gate] pass' \
    || { echo "FAIL: a test-target expect_used was not exempted"; echo "$out"; exit 1; }
echo "PASS 1: clippy::expect_used in crates/*/tests/ is exempt"

# --- 2. clippy::unwrap_used in a tests/ file is exempt TOO --------------------
{
    ok_record
    diag lib crates/operant-core/tests/evidence_seams.rs clippy::unwrap_used error
} > "$TMP/scenario.jsonl"
out="$(run_gate || true)"
echo "$out" | grep -q 'clippy-gate] pass' \
    || { echo "FAIL: a test-target unwrap_used was not exempted"; echo "$out"; exit 1; }
echo "PASS 2: clippy::unwrap_used in crates/*/tests/ is exempt"

# --- 3. the SAME lint in PRODUCTION code is NOT exempt -----------------------
# This is the half that keeps the gate honest: exempting test targets must not
# become exempting everything.
{
    ok_record
    diag lib crates/operant-core/src/agent/events.rs clippy::expect_used error
} > "$TMP/scenario.jsonl"
out="$(run_gate || true)"
echo "$out" | grep -q 'NEW warnings detected' \
    || { echo "FAIL: a lib expect_used was exempted as if it were a test"; echo "$out"; exit 1; }
echo "$out" | grep -q 'crates/operant-core/src/agent/events.rs' \
    || { echo "FAIL: the lib warning was reported but not named"; echo "$out"; exit 1; }
echo "PASS 3: the same lint in src/ still FAILS the gate"

# --- 4. a test target still fails on ANOTHER class ---------------------------
# The exemption is for the two denied lints on the test surface, not a blanket
# amnesty. A dead_code warning in a test file must still be reported.
{
    ok_record
    diag lib crates/operant-core/tests/evidence_seams.rs dead_code warning
} > "$TMP/scenario.jsonl"
out="$(run_gate || true)"
echo "$out" | grep -q 'dead_code|crates/operant-core/tests/evidence_seams.rs' \
    || { echo "FAIL: a dead_code in a test file was silently dropped"; echo "$out"; exit 1; }
echo "PASS 4: other lint classes in test files are still reported"

# --- 5. a denied lint on the boundary of the surface is still caught ----------
# `crates/x/tests2/y.rs` is not under a `tests/` directory; neither is a
# `tests.rs` module file inside src/. Only the real integration surface is
# exempt, and the match must not be a loose substring.
{
    ok_record
    diag lib crates/operant-core/src/tools/tests.rs clippy::expect_used error
    diag lib crates/operant-cli/tests_helpers.rs clippy::expect_used error
} > "$TMP/scenario.jsonl"
out="$(run_gate || true)"
echo "$out" | grep -q 'NEW warnings detected' \
    || { echo "FAIL: a lookalike path was exempted (tests.rs / tests_helpers.rs)"; echo "$out"; exit 1; }
echo "PASS 5: lookalike paths (src/**/tests.rs, tests_helpers.rs) are NOT exempt"

# --- 6. the all-targets pass cannot abort the run ----------------------------
# A denied test target makes that cargo pass exit 101. Before the fix the
# script read that as "zero diagnostics proves nothing was measured" and
# exited 1 before counting anything. Its build-finished record must be
# discarded so the lib/bin pass stays the only authority.
cat > "$TMP/bin/cargo" <<EOF
#!/usr/bin/env bash
echo "\$*" >> "$TMP/log"
if [ "\$1" = "clippy" ]; then
    case "\$*" in
        *--all-targets*)
            # The all-targets pass fails on an exempted test target.
            printf '{"reason":"build-finished","success":false}\n'
            printf '{"reason":"compiler-message","target":{"name":"lib"},"message":{"level":"error","message":"denied","code":{"code":"clippy::expect_used"},"spans":[{"is_primary":true,"file_name":"crates/operant-core/tests/evidence_seams.rs"}]}}\n'
            exit 101
            ;;
        *)
            cat "$TMP/scenario.jsonl"
            ;;
    esac
fi
exit 0
EOF
chmod +x "$TMP/bin/cargo"
cp "$TMP/bin/cargo" "$TMP/bin/rustc"
{ ok_record; } > "$TMP/scenario.jsonl"
out="$(run_gate || true)"
echo "$out" | grep -q 'clippy-gate] pass' \
    || { echo "FAIL: a failing all-targets pass still aborted the gate"; echo "$out"; exit 1; }
echo "PASS 6: a failing all-targets pass does not abort the run"

echo "all clippy-gate tests passed"