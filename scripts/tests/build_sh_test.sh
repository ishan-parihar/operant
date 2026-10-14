#!/usr/bin/env bash
# scripts/tests/build_sh_test.sh — fake-cargo test for scripts/build.sh (plan 11A).
#
# Asserts, without touching the network or the real cargo:
#   1. `update -p memory-wire` precedes `build` in the cargo invocation order
#   2. --offline skips the sync entirely
#   3. MEMORY_WIRE_NO_SYNC=1 skips the sync entirely
#   4. a failed sync exits 0 (warning) without --strict
#   5. a failed sync exits 1 with --strict
#
# Fake HOME so dev-env.sh finds no real cargo/cargo-env to prepend over the
# shim (its PATH additions are conditioned on directory existence).
set -euo pipefail

SCRIPTS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

mkdir -p "$TMP/bin" "$TMP/home"
cat > "$TMP/bin/cargo" <<EOF
#!/usr/bin/env bash
echo "\$*" >> "$TMP/log"
if [ "\$1" = "\${FAKE_CARGO_FAIL_ON:-__none__}" ]; then
    exit 1
fi
exit 0
EOF
chmod +x "$TMP/bin/cargo"

run_build_sh() {
    HOME="$TMP/home" PATH="$TMP/bin:/usr/bin:/bin" bash "$SCRIPTS_DIR/build.sh" "$@"
}

# --- 1. sync precedes build -------------------------------------------------
rm -f "$TMP/log"
run_build_sh >/dev/null 2>&1
first_update="$(grep -n '^update -p memory-wire$' "$TMP/log" | head -1 | cut -d: -f1)"
first_build="$(grep -n '^build' "$TMP/log" | head -1 | cut -d: -f1)"
[ -n "$first_update" ] || { echo "FAIL: no update -p memory-wire call"; exit 1; }
[ -n "$first_build" ] || { echo "FAIL: no build call"; exit 1; }
[ "$first_update" -lt "$first_build" ] || { echo "FAIL: build ran before sync"; exit 1; }
echo "PASS 1: update -p memory-wire precedes build"

# --- 2. --offline skips sync ------------------------------------------------
rm -f "$TMP/log"
run_build_sh --offline >/dev/null 2>&1
if grep -q '^update -p memory-wire$' "$TMP/log"; then
    echo "FAIL: --offline still ran the sync"; exit 1
fi
grep -q '^build' "$TMP/log" || { echo "FAIL: --offline did not build"; exit 1; }
echo "PASS 2: --offline skips the sync, still builds"

# --- 3. MEMORY_WIRE_NO_SYNC=1 skips sync -----------------------------------
rm -f "$TMP/log"
HOME="$TMP/home" MEMORY_WIRE_NO_SYNC=1 PATH="$TMP/bin:/usr/bin:/bin" \
    bash "$SCRIPTS_DIR/build.sh" >/dev/null 2>&1
if grep -q '^update -p memory-wire$' "$TMP/log"; then
    echo "FAIL: MEMORY_WIRE_NO_SYNC=1 still ran the sync"; exit 1
fi
echo "PASS 3: MEMORY_WIRE_NO_SYNC=1 skips the sync"

# --- 4. failed sync, no --strict: warn and continue -------------------------
rm -f "$TMP/log"
FAKE_CARGO_FAIL_ON=update run_build_sh >/dev/null 2>"$TMP/err"
[ -f "$TMP/err" ] && grep -q "WARNING: memory-wire sync failed" "$TMP/err" \
    || { echo "FAIL: no loud warning on failed sync"; exit 1; }
grep -q '^build' "$TMP/log" || { echo "FAIL: build skipped after failed sync"; exit 1; }
echo "PASS 4: failed sync warns loudly and builds anyway (exit 0)"

# --- 5. failed sync with --strict: fail -------------------------------------
FAKE_CARGO_FAIL_ON=update run_build_sh --strict >/dev/null 2>&1 \
    && { echo "FAIL: --strict did not fail on sync failure"; exit 1; }
echo "PASS 5: --strict fails on sync failure"

echo "build_sh_test: all 5 assertions passed"
