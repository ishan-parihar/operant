#!/usr/bin/env bash
# Coverage report for the workspace, via cargo-tarpaulin.
#
#   ./scripts/coverage.sh                    # whole workspace, report only
#   COVERAGE_THRESHOLD=70 ./scripts/coverage.sh   # whole workspace, gate at 70%
#   ./scripts/coverage.sh -p operant-config  # scope to one package instead

#
# Report-only by DEFAULT, and that default is deliberate: a coverage gate needs a
# measured number to be worth anything, and there is no recorded baseline for this
# workspace yet. COVERAGE_THRESHOLD=0 means "do not pass --fail-under-lines at
# all", so the default run cannot fail on coverage. Set a real number to gate.
#
# This is a report generator, not part of the pre-merge loop. See ci.yml: the
# GitHub Actions quota is exhausted, so nothing runs automatically anyway, and a
# full --workspace --all-features instrumented build+test run is far slower than
# `cargo test --workspace`. Budget the time rather than expecting it to be cheap.
set -euo pipefail

THRESHOLD="${COVERAGE_THRESHOLD:-0}"
OUT_DIR="${COVERAGE_OUT_DIR:-coverage}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# libclang / ONNX Runtime / alsa are needed to build the workspace at all; the
# same env every other script here applies. dev-env.sh is not `set -u` clean, so
# relax the nounset around it rather than editing a script this one does not own.
set +u
# shellcheck source=/dev/null
[ -f "$SCRIPT_DIR/dev-env.sh" ] && source "$SCRIPT_DIR/dev-env.sh" >/dev/null 2>&1 || true
set -u

case "$THRESHOLD" in
    ''|*[!0-9]*)
        echo "coverage: COVERAGE_THRESHOLD must be a non-negative integer, got '$THRESHOLD'" >&2
        exit 2
        ;;
esac

if ! cargo tarpaulin --version >/dev/null 2>&1; then
    echo "coverage: cargo-tarpaulin is not installed." >&2
    echo "  install it with:  cargo install cargo-tarpaulin --locked" >&2
    exit 127
fi

mkdir -p "$OUT_DIR"

args=(--all-features --out xml --output-dir "$OUT_DIR")

if [ "$#" -eq 0 ]; then
    args=(--workspace "${args[@]}")
else
    # `--workspace` and a package selector are not a narrowing pair: passing both
    # makes tarpaulin build the whole workspace anyway. So when the caller
    # supplies args, those args ARE the scope, and --workspace is left off.
    echo "coverage: extra args given, using them as the scope instead of --workspace"
fi

if [ "$THRESHOLD" -gt 0 ]; then
    # The flag is `--fail-under`, not `--fail-under-lines`; tarpaulin 0.37
    # rejects the longer spelling outright, which is how this was found.
    args+=(--fail-under "$THRESHOLD")
    echo "coverage: GATING at ${THRESHOLD}% line coverage"
else
    echo "coverage: report-only (COVERAGE_THRESHOLD=0); not gating"
fi

echo "coverage: cargo tarpaulin ${args[*]} $*"
cargo tarpaulin "${args[@]}" "$@"

echo "coverage: report written to $OUT_DIR/"
