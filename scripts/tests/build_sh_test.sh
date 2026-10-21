#!/usr/bin/env bash
# scripts/tests/build_sh_test.sh — fake-cargo test for scripts/build.sh (plan 11A).
#
# Asserts, without touching the network or the real cargo:
#   1. `update -p memory-wire` precedes `build` in the cargo invocation order
#   2. --offline skips the sync entirely
#   3. MEMORY_WIRE_NO_SYNC=1 skips the sync entirely
#   4. a failed sync exits 0 (warning) without --strict
#   5. a failed sync exits 1 with --strict
#   6. every build script resolves LOCAL_DIR to the REPO's own `local/`, and
#      none of them falls back to a machine-specific path
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

# --- 6. every build script resolves LOCAL_DIR to the REPO's own `local/` --
# One of them (check.sh) fell back to one developer's machine
# (`/home/z/my-project/local`). On every other box that path does not exist,
# so ORT_LIB_LOCATION went UNSET — the ONNX Runtime 1.24.4 the build needs
# lives in the repo's `local/` — and the gate's build linked the SYSTEM's
# 1.20.1 instead: the runtime mismatch CR-1/CR-6 are about, reached by a
# script that looked fine.
#
# The check is BEHAVIOURAL, not textual, because RT-167 changed how the
# scripts resolve it: `check.sh` no longer assigns LOCAL_DIR at all — it
# SOURCES dev-env.sh, which owns the default. A textual "every script has an
# assignment" check would now fail the one script that got the doctrine right
# (and a textual "no machine-specific path" check would pass a script that
# never resolves it).
#
# HOW it is observed. The bug this was written for is textual (a
# machine-specific fallback in check.sh), but a purely textual check cannot
# express the doctrine RT-167 installed, and an executed one cannot work:
# check.sh and build.sh end in `exec cargo` (sourcing build.sh runs a real
# build, so the assertion would take minutes and assert nothing). So the
# assertion has two arms per script, both of them observational:
#   1. no script may contain a machine-specific LOCAL_DIR fallback — the exact
#      defect that shipped;
#   2. every script must EITHER default LOCAL_DIR to its own repo-relative
#      `local/` itself, OR source dev-env.sh, which owns that default.
# Arm 2 is what check.sh satisfies now, and it is what makes the default ONE
# copy instead of four. The scripts that resolve it themselves are sourced
# (dev-env.sh and provision-build-deps.sh are safe to source — neither execs),
# and the resolved value is asserted to exist on this box, so a repo-relative
# default that points nowhere fails rather than passes silently. build.sh is
# matched by its textual default only: it ends in `exec cargo`, so sourcing it
# would run a real build.
for script in dev-env.sh check.sh build.sh provision-build-deps.sh; do
    script_path="$SCRIPTS_DIR/$script"

    # Arm 1: the defect that shipped. Rejected first, so the failure message
    # names the cause rather than the consequence.
    if grep -qE 'LOCAL_DIR="?\$\{LOCAL_DIR:-/[^}]*local\}"' "$script_path"; then
        echo "FAIL: $script falls back to a machine-specific LOCAL_DIR"
        exit 1
    fi

    # Arm 2a (textual): the script defaults LOCAL_DIR to its own local/.
    if grep -qE 'LOCAL_DIR="?\$\{LOCAL_DIR:-[^}/][^}]*local\}"' "$script_path"; then
        # Sourced verification only for scripts that are safe to source. build.sh
        # execs cargo (and takes minutes); its default is pinned by this grep
        # against the machine-specific form above.
        case "$script" in
            build.sh) continue ;;
        esac
        resolved="$(env -u LOCAL_DIR bash -c 'cd "$1" && . "./$2" >/dev/null 2>&1; printf %s "${LOCAL_DIR:-}"' _ "$SCRIPTS_DIR" "$script" 2>/dev/null)"
        if [ -n "$resolved" ]; then
            [ -d "$resolved" ] || { echo "FAIL: $script resolves LOCAL_DIR to a path that does not exist: $resolved"; exit 1; }
            continue
        fi
    fi

    # Arm 2b: the script sources the one that owns the default.
    grep -qE '^(\.|source) .*dev-env\.sh' "$script_path" || {
        echo "FAIL: $script neither defaults LOCAL_DIR to its own local/ nor sources dev-env.sh (which owns it)"
        exit 1
    }
done
echo "PASS 6: every build script resolves LOCAL_DIR to the repo's local/"

echo "build_sh_test: all 6 assertions passed"
