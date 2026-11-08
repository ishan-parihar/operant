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

# Resolve a cargo binary that actually RUNS (iter-1933).
#
# `cargo` on PATH may be a systemd-run wrapper that runs builds inside
# build.slice (MemoryHigh, oom_score) so an OOM under memory pressure kills the
# BUILD, not an agent session. Its guard is
#
#     if [ -n "$XDG_RUNTIME_DIR" ] && [ -S "$XDG_RUNTIME_DIR/bus" ]; then
#         exec systemd-run --user --scope --quiet --slice=build.slice -- /usr/bin/cargo "$@"
#     fi
#     exec /usr/bin/cargo "$@"
#
# which checks that the user bus SOCKET EXISTS, not that it is alive. A stale
# socket — a user manager that died without cleaning /run/user/<uid>/bus — makes
# the guard pass, so every build goes down the systemd-run path and dies with
# "Failed to connect to user scope bus via local transport: No data available"
# with no fallback at all. The memory-wire sync then ALSO fails, and the warning
# printed below misattributes it to being offline, so a broken local bus reads as
# a network problem.
#
# Probe the candidate before starting a build rather than assuming PATH order is
# right; `/usr/bin/cargo` is the real binary the wrapper falls back to.
CARGO_BIN="${CARGO_BIN:-$(command -v cargo || true)}"
cargo_runs() {
    [ -n "${1:-}" ] && "$1" --version >/dev/null 2>&1
}
if ! cargo_runs "$CARGO_BIN"; then
    for candidate in /usr/bin/cargo /usr/local/bin/cargo; do
        if [ -x "$candidate" ] && cargo_runs "$candidate"; then
            CARGO_BIN="$candidate"
            break
        fi
    done
fi
if ! cargo_runs "$CARGO_BIN"; then
    echo "[build] ERROR: no working cargo binary (tried PATH and /usr/bin/cargo)" >&2
    exit 1
fi
echo "[build] cargo: $CARGO_BIN"

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
    if "$CARGO_BIN" update -p memory-wire; then
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
exec "$CARGO_BIN" "${cargo_args[@]}"
