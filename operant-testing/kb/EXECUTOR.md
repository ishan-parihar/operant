# EXECUTOR — the entry point

You are working `agentic-harness/operant`. This directory (`operant-testing/`) is the red-team
knowledge base. It is here so you do not re-derive the same conclusions.

## Read in this order

1. `kb/gap-register.md` — the canonical, source-anchored work queue (G-001…G-014).
2. `kb/red-team/04-tui-lifecycle-gap-map.md` — what the TUI still owes the operator.
3. `kb/red-team/05-mcp-correlation.md` — the one defect fixed here (iter-1822) and why.
4. `kb/red-team/03-reconciliation-vs-upstream.md` — what upstream already closed.
5. `kb/red-team/docs/06-verify.md` — the harness that re-tests all of this.

## Ground rules

- **The tree moves.** Every anchor here is pinned to `43a38696` (iter-1818). Before you edit,
  `git show 43a38696:<path> | sed -n '<line>p'` and confirm. Re-anchor after you sync.
- **`origin/main` is the only numbering authority** (`AGENTS.md §Iteration Numbering`).
  `git fetch origin; git log origin/main -1 --pretty=%s` → read the top `iter-N`, use `N+1`.
  Never derive it from local history: peers push concurrently.
- **Stage explicit paths, never `git add -A`.** The shared tree carries peers' in-flight WIP.
- **One unrelated change = one commit = one push.** Check
  `git rev-list --left-right --count origin/main...HEAD` is `0<TAB>1` before pushing.
- **A row is only `closed-upstream` when `docs/audit/2026-06-19-BUGS.md` carries a
  `CLOSED iter-N` cell.** Otherwise it is open, however plausible.

## Priority order for the next iteration

1. **G-006** — narrow, self-contained, high value: widen the client's `id` to `serde_json::Value`.
2. **G-010** — one cron overlay registered in `App::overlay_flags()`; closes the lifecycle asymmetry.
3. **G-007** — MCP-backed tools must not inherit the 30 s tool timeout.
4. **G-009** — the TUI/gateway command-parity sweep (canonical RT-179).
5. **G-004/G-008/G-013** — deploy/config, not code: re-deploy the bridge, prefix the cron job models.
6. **G-005** — DONE: landed and pushed as iter-1822 (`092f53a0`).
