#!/usr/bin/env bash
# The clippy gate's TEST-SURFACE exemption must exempt the hygiene lints and
# NOTHING ELSE.
#
# Why this file exists
# --------------------
# The gate promised "test targets are exempted" but implemented it as
# `contains("/tests/")` + a two-lint allowlist (`clippy::unwrap_used`,
# `clippy::expect_used`). Two holes followed:
#
#   1. PATH: `crates/operant-cli/src/tui/app/tests.rs` is a test module by name
#      and lives under `src/`, so no `/tests/` rule matched it. It reddened the
#      gate on `unused_mut`.
#   2. LINT CLASS: `unused_imports` / `unused_mut` / `dead_code` are rustc lints,
#      not clippy lints, so the `$lint == "clippy::..."` comparison never
#      matched them even on a real `crates/*/tests/*.rs` target.
#
# iter-2742 widens both: the path test also matches `*/tests.rs`, and the lint
# set includes the dead-code family. The DANGER of widening an exemption is that
# it stops catching something real, so this test pins BOTH directions:
#
#   - exempt:   a hygiene lint inside a test surface
#   - NOT exempt: the same hygiene lint in PRODUCTION code
#   - NOT exempt: a CORRECTNESS lint (clippy::correctness) even in a test file
#
# The last one is the load-bearing case. If a future edit widens the exemption
# to "all lints on test files", a real bug found by a correctness lint in a test
# would be silently dropped, and this test fails.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
GATE="${REPO_ROOT}/scripts/clippy-warning-gate.sh"
fail=0

# Extract the jq program from the gate so the test exercises the SHIPPED filter
# rather than a copy of it. The program is the single-quoted heredoc body.
extract_filter() {
  # The jq program is the single-quoted heredoc: it opens on the `jq -r '` line
  # and closes on the line that ends with `' "${tmp_json}"`. Emit everything
  # strictly between them, exclusive of the shell quoting.
  local open close
  open=$(grep -n "^jq -r '$" "${GATE}" | head -1 | cut -d: -f1)
  close=$(grep -n "^' \"\${tmp_json}\"" "${GATE}" | head -1 | cut -d: -f1)
  if [ -z "${open}" ] || [ -z "${close}" ]; then
    return 1
  fi
  sed -n "$((open + 1)),$((close - 1))p" "${GATE}"
}

FILTER="$(extract_filter)"
if [ -z "${FILTER}" ]; then
  echo "FAIL: could not extract the jq filter from ${GATE}" >&2
  exit 1
fi

# Run one synthetic diagnostic through the gate's filter. Echoes the gate's
# output (empty means "exempt / not reported").
run_case() {
  local file="$1" lint="$2" level="${3:-warning}"
  printf '{"reason":"compiler-message","target":{"name":"operant"},"message":{"level":"%s","code":{"code":"%s"},"spans":[{"is_primary":true,"file_name":"%s"}]}}\n' \
    "${level}" "${lint}" "${file}" \
    | jq -r "${FILTER}" 2>/dev/null
}

assert_exempt() {
  local name="$1" file="$2" lint="$3"
  local out
  out="$(run_case "${file}" "${lint}")"
  if [ -n "${out}" ]; then
    echo "FAIL: ${name} -- expected EXEMPT, but the gate reported: ${out}" >&2
    fail=1
  else
    echo "ok:   ${name} (exempt)"
  fi
}

assert_reported() {
  local name="$1" file="$2" lint="$3" level="${4:-error}"
  local out
  out="$(run_case "${file}" "${lint}" "${level}")"
  if [ -z "${out}" ]; then
    echo "FAIL: ${name} -- expected REPORTED, but the gate exempted it" >&2
    fail=1
  else
    echo "ok:   ${name} (reported)"
  fi
}

# --- exempt: hygiene lints on the test surface ------------------------------
assert_exempt "unused_mut in src/**/tests.rs" \
  "crates/operant-cli/src/tui/app/tests.rs" "unused_mut"
assert_exempt "unused_imports in crates/*/tests/*.rs" \
  "crates/operant-core/tests/evidence_seams.rs" "unused_imports"
assert_exempt "dead_code in a tests.rs file" \
  "crates/operant-cli/src/tui/session_hub/tests.rs" "dead_code"
assert_exempt "unwrap_used in a test target" \
  "crates/operant-core/tests/mcp_oauth_surface_has_no_orphan_recovery.rs" "clippy::unwrap_used"

# --- NOT exempt: the same lints in production code --------------------------
assert_reported "unused_mut in production" \
  "crates/operant-core/src/mcp.rs" "unused_mut"
assert_reported "dead_code in production" \
  "crates/operant-core/src/tools/builtin.rs" "dead_code"
assert_reported "unwrap_used in production" \
  "crates/operant-core/src/mcp.rs" "clippy::unwrap_used"

# --- NOT exempt: a correctness lint is never hygiene ------------------------
# This is the guard that matters most. `clippy::correctness` on a test file is a
# real finding; an over-broad exemption would drop it.
assert_reported "clippy::correctness in a test file" \
  "crates/operant-cli/src/tui/app/tests.rs" "clippy::correctness"
assert_reported "clippy::suspicious in a test file" \
  "crates/operant-cli/src/tui/app/tests.rs" "clippy::suspicious"

# --- NOT exempt: a non-crate path is not a test surface ---------------------
assert_reported "a lint outside crates/ is not exempted" \
  "scripts/whatever.rs" "unused_mut"

if [ "${fail}" -ne 0 ]; then
  echo
  echo "clippy-gate exemption test: FAIL" >&2
  exit 1
fi
echo
echo "clippy-gate exemption test: PASS"