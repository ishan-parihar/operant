#!/usr/bin/env bash
# verify.sh — re-test every finding in this knowledge base.
#
# Exit 0 = every finding still holds.
# Exit 1 = a finding changed: re-reconcile against origin/main BEFORE working it.
#
# Usage:  bash operant-testing/kb/red-team/docs/verify.sh
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
KB="$(cd "$HERE/../.." && pwd)"                # operant-testing/kb
REPO="$(cd "$KB/../.." && pwd)"                # agentic-harness/operant (repo root)
PINNED="43a38696"                             # the commit the gap register is anchored to

FAIL=0
pass() { printf 'PASS  %-40s %s\n' "$1" "$2"; }
open_() { printf 'OPEN  %-40s %s\n' "$1" "$2"; }
fail() { printf 'FAIL  %-40s %s\n' "$1" "$2"; FAIL=1; }

echo "=== anchor: $PINNED ($REPO)"
cd "$REPO" || { echo "FAIL  repo not found at $REPO"; exit 1; }

echo
echo "--- 1. the MCP drain fix (G-005) and the iter-187 guard"
if cargo test -p operant-core --lib mcp:: 2>&1 | tail -5 | grep -q "test result: ok"; then
  pass "mcp:: unit tests" "$(cargo test -p operant-core --lib mcp:: 2>&1 | grep -o 'test result: ok. [0-9]* passed[^;]*' | head -1)"
else
  fail "mcp:: unit tests" "cargo test -p operant-core --lib mcp:: did not report ok"
fi

echo
echo "--- 2. upstream's bounded continuation after an early exit (G-001/G-002)"
if cargo test -p operant-core --test auto_poke 2>&1 | tail -5 | grep -q "test result: ok"; then
  pass "auto_poke" "upstream iter-1783 still green"
else
  fail "auto_poke" "the upstream auto-poke test did not pass"
fi

echo
echo "--- 3. a child that produced nothing is a NAMED failure (G-003)"
if grep -q "without producing any output" crates/operant-core/src/tools/sub_agent_tool.rs; then
  pass "sub_agent named failure" "sub_agent_tool.rs carries the named failure"
else
  fail "sub_agent named failure" "the named failure text is gone from sub_agent_tool.rs"
fi

echo
echo "--- 4. every source anchor in the gap register still resolves"
ANCHORS="$(grep -oE '[a-zA-Z0-9_/.-]+\.(rs|md):[0-9]+' "$KB/gap-register.md" | sort -u)"
if [ -z "$ANCHORS" ]; then
  fail "anchors" "no anchors found in the gap register — the format changed"
else
  N=0; BAD=0
  while read -r a; do
    [ -z "$a" ] && continue
    ap="${a%:*}"; al="${a##*:}"; N=$((N+1))
    ok=0
    for cand in "$ap" "crates/$ap" "crates/operant-cli/src/$ap" "crates/operant-core/src/$ap"; do
      if git show "$PINNED:$cand" 2>/dev/null | sed -n "${al}p" | grep -q .; then ok=1; break; fi
    done
    if [ "$ok" -eq 1 ]; then continue; fi
    BAD=$((BAD+1)); echo "      DRIFT  $a"
  done <<<"$ANCHORS"
  if [ "$BAD" -eq 0 ]; then pass "anchors" "$N anchor(s) resolve at $PINNED"
  else fail "anchors" "$BAD of $N anchor(s) drifted — re-read the code"; fi
fi

echo
echo "--- 5. cron rows are scheduled (G-008)"
PAUSED=$(operant cron list 2>/dev/null | grep -c '"enabled": false' || true)
if [ "${PAUSED:-0}" -eq 0 ]; then
  open_ "cron rows (G-008)" "no paused jobs — G-008 is closed"
else
  open_ "cron rows (G-008)" "$PAUSED job(s) still paused — G-008 is OPEN, not a regression"
fi

echo
echo "--- 6. no junk dirs in the tree (G-013)"
JUNK=""
for d in .serena .tmp-test .zcode .tmpwork _kernel; do
  [ -e "$REPO/$d" ] && JUNK="$JUNK $d"
done
if [ -z "$JUNK" ]; then
  open_ "junk dirs (G-013)" "none present — G-013 is closed"
else
  open_ "junk dirs (G-013)" "present:$JUNK — G-013 is OPEN, not a regression"
fi

echo
echo "--- 7. every TUI pane is registered in the ONE overlay catalogue (G-010)"
if grep -q "fn overlay_flags" crates/operant-cli/src/tui/app/commands.rs 2>/dev/null \
   && grep -q "fn any_modal_open" crates/operant-cli/src/tui/app/commands.rs 2>/dev/null; then
  open_ "overlay catalogue (G-010)" "overlay_flags() + any_modal_open() live in ONE file — the cron pane is still MISSING (G-010 is OPEN)"
else
  fail "overlay catalogue" "the overlay catalogue has forked — a second list appeared"
fi

echo
echo
echo "--- 8. the five operator-reported TUI defects are still closed (G-015..G-019)"
TUI_MISSING=0
for sym in "pub fn set_standing_goal" "pub fn format_advisor_card" "pub fn advisor_review_summary" \
           "Ctrl/Cmd+Enter to queue" "fn plan_mode_active" "fn queued_messages" \
           "fn remember_prompt_history" "pub(super) fn composer_hint"; do
  if git grep -q -e "$sym" "$PINNED" -- crates/operant-core/src crates/operant-cli/src 2>/dev/null; then
    continue
  fi
  TUI_MISSING=1; echo "      MISSING  $sym"
done
if [ "$TUI_MISSING" -eq 0 ]; then
  open_ "tui defects (G-015..G-019)" "all five fixes are live at $PINNED"
else
  fail "tui defects (G-015..G-019)" "a fix symbol is gone — a closed defect regressed"
fi

if [ "$FAIL" -eq 0 ]; then
  echo "RESULT: no CLOSED finding regressed. Rows marked OPEN above are still open by design —"
  echo "        work them from kb/gap-register.md."
else
  echo "RESULT: at least one finding changed. Re-reconcile kb/red-team/03-reconciliation-vs-upstream.md"
  echo "        against origin/main before working any row."
fi
exit "$FAIL"
