# Agent-loop Wave 1 execution plan — one loop, one compressor

**Date**: 2026-10-04
**Baseline**: `origin/main` @ `1bee63c8` (iter-605)
**Parent authority**: `docs/OPENHUMAN-AGENTIC-LOGIC-PORT-PLAN.md` §4 Wave 1 + §4c (duplication audit). This document is the executable version: exact edits, owned files, acceptance, leg order. Every line number below was re-verified against the baseline by four read-only scouts on 2026-10-04 (reports: LoopBConsumers, LoopASurface, ThirdEngine, Compressors).

**Target loop**: Loop A — `OperantAgent::run()` (`operant-core/src/agent/run.rs:258-1999`). Already the live Telegram path (`gateway_runner.rs:736`), already owns steer/budget/guardrails/compressors/sub-agents. Wave 1 migrates every other turn-engine consumer onto it, then deletes the other two engines. **No behavior porting happens in Wave 1** — parity is the acceptance gate; openhuman semantics arrive in Waves 2–4 on the single surviving loop.

---

## 0. The engine inventory (measured)

| Engine | Entry | Production consumers | Fate |
|---|---|---|---|
| **Loop A** (survivor) | `OperantAgent::run` `run.rs:258`, `&self`, `Result<Message>` | CLI one-shot + TUI (`main.rs:2380/:2600`), Telegram gateway (`gateway_runner.rs:736`), core sub-agents (`sub_agent_tool.rs:677`) | keeps + gains preflight entry |
| **Loop B** | `runtime Agent::turn` `agent.rs:1516` / `turn_streamed` `agent.rs:1757` | `turn_streamed`: gateway WS `operant-gateway/src/ws.rs:721`, ACP `orchestrator/acp_server.rs:661`. `turn`: **zero** production callers (only dead `run_single`/`run_interactive` + tests) | deleted (W1.6–W1.7 migrate, W1.10 deletes) |
| **Loop C** | `run_tool_call_loop` `loop_/tool_loop.rs:10` (28 positional args) | channels gateway `orchestrator/dispatch.rs:682` via `loop_::run`/`process_message` (`loop_/run.rs:541/:885`, live via gateway `lib.rs:1045`); sub-agent delegation `tools/delegate.rs:1211` | deleted (W1.8–W1.9 migrate, W1.10 deletes) |

Shared already: `turn_rules.rs` (`EMPTY_RESPONSE_MAX_RETRIES`, `AssistantTurn`, `EmptyResponseCounter`, `is_degenerate_final_text`) — consumed by A (`run.rs:8/439/1098/1107`) and B (`agent.rs:32/1568/1662/1830/2194`). Loop C keeps a dead shadow copy (`loop_.rs:86-89`).

NOT shared (the migration's real work): provider traits (`core::agent::ModelClient` vs `operant_providers::traits::Provider`), event enums (`AgentEvent` 23 variants vs `TurnEvent` 6), cancellation (`InterruptFlag` vs `CancellationToken`), approval shape (second `permission_tx` channel vs in-band `ApprovalRequest`), history ownership (`&self` interior-mutability on A vs `&mut self` on B vs passed-`&mut Vec` on C).

---

## 1. Leg W1.1 — harvest the canonicalising fingerprint into operant-core

**Goal**: move `hash_value`/`canonicalise` (+ sibling `hash_str`) and their two key-order tests out of `operant-runtime/src/agent/loop_detector.rs:70-103`/`:654-676` into `operant-core/src/tool_guardrails.rs` (the Wave-2 consumer), BEFORE any engine deletion. This is the §4 "Harvest ordering" decision: without it, the Loop C migration orphans a live `pub` module behind the `clippy -D warnings` gate mid-programme.

**Owned files**: `crates/operant-core/src/tool_guardrails.rs`; `crates/operant-runtime/src/agent/loop_detector.rs`.

**Exact edits**:
1. Add to `tool_guardrails.rs`: `pub fn hash_value(&serde_json::Value) -> u64`, `pub fn canonicalise(&serde_json::Value) -> serde_json::Value`, `pub fn hash_str(&str) -> u64` — verbatim bodies from `loop_detector.rs:70-103` (recursive key-sort; arrays order-sensitive). Add the two tests verbatim (`hash_value_is_key_order_independent`, `hash_value_nested_key_order_independent`, `loop_detector.rs:654-676`) into `tool_guardrails.rs`'s test module.
2. In `loop_detector.rs`: delete the three fn bodies; `use operant_core::tool_guardrails::{canonicalise, hash_str, hash_value};` — the detector's 19 remaining tests keep passing unchanged (they test the detectors, not the hashers).
3. Header note in `tool_guardrails.rs`: source `loop_detector.rs @ 1bee63c8` (drift-base rule, §8.2).

**Acceptance**: `check.sh check -p operant-core --lib` green; runtime suite green (detector tests pass against core fns); `cargo test -p operant-core tool_guardrails` shows the two moved tests; zero new clippy warnings in both files.

## 2. Leg W1.2 — parity harness `tests/agent_parity.rs` (pins ALL engines pre-migration)

**Goal**: the acceptance gate for every later leg. One scripted script (`Vec<ChatResponse>` — the type is re-exported by both engines) drives Loop A, Loop B, and Loop C through three thin adapters; asserts final-answer and message-history equivalence. After each consumer migration, the same scenarios re-run against the migrated path.

**Owned files**: `crates/operant-cli/tests/agent_parity.rs`; `crates/operant-cli/Cargo.toml` (add `[dev-dependencies]`: `operant-core`, `operant-runtime`, `tokio`, `async-trait`, `futures` — regular deps already exist at `Cargo.toml:37-39`).

**Adapters** (neither exists today; both are test-local):
- Loop A driver: `ScriptedClient` implementing `operant_core::agent::ModelClient` — copy the proven shape from `tests/grace_output_gate.rs:84` (`responses: Mutex<Vec<ChatResponse>>` + `seen: AtomicUsize`, `chat`/`chat_streaming`/`provider_name`).
- Loop B/C driver: `ScriptedProvider` implementing `operant_providers::traits::Provider` — copy from `operant-runtime/src/agent/loop_/tests.rs:49` (`VecDeque<ChatResponse>` + pop_front + exhaustion error).

**Scenario matrix** (from plan-006 step 3, executable):
| Scenario | Script | Assert |
|---|---|---|
| empty×3→answer | 3 empty `AssistantTurn`s, then text | same final string on A and B; C returns accumulated text |
| empty×4→exhaustion | 4 empties | both engines give up (A: error/exit-reason; B: same error class); no infinite retry |
| nudge interval | same tool called 3× with identical args | A: `ToolGuardrailTracker` Warn visible in events; C: LoopDetector Warning message in history — post-migration the facade must surface the same nudge |
| compression+todo re-injection | overflow error → compress | todos re-injected (A's `compress.rs:44-76` path fires; assert todo present post-compression) |
| budget exhaustion | tool loop > max_iterations | A: `TurnExitReason::BudgetExhausted`/grace path; B: `anyhow` bail at `agent.rs:2319-2322`; C: final summary call `tool_loop.rs:1280-1291` — record each engine's today-behavior as the pin |

**Acceptance**: harness green on baseline for all three engines (it may assert *recorded* divergences rather than equality where engines intentionally differ — the pin is "what today does", not "what I wish"); `cargo test -p operant-cli --test agent_parity` runs in CI suite.

## 3. Leg W1.3 — dead-code hygiene (free deletions, shrink later diffs)

**Owned files**: `crates/operant-runtime/src/agent/loop_.rs`; `crates/operant-runtime/src/agent/agent.rs`; `crates/operant-core/src/agent/run.rs`.

**Exact edits**: delete the module-private shadow `const EMPTY_RESPONSE_MAX_RETRIES: usize = 3;` (`loop_.rs:86-89` — dead: module-private, shadows the canonical `pub use` at `agent.rs:32`); delete `run_single` (`agent.rs:2325-2327`, zero consumers workspace-wide) and the `run_interactive` REPL dead path it feeds (`agent.rs:2333` — verify zero callers first, keep if the CLI REPL is live); delete `OperantAgent::run_with_healing` (zero callers outside its own definition).

**Acceptance**: full workspace `cargo check` + suites green; grep proves zero remaining references to each deleted symbol.

## 4. Leg W1.4 — compressor consolidation (core pair + preflight; delete runtime engine)

**Goal**: THREE engines → ONE. The core pair (`llm_compressor.rs` summarizer + `compress.rs` overflow/decay/todo-reinject) gains the two runtime-only capabilities as pub entry points; all four runtime call paths repoint; `context_compressor.rs` (980 LOC, 31 tests) and its config type are deleted.

**Owned files**: `crates/operant-core/src/agent/compress.rs`; `crates/operant-core/src/agent/llm_compressor.rs`; `crates/operant-runtime/src/agent/context_compressor.rs` (delete); `crates/operant-runtime/src/agent/mod.rs:4` (unregister); `crates/operant-runtime/src/agent/loop_/run.rs:960-972/:1019-1025` (repoint); `crates/operant-channels/src/orchestrator/dispatch.rs:343-373` (repoint); `crates/operant-config/src/scattered_types.rs:176-248` (`ContextCompressionConfig` + defaults, delete after call sites repoint).

**Exact edits**:
1. `compress.rs` gains two pub fns adapted from the runtime engine's signatures (state borrowed, not owned — same shape as today's `compress_context_overflow_inner` chain):
   - `pub async fn compress_if_needed(...)` — proactive pre-LLM-call preflight (from `context_compressor.rs:193-255`): estimate tokens (`estimate_current_tokens` exists at `compress.rs:146-152`), threshold → `should_compress` (`llm_compressor.rs:298`) → summarize-or-decay chain → todo re-injection (`compress.rs:44-76`, which runtime lacks today — this is the upgrade).
   - `pub async fn compress_on_error(...)` — context-limit error probe (from `context_compressor.rs:259-281` + `parse_context_limit_from_error:50`): parse limit → one compression pass → retry signal.
   - Runtime-only capability to preserve: inline tool-result trim (`fast_trim_tool_results:152` + `tool_result_retrim_chars`) — port as a preflight sub-step.
2. Repoint: `dispatch.rs:355` (proactive, per-message fresh compressor today → core entry); `loop_/run.rs:960-972` (error probe); `loop_/run.rs:1019-1025` (post-turn pre-hard-trim). Runtime re-exports the entries (`pub use operant_core::agent::compress::{...}`) so channels keeps its `channels→runtime→core` edge — no new Cargo edge.
3. Delete `context_compressor.rs`, mod.rs:4 registration, `ContextCompressionConfig` block. Preserve its memory-persistence-of-summaries behavior (test: summary lands in memory when configured).

**Acceptance**: parity harness compression scenario green on all engines post-repoint; core tests for both new fns (fault-injected: oversized history → compressed; overflow error → probe → retry); runtime+channels suites green; `grep -rn ContextCompressor` → zero outside history.

## 5. Leg W1.5 — the reconciled facade + ProviderModelClient (build, no consumer switched)

**Goal**: the ONE runtime seam every consumer migrates through. Pure translation, no logic: (a) `ProviderModelClient` — `impl operant_core::agent::ModelClient` over a boxed `operant_providers::traits::Provider` (the trait bridge; `chat`/`chat_streaming`/`provider_name` map 1:1 — signatures verified `model_client.rs:114/:121/:127`); (b) `reconciled.rs` facade: construct core `OperantAgent` (via `AgentConfig` + `OperantAgent::new`, builders.rs:47-52) with `with_events(session_sender)`, expose:
- per-call `turn_streamed`-shaped entry: takes `Sender<TurnEvent>`-equivalent + `Option<CancellationToken>`; translates `AgentEvent` → the consumer's event surface (WS/ACP: `TurnEvent` 6-variant map incl. `Content→Chunk` accumulated→delta fix and `Done{reason}` → synthesized exit signal; channels: `DraftEvent`/`StreamDelta` surface at `dispatch.rs:497/:537`).
- `CancellationToken` → `InterruptFlag` bridge (spawn a watcher task that trips the flag on cancel — full `select!` token threading is Wave 3; the bridge keeps today's cancel semantics).
- approval bridge: consumer responder ↔ core `permission_tx` (mod.rs:376) + `AgentEvent::ToolPermissionRequest` (mod.rs:258).
- cost/receipt scopes (`ToolLoopCostTrackingContext`, dispatch.rs:646/:678) + session-key scoping (`scope_session_key`, loop_.rs) preserved as pass-throughs.
- PacingConfig wiring: `loop_detection_*` knobs (helpers.rs:211-221) flow into the guardrail tracker config (thresholds themselves stay Wave 2).

**Owned files**: `crates/operant-runtime/src/agent/reconciled.rs` (new); `crates/operant-runtime/src/agent/mod.rs` (register + re-export); `crates/operant-runtime/src/tools/delegate.rs` (NO — untouched this leg).

**Acceptance**: unit tests: event translation table (each `AgentEvent` variant → expected consumer frame), token→flag bridge trips within one poll, approval round-trip under 1s; facade drives Loop A via `ProviderModelClient(ScriptedProvider)` end-to-end on the parity matrix's answer scenario; **zero production consumers changed this leg**.

## 6. Legs W1.6–W1.9 — switch consumers one at a time

**W1.6 — gateway WS** (`crates/operant-gateway/src/ws.rs:721`): `turn_streamed(agent…)` → facade call. Preserve: `state.cancel_tokens` registry (:699-705), approval multiplex in `forward_fut` (:754, cancel arm :768), post-turn persistence (:964-971), memory consolidation spawn (:981-993). **Acceptance**: parity scenarios green through the facade; `ws.rs` approval round-trip test green; suite green.

**W1.7 — ACP** (`crates/operant-channels/src/orchestrator/acp_server.rs:661`): same facade call; `is_tool_loop_cancelled` error sniff (:704) replaced by exit-reason mapping (`Interrupted` vs `Error`). **Acceptance**: parity green; cancel-vs-failure distinction test (cancel → clean stop, not error frame).

**W1.8 — channels gateway** (`dispatch.rs:682` + `loop_/run.rs:541/:885`): the biggest switch — 28-arg plumbing → facade; `run`/`process_message` (gateway `lib.rs:1045`) re-point; cost scopes + session keys preserved. **Acceptance**: parity green end-to-end on the channels path; live-gateway smoke (dev server, one scripted conversation per channel type) before push.

**W1.9 — delegation** (`delegate.rs:1211`): `execute_agentic`'s 28-arg `run_tool_call_loop` call → facade sub-agent entry (mirrors `sub_agent_tool.rs:640-677`: own `AgentConfig{max_iterations}`, own registry via allowlist-filtered `register_child_tools`, timeout wrapper, `delegate` self-nesting drop preserved at :1162-1170). Per-agent provider-by-name selection (`create_provider_with_options`, :464) routes through `ProviderModelClient`. **Acceptance**: delegation parity scenario (tool-using sub-agent reaches answer); depth-limit test; timeout test.

## 7. Leg W1.10 — delete the dead engines, re-baseline

**Exact edits**: delete `agent.rs::turn/turn_streamed/free run`; delete `loop_/` tree (`tool_loop.rs`, `run.rs`, `messages.rs`, `context.rs`, `streaming.rs`, `loop_.rs`, `tests.rs`); delete `loop_detector.rs`; collapse `agent/mod.rs:46-48` re-exports to the facade; delete the `TurnEvent` re-export at `agent.rs:26` if WS/ACP now consume the facade's surface. Loop C's time-gated identical-output abort (`tool_loop.rs:1172-1207`) and exhaustion-final-summary (`:1280-1291`) must have documented homes in the facade first (they are behavior, not substrate) — port to core `run.rs` if not already covered by grace-call/exit paths.

**Acceptance**: full gates (fmt, check.sh both crates, full suite, clippy zero-new-in-touched); parity harness now runs A-only columns; suite count re-baselined and recorded here; BUGS.md entry per fixed divergence discovered during migration.

---

## Relay mechanics (unchanged from Wave 0)

Serial legs; worktree `~/.cache/wt-wave1` (branch `fleet-wave1`, pushed per leg as `origin HEAD:main`); per-leg owned-file list is the brief's contract; never force-push, on rejection `git fetch && git rebase origin/main`; NEVER yield in_progress checkpoints — finish the leg. Gates per leg: `cargo fmt --all`; `./scripts/check.sh check -p <crate> --lib`; full suite (baseline ~2283 passed / 2 failed pre-existing K-1 `tools::kernel` pair — never touch, never let the count grow); clippy `-D warnings` zero-new-in-touched-files (14 pre-existing red sites belong to the other fleet's files). The shared main checkout stays untouched (foreign fleet is live in it).

**BUGS.md policy**: migration legs get S-entries only when they fix measured latent defects discovered mid-leg (e.g. the Loop C Block-message comment/behavior contradiction at `tool_loop.rs:1120` — Block injects a system message but does NOT replace the tool output).
