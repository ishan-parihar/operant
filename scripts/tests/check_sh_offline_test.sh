#!/usr/bin/env bash
# scripts/tests/check_sh_offline_test.sh — fake-cargo test for scripts/check.sh (iter-2642).
#
# Asserts, without touching the network or the real cargo:
#   1. `check.sh ... --offline` does NOT run `update -p memory-wire` (the sync)
#   2. the same invocation DOES forward `--offline` to cargo
#   3. without `--offline`, the sync still runs (the flag is what turns it off)
#
# Why this exists: check.sh called `build.sh --sync-only` with no offline
# channel, so `scripts/test-compile-gate.sh`'s unconditional `--offline` ran a
# `cargo update -p memory-wire` on every "offline" gate: network access and a
# rewrite of the tracked Cargo.lock under concurrent turns' in-flight checks.
# build_sh_test.sh proves build.sh honours --offline when handed it directly;
# nothing drove check.sh, so the seam between them was untested.
set -euo pipefail

SCRIPTS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

mkdir -p "$TMP/bin" "$TMP/home"
cat > "$TMP/bin/cargo" <<EOF
#!/usr/bin/env bash
if [ "\$1" = "--version" ]; then echo "cargo 0.0.0-fake"; exit 0; fi
echo "\$*" >> "$TMP/log"
exit 0
EOF
chmod +x "$TMP/bin/cargo"

run_check_sh() {
    HOME="$TMP/home" PATH="$TMP/bin:/usr/bin:/bin" CARGO_BIN="$TMP/bin/cargo" \
        bash "$SCRIPTS_DIR/check.sh" "$@"
}

# --- 1 + 2. --offline skips the sync, still reaches cargo -------------------
rm -f "$TMP/log"
run_check_sh check -p operant-core --offline >/dev/null 2>&1 || {
    echo "FAIL: check.sh --offline exited non-zero"; exit 1; }
if grep -q '^update -p memory-wire$' "$TMP/log"; then
    echo "FAIL: check.sh --offline still ran the memory-wire sync"; exit 1
fi
grep -qE '^check .*--offline' "$TMP/log" || {
    echo "FAIL: check.sh --offline did not forward the flag to cargo"; exit 1; }
echo "PASS 1: --offline skips the sync and still reaches cargo"

# --- 3. without --offline the sync runs (mutation of the guard is visible) --
rm -f "$TMP/log"
run_check_sh check -p operant-core >/dev/null 2>&1 || {
    echo "FAIL: check.sh (online) exited non-zero"; exit 1; }
grep -q '^update -p memory-wire$' "$TMP/log" || {
    echo "FAIL: check.sh without --offline did not sync"; exit 1; }
echo "PASS 3: the sync still runs without --offline"

echo "check_sh_offline_test: all 3 assertions passed"
