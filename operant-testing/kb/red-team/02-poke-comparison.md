# The `poke` Feature (jcode) vs Operant

> **SUPERSEDED — READ `03-reconciliation-vs-upstream.md` FIRST.**
> This document is the record of what the sweep believed on 2026-11-05, *before*
> the reconciliation against `origin/main` @ `758dd4c5` (iter-1818). Upstream has
> since closed the central claim in this file: **the auto-poke WAS ported**
> (iter-1783, `operant-core/src/agent/auto_poke.rs`) and the wrap-up is now kept
> and continued (iter-1769). The claims below are kept because a gap register
> that hides what it once believed is not auditable — but they are NOT current truth.

## jcode's mechanism (external-repos/jcode)
Message construction: `crates/jcode-todo-policy/src/lib.rs:653` `build_auto_poke_message(n)`
  "You have N incomplete todos. Continue working, or update the todo tool."
  + `build_actionable_poke_message` (:664) appends up to `GATE_NAMED_TODO_LIMIT = 6` named todos.
Synthetic classification: `is_auto_poke_message` (:768) - persisted as `Role::User` so the model
  treats it as a normal continuation, but the renderer never mistakes it for something the user typed.
Producers (all agree on one text):
  - TUI auto-poke
  - `jcode run` auto-poke: `src/cli/commands/todo_followup.rs` -> `RunAutoPokeFollowUp::{Incomplete,
    GateDigest, ConfidenceSummary}`; returns None when nothing is actionable ("completing the loop with a
    generic summary just spends tokens on 'all good' theater").
  - Overnight coordinator: `OvernightAutoPokeState`/`OvernightAutoPokeFingerprint`
    (`crates/jcode-tui/src/tui/app.rs:715-740`) fingerprints run state, counts `stalled_turns` /
    `error_turns` / `total_pokes_sent`, and fires morning-report + final-wrap pokes.
Guard against loops: `crates/jcode-tui/src/tui/app/commands_auto_poke_errors.rs`
  `is_non_retryable_auto_poke_error` - deterministic 400 / context-length / invalid-model /
  invalid-parameter / content-policy / 401 / 403 / quota / billing / usage-limit markers are NOT
  re-poked, because "retrying the same auto-poke cannot help and can create an infinite spam loop".
Toggle: `ctrl+p` "Toggle auto-poke" (`crates/jcode-config-types/src/keybindings.rs:293`, `lib.rs:1035`).

## What `poke` actually is
A **bandaid for mid-stream dropping**: it does not fix why the wrap-up ran out of room; it simply
issues one more synthetic turn so the model keeps going while todos remain. It converts a dead stop
into a bounded, self-driving continuation with explicit non-retryable error handling.

## Operant today
Present and healthy:
  - honest exit reasons `TurnExitReason::{TextResponse, GraceCall, BudgetExhausted, CircuitBreaker,
    Interrupted, Error}` with per-reason operator notices
  - `attempt_grace_call` + S1 degenerate-output gate (one salvage attempt)
  - circuit breaker for repeated identical failing calls
  - the `todo` tool, the `background_tasks` view, and the gateway's stopped-early notice substitution
Missing (the gap):
  - NO synthetic continuation / auto-poke producer anywhere in `crates/` (grep for
    `auto_poke|synthetic|incomplete todo|"Continue working"` -> 0 hits in operant/session.rs / turn paths)
  - NO todo-gated resume: the `todo` list records intent but never triggers another turn
  - NO non-retryable-error classification for continuations, so any implementation ported from jcode
    must ship it alongside or it will spam failing calls
