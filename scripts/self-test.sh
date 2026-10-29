#!/bin/bash
# Self-test script for operant
# Usage: ./scripts/self-test.sh
set -e

# The repo root, derived from this script's own location — the corpus step
# below `cd`s into a subdirectory, and a relative path would break there.
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# The background auto-update check must not reach GitHub from a test run.
# It shells out to `gh`, which resolves through the process's proxy
# variables, and it runs in the background — so it lands in whatever test
# is running at the time. (iter-1132)
export OPERANT_NO_AUTO_UPDATE=1

echo "=== Hermes-RS Self-Test ==="
echo ""

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Test counter
TESTS_PASSED=0
TESTS_FAILED=0

run_test() {
    local test_name="$1"
    local test_command="$2"
    
    echo -e "${YELLOW}Running: ${test_name}${NC}"
    if eval "$test_command"; then
        echo -e "${GREEN}✓ Passed: ${test_name}${NC}"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    else
        echo -e "${RED}✗ Failed: ${test_name}${NC}"
        TESTS_FAILED=$((TESTS_FAILED + 1))
    fi
    echo ""
}

# Clippy output filter.
#
# The filter below used to sit INSIDE the run_test command string:
#
#   ./scripts/check.sh clippy ... 2>&1 | grep -v '^warning' | grep -v ... 
#
# which reports the exit status of the LAST pipeline element — grep — not of
# cargo. grep exits 0 whenever it matched and removed at least one line, so a
# clippy run that failed hard still reported PASSED, and a run that failed
# silently with no `warning:`-prefixed lines at all reported FAILED. The verdict
# was essentially uncorrelated with the actual result.
#
# The fix is to keep the filter for display only and return cargo's own status.
# `out=$(cmd) && status=0 || status=$?` is deliberate: it is a `||` compound, so
# `set -e` does not abort on the failing command, and `$?` on the right-hand side
# is the status of the assignment — i.e. the status of the command substitution.
#
# Note this is an ERROR detector, not a lint gate: no `-D warnings` is added
# here on purpose. Raw clippy disagrees with .ci/clippy-allowlist.txt, so adding
# it would make "self-test passed" and "the gate passed" different statements
# (ci.yml says the same about its own clippy job). `scripts/clippy-warning-gate.sh`
# is the warning gate; this step catches builds clippy cannot complete.
run_clippy() {
    local out status
    out=$(./scripts/check.sh clippy --workspace --all-targets --all-features 2>&1) && status=0 || status=$?
    printf '%s\n' "$out" | grep -v '^warning' | grep -v '^[[:space:]]' | grep -v '^$'
    return "$status"
}

# 1. Build
run_test "Build release binary" "./scripts/check.sh build --release 2>&1"

# 2. Tests
run_test "Test targets compile (scripts/test-compile-gate.sh — the 2026-10-16 plan-02 guard)" "bash scripts/test-compile-gate.sh 2>&1"
run_test "Run workspace tests" "./scripts/check.sh test --workspace 2>&1"

# 3. Clippy
run_test "Run clippy" "run_clippy"

# 3b. Supply chain. `deny.toml` is the policy, but a policy nobody executes
# is a comment (RT-10). cargo-audit runs against the committed lockfile and
# fails on any advisory the committed allowlist does not cover. cargo-deny
# is not installed here, so the licence and duplicate-version policies stay
# unexecuted by design — see the header of deny.toml.
run_test "Supply-chain advisories (scripts/supply-chain-gate.sh)" "bash scripts/supply-chain-gate.sh 2>&1"

# 3c. The build scripts have their own regression tests: the libclang
# resolver (three scripts resolving libclang three different ways broke the
# build) and the supply-chain gate's own fail-on-new-advisory contract.
run_test "Build script tests" "bash scripts/tests/libclang_resolve_test.sh && bash scripts/tests/supply_chain_gate_test.sh 2>&1"

# 4. Formatting
run_test "Check formatting" "./scripts/check.sh fmt --all 2>&1"

# 4b. The code-surface claims in AGENTS.md, diffed against the code.
# This check existed and ran only when a human remembered to run it — which
# is how "Workspace members (12 crates)" survived the deletion of the
# `operant-macros` crate, and how the README kept advertising a grant ledger
# and a durable escalation queue that retired to Holon (RT-160). A check
# nobody runs is a comment, so it is a step of the gate now.
run_test "Doc claims (scripts/check-doc-claims.sh)" "bash scripts/check-doc-claims.sh 2>&1"

# 4c. The config settings that nothing reads. The script's `--check` used to
# exit non-zero whenever ANY candidate existed — i.e. always, for the 27
# settings RT-136 accounted for — so it could never be a gate. It now
# compares against a baseline and fails only on a setting that became unread
# without anyone deciding what to do about it (RT-159).
run_test "Config surface (scripts/audit-config-surface.py --check)" "python3 scripts/audit-config-surface.py --check 2>&1"

# 4c-1. The audit ledger's index vs its own checkboxes (RG-016). The ledger
# keeps each row's state in three places and closing a defect flips only the
# checkbox, so the index table and the count prose go stale in BOTH
# directions: a closed row still reading "OPEN" (RT-135 stayed an "accepted
# stub" for the whole life of that wording, until iter-1474 closed it and
# nothing re-derived the table) and an open row missing from the table
# entirely (RT-136/RT-190/RT-197 were all absent on 2026-10-29). A hand-read
# is what missed it; this is the set difference, run every time.
run_test "Ledger index vs checkboxes (scripts/audit-ledger-index.py)" "python3 scripts/audit-ledger-index.py 2>&1"

# 4c-2. The lint-suppression surface. rust-best-practices ch.2 says prefer
# `#[expect]` over `#[allow]`, because an allow never expires while an expect
# reports itself unfulfilled the moment the lint stops firing. A prose claim
# about that surface cannot stay true (RUST_BEST_PRACTICES_PLAN.md still
# reports 242 suppressions from a 2026-08-02 measurement; the live figure is
# 33), so the claim is a script with a reason-carrying baseline, gated here.
# `--check` fails on a suppression the baseline does not explain — a new
# escape hatch nobody decided on — and on a baseline entry with no reason.
# Mutation-checked: adding an unexplained `#[allow(dead_code)]` to
# operant-harness/src/row.rs makes it name the file and line and exit 1.
run_test "Lint suppressions (scripts/audit-lint-suppressions.py --check)" "python3 scripts/audit-lint-suppressions.py --check 2>&1"

# 4d. The rustdoc gate. `./scripts/check.sh doc` is the exact command the
# dead CI job ran (cargo doc --workspace --no-deps --all-features under
# -Dwarnings); since the Actions quota died, nothing ran it, and it had
# drifted RED with ~70 broken intra-doc links — a gate that was red and
# nobody could see it is how RT-166 happened (RT-166).
run_test "Rustdoc gate (scripts/check.sh doc)" "./scripts/check.sh doc 2>&1"

# 4e. The SECOND rustdoc profile — `cfg(test)` ON. The gate above cannot
# certify a `#[cfg(test)]` doc link at all (RG-013/RT-169): a binary target's
# doc build runs with cfg(test) OFF, so test-only scaffolding is absent from
# the AST rustdoc sees, and that is exactly where the jcode port's doc rot
# accumulated. iter-1176 measured why the obvious fix is not a flag tweak and
# closed it: `--cfg test` is not a cargo argument on cargo 1.98 (it must ride
# RUSTDOCFLAGS), an unscoped flag bleeds into every dependency and dies on
# memchr's quickcheck dev-deps (so it is one crate per invocation), and
# `--no-deps` makes the profile UNBUILDABLE rather than merely blind (it does
# not resolve [dev-dependencies], and operant-core/operant-cli both import
# one). All 11 crates GREEN.
#
# This step is LONG (it re-documents every crate, about 4 minutes) and it
# overlaps the gate above, so it is opt-in via OPERANT_DOC_TEST=1: the
# production profile runs on every self-test, and the test profile runs when
# the operator asks for it or before a push that touches test code.
if [ "${OPERANT_DOC_TEST:-0}" = "1" ]; then
  run_test "Rustdoc cfg(test) profile (scripts/check.sh doc-test)" "./scripts/check.sh doc-test 2>&1"
fi

# 3d. The TUI visual-regression corpus. This gate was RED for nine days
# and nobody noticed, because the corpus was not in ANY automated gate:
# `scripts/check.sh` forwards to cargo, and the corpus is a Python script.
#
# The corpus is a LONG run (about a minute for the full verify) and it
# needs a binary built from the SAME tree — a stale binary silently tests
# the wrong renderer. The release binary above is exactly that, so it is
# what the corpus gets. The two env vars are cleared because a scenario
# without an `agent_script` builds a real agent and would otherwise make
# live calls to the gateway (audit finding 16, hit twice).
#
# The five scenarios that could never pass are EXCLUDED with a stated
# reason in tests/tui_scenarios/excluded.json, so this gate is green.
# NEVER run `prove` at the same time as this: `prove` rewrites the
# goldens it proves, and a concurrent verify reads them mid-rewrite and
# reports dozens of phantom drifts.
run_test "TUI visual corpus (crates/operant-cli/tests/tui_scenarios/_run.py verify)" \
    "(cd '$REPO_ROOT/crates/operant-cli/tests/tui_scenarios' && env -u OPENAI_BASE_URL -u OPENAI_API_KEY OPERANT_BIN='$REPO_ROOT/target/release/operant' python3 -u _run.py verify 2>&1)"

# The corpus step above `cd`s into a subdirectory. It runs in a SUBSHELL, so
# the script's working directory stays at the repository root — otherwise
# every later step's relative path (`./target/release/operant`) resolves
# against the scenarios directory and fails with "No such file or directory".
# The binary is also named absolutely, so the steps do not depend on the
# caller's cwd at all.
BIN="$REPO_ROOT/target/release/operant"

# 5. CLI version
run_test "CLI version" "$BIN --version 2>&1"

# 5b. The shipped architecture example must stay valid (r16 audit C5:
# without this, a malformed example can ship unnoticed — the CI half was
# dropped with harness.yml, so self-test.sh is the only automated gate).
run_test "Validate architecture example" "$BIN architecture validate --file architecture.toml.example 2>&1"

# 6. CLI help
run_test "CLI help" "$BIN --help 2>&1"

# 7. CLI chat help
run_test "CLI chat help" "$BIN chat --help 2>&1"

# 8. CLI run help
run_test "CLI run help" "$BIN run --help 2>&1"

# 9. CLI dashboard help
run_test "CLI dashboard help" "$BIN dashboard --help 2>&1"

# 10. Quick run test
run_test "Quick run test" "$BIN run --query 'Hello' --max-iterations 1 2>&1"

# Summary
echo ""
echo "=== Test Summary ==="
echo -e "${GREEN}Passed: ${TESTS_PASSED}${NC}"
echo -e "${RED}Failed: ${TESTS_FAILED}${NC}"
echo ""

if [ $TESTS_FAILED -eq 0 ]; then
    echo -e "${GREEN}All tests passed!${NC}"
    exit 0
else
    echo -e "${RED}Some tests failed.${NC}"
    exit 1
fi
