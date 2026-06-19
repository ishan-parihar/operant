#!/bin/bash
# Self-test script for hermes-rs
# Usage: ./scripts/self-test.sh
set -e

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

# 1. Build
run_test "Build release binary" "cargo build --release 2>&1"

# 2. Tests
run_test "Run workspace tests" "cargo test --workspace 2>&1"

# 3. Clippy
run_test "Run clippy" "cargo clippy --workspace --all-targets --all-features 2>&1 | grep -v '^warning' | grep -v '^\s' | grep -v '^$'"

# 4. Formatting
run_test "Check formatting" "cargo fmt --all 2>&1"

# 5. CLI version
run_test "CLI version" "./target/release/hermes --version 2>&1"

# 6. CLI help
run_test "CLI help" "./target/release/hermes --help 2>&1"

# 7. CLI chat help
run_test "CLI chat help" "./target/release/hermes chat --help 2>&1"

# 8. CLI run help
run_test "CLI run help" "./target/release/hermes run --help 2>&1"

# 9. CLI dashboard help
run_test "CLI dashboard help" "./target/release/hermes dashboard --help 2>&1"

# 10. Quick run test
run_test "Quick run test" "./target/release/hermes run --query 'Hello' --max-iterations 1 2>&1"

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
