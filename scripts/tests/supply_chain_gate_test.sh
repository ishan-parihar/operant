#!/usr/bin/env bash
# scripts/tests/supply_chain_gate_test.sh — the supply-chain gate's contract.
#
# A gate that cannot fail is not a gate. This asserts the three properties
# that make scripts/supply-chain-gate.sh meaningful:
#
#   1. an allowlist that covers the findings -> pass
#   2. a finding NOT in the allowlist -> FAIL, and the finding is named
#   3. an allowlist entry that no longer appears -> reported as stale
#   4. an allowlist that does not exist -> FAIL with the command to create it
#
# The audit is stubbed: a fake `cargo-audit` writes a fixed JSON, so the test
# touches no network and no advisory database.
set -euo pipefail

SCRIPTS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

fail() { echo "FAIL: $*"; exit 1; }

# ── a fake cargo-audit ──────────────────────────────────────────────────────
# The stub prints the JSON the real cargo-audit prints, with the findings the
# test needs. It reads $FAKE_AUDIT_FINDINGS (one JSON array of
# {kind,id,name,version} objects) and the vulnerability list from
# $FAKE_AUDIT_VULNS.
mkdir -p "$TMP/bin"
cat > "$TMP/bin/cargo-audit" <<'EOF'
#!/usr/bin/env bash
# Fake cargo-audit. Ignores every flag; the script only reads stdout.
#
# It EXITS NON-ZERO when it reports a vulnerability, exactly as the real
# cargo-audit does. The gate used to treat a non-zero exit as "the
# advisory database could not be read", so every real finding went down
# the fetch-retry branch and the gate printed "the advisory database
# could not be read" WITHOUT naming the advisory — the one report the
# gate exists to produce. A stub that always exits 0 cannot catch that,
# so this one reproduces the real exit status.
findings="${FAKE_AUDIT_FINDINGS:-[]}"
vulns="${FAKE_AUDIT_VULNS:-[]}"
cat <<JSON
{
  "database": {"advisory-count": 1338},
  "lockfile": {"dependency-count": 765},
  "vulnerabilities": {"found": $([ "$vulns" = "[]" ] && echo false || echo true), "count": 0, "list": $vulns},
  "warnings": {"unmaintained": $findings}
}
JSON
# The real cargo-audit exits non-zero when it finds a vulnerability. The
# warnings (unmaintained) are not failures, so they do not change the
# status.
[ "$vulns" = "[]" ] || exit 1
exit 0
EOF
chmod +x "$TMP/bin/cargo-audit"

run_gate() {
    PATH="$TMP/bin:$PATH" \
    FAKE_AUDIT_FINDINGS="$1" \
    FAKE_AUDIT_VULNS="${2:-[]}" \
    bash "$SCRIPTS_DIR/supply-chain-gate.sh" --allowlist "$3" 2>&1
}

KNOWN='[{"kind":"unmaintained","advisory":{"id":"RUSTSEC-2025-0141"},"package":{"name":"bincode","version":"1.3.3"}}]'

# ── 1. a covered finding passes ────────────────────────────────────────────
printf 'unmaintained|RUSTSEC-2025-0141|bincode@1.3.3\n' > "$TMP/allow.txt"
out="$(run_gate "$KNOWN" '[]' "$TMP/allow.txt")" || fail "1. a covered finding failed the gate: $out"
echo "$out" | grep -q "pass" || fail "1. the gate did not report a pass: $out"
echo "$out" | grep -q "new=0" || fail "1. the gate counted a new advisory: $out"

# ── 2. an uncovered finding FAILS, and is named ────────────────────────────
# (an empty allowlist file is a legal file; it just covers nothing)
: > "$TMP/empty.txt"
out="$(run_gate "$KNOWN" '[]' "$TMP/empty.txt" 2>&1 || true)"
echo "$out" | grep -q "RUSTSEC-2025-0141" || fail "2. the failing gate did not name the advisory: $out"
echo "$out" | grep -q "new=1" || fail "2. the gate did not count the new advisory: $out"

# ── 2b. a VULNERABILITY also fails ─────────────────────────────────────────
# The stub exits NON-ZERO for this case, exactly as the real cargo-audit
# does. The gate must still NAME the advisory: a non-zero exit means
# "a vulnerability was found", not "the database could not be read".
vuln='[{"advisory":{"id":"RUSTSEC-2026-9999"},"package":{"name":"evil","version":"0.1.0"}}]'
out="$(run_gate '[]' "$vuln" "$TMP/empty.txt" 2>&1 || true)"
echo "$out" | grep -q "RUSTSEC-2026-9999" || fail "2b. a vulnerability was not reported: $out"
echo "$out" | grep -q "vulnerability|" || fail "2b. the vulnerability was not classified: $out"
echo "$out" | grep -q "could not be read" && fail "2b. a real finding was misreported as an unreadable DB: $out"

# ── 3. an allowlist entry with no finding is reported as stale ─────────────
printf 'unmaintained|RUSTSEC-2025-0141|bincode@1.3.3\nunmaintained|RUSTSEC-2000-0000|gone@1.0.0\n' > "$TMP/stale.txt"
out="$(run_gate "$KNOWN" '[]' "$TMP/stale.txt" 2>&1 || true)"
echo "$out" | grep -q "stale" || fail "3. a stale allowlist entry was not reported: $out"
echo "$out" | grep -q "RUSTSEC-2000-0000" || fail "3. the stale entry was not named: $out"
# A stale entry alone must NOT fail the gate — only a new one does.
echo "$out" | grep -q "pass" || fail "3. a stale entry failed the gate: $out"

# ── 4. a missing allowlist fails with the command to create it ─────────────
out="$(PATH="$TMP/bin:$PATH" FAKE_AUDIT_FINDINGS="$KNOWN" \
       bash "$SCRIPTS_DIR/supply-chain-gate.sh" --allowlist "$TMP/nope.txt" 2>&1 || true)"
echo "$out" | grep -q "allowlist not found" || fail "4. a missing allowlist did not fail clearly: $out"
echo "$out" | grep -q "update" || fail "4. the failure did not name the fix: $out"

echo "PASS: supply-chain gate"
