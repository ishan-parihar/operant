#!/usr/bin/env bash
# Run cargo with the dev-env applied. Usage:
#   ./scripts/check.sh check --workspace
#   ./scripts/check.sh test --workspace --no-run
#   ./scripts/check.sh doc          # the rustdoc gate
#
# The build environment has ONE source: scripts/dev-env.sh, which this script
# SOURCES. It used to re-implement the environment by hand, and the copy
# drifted: it kept the ONNX Runtime link flags out of RUSTFLAGS, so a binary
# built through check.sh carried NO RUNPATH and exited 127 ("libonnxruntime.so.1:
# cannot open shared object file") in any shell that had not sourced
# dev-env.sh. The self-test builds through check.sh, so the TUI corpus gate
# and every CLI smoke test ran against a binary that could not start — 70
# phantom drifts and 10 failures, every one of them the same missing
# library. Two copies of one environment is how they came to disagree
# (RT-167); there is one copy now.
set -e

DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"

# LOCAL_DIR, LIBCLANG_PATH, ORT_LIB_LOCATION, ORT_PREFER_DYNAMIC_LINK,
# BINDGEN_EXTRA_CLANG_ARGS, PKG_CONFIG_PATH, LD_LIBRARY_PATH, the ONNX link
# flags and the RUNPATH all come from here.
. "$DIR/dev-env.sh"

cd "$DIR/.."

# memory-wire persistent-latest (user-ordered): advance the branch-tracking
# `memory-wire` dep to upstream HEAD before building, so the tree always
# compiles the newest source. Since the 2026-10-14 batch (plan 11A) this
# delegates to scripts/build.sh --sync-only — ONE sync code path for dev,
# test, release and deploy, with NO stamp throttle (the 1h TTL let
# release/deploy builds silently compile an 18-commit-old memory-wire).
# build.sh honors MEMORY_WIRE_NO_SYNC=1, prints old→new, and never fails a
# check build on an unreachable upstream — the lock stays at its last-good
# rev. A breaking upstream release fails the build loudly until pinned. See
# crates/operant-core/Cargo.toml.
"$DIR/build.sh" --sync-only

# `doc` is a shortcut for the rustdoc gate, not a passthrough: ci.yml:140 runs
# `cargo doc --workspace --no-deps --all-features` under RUSTDOCFLAGS=-Dwarnings,
# and that is exactly the gate the local loop is supposed to stand in for. The
# flags are baked in so the local command and the CI job cannot drift; pass them
# explicitly to override. Any other subcommand is forwarded to cargo unchanged,
# so this stays backward compatible with every existing caller.
#
# `--document-private-items` is part of the gate because almost every item in
# this workspace is private: without it, every intra-doc link to a private
# item is reported as unresolved (rustdoc's own note says so), and the gate
# cannot tell a private link from a broken one. With it, the remaining
# failures are links that point at nothing that exists.
if [ "${1:-}" = "doc" ]; then
    shift
    export RUSTDOCFLAGS="${RUSTDOCFLAGS:--Dwarnings}"
    exec cargo doc --workspace --no-deps --all-features --document-private-items "$@"
elif [ "${1:-}" = "doc-test" ]; then
    # The SECOND doc profile RG-013 prescribes: `cfg(test)` ON, so a
    # `#[cfg(test)]` item is in the AST rustdoc sees and a broken doc link in
    # test-only documentation fails the gate instead of hiding from it. The
    # production gate above cannot see that class at all (measured: a
    # `#[cfg(test)]` item is absent when cfg(test) is off), so this is a
    # separate script with its own three-measurement header rather than a
    # flag on the line above — `--cfg test` is not a valid cargo argument
    # (cargo 1.98 rejects it), and it must ride RUSTDOCFLAGS scoped to one
    # crate at a time or it leaks into every dependency.
    shift
    exec "$DIR/check-doc-test-profile.sh" "$@"
fi

exec cargo "$@"
