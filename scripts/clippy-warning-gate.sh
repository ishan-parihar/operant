#!/usr/bin/env bash
# Incremental clippy warning gate.
#
# Runs `cargo clippy --workspace --all-targets` and compares the warnings
# against a committed allowlist (`.ci/clippy-allowlist.txt`). The gate FAILS
# only when NEW warnings appear (relative to the allowlist); stale allowlist
# entries are reported so they can be pruned. This keeps CI green while the
# codebase converges to zero warnings (see docs/RUST_BEST_PRACTICES_PLAN.md).
#
# Ported from hermes-agent-ultra's scripts/clippy-warning-gate.sh.
#
# Usage:
#   scripts/clippy-warning-gate.sh --update   # refresh allowlist from current warnings
#   scripts/clippy-warning-gate.sh            # check (CI)
set -euo pipefail

ALLOWLIST_FILE=".ci/clippy-allowlist.txt"
MODE="check"
EXTRA_ARGS=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --allowlist)
      ALLOWLIST_FILE="${2:?missing value for --allowlist}"
      shift 2
      ;;
    --update)
      MODE="update"
      shift
      ;;
    --check)
      MODE="check"
      shift
      ;;
    -h|--help)
      echo "Usage: scripts/clippy-warning-gate.sh [--update|--check]"
      exit 0
      ;;
    *)
      break
      ;;
  esac
done

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

if ! command -v jq >/dev/null 2>&1; then
  echo "[clippy-gate] jq is required (apt install jq)" >&2
  exit 1
fi

tmp_json="$(mktemp)"
tmp_json_tests="$(mktemp)"
tmp_seen="$(mktemp)"
tmp_allow="$(mktemp)"
tmp_new="$(mktemp)"
tmp_stale="$(mktemp)"
trap 'rm -f "${tmp_json}" "${tmp_json_tests}" "${tmp_seen}" "${tmp_allow}" "${tmp_new}" "${tmp_stale}"' EXIT

echo "[clippy-gate] collecting warnings..."
# Runs WITH --all-features: the workspace feature inventory is fully fixed
# (2026-08-03 — observability/hardware/channels vendor modules, tools probe,
# core anthropic, gateway schema-export all compile green; see the
# "--all-features" inventory note in docs/RUST_BEST_PRACTICES_PLAN.md).
# Gating on --all-features keeps feature-gated unwrap/expect regressions
# (e.g. the plugins-wasm strip_prefix expect, fixed 2026-08-03) from
# slipping past the gate under default features alone.
#
# Production code must not call `.unwrap()`/`.expect()` (rust-best-practices
# ch.4). Manifest-level workspace lints are unusable here (this environment's
# cargo rejects `lints` manifest keys), so the denies are applied explicitly;
# justified sites carry `#[expect(clippy::unwrap_used/expect_used)]` with a
# reason, and test targets are exempted via cfg_attr(test) allows / headers.
# Which cargo this gate measures with (2026-11-14, MEASURED). The line below
# used to be a bare `cargo clippy ...`, so on this host the gate ran NOTHING and
# still printed "pass": PATH's cargo is a systemd-run wrapper that exits 1 with
# "Failed to connect to user scope bus via local transport", the `|| true`
# swallowed it, the JSON file was empty, and the gate reported
#   [clippy-gate] seen=0 allowlist=0 new=0 stale=0
#   [clippy-gate] pass
# A gate that passes on zero diagnostics buys confidence the code cannot earn —
# the worst kind of green. One resolver, shared with check.sh.
. "${ROOT_DIR}/scripts/cargo-bin.sh"
if ! resolve_cargo_bin; then
    echo "[clippy-gate] ERROR: no working cargo binary (tried PATH and /usr/bin/cargo)" >&2
    exit 1
fi
echo "[clippy-gate] cargo: $CARGO_BIN"

# `set +e`: under the -D denies cargo exits 101 whenever an unannotated
# unwrap/expect fires — that's the gate's DATA, not a script failure. Without
# this, `set -e` kills the script at the cargo line, the trap deletes
# ${tmp_json}, and the gate exits 101 with zero diagnostics (BUGS.md R39-6).
set +e
# --all-targets is what makes the gate see `tests/*.rs` integration targets.
# AGENTS.md promises "Test targets are exempted", but until iter-2444 only the
# IN-TEST-MODULE half was true: `#![cfg_attr(test, allow(...))]` in lib.rs /
# main.rs reaches a `#[cfg(test)] mod tests`, NOT an integration target under
# `tests/`, which is a SEPARATE crate. Those files assert with `.expect(..)` once
# per line and carried no suppression, so the gate denied 19 sites in
# `tests/evidence_seams.rs` alone and could not go green while any of the 9
# failing no-header targets existed.
#
# So: the DENIES run over lib/bin only (the production surface the gate exists to
# protect), and the all-targets pass runs BESIDE them with the same denies to keep
# compiling every target -- its diagnostics for the two denied lints are dropped
# in the jq filter below, which is where the documented exemption belongs. The
# all-targets pass is NOT allowed to fail the script: `set +e` + `|| true`, and
# its `build-finished` record is stripped before the success check so a denied
# test target cannot abort the run before its diagnostics are counted.
"${CARGO_BIN}" clippy --workspace --lib --bins --all-features --message-format=json \
  -- -D clippy::unwrap_used -D clippy::expect_used "${EXTRA_ARGS[@]}" \
  > "${tmp_json}"
cargo_status=$?
set -e

set +e
"${CARGO_BIN}" clippy --workspace --all-targets --all-features --message-format=json \
  -- -D clippy::unwrap_used -D clippy::expect_used "${EXTRA_ARGS[@]}" \
  > "${tmp_json_tests}" 2>/dev/null
# Discard the all-targets build-finished record: its status reflects test targets
# the policy exempts. The lib/bin run above is the authority on "did it compile".
jq -c 'select(.reason != "build-finished")' "${tmp_json_tests}" >> "${tmp_json}" 2>/dev/null
rm -f "${tmp_json_tests}"
set -e

# A finished, successful build is the ONLY state in which "zero diagnostics"
# means "no warnings". Any other state — a build script that failed (espeak's
# cmake hitting the path-length limit in a long worktree path is the usual
# one), a compile error outside the denied lints, a cancellation — leaves the
# JSON without a `build-finished success=true` record, and the old `|| true`
# turned every one of them into a pass. MEASURED 2026-11-14: a gate run in a
# 72-character worktree printed "seen=0 ... pass" while espeak-rs-sys's cmake
# was failing with "make Error 2". A gate that passes without measuring is
# worse than no gate.
#
# EXACTLY ONE record, not "at least one". The all-targets pass runs beside the
# lib/bin pass (iter-2444) and its own build-finished record is stripped before
# this point; `jq -e` would accept the FIRST success record it found, so a
# future edit that stopped stripping would leave the lib/bin failure masked by
# the other pass's success and this gate would go quiet again. Requiring the
# count pins the strip in place — and it is what makes the mutation "keep the
# all-targets record" fail instead of passing silently.
build_records="$(jq -r 'select(.reason == "build-finished") | .success' "${tmp_json}" 2>/dev/null | wc -l | tr -d ' ')"
if [[ "${build_records}" != "1" ]] \
   || ! jq -e 'select(.reason == "build-finished" and .success == true)' "${tmp_json}" >/dev/null 2>&1; then
    echo "[clippy-gate] ERROR: the clippy build did not finish successfully (cargo status ${cargo_status}), so zero diagnostics proves nothing was measured." >&2
    echo "[clippy-gate] If the failure is espeak-rs-sys's cmake 'data' target, the absolute build path is too long: build from a SHORT CARGO_TARGET_DIR (e.g. /dev/shm/ot) — see AGENTS.md parallel-agent rule 6." >&2
    exit 1
fi

jq -r '
  select(.reason == "compiler-message")
  | . as $root
  | .message as $m
  | select($m.level == "warning" or $m.level == "error")
  | ($root.target.name // "unknown_target") as $target
  | (
      $m.spans
      | map(select(.is_primary == true))
      | .[0] // ($m.spans[0] // {})
    ) as $span
  | ($span.file_name // "unknown_file") as $file
  | ($m.code.code // ("rustc:" + ($m.message | split(":")[0]))) as $lint
  # AGENTS.md promises "Test targets are exempted", but until iter-2444 only the
  # IN-TEST-MODULE half was true: `#![cfg_attr(test, allow(...))]` in lib.rs /
  # main.rs reaches `#[cfg(test)] mod tests`, NOT an integration target under
  # `tests/`, which is a SEPARATE crate. Those files assert with `.expect(..)`
  # once per line and carried no suppression, so the gate denied 19 sites in
  # `tests/evidence_seams.rs` alone -- and could not go green while any of the 9
  # failing no-header targets existed. Exempt the denied lints on that surface
  # ONLY: a test target still fails the gate on a compile error, an unwrap in a
  # non-test helper, or a lint of any other class.
  | select(
      ($file | startswith("crates/") and ($file | contains("/tests/")))
      and ($lint == "clippy::unwrap_used" or $lint == "clippy::expect_used")
      | not
    )
  | "\($target)|\($lint)|\($file)"
' "${tmp_json}" | LC_ALL=C sort -u > "${tmp_seen}"

if [[ "${MODE}" == "update" ]]; then
  mkdir -p "$(dirname "${ALLOWLIST_FILE}")"
  cp "${tmp_seen}" "${ALLOWLIST_FILE}"
  count="$(wc -l < "${ALLOWLIST_FILE}" | tr -d ' ')"
  echo "[clippy-gate] updated ${ALLOWLIST_FILE} (${count} entries)"
  exit 0
fi

if [[ ! -f "${ALLOWLIST_FILE}" ]]; then
  echo "[clippy-gate] allowlist not found: ${ALLOWLIST_FILE}" >&2
  echo "[clippy-gate] run: scripts/clippy-warning-gate.sh --update" >&2
  exit 1
fi

# `grep -v` exits 1 when it selects nothing, which `set -e` would read as a
# script failure: an allowlist that is all comments aborts the gate before
# it can report what it found. Same shape as the supply-chain gate.
grep -vE '^\s*(#|$)' "${ALLOWLIST_FILE}" | LC_ALL=C sort -u > "${tmp_allow}" || true

# `comm` compares using the collation of the ambient locale, but the inputs are
# byte-sorted under LC_ALL=C, so it must run under the same locale to avoid
# spurious "not in sorted order" errors.
LC_ALL=C comm -23 "${tmp_seen}" "${tmp_allow}" > "${tmp_new}"
LC_ALL=C comm -13 "${tmp_seen}" "${tmp_allow}" > "${tmp_stale}"

seen_count="$(wc -l < "${tmp_seen}" | tr -d ' ')"
allow_count="$(wc -l < "${tmp_allow}" | tr -d ' ')"
new_count="$(wc -l < "${tmp_new}" | tr -d ' ')"
stale_count="$(wc -l < "${tmp_stale}" | tr -d ' ')"

echo "[clippy-gate] seen=${seen_count} allowlist=${allow_count} new=${new_count} stale=${stale_count}"

if [[ "${new_count}" -gt 0 ]]; then
  echo "[clippy-gate] NEW warnings detected:" >&2
  sed 's/^/  + /' "${tmp_new}" >&2
  exit 1
fi

if [[ "${stale_count}" -gt 0 ]]; then
  echo "[clippy-gate] stale allowlist entries (safe to prune via --update):"
  sed 's/^/  - /' "${tmp_stale}"
fi

echo "[clippy-gate] pass"
