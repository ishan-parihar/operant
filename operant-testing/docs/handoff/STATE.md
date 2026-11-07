# State

Current snapshot only (no history: that is the worklog). The first `## ` block is
what `genome wake` shows.

## State — 2026-11-06

**The knowledge base is reconciled to `origin/main` @ `758dd4c5` (iter-1818) and is now
the executable entry point for the next iteration.**

What is done:
- Every reported failure reproduced and classified into five layers —
  TOOL / LOGIC / NETWORK / CONFIG / **DEPLOY**. The DEPLOY layer was invisible to the
  first report and explains three of the fourteen rows.
- **Root cause found and fixed for the real defect (G-005, landed as iter-1822):** the MCP stdio client had
  no drain/resync path — one foreign stdout line wedged it for the process's life.
  iter-1822 (in-tree, 27/27 tests green, uncommitted pending Q2).
- Seven of the sweep's fourteen rows were already closed upstream and are marked
  `closed-upstream` with the closing commit and the ledger id, so nobody re-fixes them.
- The gap register is canonical (`G-001`…`G-014`), anchored to a pinned commit, carries
  the literal grep-able symptom per row, and a verify harness that re-tests every finding.

The operator then reported five TUI defects (goal, advisor preview, ctrl+enter queue,
prompt-history, input-area tool-tips). **All five are closed upstream** — iter-1839
(`15cec34e`), iter-1842 (`23015b38`), iter-1845 (`64094d60`) and iter-1797 (`2acd8efa`) —
and are recorded as G-015…G-019, `closed-upstream`, with their regression tests. The
operator is testing them now; the KB carries the anchors, and `verify.sh` check 8 fails
if any of the eight fix symbols disappears — so a regression is caught by the harness
rather than by the operator.

What is open, in order: **G-006** (widen the MCP `id` type) → **G-010** (cron TUI pane)
→ **G-007** (MCP tools inherit a 30 s timeout) → **G-009** (TUI/gateway parity,
canonical RT-179) → the three DEPLOY gaps →
**G-011**, **G-012**, **G-013**.

What needs the operator: Q1 (junk dirs) and Q2 (commit the MCP fix over peers' WIP).
