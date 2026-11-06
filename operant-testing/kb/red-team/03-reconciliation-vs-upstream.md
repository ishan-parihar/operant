# Reconciliation — what upstream already closed

**Reconciled to `origin/main` @ `43a38696` (iter-1818).**
A row counts as closed only when `docs/audit/2026-06-19-BUGS.md` carries a `CLOSED iter-N` cell.

| Was reported by the sweep | Upstream verdict | Closing commit | What actually changed |
|---|---|---|---|
| "Turn ended early — the wrap-up ran out of room mid-summary" | **RT-234 CLOSED iter-1783** | `8264cdea` | The notice now names the cause that happened: `response_len == 0` → "the wrap-up produced no usable summary"; otherwise "ran out of room". A **bounded auto-poke** (`operant-core/src/agent/auto_poke.rs`) continues the turn itself; the operator no longer has to type `continue`. `operant-core/tests/auto_poke.rs` (425 lines) pins it. |
| "The grace call was the only recovery and it was not persisted" | **RT-225 CLOSED iter-1769** | `6bc1b7af` | The wrap-up is kept in the conversation and the transcript; a provider that cuts it off is continued (bounded at 2); a failed wrap-up reports through the same diagnostics every other turn failure uses. Context soft/hard caps landed too (`config.rs:671`). |
| "A delegation that produced nothing looked like success" | **RT-249 CLOSED iter-1800** | `c1870d71` | `sub_agent_tool.rs:1404` — "The sub-agent finished its turn without producing any output" is a **named failure**, not an empty result. |
| "`aft_inspect` answered `unknown error`" | **RT-247 CLOSED iter-1799** | `e08a643c` | The bridge's error path now reads the child's 64-line stderr ring instead of the literal "unknown error". **The source is fixed; the deployed bridge binary (v0.63.5) is not.** → G-004 is a DEPLOY gap. |
| "The TUI mouse gate named ten overlays; nineteen fell through" | **RT-251 CLOSED iter-1802** | `d5c67fb6` | The gate derives from `App::overlay_flags()` — the ONE catalogue (29 entries). Any new pane must be registered there, not in a second list. → the rule G-010 must follow. |
| "A failing tool call lost the operator's real reason" | **RT-247 family, iter-1799** | `e08a643c` | The child's stderr is never lost again. |

## What upstream did NOT close — the sweep's remaining contributions

| Contribution | Where it lives now |
|---|---|
| The MCP stdio client had **no drain/resync path** — one foreign stdout line wedged it permanently | G-005; **fixed and pushed as iter-1822** (`092f53a0`) — the drain loop, the parked-response buffer and the bounded drain |
| The MCP client's `id` is `u64`; the server's is `Value` — a string-echo server cannot be deserialised | G-006 |
| Two timeout budgets disagree: tool 30 s vs MCP HTTP 60 s — every MCP tool is cut off early | G-007 |
| A source fix is not a deployed fix — three distinct DEPLOY gaps | G-004, G-008, G-013 |
| The TUI is not yet the whole interface: no cron pane; 72 commands without a gateway twin | G-009, G-010 |
| Index-backed search silently degrades to a capped walk | G-011 |

## Consequence for the executor

Seven of the fourteen rows in the sweep's first report were **already fixed** by the time this
reconciliation ran. A gap register that is not re-reconciled against `origin/main` before
each iteration sends the executor to re-fix closed work. Run this reconciliation first, every time.
