# Agentic core — verified remaining work (audit round 2, 2026-10-07)

> **Baseline:** iter-663 `ff013938`, deployed and byte-verified. **Method:** five
> read-only scouts re-verified every claim in the Waves 2–4 plan against the live
> source with file:line evidence, plus inline spot-checks. **Parent authority:**
> `docs/OPENHUMAN-AGENTIC-LOGIC-PORT-PLAN.md` (this doc is its audit-qualified
> refinement), `docs/REMAINING-GAPS.md` (org layer), and
> `docs/plan-2026-10-07-organism-operational-memory.md` (org operational slices).
>
> **Purpose:** kill redundant work. Several items the port plan lists as "to
> build" already exist, and several items my earlier outline listed as "dead"
> are live. Only §3–§5 plus §6's org read-sides are genuinely unimplemented.

---

## §0 The engine census after Wave 1 (corrected)

Everything lands on ONE engine: core `OperantAgent::run` (`operant-core/src/agent/run.rs:288`).

| Surface | Entry today | Engine |
|---|---|---|
| CLI one-shot, TUI, Telegram gateway (live runner) | `OperantAgent::run` direct / `gateway_runner.rs:1149-1155` (iter-632 metering + iter-633 seat-budget wires on this path) | core |
| Gateway WS (live), ACP, channels dispatch, delegate sub-agents | `runtime Agent::turn_streamed` (`agent.rs:1757`) → `build_facade_agent` → `ReconciledAgent` | facade → core |
| Gateway HTTP | `operant-gateway/src/lib.rs:1037` → `loop_::process_message` | loop_ → core |

`turn_streamed` is **not residue** — it is the production facade driver. The genuinely
dead code is narrower than previously stated (see §7).

**Corrected dispatch finding:** the live turn paths are `ws.rs:644-667` (WS) and
`lib.rs:1037` (HTTP). The channels orchestrator dispatch loop
(`start_channels` → `run_message_dispatch_loop` → `dispatch_worker` →
`process_channel_message`) is confirmed **dead in the shipped binary** (tests only),
which is why LTO strips it. `gateway_runner.rs:1152` is a shipped-but-unused legacy
adapter.

## §1 Do-not-rebuild ledger (verified live — ANY plan item duplicating these is disqualified)

| Capability | Implementation | Evidence |
|---|---|---|
| Iteration budget (90) + env override | config | `config.rs:452` default 90; `HERMES_MAX_ITERATIONS` at `config.rs:1802` |
| Turn wall clock 1200s | `TURN_WALL_CLOCK_LIMIT_SECS` | `run.rs:391` |
| Per-call request timeout 120s + env override | config | `config.rs:1804-1807` (`HERMES_REQUEST_TIMEOUT`) |
| Identical-repeat guard (skip+inform) | `ToolGuardrailTracker` | `state.rs:100-130` thresholds; wiring `stream.rs:597-643`; success-shaped skip via `SKIP_MESSAGE_PREFIX = "Guardrail: "` |
| R35 loop abort + pre-abort nudge | core breaker | `run.rs` (streak ≥6 abort with stopped-early copy; nudge at 4); gated by `AgentConfig.loop_detection_enabled` |
| S2 per-tool consecutive-timeout breaker | `timeout_streaks` | `builders.rs:89/162`, run.rs wiring |
| S1 grace-call quality gate | `turn_rules::is_degenerate_final_text` + `should_retry` | `run.rs:158-162`, `run.rs:1193` |
| Canonicalising fingerprint fns | harvested into core guardrails | Wave 1 |
| Steer queue (queued user message between model calls) | `steer()` | `run.rs:2063-2083` |
| Cancellation at the facade driver | `Option<CancellationToken>` + `select!` on stream loop and non-stream LLM call | `agent.rs:1757-1762`, `1842-1857`, `1972-1986`, `2103-2110`; WS cancel/deny wiring `ws.rs:640-730` |
| Pre-LLM turn-prompt expiry sweep | expiry notice + rewrite-to-interrupted | `run.rs:795-806`, `ws.rs:662-680` |
| Preflight deterministic decay (80% threshold) + on-overflow LLM summarizer (reactive) + memory hook on pre-compress | `build_messages` | `run.rs:2097`, frozen/volatile split `2100-2116`, preflight `2213-2257`, assemble/evict `2270-2292`, safety repairs `2301-2330`; summarizer `llm_compressor.rs` |
| `compress_if_needed`/`compress_on_error` post-W1.4 single pair | reached from `loop_/run.rs:1181-1190`, `1238-1259`, `reconciled.rs:551-554` | live on the gateway HTTP path |
| Tool-result re-trim (preflight, no LLM) | `fast_trim_tool_results` via `tool_result_retrim_chars` (default 2000) + exempt list | `compress.rs:44/:116/:520/:521` |
| Per-channel tool policy + approval gate | permission gate | slice E iter-580; `stream.rs:830-842` (120s deny) |
| Delegate + sub_agents on facade | `with_facade_data_dir` | `tools/delegate.rs:92-196` |

## §2 The one ruling that gates Wave 4's host: dispatch consolidation

**Decision needed before Wave 4 lands.** Options:

- **A (recommended):** crown `gateway_runner` (ws.rs HTTP/WS) + `loop_::process_message`
  as the two production entry surfaces; delete the dead channels orchestrator dispatch
  (`start_channels`, `run_message_dispatch_loop`, `dispatch_worker`,
  `process_channel_message`) from the binary. Wave-4 preflight lives in core
  `build_messages` (already shared by every engine).
- **B:** wire `start_channels` as the single dispatch for the gateway binary (rewires
  WS+HTTP away from their current paths — larger blast radius, no measured gain).
- **C:** leave dual; keep paying dual maintenance + ~14k LOC of dead-orchestrator
  compile weight.

A is the OpenHuman lesson applied: one dispatch per concern, no parallel engine.

## §3 Wave 2 — guardrail ladder completion (what is genuinely MISSING)

| Gap | Verdict | Work |
|---|---|---|
| Ping-pong pattern (A,B,A,B alternating calls) | **EXECUTED iter-669** — 4-cycle warn / 5-cycle skip in `ToolGuardrailTracker`, 20-name window | done |
| Identical-result pattern (same tool+args, same result ≥5) | **EXECUTED iter-669** — 5× warn / 6× armed next-call skip; args may vary | done |
| No-progress ladder (varied-args backstop: same tool, different args, no progress) | **EXECUTED iter-669** — armed skip ignores arguments | done |
| Successful-repeat guard (`successful_repeat.rs`, 165 LOC) | **EXECUTED iter-682** — run-wide `(tool, result-hash)` recurrence ledger catches A, B(reset), A cycles the consecutive streak can never see; same 5/6 warn/arm escalation (warn/skip, never halts a successful turn — deliberate deviation from openhuman's halt) | done |
| Per-tool consecutive-failure counting (any error class, thresholds 8/12) | **EXECUTED iter-682** — `failure_streaks` per tool, warn 8 / Halt 12, reset on that tool's success; S2's 3-strike timeout mask stays on top | done |
| Hard-reject halt threshold (2) | **EXECUTED iter-682** — second consecutive `Blocked by security policy` result halts; shared `HARD_REJECT_PREFIX` const so classifier and blocked arm cannot drift; seat-policy denials excluded (a minted grant can make them succeed) | done |
| Halt verdicts with `failure_copy` (root-cause summary) | **EXECUTED iter-682** — `GuardrailDecision::Halt(String)` from the tracker's copy fns; `observe_guardrail_results` (now async two-phase) triggers the interrupt flag and surfaces the summary as final content; 12-test adversarial port of `mod_tests.rs` semantics in `ladder_tests` | done |
| Exemption list (`is_repeat_call_exempt`; fills the W1.8c-dropped `tool_call_dedup_exempt`) | **PARTIAL iter-669** — `with_exempt_tools` API live on the tracker, survives reset | AgentConfig/config-schema wiring pending |
| Per-tool activation gating on the facade path (beyond CLI exclusions) | MISSING | Add to `FacadeConstruction` |
| Ingestion-time tool-result offload + artifact TOC | MISSING — `ArtifactIndex`/`toc`/`offload` = no matches anywhere; `max_tool_result_chars` (config, default 50000) has ZERO consumers: dead knob | Wire-or-drop the dead knob; add ingestion-time offload+TOC |
| Port openhuman's 486-LOC `no_progress/mod_tests.rs` adversarial suite | PARTIAL — 13 pattern tests ported from the runtime loop_detector (iter-669) | openhuman fault-injection suite still to port |

**OpenHuman thresholds verified at source** (vendor commits in transcript): identical-halt 3 /
nudge 2; no-progress halt 6 / nudge 4; hard-reject halt 2; recoverable-repeat-failure 8;
recoverable-no-progress-failure 12. Copy verbatim with source-commit headers.

**S6/S7 caveat (from W1.10):** the skip@3 guardrail runs pre-execution; R35 abort@6 runs
post-streak. Decide the single ladder order (guardrail skip → warn → R35 abort) and encode
it once.

## §4 Wave 3 — steering / cancellation / journal (what is genuinely MISSING)

| Gap | Verdict | Work |
|---|---|---|
| Token threaded into core `run()` | MISSING — core has only `interrupt_flag` checks between batches (`run.rs:600-603`, `751-754`, `851-853`, `973-976`); facade driver has full select! | Thread `CancellationToken` into `run()`; `select!` at `execute_tools` (run.rs:1455-1473) and `request_approval` (run.rs:1258); bridge `interrupt_flag` |
| Steer vocabulary: model-switch (pacing.switch_model) + request-stop | PARTIAL — queued-message variant done | Two variants; wire preference-sync on switch |
| Per-turn journal rows with exit_reason/halt_verdict columns + event journal | PARTIAL — `.turn_state` rows exist with terminal status only | Extend schema; write verbose/verdict/reason on close |
| Startup reaper that CLOSES non-terminal runs as Cancelled | PARTIAL — sweep detects/notifies/rewrites-to-interrupted, never closes | Add close path; kill the "still in-flight after restart" retry bug |
| Mid-turn steering on the live gateway path (vs cancel-and-replace) | MISSING — ware session lock serializes, new message cancels previous turn | Route new-message through `steer()` queue when same conversation |
| Cron trigger cancellation/steering surface | MISSING (fire-and-forget) | Optional: expose token in the cron session |

## §5 Wave 4 — context ladder + ceilings (what is genuinely MISSING)

| Gap | Verdict | Work |
|---|---|---|
| Ordered preflight ladder summarizer → decay → wrap-up → TOC/trim in `build_messages` | PARTIAL — preflight decay + reactive summarizer exist; no explicit ordering, no wrap-up rung, no TOC rung | Rework `build_messages` into the ordered ladder; summarize-before-evict; final-call wrap-up copy |
| Prompt-cache guard middleware (protect frozen prefix) | MISSING (frozen/volatile split exists at run.rs:2100-2116, no guard) | Add guard middleware (PromptCacheGuard port) |
| Turn-wall-clock env override | MISSING (`TURN_WALL_CLOCK_LIMIT_SECS` hardcoded `run.rs:391`) | Add `HERMES_TURN_TIMEOUT`-style override; document turn-vs-call split |
| Vision routing on the facade path | DROPPED in migration — `multimodal` field deleted iter-663; `with_multimodal`/`with_vision_route` only reachable via tests + ACP direct construction | Decide: port per-iteration vision routing to facade, or formally accept single-provider vision at construction |
| Wave-4 preflight entry on the live path | Depends on §2 | If A: preflight already lives in core `build_messages` — no move needed |

## §6 Org layer — genuinely open (adjacent programme; not the agentic core)

| Gap | Verdict | Work |
|---|---|---|
| Notice-board READ wiring | PARTIAL — `NoticeBoard::inbox`/`query_inbox` + resolver built (`notice_db.rs:302/320`, `resolver.rs:1-47`), **zero production callers** | Wire `org notice inbox` CLI + prompt/tool consumption; do NOT rebuild the resolver |
| Cross-department worklog scope enforcement | PARTIAL — `WorklogDb::list` reads any department unfiltered (`worklog_db.rs:278`, CLI cmd_org.rs:1742) | Add authority/scope gate |
| Feed/DM read adapters | MISSING — adapters are outbound-push only | Build read/poll surface |
| IdentityGate on the run path | DEAD (tests only) | Wire or delete |
| Seat-budget provisioning CLI | PARTIAL — enforcement live (`gateway_runner.rs:1042-1083`), 0 rows, no CLI | Provisioning CLI only |
| Two-tier memory (org bank vs seat banks) | MISSING | Add bank split; governance actors sole org-bank writers |
| Org loop guard (GAP-2.1 acceptance: hop counter, generation ceiling, per-employee/tick budget) | PARTIAL — DM `TurnBudget` (3-turn) exists, acceptance not met | Extend to acceptance |
| Recipient resolver Team recipients | PARTIAL — resolver built, Team path empty (GAP-3.3) | Wire teams table |
| Teams table | MISSING | Build |
| Org check exit codes | PARTIAL (`findings` exits 1; `cmd_check` returns Ok) | Uniform exit contract |
| CEO loop | MISSING | Build |
| AD-RG (governance archival) | MISSING | Build |
| `retention_gc` | DEAD | Wire or delete |

## §7 Dead-code deletion list (post-§2-ruling; each entry has zero production callers, tests only)

1. `start_channels` (`orchestrator/startup.rs:26`) + `run_message_dispatch_loop` + `dispatch_worker` + `process_channel_message` (`orchestrator/dispatch.rs`) — channels orchestrator loop. **EXECUTED in iter-664**, together with the full sibling cluster (commands/consts/factory/health/history/identity/memory_ctx/media_pipeline/prompts/routing/runtime_types/sanitize/supervision), the dead `orchestrator::deliver_announcement` + `ChannelNotifyObserver`, and the orchestrator-level tests. `strip_tool_call_tags` relocated to `telegram::helpers`.
2. `Agent::turn` (`agent.rs:1516`).
3. `loop_::run` (`loop_/run.rs:102`) — keep `process_message` (live via gateway HTTP).
4. `loop_detector.rs` — after §3 harvest completes.
5. ~~gateway_runner.rs legacy adapter~~ — **RESCINDED (audit r2):** `gateway_runner.rs:1149-1155` is the LIVE Telegram turn path (iter-632 metering + iter-633 seat-budget envelope wired there). Nothing to delete.
6. Peer-WIP files are still uncommitted+staged (Cargo.toml/lock, cmd_org.rs, loop_/messages.rs, loop_/run.rs, plan-2026-09-29-telegram-loop.md) — §7's deletion MUST wait for the peer to land.

## §8 Ops blockers (unchanged from iter-663 report)

- Telegram bot-auth credential: 401/400-era blocker remains the delivery-hop constraint (memory).
- Cron hygiene: 4 ephemeral test regs to archive; 9 DUE-jobs backlog.
- **Delivery-fn gap (audit r2):** no production caller of `register_delivery_fn` — the shipped binary never wires cron delivery, so every announce-mode job takes the "skipped: no handler registered" arm (`scheduler.rs:602-620`). Wire a delivery fn at gateway startup or accept the skip. The channels `orchestrator::deliver_announcement` registry variant is dead (no callers; registry never populated because `start_channels` never runs) — deleted with iter-664.
- Verification rule: marker checks on the shipped binary must be reachable-marker only (LTO).

## §9 Suggested order

1. §2 ruling (A recommended) → delete §7.1/§7.5.
2. §3 guardrail harvest (loop_detector → tracker; adversarial suite port).
3. §4.1 token threading (CLI/TUI path).
4. §5 ladder + ceilings.
5. §4.2–§4.6 steering/journal/reaper.
6. §6 org read-sides (independent track; org slices 7→8→4→9→10 from the 2026-10-07 doc).

---

### Audit corrections vs the earlier outline (the anti-redundancy record)

- `turn_streamed` is **live** (gateway WS + ACP), not residue — the residue shrinks to §7.
- `request_timeout_secs` env override **exists** (`HERMES_REQUEST_TIMEOUT`); the plan's "env override missing" was wrong for the per-call ceiling.
- Preflight deterministic decay **exists** (`run.rs:2213`); only the ordered-ladder and proactive summarizer rungs are missing.
- Seat-budget enforcement **is live**; only provisioning is missing.
- Recipient resolver + notice inbox API **exist but are unwired** — the org gap is wiring, not building.
- `loop_/run.rs` is **partially live** (gateway HTTP `process_message`) — deletion is wrong; `loop_::run` (the fn) is the dead part.
