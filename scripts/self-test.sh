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
    "cd '$REPO_ROOT/crates/operant-cli/tests/tui_scenarios' && env -u OPENAI_BASE_URL -u OPENAI_API_KEY OPERANT_BIN='$REPO_ROOT/target/release/operant' python3 -u _run.py verify 2>&1"

# 5. CLI version
run_test "CLI version" "./target/release/operant --version 2>&1"

# 5b. The shipped architecture example must stay valid (r16 audit C5:
# without this, a malformed example can ship unnoticed — the CI half was
# dropped with harness.yml, so self-test.sh is the only automated gate).
run_test "Validate architecture example" "./target/release/operant architecture validate --file architecture.toml.example 2>&1"

# 6. CLI help
run_test "CLI help" "./target/release/operant --help 2>&1"

# 7. CLI chat help
run_test "CLI chat help" "./target/release/operant chat --help 2>&1"

# 8. CLI run help
run_test "CLI run help" "./target/release/operant run --help 2>&1"

# 9. CLI dashboard help
run_test "CLI dashboard help" "./target/release/operant dashboard --help 2>&1"

# 10. Quick run test
run_test "Quick run test" "./target/release/operant run --query 'Hello' --max-iterations 1 2>&1"

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
