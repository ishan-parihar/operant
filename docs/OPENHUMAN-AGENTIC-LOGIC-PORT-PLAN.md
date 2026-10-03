# OpenHuman agentic-logic port — gap contract and implementation plan

**Date**: 2026-10-03
**Baseline**: `origin/main` @ `f3bc2701` (iter-586)
**OpenHuman source**: `parent-projects/openhuman` @ `cbbe80208a` (v0.64.10), submodule `vendor/tinyagents` @ `3c411b44` (fetched 2026-10-03; **it was empty on disk before this session** — any "copy the files" work must run `git submodule update --init vendor/tinyagents` first)
**Supersedes**: nothing. Composes with `plans/006-agent-loop-reconciliation.md` (this plan is its execution vehicle), `docs/NEXT-IMPLEMENTATION-OUTLINE.md` §11 (disjoint files), and `docs/JCODE-VISUAL-LAYER-IMPLEMENTATION-PLAN.md` (disjoint files).
**Investigation method**: four read-only scouts (openhuman core, operant loop, stuck-run forensics, openhuman TUI) + direct source verification of every load-bearing claim. Log evidence spans `~/.operant/logs/gateway.log` (56 MB, 102,897 records, 2026-08-17 → 2026-10-03).

**Status 2026-10-04**: Wave 0 EXECUTED as iters 595 (A1), 600 (A2), 601 (B1), 602 (B2), 603 (C), plus 604 (BUGS.md stale-header cleanup) — see `docs/AGENT-LOOP-FIX-EXECUTION-PLAN.md` and each slice's BUGS.md S-entry. §4c (below) is the 2026-10-04 duplication audit that refined Waves 1–4 against existing mechanisms: the port SUBSUMES `ToolGuardrailTracker` and reuses `loop_detector`'s canonicalising hash, extends the existing `steer()` queue and `.turn_state` journal rather than paralleling them, threads the already-stack-wide `CancellationToken` into Loop A, and consolidates THREE context compressors into one. `StallWatchdog` (transport-idle detector, `operant-infra`) is a different layer and stays untouched.

---

## 0. Executive summary — three decisions, evidence-first

1. **The agentic loop is not missing a guard; it is missing behavior.** Operant already has an iteration budget (90), a 1200s turn wall clock, and two cross-iteration circuit breakers. The stuck runs happen because five specific behaviors are absent or wired to the wrong path (§1), and every one of them terminates a 20-minute turn in either silence or a garbage string. **Wave 0 fixes those five in `operant-core` — no port dependency, immediate relief.**

2. **Port the behavior layer, not the harness.** OpenHuman's loop is a vendored 112,726-LOC crate (`tinyagents-harness`) plus a 52,755-LOC host adapter layer. Wholesale transplant would create a **third** agent loop beside the two operant already runs (plan 006 exists precisely to remove that class of bug), and would drag ~200k LOC of `tinyagents-*` substrate (graph 45,649; session 29,979; orchestration 9,968; runtime 8,898). What is actually worth porting is ~2,500 LOC of **pure, harness-free loop-behavior primitives** — the no-progress ladder, the loop-guard thresholds, the halt-summary copy table, the policy trio, the journal/reaper durability pattern — installed into ONE reconciled operant loop (§2–§4).

3. **Do not port the openhuman TUI.** `openhuman-tui` is 4,519 LOC; operant's TUI is 22,488 LOC and strictly more capable. OpenHuman's event bus is a lossy `broadcast` (drops `text_delta` on lag; mid-turn text is unrecoverable) where operant already uses backpressured `mpsc`. Its tool display flattens arguments to a string — a downgrade of the `DisplayMessage.tool_data: Option<ToolCall>` model landed at iter-583. The jcode visual-layer port (W1 seam landed, W2 in flight) is the visual programme; openhuman composes with it only as a second *source* adapter if ever needed, and zero rework of `jcode_model/` would be required (§5).

---

## 1. Ground truth — why the loop gets stuck (measured, ranked)

The 2026-10-03 05:10 run (session `gw_69f0da80d56f2d77`) is the complete specimen: **21 minutes, 15 chat messages, 4 background reviews, 2 network retries, 1 tool timeout — ending in the 13-character answer `]<]minimax[>[`** (reasoning-leak artifact; verified in `~/.operant/database.db`, `messages` table). `empty_response_fallback: false` because 13 chars is technically non-empty.

| # | Root cause | Evidence | Fix site |
|---|---|---|---|
| S1 | **Grace-call output is never validated.** On budget/iteration/wall-clock exhaustion, `attempt_grace_call` returns `Message::assistant(&text)` unconditionally — no empty/garbage gate. The loop's ONLY output validator (`turn_rules::EmptyContentRetry::should_retry`) is wired at one call site (streaming path, `run.rs:1000`) and never on the grace return. | `run.rs:104` (verified); 200 assistant messages DB-wide are 1–24 chars with no whitespace | `agent/run.rs:102-104` |
| S2 | **Tool timeouts return to the model as ordinary results.** `error_with_name("Tool timed out after 30s")` is a retryable tool message; no per-tool consecutive-failure breaker. 35 timeouts in the log, all at the generic 30s cap; `aft_bash` burned 30s and the turn ran 19 more minutes. | `tools.rs:341` (`execute_with_timeout`), dispatch `stream.rs:1068-1075` | `tools.rs:325-350` |
| S3 | **Background review burns the turn's wall-clock budget.** 4 reviews inside one 21-minute turn (`iters: 10, interval: 10`), each returning `memory_search {"count":0}` noise; reviews also tripped the whitelist 149 times. The 1200s limit is charged to the turn while the reviews run inside it. | log 05:14:01 / 05:18:05 / 05:21:50 / 05:24:45 | `agent/run.rs:401` accounting + review spawn site |
| S4 | **Unrepairable tool args degrade to `{}`**, manufacturing a schema-validation failure that costs a full LLM round trip per occurrence. 1,938 substitutions → 522 validation failures → 7,676 log lines in one 32-minute window (2026-08-22, burst to 612 lines/min). The cross-iteration breakers were added as fire-fighting for this and only *bounded* it. | `message_safety.rs:171-177` (verified) | `message_safety.rs:171-177` |
| S5 | **Terminal states are invisible to the operator.** The 25-day log silence starts immediately after a `Degenerate tool-call loop` warning; the user got no "stopped" notice. Exit reasons under-report: the two in-band circuit-breaker aborts masquerade as `TextResponse` in `TurnExitReason` (`turn_finalizer.rs:50-59`). | log 2026-08-31T10:12:04Z; `turn_finalizer.rs` | `gateway_runner.rs` turn-end handler |

**Already fixed — do not re-plan** (BUGS.md headers are stale, verified against `origin/main`): D-1 cron data loss (iter-570: `scheduler` now uses `set_session_id`; `gateway_runner.rs:511-649` documents the swap), D-1b cron allowlist escalation (iter-571), D-5 double `MemoryManager` (iter-577), B′ write barrier (iter-578). Still genuinely open and loop-adjacent: **D-6** (process-global sub-agent limits shared between cron and gateway) and **D-5b** (cron memory session-boundary signal is a no-op).

**Architecture defect that amplifies all five:** operant runs **two live loops** — Loop A `operant_core::agent::OperantAgent::run()` (`agent/run.rs:197-1749`, ~1550-line async fn: CLI/TUI/gateway chat) and Loop B `operant_runtime::agent::Agent::turn()`/`turn_streamed()` (`operant-runtime/src/agent/agent.rs:1516/:1757`, gateway WS `operant-gateway/src/ws.rs:343` + ACP `acp_server.rs:563`). They share exactly one constant. Every behavioral fix must be applied twice or it silently diverges (R23/R24/R25 history). Loop A has **zero `tokio::select!` and zero `CancellationToken`** — the only cancellation primitive is `InterruptFlag`, an `AtomicBool` polled between steps; a dispatched tool cannot be cancelled.

---

## 2. What OpenHuman actually is — the measured port surface

```
openhuman-core (52,755 LOC, host layer)
└─ src/agent/tinyagents/            ← the turn seam openhuman owns
   ├─ turn_runner.rs (740)          drives a turn through the harness
   ├─ harness_assembly.rs (730)     assembles the middleware stack
   ├─ harness_context_ladder.rs (229) ordered context-reduction steps
   ├─ turn_policy.rs (316)          RunPolicy: turn ceiling 3600s, per-model-call 900s
   ├─ turn_outcome.rs (143) + turn_run_finalize.rs (291)  terminal outcome object
   ├─ journal.rs (253)               durable per-turn status journal
   ├─ reaper.rs (27)                 startup sweep: orphaned runs → Cancelled
   ├─ middleware/                    openhuman behavior, as harness middleware:
   │   repeated_failure.rs (691), loop_guards.rs (39), approval.rs (167),
   │   cost_budget.rs (121), credential_scrub.rs (70), tool_output.rs (642),
   │   tool_policy.rs (429), packed_tool_route.rs (137), tool_exposure.rs (215)…
   └─ policy_denial.rs (229), stop_hooks.rs (144), steering_forwarder.rs (127)
vendor/tinyagents (submodule; 112,726 LOC harness + 88k sibling crates)
└─ crates/tinyagents-harness/src/no_progress/   ← THE portable gem
    mod.rs (329) + classified.rs (70) + successful_repeat.rs (165) + types.rs (192)
    = 756 LOC, "deliberately free of harness types so it can be unit tested
      in isolation and reused by higher-level reliability layers" (its own doc)
```

**The no-progress ladder** (the behavior operant's S1/S2/S4 families lack), from `no_progress/mod.rs`:
- Track `(tool, arg_fingerprint) → outcome` across the turn.
- On failure classify: `hard_reject` (security/approval denial re-issued unchanged — can never succeed), `recoverable_miss` (unknown-tool recovery sentinel), or ordinary failure.
- Verdict: `Continue` → `Nudge` (inject a **system** message "no progress since step X" — not a tool result, so the harness's instruction isn't attributed to the tool) → `Halt` (stop, surface the message as the final response; tracker self-resets so a resumed run doesn't re-trip).
- Recoverable failures get larger headroom: identical-repeat threshold **8**, varied-args no-progress threshold **12** (`loop_guards.rs:23-27`); tools whose contract is identical re-invocation sit on an exemption list (`is_repeat_call_exempt`).
- Successful-repeat guard: identical output ≥ 4 or identical call batch ≥ 3 (`successful_repeat.rs:21-23`) — this is the **text-domination/repetition guard BUGS.md R33 explicitly defers**.

**Halt summaries name root causes** instead of a generic cap error: `failure_copy/` (`halt.rs` 172 + `table.rs` 311 + mod 22 = 505 LOC) holds the copy table — recoverable-identical, recoverable-no-progress, terminal-inference, and `user_actionable_escalation` — keyed by failure classification. This is what turns a bounded failure into an operator-readable one (S5's fix pattern).

**Structured run policies** (`turn_policy.rs`, harness `RunPolicy`): per-model-call wall clock with a **fresh budget per call** (900s) distinct from the turn runaway ceiling (3600s), `InvalidArgsPolicy`, `UnknownToolPolicy`, `RetryPolicy`, `RunLimits::max_tool_calls = 50`. OpenHuman's own history (#5766) mirrors operant's S2: their old 600s turn ceiling doubled as the per-call bound and killed productive turns — they split the two.

**Durability**: per-turn journal + `reap_orphaned_runs` at startup (orphaned non-terminal runs → `Cancelled`, best-effort, never blocks boot). Operant's nearest equivalent is `collect_interrupted_turns()` (`gateway_runner.rs:3635`, file-based `.turn_state`) — coarser, and it only reports; it does not durably close the runs.

---

## 3. The gap contract — capability by capability

Status legend: `HAVE` = operant equivalent exists and is wired; `WEAK` = exists but on the wrong path or under-specified; `MISSING` = no equivalent.

| Capability | OpenHuman site | Operant status | Wave |
|---|---|---|---|
| No-progress ladder (nudge→halt, per-tool fingerprints, exemption list) | `no_progress/` 756 LOC | **MISSING** — only all-failed-iteration counters + identical-signature streak, both *between* iterations | 2 |
| Recoverable/terminal failure classification | `no_progress/classified.rs`, `failure_copy/` | **MISSING** — every tool error is an ordinary result (S2) | 2 |
| Halt summaries naming root cause | `failure_copy/` 505 LOC | **MISSING** — generic `Tool timed out…` / `MaxIterationsExceeded` | 2 |
| Grace-output quality gate | n/a (their finalize validates) | **MISSING** — S1, `run.rs:104` | 0 |
| Per-tool consecutive-timeout breaker | `turn_policy.rs` policies | **MISSING** — S2, `tools.rs:341` | 0 |
| Background work off the turn budget | review runs outside turn scope | **WEAK** — S3, reviews inside the 1200s budget | 0 |
| Invalid-args surfaced as tool error (not `{}`) | `InvalidArgsPolicy` | **WEAK** — S4, `message_safety.rs:171-177` | 0 |
| Terminal-state notice to the operator | `TurnCompleted` event + halt copy | **WEAK** — S5, in-band aborts invisible | 0 |
| One loop | single harness loop | **MISSING** — two loops sharing one constant (plan 006) | 1 |
| Structured cancellation (`CancellationToken`, `select!`) | `steering/` + harness runtime | **MISSING** — `InterruptFlag` polling only | 3 |
| Mid-turn operator steering | `SteeringCommand`/`SteeringHandle` | **MISSING** | 3 |
| Durable turn journal + startup reaping | `journal.rs`, `reaper.rs` | **WEAK** — `.turn_state` files, report-only | 3 |
| Text-domination/repetition guard | `successful_repeat.rs`, `StreamTextStallDetector` | **MISSING** — BUGS.md R33 defers it | 2 |
| Ordered context ladder (compress → microcompact → wrap-up → artifact TOC → trim) | `harness_context_ladder.rs` + harness middleware | **WEAK** — reactive-first (overflow error triggers), preflight is deterministic decay only | 4 |
| Prompt-cache guard | `PromptCacheGuardMiddleware` | **WEAK** — frozen/volatile split exists in `build_messages`; no guard middleware | 4 |
| Fresh per-call model ceiling | `max_model_call_ms` 900s | **HAVE-ish** — `request_timeout_secs: 120` per call; missing the turn-vs-call split rationale + env override | 4 |
| Tool result artifacts (offload + TOC) | `ArtifactIndexTocMiddleware`, artifact store | **WEAK** — `guard_tool_output_spend` truncates only | 4 |
| Per-channel tool policy enforcement | `ToolPolicyMiddleware`, `policy_denial.rs` | **HAVE** (wave-2 slice E landed iter-580; `PERMISSION-SCOPING-PLAN`) | — |
| Approval middleware | `middleware/approval.rs` | **HAVE** — permission gate `stream.rs:830-842` (120s hard-coded deny; wave-2 F owns escalation) | — |

## 3b. What we will NOT port (with evidence)

| Not ported | LOC | Why |
|---|---|---|
| `tinyagents-harness` runtime whole | 112,726 | Third loop beside two existing ones; plan 006 exists to *remove* duplicated loop behavior. Port behavior, not substrate. |
| `tinyagents-graph` | 45,649 | Flow/graph orchestration operant already has its own answer for (org substrate, §11). |
| `tinyagents-session` | 29,979 | Operant has `SessionStore` + `database.rs`; swapping persistence is a data-loss risk with no behavioral gain. |
| `openhuman-tui` | 4,519 | §5. Operant TUI is 5× larger, non-lossy, and mid-way through the jcode visual port. |
| OpenHuman event bus (`WebChannelEvent`) | — | 44 fields, ~15 read by the TUI, built web-first; `broadcast` drops deltas on lag. Porting it imports a web client's schema into a terminal app. |

---

## 4. The waves

### Wave 0 — the five stuck-run fixes — **EXECUTED (iters 595, 600–603; BUGS.md cleanup iter-604)**

Landed with fault-injected acceptance tests per slice; see `docs/AGENT-LOOP-FIX-EXECUTION-PLAN.md` and BUGS.md S1–S5 entries. B1 additionally fixed three `stream.rs` timeout arms emitting empty tool names. The artifacts this wave produced (`ToolResult.timed_out`, the streak tracker, `turn_end_content`, exit-reason plumbing) are the inputs later waves fold in — not throwaway.

1. **W0.1 Grace gate** — `run.rs:102-104`: route the grace text through the existing `should_retry` quality gate; on rejection, fall back to the gateway's existing empty-fallback message. Acceptance: a test that injects a garbage grace response and asserts the fallback message, plus a positive control (valid grace text still passes).
2. **W0.2 Tool-timeout circuit breaker** — `tools.rs` `ToolExecutor`: track consecutive timeouts per tool name; N=3 → non-retryable error naming the tool. Acceptance: fault-injected timeout ×3 ends with a terminal error result, and a timeout-succeed-timeout sequence resets the counter.
3. **W0.3 Review off-budget** — exclude background-review wall-clock from the 1200s turn budget (or spawn reviews only after turn end). Acceptance: a scripted run with 4 reviews must not shorten the turn's usable budget.
4. **W0.4 Honest invalid-args** — `message_safety.rs:171-177`: return the unrepairable call to the model as a **tool error result** ("arguments could not be parsed: <first 80 chars, redacted>") instead of a valid call with `{}`. Acceptance: the S4 cascade test — malformed `process` args produce one error round-trip, not a validation-failure + dedupe + repeat cascade.
5. **W0.5 Terminal notice** — on wall-clock/circuit-breaker exits, send the operator an explicit stopped-state message ("stopped after 20 min: <reason>") via the turn-end handler; extend `TurnExitReason` so in-band aborts stop masquerading as `TextResponse`.

**Order**: W0.1 and W0.5 first (they determine what the operator sees), then W0.2, W0.4, W0.3.

### Wave 1 — one loop (the port substrate; executes plan 006)

Plan 006's four steps, with the parity harness as the acceptance gate: extract shared turn rules into `turn_rules.rs` (started — both loops already share `EmptyResponseCounter`/`AssistantTurn`; Loop B still holds a dead shadowing local at `loop_.rs:89`), delete that shadow, migrate `ws.rs:343`/`acp_server.rs:563` onto the reconciled loop, land `tests/agent_parity.rs` (scripted provider × scenario matrix: empty×3→answer, empty×4→exhaustion, nudge intervals, compression+todo re-injection, budget exhaustion). **Every later wave lands once, on one loop.**

**Wave 1 also consolidates the THREE context compressors** (2026-10-04 audit): Loop A's reactive pair — `agent/llm_compressor.rs` (934 LOC, the LLM summarization engine, wired via `with_llm_compressor` from `operant-cli/src/main.rs:1883/:1963`) + `agent/compress.rs` (266 LOC, overflow fallback: LLM summarize → deterministic decay, hermes parity incl. todo re-injection) — and `operant-runtime/src/agent/context_compressor.rs` (980 LOC, invoked PROACTIVELY by `operant-channels/src/orchestrator/dispatch.rs:343-352` on the WS path). The reconciled loop keeps ONE engine: the core pair (`llm_compressor` as the summarizer, `compress.rs` as its overflow/decay fallback) extended with runtime's proactive preflight entry point; `context_compressor.rs` is deleted with Loop B and `dispatch.rs` calls the survivor. **Plus a third loop consumer**: `operant-runtime/src/tools/delegate.rs:1` runs sub-agent delegation on the separate `loop_::run_tool_call_loop` engine (`loop_/tool_loop.rs`), which carries the only wired no-progress detector (`loop_detector.rs`, see §4c). Wave 1 must migrate the delegation path onto the reconciled loop like `ws.rs`/`acp_server.rs` — otherwise a third turn engine survives the merge.

### Wave 2 — the no-progress ladder (the core port; ~1,300 LOC + tests) — **REFINED 2026-10-04: evolve `ToolGuardrailTracker`, do NOT port a parallel module**

The audit (§4c) found operant already runs a per-turn repeat controller — `operant-core/src/tool_guardrails.rs` (`ToolGuardrailTracker`: (tool_name, normalized-args) counts → Allow/Warn/Skip, wired at `stream.rs:597-643`, side-effecting tools skipped one repeat earlier, hermes R4 parity). Porting openhuman's `no_progress/` as a sibling module would create the exact duplicate architecture this port must not have. The ladder therefore **grows inside the existing controller and its consumption seam**:

1. **Fingerprint upgrade — reuse IN-TREE code, don't port**: `operant-runtime/src/agent/loop_detector.rs:73/:81` already has `hash_value`/`canonicalise` — recursively key-sorted JSON, order-sensitive arrays, proven key-order-independent by its own test (`:657`). Move that fn pair into `tool_guardrails.rs` and replace `normalize_args` (whitespace-strip only, which reads `{"a":1,"b":2}` and `{"b":2,"a":1}` as different calls — the exact gap openhuman's doc warns defeats repeat detection). Openhuman's fingerprint stays as the semantic reference.
2. **Absorb `loop_detector`'s patterns too**: exact-repeat AND ping-pong (A→B→A→B ≥4 cycles) AND identical-result no-progress (same tool, varied args, identical result hash ≥5) — escalating Warning→Block→Break. These are three rungs openhuman's ladder does not have in this form; the merged controller keeps all of them.
3. **Classification — extend the EXISTING taxonomy**: `agent/error_classifier.rs` already owns `FailoverReason` (:16) and `ClassifiedError` (:123) for provider errors. Extend `ClassifiedError` with tool-attempt classes (recoverable / terminal / hard-reject — openhuman's `ClassifiedFailure` semantics) instead of creating a second enum. Thresholds from openhuman `loop_guards.rs` (identical 8, varied-args 12, hard-reject 2) replace the flat 3/3/4.
4. **Add the missing rungs**: varied-args no-progress backstop, successful-repeat guard (identical output ≥4 / identical batch ≥3 — BUGS.md R33), and Halt verdicts that surface `failure_copy`'s root-cause summaries instead of generic skip messages.
5. **Exemption list**: port openhuman's `is_repeat_call_exempt` concept; keep operant's existing `NO_EFFECT_TOOL_NAMES` alongside it (no-effect tunes skip severity; exemption lists legitimately-identical re-invocation — adjacent problems, both constants stay, documented).
6. **Fold in and retire the duplicates**: B1's per-tool `TimeoutStreak` fields, run.rs's two between-iteration breaker locals (`consecutive_failed_iters`, `identical_streak`), and `loop_detector.rs` itself (harvested per items 1-2) all collapse into this one controller; `loop_detector.rs` is deleted when the `loop_` engine goes in Wave 1.
7. **Port the tests**: openhuman's `no_progress/mod_tests.rs` (486 LOC) adversarial suite adapted to the merged controller; keep the existing `tool_guardrails` and `loop_detector` unit tests green by carrying them over (behavior extension, not replacement).

### Wave 3 — cancellation, steering, durability

1. **Structured cancellation — thread the EXISTING token, don't introduce one**: `CancellationToken` is already the stack's primitive (`operant-api/src/channel.rs:61`, `acp_server.rs:289/:649/:727`, `operant-gateway/src/ws.rs`, orchestrator supervision). Loop A is the odd one out (only `InterruptFlag`). Thread the channel's existing token into the reconciled loop via `tokio::select!` on the LLM call, tool batch, and permission wait; `InterruptFlag` stays as the UI/TUI-facing edge that cancels the token — one primitive internally, not two.
2. **Steering — EXTEND the existing queue**: `OperantAgent` already has `steer()` (`agent/builders.rs:425`), the `steer_queue` field, `drain_steers()` (`:527`), and `steer_queue_handle()` for external producers. Do NOT port openhuman's `steering_forwarder.rs` alongside it. Extend the existing queue's drain semantics with openhuman's `SteeringCommand` vocabulary (queued message lands between model calls, model switch, request-stop) and expose it to the gateway/TUI surfaces that already own an `InterruptFlag` path.
3. **Durability upgrade**: extend the `.turn_state` mechanism into a per-turn journal row (status, exit reason, breaker verdict, started/ended) with the startup sweep closing non-terminal runs as `Cancelled` — openhuman `reaper.rs` pattern. This makes D-5b-class bookkeeping observable and gives the stuck-run forensics of §1 a durable record instead of log mining. (`operant-infra`'s `StallWatchdog` is transport-idle detection for channel reconnects — a different layer; it stays untouched.)

### Wave 4 — context ladder + run policies (last; touches the most callers)

1. Ordered context-reduction ladder in `build_messages` preflight, built on the ONE surviving compressor from Wave 1 (not a fourth path): LLM summarizer (`llm_compressor.rs`) → deterministic decay (`compress.rs`'s fallback — operant's existing microcompact equivalent) → final-call wrap-up → oversized-result artifact TOC + trim (`guard_tool_output_spend` is the trim rung) (openhuman `harness_context_ladder.rs` ordering). The proactive invocation at `orchestrator/dispatch.rs:343-352` becomes the ladder's preflight entry point on the WS path. Today operant pays a wasted round-trip *before* reactive compression fires.
2. Prompt-cache guard middleware around the frozen/volatile split.
3. Split turn ceiling vs fresh-per-call ceiling with env override (`0` = unbounded for deliberate long autonomous runs) — openhuman #5766 rationale; operant already has `request_timeout_secs` but the split's *reasoning* should be adopted so nobody re-collapses it.

### LOC budget

| Wave | New/adapted LOC | Tests |
|---|---|---|
| 0 | **EXECUTED** (~150 production + ~1,600 test across 5 slices) | 5 fault-injected suites landed |
| 1 | ~0 new logic (extraction/migration) + compressor consolidation (delete 980, keep 934+266) | parity harness ~400 |
| 2 | ~800 (harvest `loop_detector` fns + patterns ~150, extend `ToolGuardrailTracker` rungs ~150, classification extension ~100, halt-copy table ~505 adapted) + retire ~700 duplicated | ported 486 + carried-over `loop_detector` suite + new adversarial |
| 3 | ~450 (token threading ~150, `steer()` extension ~100, journal ~200) | cancellation + sweep tests |
| 4 | ~400 ladder ordering + wrap-up/TOC rungs | per-rung |
| **Total remaining** | **~1,650 production** (net; the audit removes ~700 duplicated LOC) | — |

---

## 4c. Duplication audit — existing mechanisms the port must subsume (2026-10-04)

Method: enumerate operant's existing implementation of each Wave 1–4 capability (indexed grep + direct reads, every site cited), then classify the port action. **REUSE** = port on top of it; **EXTEND** = grow it; **MERGE** = fold sources into one; **UNTOUCHED** = different layer, leave alone. This is what keeps the port non-redundant: nothing below gets a sibling.

| Capability | Existing mechanism (site) | Action |
|---|---|---|
| Turn engines | Loop A `agent/run.rs:197`; Loop B `runtime/agent/agent.rs:1516/:1757` (WS/ACP); **third engine** `runtime/agent/loop_/tool_loop.rs` via `tools/delegate.rs:1` (sub-agent delegation) | Wave 1 merges all three into Loop A's successor |
| Repeat/loop guard | `tool_guardrails.rs` — `ToolGuardrailTracker`, (tool, normalized-args) → Allow/Warn/Skip, wired `stream.rs:597-643` (Loop A, live) | **EXTEND** — becomes the ladder's home (Wave 2) |
| No-progress patterns | `runtime/agent/loop_detector.rs` (696 LOC): exact-repeat, ping-pong, identical-result hash; `canonicalise`/`hash_value` `:73/:81`, key-order-independence test `:657`; wired only into the `loop_` engine | **HARVEST then DELETE** — move the fn pair + patterns into the controller (Wave 2); dies with the `loop_` engine (Wave 1) |
| Failure taxonomy | `agent/error_classifier.rs` — `FailoverReason` `:16`, `ClassifiedError` `:123` (provider-error side) | **EXTEND** — add tool-attempt classes; no second enum |
| Timeout streaks | B1's `ToolResult.timed_out` + streak fields on `OperantAgent` (iter-601) | **MERGE** into the ladder (Wave 2) |
| Between-iteration breakers | run.rs locals `consecutive_failed_iters`, `identical_streak` | **MERGE + DELETE** (Wave 2) |
| Cancellation | `CancellationToken`: `operant-api/channel.rs:61`, `acp_server.rs:289/:649/:727`, `operant-gateway/ws.rs`, orchestrator supervision; `InterruptFlag` (core, UI edge) | **REUSE** — thread the existing token into Loop A (Wave 3) |
| Steering | `OperantAgent::steer()` `builders.rs:425`, `steer_queue`, `drain_steers()` `:527`, `steer_queue_handle()` `:456` | **EXTEND** with `SteeringCommand` semantics; do NOT port `steering_forwarder.rs` (Wave 3) |
| Run journal / crash recovery | `.turn_state` files + `collect_interrupted_turns` (`gateway_runner.rs:3635`) | **EXTEND** into the per-turn journal + startup sweep (Wave 3) |
| Transport stall | `operant-infra/stall_watchdog.rs` (channel-idle → reconnect; discord et al.) | **UNTOUCHED** — different layer from run-level progress |
| Context compression | `agent/llm_compressor.rs` (934, LLM summarizer, wired `operant-cli/main.rs:1883/:1963`) + `agent/compress.rs` (266, overflow decay + todo re-injection) + `runtime/context_compressor.rs` (980, proactive `dispatch.rs:343-352`) | Wave 1 **MERGE** to one engine; Wave 4 orders its rungs into the ladder |
| Oversized-result trim | `guard_tool_output_spend` (post-dispatch truncation) | **REUSE** — the ladder's trim rung (Wave 4) |
| Prompt caching | frozen/volatile split in `build_messages` | **REUSE** — add only the guard middleware (Wave 4) |
| Session distillation | `distillation.rs` + `spawn_session_distillation` | **UNTOUCHED** — session-level memory, orthogonal |
| Platform tool policy | `operant-tool-planning` (pure filter) | **UNTOUCHED** — orthogonal to per-turn repeat policy |
| Harness kernel | `operant-harness` (plan-016 claims/seams — not a turn loop) | **UNTOUCHED** |
| Sidecars | `kernel-sidecar`/`pk-sidecar` (plan-015 process supervision stubs) | **UNTOUCHED** — not agent loops |

Corrections recorded: the pre-audit draft of this plan would have ported openhuman's `no_progress/` module as a sibling of `ToolGuardrailTracker`, ported `steering_forwarder.rs` beside `steer()`, "introduced" CancellationToken, and treated compaction as two engines instead of three. The advisory-era claim that `loop_detector` had zero production call sites is corrected above: it is wired into the live `loop_` delegation engine (which is itself a third turn engine Wave 1 must absorb).

---

## 5. The terminal UI visual layer — verdict: no port (evidence)

| Dimension | openhuman-tui | operant TUI | Verdict |
|---|---|---|---|
| Size | 4,519 LOC (655 test) | 22,488 LOC | operant larger |
| Framework | ratatui 0.30 + crossterm 0.29 | identical versions | zero framework cost either way |
| Event channel | `broadcast(512)`, **drops `text_delta` on lag**, logs `Lagged(n)` | `mpsc(256)` backpressure | operant strictly better — do not port the bus |
| Tool display | flattens to `"→ {name}{args}"` at reducer time | `DisplayMessage.tool_data: Option<ToolCall>` full args | openhuman would *degrade* iter-583's seam |
| Render staleness | draws before `select!` — one event stale | draw-after-event | openhuman worse |
| `terminal.rs` RAII guard | 105 LOC | `terminal_setup.rs` exists | verify redundancy, don't port |

**Composition note**: if openhuman agentic semantics are ever surfaced in the TUI, they arrive as a second *source* adapter into `DisplayMessage` (`adapter.rs:9-21` mapping table already covers streaming/tool/thinking). Zero rework of `jcode_model/`. The visual programme remains `JCODE-VISUAL-LAYER-IMPLEMENTATION-PLAN.md` (W1 landed, W2 markdown in flight).

---

## 6. Sequencing vs in-flight work

- **§11 permission-genome waves** (seat policies, write barrier, escalation queue): file-disjoint from every wave here (org/ vs agent/). Concurrent execution is safe; W0.5 touches `gateway_runner.rs` turn-end — coordinate the file with the wave-2 escalation slice owner.
- **JCODE visual port (W2 markdown)**: disjoint (`tui/jcode_markdown/`, `tui/jcode_render/`).
- **Loop B callers** (`ws.rs`, `acp_server.rs`): frozen for wave 1; nothing else should touch them before the reconciliation lands.
- **Wave 0 first, always**: it is the only wave whose fixes are correct independent of the port decision.

## 7. Verification strategy (house rules)

- Every zero/“clean” claim carries a positive control (2026-09-29 rule).
- Fault injection over mocking: S1 garbage-grace, S2 timeout×3, S4 malformed-args cascade are all scripted-failure tests — a test that cannot fail measures nothing.
- Parity harness (wave 1) runs the same scripted provider through both loop entry points until Loop B is deleted, then through the one loop.
- Live smoke: run the gateway against the Telegram path once per wave (the 05:10 specimen's shape: user msg → tools → bounded exit with a named reason) and verify the operator-visible message, not just the log line.
- Restores proven by `sha256` (both MATCH), not by a green suite.

## 8. Risks

1. **Wave 1 is the schedule risk** — migrating live WS/ACP callers can regress gateway conversations; mitigate by landing the parity harness *first* (006 step 3), on the current twin loops.
2. **Verbatim copies drift** — no_progress and loop_guards carry openhuman's thresholds; record the source commit (`3c411b44`) in each file header so a future re-sync has a diff base.
3. **Steering + cancellation change interrupt semantics** — `InterruptFlag` consumers (TUI, gateway) must keep compiling; feature-gate nothing, migrate all callers (clean cutover rule).
4. **The submodule is now fetched on this machine only** — any agent re-measuring "what exists in openhuman" must run `git submodule update --init vendor/tinyagents` or it will report the harness missing (exactly what the first scout hit).

## 9. Non-goals

- No provider/model layer changes (openhuman uses `tinyinference_llm`; operant's provider stack stays).
- No session-store replacement.
- No new TUI work of any kind (see §5).
- No organism/permission-genome work (§11 owns it).
