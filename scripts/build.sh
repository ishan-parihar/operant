#!/usr/bin/env bash
# scripts/build.sh — the ONLY documented build entry point (2026-10-14 batch,
# plan 11A).
#
# Why this exists: memory-wire is a branch-tracking git dependency (user-ordered
# persistent-latest). Every build path — dev, test gates, release, deploy —
# must advance the lock to upstream HEAD first, so the tree never drifts from
# upstream. The old 1h stamp throttle (in check.sh) meant release/deploy builds
# and back-to-back dev builds silently compiled an 18-commit-old memory-wire.
#
# Usage:
#   scripts/build.sh [--release] [--offline] [--strict] [--sync-only] [cargo args...]
#
#   --release     pass --release to the default build
#   --offline     skip the memory-wire sync (offline / no-network)
#   --strict      a failed sync is a FAILURE; deploy uses this
#   --sync-only   only run the sync, do not build
#
# With no cargo args: cargo build [-p operant-cli --bin operant].
# With cargo args, the first one is the subcommand:
#   scripts/build.sh test -p operant-core --lib
#
# Sync failure policy: loud warning, build continues with the locked rev,
# exit 0 — unless --strict. Offline upstream never breaks a local build.
set -euo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$DIR/.."

release=0; offline=0; strict=0; sync_only=0
cargo_args=()
while [ $# -gt 0 ]; do
    case "$1" in
        --release)   release=1 ;;
        --offline)   offline=1 ;;
        --strict)    strict=1 ;;
        --sync-only) sync_only=1 ;;
        *)           cargo_args+=("$1") ;;
    esac
    shift
done

# Build-time environment (libclang, ONNX Runtime, rpath) — same LOCAL_DIR
# derivation as dev-env.sh itself. Suspend `set -u` while sourcing it:
# dev-env.sh reads LIBCLANG_PATH (and other optional vars) before defining
# them, and inherited nounset turns that into a fatal "unbound variable".
export LOCAL_DIR="${LOCAL_DIR:-$PWD/local}"
# shellcheck source=scripts/dev-env.sh
set +u
source scripts/dev-env.sh
set -u
# A foreign `ar` CLI shadowing binutils broke builds twice before
# (AGENTS.md fleet rule 6); pin the real one.
export AR="${AR:-/usr/bin/ar}"

# Locked memory-wire rev from Cargo.lock (empty when absent).
rev_of() {
    awk '
        /^\[\[package\]\]/{found=0}
        $0 == "name = \"memory-wire\""{found=1}
        found && /^source = "git\+/{print; exit}' Cargo.lock \
        | grep -oE '[0-9a-f]{40}' | head -1
}

sync_memory_wire() {
    if [ "$offline" = 1 ] || [ "${MEMORY_WIRE_NO_SYNC:-0}" = "1" ]; then
        echo "[build] memory-wire sync skipped (--offline / MEMORY_WIRE_NO_SYNC=1)"
        return 0
    fi
    local before
    before="$(rev_of)"
    if cargo update -p memory-wire; then
        local after
        after="$(rev_of)"
        echo "[build] memory-wire: ${before:-unknown} → ${after:-unknown}"
        return 0
    fi
    if [ "$strict" = 1 ]; then
        echo "[build] ERROR: memory-wire sync failed (--strict)" >&2
        exit 1
    fi
    echo "[build] WARNING: memory-wire sync failed (offline?); building with lock rev ${before:-unknown}" >&2
    return 0
}

sync_memory_wire
[ "$sync_only" = 1 ] && exit 0

if [ ${#cargo_args[@]} -eq 0 ]; then
    cargo_args=(build -p operant-cli --bin operant)
    [ "$release" = 1 ] && cargo_args=(build --release -p operant-cli --bin operant)
fi
exec cargo "${cargo_args[@]}"
