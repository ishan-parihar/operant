#!/usr/bin/env bash
# Supply-chain gate: cargo audit against a committed advisory allowlist.
#
# WHY THIS EXISTS. `deny.toml` is the policy, but nothing ran it (RT-10,
# open since iter-944). A policy nobody executes is a comment. This gate
# makes the check real on the machine that builds the binary, which is the
# only place it can run: the Actions quota is exhausted, so no workflow
# fires.
#
# WHY cargo audit AND NOT cargo deny. cargo-deny is not installed here and is
# not a build dependency, so a `cargo deny` gate would be a gate nobody can
# pass — which is worse than no gate, and is the exact failure mode
# `deny.toml` documents in its own header. `cargo-audit` IS installed
# (~/.cargo/bin/cargo-audit) and checks the thing that matters most: a
# published advisory against the committed lockfile. The licence and
# duplicate-version policies stay in `deny.toml` for a machine that has
# cargo-deny.
#
# THE ALLOWLIST IS NOT A BLANKET EXEMPTION. Each entry is one advisory id
# that has been read, triaged, and recorded in BUGS.md with its reason. A
# new advisory fails the gate. A fixed advisory is reported as stale, so it
# gets pruned. This is the same shape as the clippy gate
# (scripts/clippy-warning-gate.sh): the delta against a committed list is
# what fails, not the absolute count.
#
# Usage:
#   scripts/supply-chain-gate.sh            # check (fails on a new advisory)
#   scripts/supply-chain-gate.sh --update   # refresh the allowlist
#   scripts/supply-chain-gate.sh --report   # print the findings, never fails
set -euo pipefail

ALLOWLIST_FILE=".ci/advisory-allowlist.txt"
MODE="check"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --allowlist) ALLOWLIST_FILE="${2:?missing value for --allowlist}"; shift 2 ;;
        --update)    MODE="update"; shift ;;
        --report)    MODE="report"; shift ;;
        -h|--help)
            echo "Usage: scripts/supply-chain-gate.sh [--update|--report]"
            exit 0
            ;;
        *) break ;;
    esac
done

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

# json_is_readable FILE
# True when FILE holds JSON cargo-audit could have written.
#
# The test is "the advisory list is there", not "the file is non-empty":
# cargo-audit writes the JSON even when it exits non-zero because it
# found a vulnerability, and that is the case this gate exists to
# report. A file with no advisories section is an unreadable DB.
json_is_readable() {
    local file="${1:-}"
    [[ -s "$file" ]] || return 1
    if command -v jq >/dev/null 2>&1; then
        jq -e 'has("vulnerabilities") or has("warnings")' "$file" >/dev/null 2>&1
        return $?
    fi
    grep -q '"vulnerabilities"\|"warnings"' "$file"
}

# cargo-audit is not on PATH by default in this environment; it lives in
# ~/.cargo/bin. Look there too rather than failing with "command not found".
AUDIT_BIN="$(command -v cargo-audit || true)"
if [[ -z "${AUDIT_BIN}" && -x "${HOME}/.cargo/bin/cargo-audit" ]]; then
    AUDIT_BIN="${HOME}/.cargo/bin/cargo-audit"
fi
if [[ -z "${AUDIT_BIN}" ]]; then
    echo "[supply-gate] cargo-audit is not installed." >&2
    echo "[supply-gate] install it with: cargo install cargo-audit --locked" >&2
    exit 1
fi

tmp_json="$(mktemp)"
tmp_seen="$(mktemp)"
tmp_allow="$(mktemp)"
tmp_new="$(mktemp)"
tmp_stale="$(mktemp)"
trap 'rm -f "${tmp_json}" "${tmp_seen}" "${tmp_allow}" "${tmp_new}" "${tmp_stale}"' EXIT

echo "[supply-gate] auditing the lockfile..."
# The advisory DB is fetched on first run and cached after that.
#
# The JSON is the AUTHORITY, not the exit status. cargo-audit exits
# non-zero whenever it finds a vulnerability — which is the normal case
# this gate exists to report — so treating a non-zero exit as "the DB
# could not be read" sent every real finding down the fetch-retry branch
# and printed "the advisory database could not be read" WITHOUT ever
# naming the advisory. The one report the gate exists to produce was
# lost. (The script's own comment said the JSON was authoritative while
# the code did the opposite.)
#
# So: run the audit, and use the JSON whenever it PARSES. The exit status
# only decides whether to re-run with a fetch.
audit_ok=0
"${AUDIT_BIN}" audit --json --no-fetch > "${tmp_json}" 2>/dev/null || true
if ! json_is_readable "${tmp_json}"; then
    # --no-fetch fails when there is no cached DB. Retry with the fetch.
    "${AUDIT_BIN}" audit --json > "${tmp_json}" 2>/dev/null || true
    if json_is_readable "${tmp_json}"; then
        audit_ok=1
    fi
else
    audit_ok=1
fi

if [[ "${audit_ok}" -ne 1 ]]; then
    echo "[supply-gate] cargo-audit failed; the advisory database could not be read." >&2
    echo "[supply-gate] this is NOT a pass: an unreadable DB cannot clear the gate." >&2
    exit 1
fi

# Every advisory the audit reports, one id per line: a vulnerability OR an
# informational warning (unmaintained / unsound / notice). The kinds are
# kept so a maintainer can tell a real vulnerability from a staleness note.
#
# jq is not assumed: this script has to run on the build box, and jq is
# present there (the clippy gate already requires it), but the parse also
# falls back to python3 for a box without jq.
if command -v jq >/dev/null 2>&1; then
    # Two separate filters, not one filter with a top-level comma: the comma
    # form parses but yields nothing when the first branch produces no
    # output, which silently reported an empty audit as a clean pass.
    {
        jq -r '(.vulnerabilities.list // [])[] | "vulnerability|\(.advisory.id)|\(.package.name)@\(.package.version)"' "${tmp_json}"
        jq -r '(.warnings // {} | to_entries[] | .value[]) | "\(.kind)|\(.advisory.id)|\(.package.name)@\(.package.version)"' "${tmp_json}"
    } | LC_ALL=C sort -u > "${tmp_seen}"
else
    python3 - "${tmp_json}" > "${tmp_seen}" <<'PY'
import json, sys
data = json.load(open(sys.argv[1]))
rows = set()
for v in data.get("vulnerabilities", {}).get("list", []) or []:
    a, p = v["advisory"], v["package"]
    rows.add(f"vulnerability|{a['id']}|{p['name']}@{p['version']}")
for kind, items in (data.get("warnings") or {}).items():
    for w in items or []:
        a, p = w["advisory"], w["package"]
        rows.add(f"{kind}|{a['id']}|{p['name']}@{p['version']}")
for r in sorted(rows):
    print(r)
PY
fi

if [[ "${MODE}" == "report" ]]; then
    echo "[supply-gate] findings:"
    if [[ -s "${tmp_seen}" ]]; then
        sed 's/^/  /' "${tmp_seen}"
    else
        echo "  (none)"
    fi
    exit 0
fi

if [[ "${MODE}" == "update" ]]; then
    mkdir -p "$(dirname "${ALLOWLIST_FILE}")"
    cp "${tmp_seen}" "${ALLOWLIST_FILE}"
    count="$(wc -l < "${ALLOWLIST_FILE}" | tr -d ' ')"
    echo "[supply-gate] updated ${ALLOWLIST_FILE} (${count} entries)"
    exit 0
fi

if [[ ! -f "${ALLOWLIST_FILE}" ]]; then
    echo "[supply-gate] allowlist not found: ${ALLOWLIST_FILE}" >&2
    echo "[supply-gate] run: scripts/supply-chain-gate.sh --update" >&2
    exit 1
fi

# `grep -v` exits 1 when it selects nothing, which `set -e` would read as a
# script failure — an empty allowlist (a legal file that covers nothing)
# would abort the gate before it could report the finding.
grep -vE '^\s*(#|$)' "${ALLOWLIST_FILE}" | LC_ALL=C sort -u > "${tmp_allow}" || true
LC_ALL=C comm -23 "${tmp_seen}" "${tmp_allow}" > "${tmp_new}"
LC_ALL=C comm -13 "${tmp_seen}" "${tmp_allow}" > "${tmp_stale}"

seen_count="$(wc -l < "${tmp_seen}" | tr -d ' ')"
allow_count="$(wc -l < "${tmp_allow}" | tr -d ' ')"
new_count="$(wc -l < "${tmp_new}" | tr -d ' ')"
stale_count="$(wc -l < "${tmp_stale}" | tr -d ' ')"

echo "[supply-gate] seen=${seen_count} allowlist=${allow_count} new=${new_count} stale=${stale_count}"

if [[ "${new_count}" -gt 0 ]]; then
    echo "[supply-gate] NEW advisories detected:" >&2
    sed 's/^/  + /' "${tmp_new}" >&2
    echo "[supply-gate] triage each one and record it in BUGS.md before allowlisting it." >&2
    exit 1
fi

if [[ "${stale_count}" -gt 0 ]]; then
    echo "[supply-gate] stale allowlist entries (safe to prune via --update):"
    sed 's/^/  - /' "${tmp_stale}"
fi

echo "[supply-gate] pass"
