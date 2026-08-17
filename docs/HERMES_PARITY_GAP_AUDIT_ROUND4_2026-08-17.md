# Hermes-Agent Parity Gap Audit — Round 4 (Core Agentic Loop, Deep Pass)

**Date:** August 17, 2026
**Scope:** Fourth re-audit of the **core agentic loop** (user-input → context assembly →
model call → stream processing → tool execution → turn finalization) against `hermes-agent`
(`run_agent.py` ~8.2k LOC + `agent/conversation_loop.py` + `agent/*.py`), focused on
**whatever remains unported** after Rounds 1–3.
**Baseline:** Rounds 1–3 fully implemented and live-verified (R1 @-refs, R2 reasoning
timeouts, R3 bounded error reads, R4 guardrails, R5 turn summary, R6 activity heartbeat,
R7 think scrubber, G9 credential files + AFT bypass, G10 env probe, fallback wiring,
credential pools, 429 rotation, MoA, steer, prompt caching, request-timeout ceiling).
Commits through `872739ee`. 8995 workspace tests green.
**Method:** Symbol-level mapping of every loop-relevant hermes module/method against
`operant/crates/**/*.rs`. Only verified gaps are listed as gaps; equivalents found under
different names are listed in §2. This round deliberately stress-checks the *edges*
previous rounds assumed covered.

---

## 1. Confirmed Gaps

### Tier 2 — Core-loop capability & correctness (medium value)

| # | Feature | hermes source | operant status | Impact |
|---|---------|---------------|----------------|--------|
| T1 | **Turn-end truncation heuristics** — detect a cut-off response and continue the turn instead of surfacing it as final. `_should_treat_stop_as_truncated` (Ollama-GLM stop→length misreport detection: `_is_ollama_glm_backend` + `_has_natural_response_ending` + `_strip_think_blocks`), `_has_content_after_think_block` | `run_agent.py:1652-1750` (`_has_content_after_think_block`, `_has_natural_response_ending`, `_is_ollama_glm_backend`, `_should_treat_stop_as_truncated`), `conversation_loop.py` continuation on truncated stops | **Absent.** The loop never reads the provider's real `finish_reason`: `agent/mod.rs:1874` hardcodes `finish_reason: Some("tool_calls")` on persist, and the only exit conditions are `tool_calls.is_empty()` → text_response (mod.rs:1911) or empty-content retry (mod.rs:1789). A provider that misreports a max-token cut-off as `stop`, or a reasoning model that stops mid-thought with content after a think block, ends the turn with a truncated answer — hermes continues with a continuation prompt. | 🟠 Medium — correctness: cut-off answers surface as final on some providers; no continuation round-trip |
| T2 | **In-flight request abort on interrupt** — `_abort_request_openai_client` / `_abort_request_anthropic_client` / `_interruptible_api_call` tear down the active HTTP request so an interrupt lands mid-stream | `run_agent.py:5133` (`_abort_request_openai_client`), `5251` (`_abort_request_anthropic_client`), `6058` (`_interruptible_api_call`) | **Partial.** TUI ESC aborts the whole agent task (drops the stream — good for the interactive path). But the core loop's `InterruptFlag` is only *checked at iteration boundaries* (`agent/mod.rs:1470`); a long model call or a long-running tool executes to completion before the interrupt takes effect, and the one-shot `run -q` path has no in-flight abort at all. | 🟡 Medium — UX: interrupt latency on long calls; one-shot path can't stop a stuck request (bounded only by the new timeout ceiling) |

### Tier 3 — Loop-adjacent observability & provider breadth (low value / partial)

| # | Feature | hermes source | operant status | Impact |
|---|---------|---------------|----------------|--------|
| T3 | **Provider response-header state capture** — `_capture_rate_limits`/`get_rate_limit_state`, `_capture_credits`/`_emit_credits_notices`, `_capture_anthropic_response_headers`, `_check_openrouter_cache_status` | `run_agent.py:3812-4022` | **Partial.** Retry-After parsing exists (`rate_limiter.rs::parse_retry_after_header`, `Error::RateLimited { retry_after }`) and `AgentEvent::Cost` derives from `models_dev` pricing, but no header-based rate-limit-state / credit-balance / cache-hit capture is surfaced to the user or CLI. | 🟢 Low-Med — observability: "limit state" and credit notices only visible via errors |
| T4 | **Per-session transcript log file** — `_save_session_log` writes a JSONL transcript per session for debugging/replay | `run_agent.py:2955` | **Alternative coverage.** Operant persists full message history to SQLite (`save_message_full` incl. tool_calls + finish_reason), plus trajectory (`record_trajectories`) and `~/.operant/stats.jsonl`. No separate JSONL transcript artifact. | 🟢 Low — arguably covered; hermes log is a debugging artifact |
| T5 | **Per-provider credential refresh breadth** — `_try_refresh_codex/nous/env/vertex/copilot/anthropic_client_credentials` | `run_agent.py:5301-5767` | **Partial.** `oauth_refresh.rs` covers Anthropic/Claude + Codex OAuth refresh only. Copilot (ACP), Vertex, nous, env-var refresh paths absent (copilot ACP + `env_passthrough.rs` cover parts). | 🟢 Low — provider-specific; rotation via credential pool covers the common case |
| T6 | **Within-batch tool-call hygiene** — `_deduplicate_tool_calls`, `_uniquify_tool_call_ids`, `_cap_delegate_task_calls` before dispatch | `run_agent.py:4617-4676` | **Partial.** R4 guardrails (`tool_guardrails.rs`) catch the 3rd+ identical call *across* the turn, and `repair_tool_call_arguments` fixes malformed JSON, but two identical calls in a single assistant message both execute; delegate calls aren't capped per turn (only recursion depth in `sub_agent_tool.rs`). | 🟢 Low — loop hygiene on degenerate batches |

---

## 2. Verified NOT Gaps (deep-checked this round — equivalent exists)

| hermes feature | operant equivalent | Notes |
|----------------|--------------------|-------|
| `agent/moa_loop.py` | `moa.rs` (`aggregate_moa_context`) + `Agent::set_moa_guidance` + per-turn injection at mod.rs:2431 | Fully ported, config-driven, inert by default. |
| `agent/prompt_caching.py` | `agent/clients/prompt_caching.rs` (system + last-3 `cache_control`, envelope + native Anthropic layouts) | Fully ported incl. TTL policies. |
| `agent/title_generator.py` | `database.rs::generate_session_title` + auto-title on persist (`maybe_auto_title_session`) | Fully ported (heuristic, no LLM round-trip — matches hermes). |
| `_record_file_mutation_result` / `_format_file_mutation_failure_footer` | `agent/turn_finalizer.rs::file_mutation_verifier_footer` + `_neutralize_footer_paths` equivalent | Fully ported, wired at mod.rs:2212. |
| `steer()` / `redirect()` | `Agent::steer` + steer queue (`steer_queue_handle`) + TUI `/steer`, `/queue` | Fully ported incl. drain-at-iteration-boundary. |
| `interrupt()` / `hard_interrupt()` | `InterruptFlag` + TUI ESC task-abort + grace call on interrupt exit | Partial (T2) but the interactive path is solid. |
| `_handle_max_iterations` | `attempt_grace_call` (budget-exhausted + interrupt exits) | Fully ported — grace call with tools stripped. |
| `agent/error_classifier.py` | `agent/error_classifier.rs` (27-reason taxonomy, billing/rate-limit/SSL/thinking-sig patterns) + `fallback.rs` wiring | Fully ported and wired to rotation. |
| `_drop_thinking_only_and_merge_users`, `_repair_message_sequence`, `_format_tool_call_arguments` | `agent/message_safety.rs` (`drop_thinking_only_and_merge_users`, `repair_message_sequence`, `close_interrupted_tool_sequence`, `repair_tool_call_arguments`) | Fully ported, used at mod.rs:2542/2531/1499/3431. |
| `agent/todo_tool.py` (`format_for_injection`, `_dedupe_by_id`) | `tools/todo_tool.rs` + `todo_injection_for_session` (re-injected post-compression) | Fully ported. |
| `tools/delegate_tool.py` | `tools/sub_agent_tool.rs` (`delegate_task`, roles, depth limits) + `async_delegation.rs` | Fully ported. |
| `tools/tool_search.py` | `tools/tool_search.rs` (tool_search / tool_describe / tool_call bridge, `assemble_tools`) | Fully ported, wired into `get_schemas_for_request`. |
| `tools/vision_tools.py` (native + auxiliary fallback) | `tools/vision_tool.rs` (native multimodal envelope + aux vision model, SSRF guard, auto-resize) | Fully ported; image *message* handling in the wire path differs (vision-as-tool vs inline parts) — acceptable. |
| `agent/turn_summary.py` / `agent/session_activity.py` | `turn_summary.rs` + `TurnSummaryObserver` / `touch_session_activity` (60s throttle) | Round-3, re-verified live. |
| `_emit_interim_assistant_message` (interim streamed text during tool loops) | Suppressed by design (`effective_text = ""` when tool calls present, mod.rs:1824) — matches hermes' default "thinking/planning shouldn't be shown" | N/A by design. |
| `_persist_session` / `_flush_messages_to_session_db` | `save_message_full` / `save_message` at mod.rs:1853-1888, 2159 | Fully ported. |
| `_compress_context` (fence + snapshot worker + timeout) | `context/rollup.rs`, `context/adaptive.rs`, `agent/llm_compressor.rs` (threshold %, deterministic fallback) | Fully ported. |
| `agent/bounded_response.py` | `client.rs::read_bounded_body` (R3) | Done in round 3. |
| `agent/reasoning_timeouts.py` + `thinking_timeout_guidance.py` | `reasoning_timeouts.rs` + `annotate_thinking_timeout` | Done in round 3. |
| `agent/tool_guardrails.py` | `tool_guardrails.rs` (identical-call detection, idempotent classification, synthetic halt) | Done in round 3. |
| `agent/background_review.py` | `agent/background_review.rs` + `spawn_background_review` | Fully ported. |
| `agent/learning_graph.py` / `insights.py` / `curator.py` | `agent/learning_graph.rs` + `learning_mutation_tool.rs` / `agent/insights.rs` + `insights_tool.rs` / `curator/` | Fully ported. |
| `agent/oneshot.py` | CLI `run --query` one-shot path | Covered. |
| `agent/context_engine.py` | `context/lcm.rs` (WAL DAG + auto-recall) | Covered. |
| `agent/estop.py` | `estop.rs` + scheduler/gateway/kanban/CLI estop guards | Covered. |

---

## 3. Drift / Dead-Code Observations (round 4)

1. **Hardcoded `finish_reason` on persist** (`agent/mod.rs:1874`) — the DB layer stores
   `"tool_calls"` for assistant messages regardless of the provider's real finish reason.
   Harmless for replay (the schema supports it) but it means truncation signals are
   discarded at the source; fixing T1 starts by surfacing the real finish_reason from
   `client.rs` `ChatResponse`/stream final chunk into the loop's exit decision.
2. **`request_timeout` ceiling** (round-3.5) — now applied at all four client call sites
   with the reasoning floor; the remaining observability gap is T2 (no abort, only a
   timeout).
3. **`tools` flag audit**: the user-directed removals (xurl, himalaya, searxng/ddg,
   apple/computer-use) are reflected — `web_providers/` retains igs/tavily/exa + the
   searxng/ddg modules as *code* but they are not the preferred path; no action needed.

---

## 4. Recommended Implementation Order

1. **T1 — finish_reason surfacing + truncation continuation** (highest value, self-contained):
   - Thread the real `finish_reason` from `client.rs` (`ChatResponse.choices[0]` + stream
     final chunk) into the loop's response tuple.
   - Port `_has_natural_response_ending` / `_should_treat_stop_as_truncated` as a small
     `turn_end_heuristics` module (Ollama-GLM backend detection is optional; the
     natural-ending + content-after-think-block checks generalize).
   - On detected truncation, append a continuation prompt (`<continue>`) and loop instead
     of exiting — bounded by the existing iteration budget.
2. **T2 — interrupt abort for the one-shot path**: wrap the in-flight request in a
   `tokio::select!` on `interrupt_flag.wait_for_interrupt` so Ctrl-C on `run -q` tears
   down the request (mirrors the TUI's task-abort) instead of waiting for the timeout.
3. **T3 — rate-limit/credit header capture**: parse `Retry-After`/`X-RateLimit-*` (partly
   done) and surface a `RateLimitState` via `AgentEvent` so the CLI can show "limit
   reached, retry in Ns" — cheap, high-visibility.
4. **T6 — within-batch dedupe**: dedupe identical (name, args-hash) calls in one assistant
   message before dispatch (hermes `_deduplicate_tool_calls` semantics), preserving the
   R4 cross-turn guardrail.
5. **T4/T5** — opportunistic (transcript artifact optional; per-provider refresh breadth
   only where a provider is actually used).

---

## Status — 2026-08-17 (audit only; no fixes in this round)

This is a findings report. T1–T6 are open; §2 confirms the previously-ported surface
holds up under deep re-checking. Baseline gates at audit time: `cargo fmt` clean,
`clippy --workspace --all-targets --all-features -D warnings` 0 warnings,
8995 workspace tests green, live smoke test (laguna-s-2.1-free) passing end-to-end.
