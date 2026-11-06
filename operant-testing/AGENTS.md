# AGENTS.md — the entry contract

Orientation, the gate, and **which document answers which question**.
Start every session with `genome wake`; find an answer with `genome ask "<question>"`;
end every iteration with `genome close`.

## What this project is

The red-team knowledge base for the **operant agentic-core** (`agentic-harness/operant`).
It holds the reconciled, source-anchored register of that harness's open gaps, the
evidence for every finding, and a harness that re-tests them. It exists so that the
next agent to work the agentic-core starts from what is already known instead of
re-deriving it — and so that nothing already fixed upstream gets re-fixed.

## Architecture

Three layers, each with one owner:

- **The register** (`TODO.md`, detail in `kb/gap-register.md`) — what is open, in what
  order. Rows update in place; verdicts append.
- **The evidence** (`kb/red-team/`) — dated findings: the raw round-1 reports, the
  reconciliation against upstream, the MCP root cause, the TUI gap map, the harness.
- **The lifecycle** (`docs/kb/WORKLOG.md`, `docs/kb/CLOSED.md`, `docs/kb/ASKS.md`,
  `docs/kb/HYPOTHESES.md`) — what happened, what was rejected and why, what the operator
  ruled on, what was tested.

`docs/INDEX.md` is the ownership map: one owner per question.

## Gate

```
cargo test -p operant-core --lib mcp::     # the drain fix and the iter-187 guard
bash operant-testing/kb/red-team/docs/verify.sh   # every finding in this KB
python3 scripts/genome lint                 # the KB's own structural gate
```

## Routing table — which document answers what

| question | answer lives in |
|---|---|
| what is this / how to build / the rules (for agents) | this file |
| where do I start, what are the rules | `kb/EXECUTOR.md` |
| which gaps are open and where exactly in the source | `kb/gap-register.md` |
| what did upstream already close | `kb/red-team/03-reconciliation-vs-upstream.md` |
| what does the TUI still owe the operator | `kb/red-team/04-tui-lifecycle-gap-map.md` |
| why did the MCP tools wedge, and what fixed it | `kb/red-team/05-mcp-correlation.md` |
| what can this agentic-core actually do | `kb/feature-inventory.md` |
| what is structurally weak | `kb/blind-spots.md` |
| how do I re-test every finding | `kb/red-team/docs/06-verify.md` |
| what is next, in what order | `TODO.md` |
| where does the project stand right now | `docs/handoff/STATE.md` |
| what happened, one entry per unit of work | `docs/kb/WORKLOG.md` |
| what was tried and rejected | `docs/kb/CLOSED.md` |
| what did the operator rule on | `docs/kb/ASKS.md` |
| what did we test, and what did we find | `docs/kb/HYPOTHESES.md` |
| what was the raw round-1 evidence | `kb/red-team/01-round1-reports/` |

## The rules that are not in the parent repo's AGENTS.md

1. **The tree moves.** Every `file:line` in this KB is pinned to `43a38696` (iter-1818).
   Confirm it with `git show 43a38696:<path> | sed -n '<line>p'` before editing the code
   it points at. `verify.sh` reports any anchor that has drifted.
2. **A row is only `closed-upstream` when the parent repo's canonical defect ledger
   (agentic-harness/operant/docs/audit/2026-06-19-BUGS.md) carries a `CLOSED iter-N`
   cell.** The rows that matter are quoted in `kb/red-team/03-reconciliation-vs-upstream.md`,
   which lives here and resolves. Seven of the first report's fourteen rows were already closed
   when the reconciliation ran. Re-reconcile before you work a row.
3. **A source fix is not a deployed fix.** Three rows (G-004, G-008, G-013) are broken
   in the running system while the source is correct. Check the layer column before
   you go looking for a bug in the code.
4. **Never `git add -A`.** The shared tree carries peers' in-flight WIP. Stage explicit
   paths.
