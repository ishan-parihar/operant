# Agent-loop fix execution plan — the five measured defects

**Date**: 2026-10-03
**Baseline**: `origin/main` @ `3ea2d6df` (iter-587)
**Parent authority**: `docs/OPENHUMAN-AGENTIC-LOGIC-PORT-PLAN.md` §1 (evidence) and §4 Wave 0 (scope). This document is the executable version: exact edits, acceptance tests, ownership, sequencing. The no-progress ladder (Wave 2 of the port plan) is the structural backstop that makes this class of bug unrepresentable afterward; this plan is the immediate repair.

**Target loop**: Loop A — `OperantAgent::run()` (`operant-core/src/agent/run.rs:197-1749`). Verified as the path every stuck Telegram run takes (`gateway_runner.rs:736`, `match self.agent.run(query)`). Loop B (`operant-runtime/src/agent/agent.rs:1516/:1757`, WS + ACP) is **not touched here**; it inherits all five fixes via the plan-006 reconciliation, proven by the parity harness. Porting any of these five to Loop B by hand would rebuild the double-maintenance defect this programme exists to remove.

**Every line number below was re-verified against the baseline working tree on 2026-10-03.**

---

## 0. The five defects (one line each; full evidence in iter-587 §1)

| ID | Defect | Site (verified) |
|---|---|---|
| S1 | Grace-call output returned unvalidated; 13-char reasoning-leak garbage counts as "answered" | `run.rs:102-104` |
| S2 | Tool timeout → ordinary retryable result; no per-tool breaker; 11 `aft_bash` timeouts in one turn | `tools.rs:342-349`, dispatch `stream.rs:1068-1075` |
| S3 | Background review fires up to 4× inside one long turn (measured 05:10 run), all no-op noise, invisible value | `run.rs:1728-1731`, `prompting.rs:174-180`, review loop in `agent/background_review.rs` |
| S4 | Unrepairable args → `"{}"` → guaranteed schema-validation failure next iteration; 7,676 log lines in one window | `message_safety.rs:171-177`, callers `stream.rs:325/:563` |
| S5 | Non-`TextResponse` exits invisible to the operator; breakers masquerade as `TextResponse` in diagnostics | `turn_finalizer.rs:50-59`, `gateway_runner.rs:736-783` |

---

## 1. Slice A (first) — F1 grace gate + F5 terminal truth

One owner; two files' regions in `run.rs` plus the reason plumbing, landed as **one iteration each (A1, A2)** because `run.rs` is shared.

### A1 — Grace-output quality gate (S1)

**Change** (`run.rs:102-104`, the `Ok((text, reasoning))` arm of `attempt_grace_call`):

1. New pure helper in `turn_rules.rs` (the module both loops already share):
   ```rust
   /// True for output that must not be surfaced as a turn's final answer:
   /// empty per `AssistantTurn::is_empty`, or a short whitespace-free
   /// artifact (the observed class: `]<]\u{200b}minimax[>[`, 13 chars —
   /// reasoning-leak markers, not prose). Deliberate ceiling: a genuine
   /// sub-80-char single-word summary is sacrificed; the operator gets the
   /// named-reason stopped notice instead, which is strictly more honest.
   pub fn is_degenerate_final_text(text: &str) -> bool {
       let t = text.trim();
       t.is_empty() || (t.len() < 80 && !t.chars().any(char::is_whitespace))
   }
   ```
2. In the grace `Ok` arm: if `is_degenerate_final_text(&text)` → return `Message::assistant(String::new())` instead of `Message::assistant(&text)`. **Empty is the integration**: `gateway_runner.rs:738-762` already substitutes the user-facing fallback whenever `content.trim().is_empty()` — no new gateway wiring needed for A1. The TUI's empty-`Done` handling (R32) is already correct for this shape.

**Acceptance** (fault-injected, in `turn_rules` tests + a `run.rs`-adjacent integration test):
- `is_degenerate_final_text` unit tests: empty → true; `]<]minimax[>[`-class garbage → true; 942-char real summary → false; multi-sentence short answer ("Done. Two files changed.") → false.
- Scripted-provider test: budget exhausted, grace returns garbage → the returned `Message` is empty; grace returns valid text → the message carries it. **Positive control included** (valid-grace case) — a gate test that cannot pass on valid input measures nothing.

### A2 — Exit reasons tell the truth + operator notice (S5)

**Changes**:
1. `turn_finalizer.rs:50-59`: add variants — `GraceCall` (all three grace sites) and `CircuitBreaker` (both in-band aborts). Update `Display` (`"grace_call"`, `"circuit_breaker"`).
2. `mod.rs:213`: `Done { message: Message }` → `Done { message: Message, reason: TurnExitReason }`. Consumer sites to update in the same iteration (grep-verified): `cmd_tui_debug.rs:1435` (mock), `gateway_runner.rs:1919` (event consumer), `tui/app/agent_events.rs:228`, plus test constructors in `agent/mod.rs` tests and `cmd_tui_debug.rs` mocks. Clean cutover — no shim field, no deprecated alias.
3. Emit sites in `run.rs`: the three grace returns (`:389`, `:408-411`, `:472-474`) and the two breaker aborts (`:1407+`, `:1412` abort_msg construction region) stamp the new reasons; the normal exit (`:1163-1200`) stamps `TextResponse`.
4. `gateway_runner.rs:736-783` (the `Ok(response)` arm): read the reason from the consumed `Done` event (the event consumer at `:1919` already receives it; stash it on the handler for the turn-end block). When `reason != TextResponse`, the fallback/substitution message becomes: `"⚠️ I stopped early after {elapsed} — {reason word}: {partial answer if non-degenerate, else one-line status}"`. Today's wording (`turn_rules.rs:88` comment: "provider returned an empty response after retries") stays for the true-empty mid-conversation case so hermes parity tests keep their meaning.

**Acceptance**:
- Diagnostics test: garbage-grace turn reports `reason=grace_call` in `TurnDiagnostics`, not `text_response`.
- Breaker path reports `circuit_breaker`.
- Gateway-level test (scripted provider + handler harness): non-`TextResponse` exit produces an operator-visible message containing the word "stopped" and the elapsed time. This is the test that would have caught the 25-day silence-after-degenerate-warning specimen.

---

## 2. Slice B (second) — F2 timeout breaker + F4 honest invalid-args

One owner; touches `tools.rs`, `stream.rs`, `message_safety.rs`, plus a `ToolResult` field. Serialized after Slice A only because both touch `run.rs`/`mod.rs` regions Slice A edits; the slices are otherwise independent.

### B1 — Per-tool consecutive-timeout circuit breaker (S2)

**Changes**:
1. `ToolResult` gains `pub timed_out: bool` (default `false`), set **only** in `tools.rs:342-349` (`execute_with_timeout`'s `Err(_)` arm). One field, one writer — the dispatch at `stream.rs:1068-1075` maps it through like success/error.
2. Turn-scoped breaker in `run.rs`, mirroring the existing `consecutive_failed_iters`/`identical_streak` locals (`:1385-1406`, the established pattern): a local `HashMap<String, u32> timeout_streak` + `HashSet<String> masked_tools`, updated in the post-dispatch block. On the 2nd consecutive timeout of a tool: append a **system** message ("`{tool}` timed out twice in a row — it is disabled for the rest of this turn; take a different approach") — a nudge, not a kill. On the 3rd: add to `masked_tools`.
3. `tools_for_turn` (`run.rs:55-69`) — mask: `run()` filters the returned schemas by `masked_tools` before each request. The mask is turn-local (a fresh `HashSet` per `run()` call); the registry itself is untouched, so other sessions and later turns are unaffected.
4. Success resets the streak (same line as the `consecutive_failed_iters = 0` reset).

**Acceptance** (fault-injected):
- Scripted tool that times out 3× → the 3rd request's schema list excludes it; a 4th model attempt to call it yields `ToolNotFound`, and the model has already been told why (the nudge).
- timeout → success → timeout → streak is 1, not 2 (reset proven).
- The 2026-10-03 specimen shape (11 timeouts of one tool in a turn) becomes: 2 timeouts, one nudge, mask on the 3rd, turn continues on other tools. **This is the behavioral delta the test asserts.**

### B2 — Unrepairable args surface as a tool error, never `{}` (S4)

**Changes**:
1. `message_safety.rs:37`: `repair_tool_call_arguments(&str, &str) -> String` becomes `-> RepairOutcome` where `RepairOutcome` is `{ Fixed(String), Unrepairable }`. `:171-177` returns `Unrepairable` instead of `"{}"`. The empty/whitespace-only fast path (`:38-40`, "empty → `{}`") **stays** `Fixed("{}")` — a model that legitimately sends no args is not the defect; the defect is destroyed args.
2. Both call sites (`stream.rs:325`, `stream.rs:563`): on `Unrepairable`, do not execute. Synthesize `ToolResult::error_with_name(name, call_id, "Arguments could not be parsed and were NOT executed: <first 80 chars, passed through the existing redactor>")` and treat the call as answered (skip phase-2 execution for it). One round trip, and the model gets an error it can adapt to — the opposite of the measured cascade (1,938 substitutions → 522 validation failures → dedupe skips → repeat).
3. Delete the now-dead warn at `message_safety.rs:173-176` (replaced by the result itself carrying the message).

**Acceptance**:
- The S4 cascade test: malformed `process` args produce exactly ONE error result in the transcript and the model's next turn sees the error text — assert no `"{}"`-validated call, no `Missing required field` round.
- Existing repair tests (`message_safety.rs:566-670`) migrate to `RepairOutcome::Fixed` asserts; **the `test_repair_tool_call_arguments_none` case ("None" → `{}`) moves to Unrepairable** — that is the behavior change, and the test rename records it.

---

## 3. Slice C (third) — F3 review discipline (S3)

One owner; touches `run.rs` tail + `agent/background_review.rs`.

**Changes**:
1. **Once per turn**: `run()` holds a local `review_fired: bool`; the skill-review spawn at `run.rs:1728-1731` fires only when `!review_fired`. The per-iteration cadence counter semantics are preserved across turns (session metadata persistence at `:1720-1724` untouched) — only the *within-turn* repetition is capped. Measured pathology: 4 reviews in one 21-minute turn, all no-op. Ceiling recorded: a 90-iteration turn gets at most one skill review — deliberate.
2. **Defer to turn exit**: move the spawn to the exit path — after the loop breaks, alongside `spawn_session_distillation` (`run.rs:1163-1200` region), not before the next iteration. A review racing the turn for provider slots mid-loop is the measured interference; post-turn it reviews a complete transcript instead of 4 partial ones.
3. **No-op early exit**: in the review daemon (`background_review.rs`), stop after 2 consecutive rounds with zero memory/skill writes (the `{"count":0}` specimen). The daemon's own `iters: 10, interval: 10` budget remains the outer cap.

**Acceptance**:
- Scripted 30-iteration turn with review interval 10: exactly ONE review spawn, and it happens after the turn's final response, not between iterations.
- Review-daemon unit test: two consecutive no-write rounds → daemon exits before its iteration cap.
- The 05:10 specimen becomes: 1 post-turn review instead of 4 in-turn reviews; the 1200s wall clock the turn actually experienced shrinks by the review interference that no longer competes.

---

## 4. Sequencing and ownership

| Order | Slice | Iterations | Files owned (nothing else) |
|---|---|---|---|
| 1 | A1 grace gate | 1 | `agent/turn_rules.rs`, `agent/run.rs:102-115` region |
| 2 | A2 exit truth + notice | 1 | `agent/turn_finalizer.rs`, `agent/mod.rs` (`Done` variant + tests), `agent/run.rs` (3 grace sites + 2 breaker sites + emit sites), `gateway_runner.rs:736-783/:1919`, `cmd_tui_debug.rs:1435`, `tui/app/agent_events.rs:228` |
| 3 | B1 timeout breaker | 1 | `tools.rs` (`ToolResult` + `:342-349`), `agent/stream.rs` (result mapping), `agent/run.rs` (streak locals + mask + `tools_for_turn` filter) |
| 4 | B2 honest invalid-args | 1 | `agent/message_safety.rs`, `agent/stream.rs:325/:563` |
| 5 | C review discipline | 1 | `agent/run.rs:1163-1200/:1728-1731`, `agent/background_review.rs` |

A1 before A2 (A2's gateway notice wording depends on which grace texts A1 lets through). B after A because both edit `run.rs`/`stream.rs` regions A touches — no merge contention. Each iteration: `cargo fmt --all`, `./scripts/check.sh check -p operant-core --lib` (+ `operant-cli --bin operant` on A2), the slice's fault-injected test, `cargo clippy -p <crate> --all-targets -- -D warnings`, commit + push (Iron Rule).

**BUGS.md**: each slice appends its entry (fixed-on-landing), and one cleanup entry corrects the stale OPEN headers for D-1/D-1b/D-5 (verified fixed at iters 570/571/577) — stale headers already sent one scout re-planning a fixed bug this week.

---

## 5. Verification

1. **Fault-injection over mocking** — every acceptance test above scripts the failure (garbage grace text, timing-out tool, malformed args, no-op review rounds). A test that cannot fail measures nothing (2026-09-29 rule).
2. **Live smoke after each slice** (the specimen protocol): gateway against Telegram — user message → tool use → bounded exit; verify the *operator-visible* message, not the log line. After A2: kill the gateway mid-turn, restart, and confirm the notice arrives (the 25-day-silence class).
3. **Suite gate**: `cargo test -p operant-core --lib` green (baseline 2235 passed / 2 failed — the 2 are the pre-existing `tools::kernel` K-1 failures, unowned, unchanged by this programme; a slice that turns them into 3 has broken something).
4. **No Loop B edits**: `git diff origin/main -- crates/operant-runtime` must be empty across all five iterations.

## 6. Explicitly out of scope (pointers, not re-plans)

- **Loop B parity + migration** → plan 006, executed as port-plan Wave 1.
- **No-progress ladder, halt summaries, failure classification** (the structural fix that retires `consecutive_failed_iters`/`identical_streak` in favor of fingerprints + classification) → port-plan Wave 2. Slices B1/B2 here are compatible with it: the streak locals and the `timed_out` flag are exactly the inputs the ladder's `ToolAttempt` constructor wants.
- **Structured cancellation, steering, journal/reaper** → port-plan Wave 3.
- **Context ladder / prompt-cache guard** → port-plan Wave 4.
- **Permission/escalation work** → §11 wave-2 slice F (already in flight).
