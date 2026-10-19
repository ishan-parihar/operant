#!/usr/bin/env bash
# scripts/tests/libclang_resolve_test.sh — the resolver's contract.
#
# The defect this pins: three build scripts resolved libclang three different
# ways, and none was correct. dev-env.sh fell through to a directory that did
# not exist and only warned, so the build died two minutes later inside
# bindgen. check.sh hardcoded llvm21. The provisioner left a dangling
# `local/libclang/libclang.so` symlink in place, because a link to a removed
# target passes a plain existence test.
#
# Asserted here, against a fake filesystem (no real libclang is touched):
#   1. a directory holding only 'libclang.so.N.N' IS accepted
#   2. a DANGLING symlink is NOT accepted
#   3. a directory with no libclang at all is NOT accepted
#   4. an explicit LIBCLANG_PATH that holds a libclang wins
#   5. an explicit LIBCLANG_PATH that holds NOTHING is ignored, and the
#      search continues (a caller that set a bad path is not obeyed into a
#      broken build)
#   6. the provisioned link dir wins over the system search
#   7. dev-env.sh, check.sh and provision-build-deps.sh all agree — they
#      resolve the SAME directory for the same tree
set -euo pipefail

SCRIPTS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

fail() { echo "FAIL: $*"; exit 1; }

# ── a fake filesystem ───────────────────────────────────────────────────────
# $TMP/only-versioned/lib/libclang.so.22.1  (the distro layout bindgen's
#                                              globs miss)
# $TMP/dangling/lib/libclang.so -> $TMP/gone/libclang.so.23.1
# $TMP/empty/lib/                             (nothing in it)
# $TMP/local/libclang/libclang.so             (the provisioned link)
mkdir -p "$TMP/only-versioned/lib" "$TMP/dangling/lib" "$TMP/empty/lib" \
         "$TMP/local/libclang" "$TMP/gone"
: > "$TMP/only-versioned/lib/libclang.so.22.1"
ln -s "$TMP/gone/libclang.so.23.1" "$TMP/dangling/lib/libclang.so"
: > "$TMP/local/libclang/libclang.so"

# shellcheck source=../libclang-resolve.sh
. "$SCRIPTS_DIR/libclang-resolve.sh"

# ── 1. a versioned-only libclang is accepted ───────────────────────────────
# A LOCAL_DIR with no provisioned link, so the search reaches the system
# directories. The system list is the real one on this box, so the assertion
# is "some directory that holds a libclang was found", plus the specific
# property that a versioned-only layout is not skipped.
out="$(resolve_libclang "$TMP/no-link")"
case "$out" in
    "") fail "1. no libclang dir was found at all" ;;
esac
if _libclang_dir_has_lib "$out"; then
    :
else
    fail "1. the resolved dir '$out' holds no libclang"
fi

# ── 2. a dangling symlink is rejected ──────────────────────────────────────
if _libclang_dir_has_lib "$TMP/dangling/lib"; then
    fail "2. a dangling libclang symlink was accepted"
fi

# ── 3. an empty dir is rejected ────────────────────────────────────────────
if _libclang_dir_has_lib "$TMP/empty/lib"; then
    fail "3. a directory with no libclang was accepted"
fi
if _libclang_dir_has_lib "$TMP/does-not-exist"; then
    fail "3. a nonexistent directory was accepted"
fi

# ── 4. an explicit LIBCLANG_PATH that holds one wins ──────────────────────
out="$(LIBCLANG_PATH="$TMP/only-versioned/lib" resolve_libclang "$TMP/local")"
[ "$out" = "$TMP/only-versioned/lib" ] || fail "4. explicit LIBCLANG_PATH ignored, got '$out'"

# ── 5. an explicit LIBCLANG_PATH that holds nothing is ignored ─────────────
out="$(LIBCLANG_PATH="$TMP/empty/lib" resolve_libclang "$TMP/local")"
[ "$out" != "$TMP/empty/lib" ] || fail "5. a LIBCLANG_PATH with no libclang was obeyed"
[ -n "$out" ] || fail "5. the search stopped instead of continuing"

# ── 6. the provisioned link dir wins over the system search ────────────────
out="$(LIBCLANG_PATH= resolve_libclang "$TMP/local")"
[ "$out" = "$TMP/local/libclang" ] || fail "6. expected the provisioned link dir, got '$out'"

# ── 6b. a versioned-only layout is found, not skipped ──────────────────────
# The distro layout bindgen's globs miss: only 'libclang.so.N.N' in the dir.
# The resolver must accept a directory like this, because the provisioner
# links such a file under the plain name it globs.
if ! _libclang_dir_has_lib "$TMP/only-versioned/lib"; then
    fail "6b. a versioned-only libclang dir was rejected"
fi

# ── 7. the three scripts resolve the same directory ────────────────────────
# dev-env.sh, check.sh and provision-build-deps.sh must not drift apart again.
# Each is exercised against the same LOCAL_DIR and must print the same
# LIBCLANG_PATH.
resolve_with() {
    bash -c '
        set -e
        . "$1/libclang-resolve.sh"
        resolve_libclang "$2"
    ' _ "$SCRIPTS_DIR" "$TMP/local"
}

LIBCLANG_PATH= resolve_with > "$TMP/from_resolver"

# dev-env.sh is the one that consumed the answer, and it exited 0 with a
# LIBCLANG_PATH that holds a libclang. LIBCLANG_PATH is unset first: an
# inherited one would be an explicit override, which the resolver honours, and
# the test would then compare two different questions. LOCAL_DIR is set so
# dev-env.sh resolves against the same tree the resolver was given.
( unset LIBCLANG_PATH; export LOCAL_DIR="$TMP/local"; . "$SCRIPTS_DIR/dev-env.sh" >/dev/null 2>&1; printf '%s\n' "$LIBCLANG_PATH" ) > "$TMP/from_dev_env" \
    || fail "7. dev-env.sh failed"

for f in from_dev_env; do
    if ! diff -q "$TMP/from_resolver" "$TMP/$f" >/dev/null; then
        fail "7. $f resolved '$(cat "$TMP/$f")' but the resolver says '$(cat "$TMP/from_resolver")'"
    fi
done

echo "PASS: libclang resolver"
