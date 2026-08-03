# Operant vs Hermes-Agent: Core Agentic Loop Audit (refresh)

**Date:** 2026-08-03
**Supersedes:** `docs/HERMES_VS_OPERANT_AUDIT.md` (2026-07-23) — several "missing" items
there have since shipped (prompt caching, streaming scrubber, TDG→agentmemory,
Obscura→igs). This refresh reconciles the loop state as of today.
**Method:** file-level contrast of `operant/crates/operant-core/src/agent/` +
`operant-runtime/src/agent/` vs `hermes-agent/agent/` (Python reference) and
`hermes-agent-ultra/crates/` (Rust successor), plus a rust-best-practices state sweep.

---

## 1. Loop Architecture — Status: Full Parity

| Lifecycle point | hermes-agent (Python) | operant (Rust) | Status |
|-----------------|----------------------|----------------|--------|
| Per-turn setup | `build_turn_context()` | `turn_context::build_turn_context()` | ✅ |
| Main loop | `run_conversation()` | `OperantAgent::run()` (`agent/mod.rs:992`) | ✅ |
| Post-loop finalize | `finalize_turn()` | `turn_finalizer.rs` + `run_with_healing` (`mod.rs:3189`) | ✅ |
| Retry state | `turn_retry_state.py` | `turn_retry_state.rs` (retry budget, one-shot compress/rotate/fallback guards) | ✅ |
| Iteration budget | `iteration_budget.py` (consume/refund/grace) | `iteration_budget.rs` (CAS-based consume/refund) | ✅ |
| Interrupt | signal handler | `interrupt_flag` checked before each LLM round-trip | ✅ |
| Steer injection | `/steer` between iterations | `drain_steers()` | ✅ |
| Provider fallback | `restore_primary_runtime()` per turn | `provider_registry.reset_to_primary()` at turn start | ✅ |
| Grace call on budget exhaustion | ✅ | `attempt_grace_call()` | ✅ |

The runtime (channels/gateway) loop lives in `operant-runtime/src/agent/loop_.rs`
(`run_tool_call_loop`, `agent_turn`) with cancellation-token-aware streaming,
reasoning round-tripping (DeepSeek V4 thinking mode, #6059), and tool-marker
suppression. Both loops are distinct by design (core = CLI/TUI agent,
runtime = channel/gateway agents).

## 2. Retry / Error Classification — Status: Full Parity

- `error_classifier.rs` ports `error_classifier.py`'s `FailoverReason` taxonomy
  (Auth, AuthPermanent, Billing, RateLimit, UpstreamRateLimit, Overloaded,
  ServerError, Timeout, SslCertVerification, ContextOverflow, PayloadTooLarge,
  ImageTooLarge, ModelNotFound, ProviderPolicyBlocked, ContentPolicyBlocked…).
- `client.rs` has rate-limit-aware retry with `exponential_backoff_secs` +
  `parse_retry_after_header`; streaming retries transient 5xx mid-stream.
- `fallback.rs` (`FallbackModelClient`) tries fallback models on retryable
  errors; non-retryable 4xx returned immediately.
- Context-overflow auto-compression wired (`retry_state.compress_attempted`
  one-shot guard, `llm_compressor.rs`).
- `run_with_healing()` adds post-turn self-healing.

## 3. Context Management — Status: Full Parity

| Capability | hermes-agent | operant | Status |
|-----------|--------------|---------|--------|
| LLM summarization compression | `context_compressor.py` | `llm_compressor.rs` (head/tail protection, iterative merges, `SUMMARY_PREFIX`/`SUMMARY_END_MARKER`) | ✅ |
| Preflight compression | `turn_finalizer.py` constants | `turn_finalizer.rs` (80% threshold, H50=100 decay) | ✅ |
| Token warnings | `check_token_warnings()` | wired to `AgentEvent::Usage` (operant-cli iter-255, incl. threshold reset bugfix) | ✅ |
| Streaming cost/usage | account_usage | `emit_usage_and_cost` shared helper + `stream_options.include_usage` + Anthropic `message_start`/`message_delta` merge (iter-247) | ✅ |
| Loop detection (runtime) | — | `loop_detector.rs` (exact-repeat / ping-pong / no-progress → Warning→Block→Break) | ✅ (operant-exclusive) |
| Context analyzer/pruner (runtime) | — | `context_analyzer.rs`, `history_pruner.rs`, `context_compressor.rs` | ✅ (operant-exclusive) |

## 4. Memory — agentmemory replaces TDG (status: done)

- `tdg_tools.rs` REMOVED; `memory.provider = "tdg"` config gone.
- `agent_memory.rs` (`AgentMemoryProvider`) implements the `MemoryProvider`
  trait against the agentmemory server (https://github.com/rohitg00/agentmemory):
  - `prefetch` → `POST /agentmemory/smart-search`
  - `sync_turn` → `POST /agentmemory/remember`
  - tools `memory_smart_search`, `memory_save`
  - auto-spawn `npx -y @agentmemory/agentmemory@latest` (port 3111, 60s warmup,
    killed on `shutdown()`), graceful degradation when unreachable.
- All 15 `MemoryProvider` trait hooks remain wired through `MemorySyncExecutor`
  (FIFO background worker, 8s prefetch timeout, 5s shutdown drain).
- **Note:** the old "Streaming context scrubber MISSING" row is now
  **implemented** — `strip_memory_context_tags()` in `agent/mod.rs:285` strips
  `<long_term_memory>`/`<memory-context>`/`<workspace_context>` from streaming
  output (ported from hermes's StreamingContextScrubber).

## 5. Browser / Web — igs-rust replaces Obscura (status: done)

- `browser_tool.rs` lists `igs` as default provider (Obscura).
- `tools/igs.rs`: `IgsCli` (find binary / run_json / run_extract), `scrape_url`,
  `web_search_igs`, `browser_command`, `WebScrapeTool`, `WebExtractTool`,
  `IgsBrowserProvider` (implements `BrowserProvider`).
- Wired in `builtin.rs` (`pub use super::igs::{WebExtractTool, WebScrapeTool}`),
  graceful `is_available()` when the `igs` binary isn't installed.

## 6. Prompt Caching — Status: DONE (was flagged missing in July)

- `agent/clients/prompt_caching.rs`: `CacheTtl` (5m/1h), single `system_and_3`
  layout (system + last 3 non-system messages, ≤4 breakpoints).
- `agent/clients/anthropic.rs` injects `cache_control: {type: "ephemeral"}`.

## 7. Remaining Real Gaps (this audit)

### 7.1 rust-best-practices / hygiene (mechanical, high confidence)

| # | Gap | Evidence | Effort |
|---|-----|----------|--------|
| G1 | **Gateway still uses `anyhow` in lib code** (Phase 5 half-done — memory done, gateway pending) | `anyhow = "1.0"` in `operant-gateway/Cargo.toml`; 11 `anyhow::Result` sites across 7 files | ~1 hr |
| G2 | **No `#![deny(...)]` enforcement anywhere** — 0 deny-attrs in all 10 lib crates (missing_docs, unwrap_used, expect_used) | grep across lib.rs files | ~2 hr incl. escapes |
| G3 | **~104 justified `expect()` in gateway** need `#[expect]` escapes before `expect_used` deny can land | grep | part of G2 |
| G4 | **`--all-features` still broken**: runtime observability (otel/prometheus, deps never declared, 1,669 LOC) + hardware `hardware` feature (`include_str!` firmware) | verified inventory | wire or remove |
| G5 | **BUGS.md stale** (2026-06-19): lists many resolved items as open (patch tool, tool executor, turn context, session resume, slash dispatch — all implemented) | doc diff | ~30 min |
| G6 | **`HERMES_VS_OPERANT_AUDIT.md` stale** (superseded by this doc) | — | done here |

### 7.2 Capability gaps vs hermes-agent-ultra (Rust successor)

| # | Gap | hermes-agent-ultra | operant | Priority |
|---|-----|--------------------|---------|----------|
| C1 | **Tool-planning layer** | `hermes-tool-planning` crate | `operant-tool-call-parser` (parsing only) | Medium — feature |
| C2 | **Telemetry crate** | `hermes-telemetry` (incl. `otlp.rs`) | inline `Observer` only | Low — feature |
| C3 | **Eval/parity harness** | `hermes-eval` (runner/verifier/reporter/tblite) + `hermes-parity-tests` | `operant-runtime/src/agent/eval.rs` (internal only) | Low — CI hardening |
| C4 | **`node_detail()`** (inspect learning-graph node before edit) | `learning_mutations.py` | missing | Low — UX |

### 7.3 Loop-internal notes (no action, informational)

- `agent_memory.rs` REST client is `Error::message`-based (no `reqwest` typed
  conversion) — acceptable since errors degrade gracefully.
- `cache_control` TTL is fixed 5m/1h; no dynamic TTL or OpenRouter layout.
- The runtime loop uses `anyhow::Result` (`loop_.rs`) — binary-adjacent crate,
  acceptable per anyhow-for-binaries guidance; not a library contract.

---

## 8. Bottom Line

The core agentic loop is at **~full parity** with the Python reference and
**ahead** of it on loop-detection, context pruning, and streaming cost
fidelity. The two prior "big red" items (prompt caching, streaming scrubber)
are implemented; TDG→agentmemory and Obscura→igs migrations are complete.

What remains is **engineering hygiene and enforcement** (G1–G6), not loop
architecture. See `RUST_BEST_PRACTICES_PLAN.md` Phase tracker for the
execution plan; Phase 5 (anyhow→typed errors) is the only partially-finished
hygiene phase (memory ✅, gateway pending).
