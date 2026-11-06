# Turn-Ended-Early: Root Cause & In-Harness Fix

> **SUPERSEDED — READ `03-reconciliation-vs-upstream.md` FIRST.**
> This document is the record of what the sweep believed on 2026-11-05, *before*
> the reconciliation against `origin/main` @ `43a38696` (iter-1818). Upstream has
> since closed the central claim in this file: **the auto-poke WAS ported**
> (iter-1783, `operant-core/src/agent/auto_poke.rs`) and the wrap-up is now kept
> and continued (iter-1769). The claims below are kept because a gap register
> that hides what it once believed is not auditable — but they are NOT current truth.

Source of truth (repo-verified, 2026-11-05):
- notice arm: `crates/operant-cli/src/tui/app/agent_events.rs:389-393` (GraceCall -> "wrap-up ran out of room mid-summary")
- producer: `crates/operant-core/src/agent/run.rs:157-221` (`attempt_grace_call`, emits `Done{reason: TurnExitReason::GraceCall}`)
- enum: `crates/operant-core/src/agent/turn_finalizer.rs:48-110` (`TurnExitReason`)
- gateway notice: `crates/operant-cli/src/gateway_runner.rs:3172` ("substituting stopped-early notice for the operator")

## The exact causal chain
1. Task scope exceeds `config.max_iterations` (default 8; `tests/nested_agent_run.rs:PARENT_MAX_ITERATIONS=8`).
2. The main loop burns every iteration on tool calls. No iteration is *reserved* for a complete final answer.
3. Budget exhausts -> `attempt_grace_call` fires ONE toolless summary request with the full message history.
4. That single request cannot hold a real wrap-up: it either truncates, or returns degenerate text that the S1 gate (`is_degenerate_final_text`) rejects -> empty message.
5. `Done{reason: GraceCall}` -> TUI prints the mid-summary notice. The operator is told to type `continue`. A new turn then re-enters the same budget; the work is not resumable in place, it just restarts.

## Compounding defect found THIS session (not previously reported)
The aft bridge keeps an **undelivered-completion backlog** and injects it into every non-bash tool response:
- `bash_tasks` ledger: 74 rows with `completion_delivered=0` (73 in session `__default__`).
- The `runner` harness has 5 completed tasks and has marked **0 of 5** delivered (`harness='runner'` -> 5 rows, `SUM(completion_delivered)=0`). Every `aft_bash` call therefore adds to a permanent, ever-growing backlog.
- Effect: `aft_read`, `aft_write`, `aft_glob`, `aft_outline`, `aft_status` return a truncated `bg_completions` array built from the oldest undelivered task **instead of their own payload**. `aft_bash` is immune only because its own envelope is the newest entry.
- Consequence for this exact bug: tool results come back empty/garbled, the model re-asks for the same fact, and iterations that should have produced the answer are consumed. The bridge's contract (one result per request) is broken by a delivery ack that is never written.

## In-harness remediation (ordered)
R1 (core, fixes the reported symptom): reserve a **dedicated wrap-up iteration budget** away from the tool-call budget, i.e. do not spend the last N iterations on tools; make the grace call run with a *tightened* context window (deca
yed history) so it can actually complete mid-summary. At minimum: emit `GraceCall` *before* every iteration is spent on tools, not after.
R2 (core, fixes the compounding defect): make the runner harness write `completion_delivered=1` on first delivery and cap/drain the backlog (hold only K recent), so non-bash tool payloads are never displaced.
R3 (TUI, the bandaid): add a **Todo-gated auto-poke** (see `02-poke-comparison.md`) so a GraceCall turn self-resumes when the agent's own `todo` list still has pending items, instead of stopping dead.
R4 (TUI): make the notice actionable in place - one keypress ("continue"-key) that resumes the same session with the remaining todo set, rather than a free-text re-entry that restarts the budget.
