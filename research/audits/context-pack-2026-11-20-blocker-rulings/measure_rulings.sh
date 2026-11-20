#!/usr/bin/env bash
# measure_rulings.sh — re-derives every figure this round's pack states.
#
# Kept beside pack.spec so the figures are re-derivable rather than asserted
# (context-pack-review hard rule 8: ship the measurement with the round).
#
# Usage, from the subject repo root:
#   bash research/audits/context-pack-2026-11-20-blocker-rulings/measure_rulings.sh
#
# Every section prints the command it ran and that command's own output, so a
# reader can check a figure without trusting the prose that cites it.

set -uo pipefail
# The subject repo root: $1, else the script's own ../../.. (this file lives at
# <root>/research/audits/<round>/measure_rulings.sh), else the cwd.
# Deliberately NOT `git rev-parse --show-toplevel`: this round is built in a
# throwaway worktree whose .git is a file, and a subshell that loses the git env
# resolves it to nothing.
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="${1:-$(cd "$HERE/../../.." && pwd)}"
cd "$ROOT" || { echo "cannot cd to subject root: $ROOT" >&2; exit 1; }

echo "=== E1: the open ledger rows (RT-190, RT-280, RT-543 expected) ==="
echo "\$ rg -c '^- \\[ \\]' docs/audit/2026-06-19-BUGS.md"
rg -c '^- \[ \]' docs/audit/2026-06-19-BUGS.md
echo "\$ rg -c '^- \\[x\\]' docs/audit/2026-06-19-BUGS.md"
rg -c '^- \[x\]' docs/audit/2026-06-19-BUGS.md
echo "\$ rg -n '^- \\[ \\]' docs/audit/2026-06-19-BUGS.md | cut -c1-60"
rg -n '^- \[ \]' docs/audit/2026-06-19-BUGS.md | cut -c1-60
echo "\$ python3 scripts/audit-ledger-index.py >/dev/null; echo \$?"
python3 scripts/audit-ledger-index.py >/dev/null 2>&1; echo $?

echo
echo "=== E2: the RT-543 sharing pin exists (the premise correction) ==="
echo "\$ ls crates/operant-core/tests/rt543_one_socket_shares_one_mcp_child_set.rs"
ls crates/operant-core/tests/rt543_one_socket_shares_one_mcp_child_set.rs 2>&1
echo "\$ rg -n 'run_session_socket' crates/operant-cli/src/gateway_runner.rs | head -3"
rg -n 'run_session_socket' crates/operant-cli/src/gateway_runner.rs | head -3

echo
echo "=== E3: RT-280's writers have no production caller ==="
for f in suspend_session mark_resume_pending suspend_recently_active; do
  echo "\$ rg -n '$f' crates/ --glob '*.rs'"
  rg -n "$f" crates/ --glob '*.rs' | cut -c1-110
done
echo "\$ rg -c 'RT-394' crates/operant-core/src/gateway_session.rs   (the closed half)"
rg -c 'RT-394' crates/operant-core/src/gateway_session.rs

echo
echo "=== E4: the two memory banks, sized ==="
for f in "$HOME/.operant/memory_wire.sqlite" "$HOME/.local/share/memory-wire/memory.db"; do
  if [ -f "$f" ]; then
    echo "$f  $(stat -c%s "$f") bytes"
  else
    echo "$f  MISSING"
  fi
done

echo
echo "=== E5: the ACP daemon builds its own runtime ==="
echo "\$ rg -n 'AcpRuntime::start' crates/operant-cli/src/cmd_acp.rs"
rg -n 'AcpRuntime::start' crates/operant-cli/src/cmd_acp.rs

echo
echo "=== E6: enhanced-keys fallback surface (ASKS Q4) ==="
echo "\$ rg -n 'terminal-setup' crates/operant-cli/src/commands.rs"
rg -n 'terminal-setup' crates/operant-cli/src/commands.rs
echo "\$ rg -n 'enable_keyboard_enhancement' crates/operant-cli/src/tui/adapter_types/tui_app.rs | head -3"
rg -n 'enable_keyboard_enhancement' crates/operant-cli/src/tui/adapter_types/tui_app.rs | head -3

echo
echo "=== E7: the pub-fn queue (agent work, not a ruling) ==="
python3 scripts/audit-pub-fn-surface.py --check --limit 200 2>&1 | sed -n '1,7p'
echo "\$ ... tests=0 count:"
python3 scripts/audit-pub-fn-surface.py --check --limit 200 2>&1 | grep -cE 'tests=0 '

echo
echo "=== STATE.md: the counts it states, re-derived ==="
echo "\$ grep -c '^    \"' Cargo.toml   (rough member count; the gate is authoritative)"
bash scripts/check-doc-claims.sh 2>&1 | grep -E 'workspace members|CLI command variants|gateway platforms'

echo
echo "=== The gate battery STATE.md tabulates ==="
for g in "bash scripts/check-doc-claims.sh" "python3 scripts/audit-ledger-index.py" \
         "bash scripts/supply-chain-gate.sh" "./scripts/genome lint"; do
  printf '%-45s ' "$g"
  eval "$g" >/dev/null 2>&1
  echo "exit=$?"
done