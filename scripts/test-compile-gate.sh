#!/usr/bin/env bash
# Test-compile gate: every KEPT crate's test targets must compile.
#
# Why this exists (2026-10-16 audit, plan 02): ten test targets had been
# dead since the org layer retired (iter-838) — they still imported
# `authority`, `employee_db`, `notice`, `worklog_db`, `write_barrier`,
# `CamofoxStateTool` — so `cargo test -p operant-cli --bin operant` could
# not compile, and every "gates green" claim after that was unverifiable.
# Nothing in the per-iteration battery compiled the *other* crates' test
# targets, so a module deletion could (and did) leave dead tests behind
# for weeks. This gate is the cheap compile-only half of the battery;
# run it before the test gates.
#
# Crates slated for deletion (audit plan 10, phase 1:
# docs/audit/2026-10-16-operant-audit/10-crate-and-dead-code-purge.md)
# are excluded until they are gone — their test targets are not
# maintained. When plan 10 removes a crate, nothing needs editing here;
# when it removes the LAST excluded crate, drop KEEP_CRATES and use
# `--workspace` instead.
set -euo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$DIR/.."

# Kept crates (every workspace member except the six phase-1 deletion
# targets). `--keep-going` so one broken crate reports every broken
# target in a single run instead of failing at the first.
KEEP_CRATES=(
    operant-core
    operant-cli
    operant-os
    operant-harness
    operant-plugins
    operant-api
    operant-macros
)
PS_ARGS=()
for c in "${KEEP_CRATES[@]}"; do PS_ARGS+=(-p "$c"); done

exec ./scripts/check.sh check --tests --keep-going --offline "${PS_ARGS[@]}" "$@"
