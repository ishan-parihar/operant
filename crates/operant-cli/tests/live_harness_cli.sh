#!/usr/bin/env bash
# Live-loop end-to-end test for the harness CLI (plan 016).
#
# This is a shell-driven test (not a Rust #[test]) so it can drive the
# actual `operant` binary, write real fixture files, and assert stdout /
# stderr / exit codes. Run with:
#
#   bash crates/operant-cli/tests/live_harness_cli.sh
#
# Pre-req: `cargo build -p operant-cli --no-default-features --bin operant`
# has run and TARGET points at the resulting binary.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
BIN="${TARGET:-${REPO_ROOT}/target/debug/operant}"
TMPDIR="${TMPDIR:-/tmp}"
WORK="$(mktemp -d -t harness-cli-XXXXXX)"
trap "rm -rf $WORK" EXIT

pass=0
fail=0
report() {
    local name="$1" actual="$2" expected="$3"
    if [[ "$actual" == "$expected" ]]; then
        echo "PASS $name"
        pass=$((pass+1))
    else
        echo "FAIL $name"
        echo "  expected: $expected"
        echo "  actual:   $actual"
        fail=$((fail+1))
    fi
}

contains() {
    local name="$1" haystack="$2" needle="$3"
    if [[ "$haystack" == *"$needle"* ]]; then
        echo "PASS $name (contains '$needle')"
        pass=$((pass+1))
    else
        echo "FAIL $name (missing '$needle' in output)"
        echo "  ---haystack---"
        echo "$haystack"
        echo "  ---------------"
        fail=$((fail+1))
    fi
}

# ── fixture: architecture.toml with one wasm and one native row ────────
ARCH="$WORK/architecture.toml"
cat > "$ARCH" <<'EOF'
[[rows]]
id = "core-cli"
source = "native"
disabled = false
[rows.config]
version = "0.1.0"

[[rows]]
id = "kernel-toolset"
source = "native"
disabled = false
[rows.config]
max_tools = 12
EOF

# ── fixture: malformed architecture (empty id) ─────────────────────────
BAD="$WORK/bad.toml"
cat > "$BAD" <<'EOF'
[[rows]]
id = ""
source = "native"
EOF

# ── fixture: a pool yaml (read-only) ────────────────────────────────────
POOL="$WORK/relationship_intel_pool.yaml"
cat > "$POOL" <<'EOF'
name: relationship-intel
services_offered:
  - name: query.contacts
    description: Find contacts by name/email
  - name: search.history
    description: Search communication history
services_consumed: []
pooled_sub_systems:
  - name: contacts
    path: ~/.hermes/systems/relationship-intel/contacts
  - name: history
    path: ~/.hermes/systems/relationship-intel/history
EOF

# ── fixture: a pool yaml attempting a write verb (must reject) ─────────
POOL_WRITE="$WORK/write_pool.yaml"
cat > "$POOL_WRITE" <<'EOF'
name: relationship-intel-write
services_offered:
  - name: delete.contacts
    description: Delete a contact (forbidden in v1)
services_consumed: []
pooled_sub_systems: []
EOF

# ── test 1: architecture dump emits canonical JSON (with --json) ──
echo "--- test 1: dump ---"
DUMP_OUT=$("$BIN" architecture dump --file "$ARCH" --json 2>&1) || {
    echo "FAIL test 1: dump command itself failed"; fail=$((fail+1)); dump_rc=1
}
dump_rc=$? || true
if [[ "$DUMP_OUT" == *'"rows"'* && "$DUMP_OUT" == *'core-cli'* && "$DUMP_OUT" == *'kernel-toolset'* ]]; then
    echo "PASS test 1: dump --json includes both rows"
    pass=$((pass+1))
else
    echo "FAIL test 1: dump --json missing expected content"
    echo "$DUMP_OUT"
    fail=$((fail+1))
fi
# Also check text mode (no --json) still lists both rows.
DUMP_TEXT=$("$BIN" architecture dump --file "$ARCH" 2>&1)
if [[ "$DUMP_TEXT" == *'core-cli'* && "$DUMP_TEXT" == *'kernel-toolset'* ]]; then
    echo "PASS test 1b: dump (text) includes both rows"
    pass=$((pass+1))
else
    echo "FAIL test 1b: dump (text) missing expected content"
    echo "$DUMP_TEXT"
    fail=$((fail+1))
fi

# ── test 2: architecture validate succeeds on a clean file ───────────
echo "--- test 2: validate ok ---"
set +e
"$BIN" architecture validate --file "$ARCH" >/dev/null 2>"$WORK/validate-stderr"
rc=$?
set -e
report "test 2: validate exits 0" "$rc" "0"

# ── test 3: architecture validate fails on empty id ──────────────────
echo "--- test 3: validate rejects bad input ---"
set +e
"$BIN" architecture validate --file "$BAD" >/dev/null 2>"$WORK/validate-bad-stderr"
rc=$?
set -e
report "test 3: validate exits non-zero" "$rc" "1"
contains "test 3b: error mentions empty id" "$(cat "$WORK/validate-bad-stderr")" "empty id"

# ── test 4: dump of a missing file errors clearly ────────────────────
echo "--- test 4: missing file ---"
set +e
"$BIN" architecture dump --file "$WORK/nonexistent.toml" >/dev/null 2>"$WORK/missing-stderr"
rc=$?
set -e
report "test 4: missing file exits 1" "$rc" "1"

# ── test 5: pool-import compiles a read-only pool ────────────────────
echo "--- test 5: pool-import read-only ---"
set +e
POOL_OUT=$("$BIN" architecture pool-import --path "$POOL" 2>&1)
rc=$?
set -e
report "test 5: pool-import exits 0" "$rc" "0"
if [[ "$POOL_OUT" == *'relationship-intel'* ]]; then
    echo "PASS test 5b: pool-import output mentions pool name"
    pass=$((pass+1))
else
    echo "FAIL test 5b: pool-import output missing pool name"
    echo "$POOL_OUT"
    fail=$((fail+1))
fi
# Also check --json mode.
set +e
POOL_JSON=$("$BIN" architecture pool-import --path "$POOL" --json 2>&1)
rc=$?
set -e
if [[ "$rc" == "0" && "$POOL_JSON" == *'"family_row"'* ]]; then
    echo "PASS test 5c: pool-import --json includes family_row"
    pass=$((pass+1))
else
    echo "FAIL test 5c: pool-import --json missing family_row"
    echo "$POOL_JSON"
    fail=$((fail+1))
fi

# ── test 6: pool-import refuses write verbs ─────────────────────────
echo "--- test 6: pool-import rejects write ---"
set +e
"$BIN" architecture pool-import --path "$POOL_WRITE" >/dev/null 2>"$WORK/pool-write-stderr"
rc=$?
set -e
if [[ "$rc" != "0" ]]; then
    echo "PASS test 6: pool-import exits non-zero on write verb"
    pass=$((pass+1))
else
    echo "FAIL test 6: pool-import accepted a write-verb pool"
    fail=$((fail+1))
fi

# ── summary ──────────────────────────────────────────────────────────
echo
echo "=== harness CLI live-loop summary ==="
echo "pass: $pass"
echo "fail: $fail"
[[ "$fail" -eq 0 ]] || exit 1
exit 0
