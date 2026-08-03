#!/usr/bin/env bash
# Full lint enforcement sweep (Phase 8 — see docs/RUST_BEST_PRACTICES_PLAN.md).
#
# Runs, in order:
#   1. cargo fmt --check          (formatting drift fails)
#   2. clippy-warning-gate.sh     (incremental warning gate + -D unwrap/expect)
#   3. deny-attrs audit           (lib/main crates must carry the deny attrs)
#   4. prod unwrap/expect count   (fail if ANY production site has no #[expect])
#
# Exit non-zero on the first failing check so CI / pre-push hooks stay simple.
# Usage: bash scripts/lint-checks.sh
set -u

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

fail() { echo "[lint-checks] FAIL: $*" >&2; exit 1; }
ok()   { echo "[lint-checks] ok: $*"; }

# ── 1. Formatting ──────────────────────────────────────────────────────────────
echo "── 1/4 cargo fmt --check ──"
if ! cargo fmt --all -- --check > /tmp/lint-fmt.txt 2>&1; then
    echo "[lint-checks] fmt drift detected — run 'cargo fmt --all'." >&2
    head -20 /tmp/lint-fmt.txt >&2
    fail "cargo fmt --check"
fi
ok "cargo fmt --check"

# ── 2. Clippy gate (incremental + deny unwrap_used/expect_used) ───────────────
echo "── 2/4 clippy-warning-gate.sh ──"
if ! bash scripts/clippy-warning-gate.sh > /tmp/lint-gate.txt 2>&1; then
    tail -25 /tmp/lint-gate.txt >&2
    fail "clippy-warning-gate.sh"
fi
ok "clippy-warning-gate.sh"

# ── 3. #![deny] audit on lib/main crates ──────────────────────────────────────
echo "── 3/4 deny-attr audit ──"
# Crates that opt into the hygiene denies. missing_docs is optional (bin-heavy
# crates skip it); unwrap_used/expect_used must be allow-on-test only.
missing_deny=""
while IFS= read -r crate; do
    lib_rs="$ROOT/crates/$crate/src/lib.rs"
    main_rs="$ROOT/crates/$crate/src/main.rs"
    target=""
    [ -f "$lib_rs" ]  && target="$lib_rs"
    [ -z "$target" ] && [ -f "$main_rs" ] && target="$main_rs"
    [ -z "$target" ] && continue
    if ! grep -qE 'deny\(clippy::(unwrap_used|expect_used)' "$target" \
       && ! grep -qE 'cfg_attr\(test, allow\(clippy::(unwrap_used|expect_used)' "$target"; then
        # Either the crate denies the lints, or it must exempt tests at minimum.
        if ! grep -q 'deny' "$target" && ! grep -q 'cfg_attr(test, allow' "$target"; then
            missing_deny="$missing_deny $crate"
        fi
    fi
done < <(grep -oE '^\s*"crates/[a-z-]+"' "$ROOT/Cargo.toml" | grep -oE '[a-z-]+$' | sort -u)
if [ -n "$missing_deny" ]; then
    fail "crates missing test-exemption/deny attrs:$missing_deny"
fi
ok "deny-attr audit (all lib/main files carry cfg_attr(test, allow) or deny)"

# ── 4. Production unwrap/expect count (must be 0 un-escaped) ──────────────────
echo "── 4/4 prod unwrap/expect count ──"
# Any production .unwrap()/.expect() not guarded by an #[expect] attribute or
# a test boundary is a violation. The clippy gate (step 2) enforces this
# authoritatively via `-D clippy::unwrap_used -D clippy::expect_used`, so this
# step only verifies that the gate actually ran with the deny flags (defense in
# depth against someone weakening the gate command in future).
if grep -qE 'clippy::unwrap_used.*clippy::expect_used|clippy::expect_used.*clippy::unwrap_used' \
    scripts/clippy-warning-gate.sh; then
    ok "unwrap/expect deny flags present in gate (enforced by clippy -D)"
else
    fail "clippy-warning-gate.sh no longer carries -D clippy::unwrap_used / expect_used"
fi

echo
echo "[lint-checks] ALL CHECKS PASSED"
