# Hermes-Agent Parity Gap Audit — Round 3 (Agentic-Loop Focus)

**Date:** August 16, 2026
**Scope:** Re-audit of the **core agentic loop** (user-input → context assembly → model call →
stream processing → tool execution → turn finalization) against `hermes-agent`, focused on
**production-grade** behavior: reliability, security, and loop hygiene.
**Baseline:** Rounds 1–2 audits + G1–G8 implemented and pushed (`35b175da`…`3918c965`),
1584 core tests + 653 CLI tests green. G9/G10 from Round 2 remain open (Tier 3).
**Method:** Symbol-level grep of `hermes-agent/agent/*.py` + `hermes-agent/tools/*.py` against
`operant/crates/**/*.rs`. Every "gap" below was verified by 0–few matches for the core behavior;
false positives (equivalent implementation under a different name) are listed in §2.

---

## 1. Confirmed Gaps

### Tier 1 — Core loop capability & reliability (high value)

| # | Feature | hermes source | operant status | Impact |
|---|---------|---------------|----------------|--------|
| R1 | **@-reference expansion in user input** (`@file:`, `@folder:`, `@git:diff/staged`, `@url:`) | `agent/context_references.py` (REFERENCE_PATTERN, quoted-value parsing, line-range `:N-M`, sensitive-dir refusal for `.ssh/.aws/.gnupg/.kube/.docker/.azure`) + `agent/coding_context.py`, `agent/subdirectory_hints.py` | **Absent.** TUI renders @-mention *suggestions* (`tui/prompt_input/suggestions.rs`) and transcript highlight, but the **agent loop never expands references** — grep `@file:|@folder:|@git:|@url:` in core → 0 non-test matches. A user typing `@file:src/main.rs:10-40` gets the literal string sent to the model. | 🟠 High — daily UX + correctness: file content never reaches the model from the reference syntax the CLI already suggests |
| R2 | **Reasoning-model stale-timeout floor + thinking-timeout guidance** | `agent/reasoning_timeouts.py` (`get_reasoning_stale_timeout_floor`, applied as `max(default, floor)` so long-thinking models aren't killed mid-think), `agent/thinking_timeout_guidance.py` (distinct "thinking phase exceeded upstream idle timeout" message for pre-first-token transport errors) | **No stale detector at all** — grep `stale_timeout|STREAM_STALE` → 0. Loop relies on the client's single HTTP `.timeout()` (client.rs:90). A Nemotron/o1/R1/QwQ-class model that thinks 120s+ before the first token gets killed by proxy/load-balancer idle timeouts, surfacing as a generic broken-pipe with no actionable guidance. | 🟠 High — production reliability for every reasoning model behind a gateway |
| R3 | **Bounded HTTP error-body reads** | `agent/bounded_response.py` (`read_streaming_error_body`: byte cap + hard wall-clock deadline so a hostile/broken proxy can't balloon memory or hang the agent reading an error body) | **Unbounded** `response.text().await?` in two hot paths: `client.rs:379` (chat_streaming non-2xx) and `client.rs:443` (execute_with_retry). A server that streams a huge or never-ending error body stalls/hangs the turn. | 🟠 High — memory/DoS hardening on the wire path |
| R4 | **Tool-call loop guardrails** (repeated-identical-call detection, idempotent-tool classification) | `agent/tool_guardrails.py` (pure per-turn controller: identical name+args repeat → warn/synthesize/halt; `IDEMPOTENT_TOOL_NAMES`), `agent/tool_result_classification.py` (`tool_may_have_side_effect`, `file_mutation_result_landed`, `NO_EFFECT_TOOL_NAMES`) | **Partial.** `agent/message_safety.rs::close_interrupted_tool_sequence` handles interrupted-sequence cleanup, but there is **no proactive guardrail**: the model can call the same tool with identical args N times per turn with no signal, and no-effect tools aren't classified for interruption/skip decisions. | 🟡 Medium — loop-hygiene; wasted turns + cost on degenerate loops |

### Tier 2 — Still open from Round 2

| # | Feature | hermes source | operant status |
|---|---------|---------------|----------------|
| G9 | **Credential-file registry** (`register_credential_file`, refuse-to-read/send protected mounts) | `tools/credential_files.py` | Absent (redaction + `pii.rs` cover output, not read-refusal of known credential files). |
| G10 | **Environment exposure audit** (`env_probe` — proactive "which env vars look secret & are exposed" report) | `tools/env_probe.py` | Absent (only passive redaction). |

### Tier 3 — Loop-adjacent ergonomics & durability

| # | Feature | hermes source | operant status | Impact |
|---|---------|---------------|----------------|--------|
| R5 | **Per-turn accounting line** (post-turn "edited 2 files +18 -3 · read 4 files · ran 3 commands") | `agent/turn_summary.py` (`TurnSummaryCollector`, `format_turn_summary`) | Absent — no post-turn tally surfaced in TUI/CLI. | 🟢 Low-Med — UX |
| R6 | **Durable session activity heartbeat** (SessionDB activity stamp, ≥30s cadence, `force_persist` on terminal stamps) | `agent/session_activity.py` | `active_sessions.rs` tracks locks with stale pruning, but no per-session persisted activity heartbeat. | 🟢 Low-Med — gateway/session liveness visibility |
| R7 | **Streaming think-block scrubber state machine** (hold partial tags at delta boundaries; flush held-back prose at EOS) | `agent/think_scrubber.py` | Operant has `OPEN_REASONING_TAGS`/`CLOSE_REASONING_TAGS` + `find_first_tag` + a `pending` buffer in `agent/mod.rs:3805` (StreamingContextScrubber port) — **stateful scrubber present**, but verify it holds partial tags across deltas (see §2). | 🟢 Low (probably covered) |

---

## 2. Verified NOT Gaps (checked this round — equivalent exists under different names)

| hermes feature | operant equivalent | Notes |
|----------------|--------------------|-------|
| `agent/think_scrubber.py` | `agent/mod.rs` StreamingContextScrubber (`OPEN/CLOSE_REASONING_TAGS`, `find_first_tag`, pending-buffer flush at 3805–3833) | Stateful stream scrubber exists; needs a cross-delta partial-tag test to be certain of parity. |
| `agent/bounded_response.py` | — | Real gap (R3) — listed above. |
| `agent/stream_single_writer.py` (stream fence) | Single-writer async design; `AgentEvent` channel serializes stream consumers | N/A — operant has one stream writer per turn by construction. |
| `agent/secret_scope.py` (profile-scoped secrets) | `credential_pool.rs`, `mcp_oauth.rs`, per-profile `.env` loading | Operant doesn't do in-process multi-profile multiplexing; scope is moot. |
| `agent/reasoning_summaries.py` (gpt-5.x summary-part boundary) | `client.rs` `with_reasoning` + ReasoningDelta accumulation | Summary-index joins are Responses-API-specific; chat wire handled. |
| `agent/usage_pricing.py` / billing views | `models_dev.rs` cost-per-million × `AgentEvent::Cost` (R3 round-1) | Covered. |
| `tools/approvals_suggest.py`, `slash_confirm.py` | `approval.rs`, `approval_mode.rs`, `approval_actions.rs` | Covered. |
| `tools/hook_output_spill.py` | `agent/hooks.rs` output handling | Covered. |
| `tools/tool_result_storage.py` | `truncate_tool_result` + `max_tool_result_chars` + trajectory store | Covered. |
| `tools/terminal_hints.py` / `focus_pane_tool.py` / `website_policy.py` / `working_diff.py` / `read_preview_tool.py` | `terminal_tool.rs` hints, `browser_provider.rs` policies, `patch_tool.rs` diff, `file_tools.rs` | Terminal hints/website policy/read-preview: verify flag-by-flag; none are loop-critical. |
| `tools/schema_sanitizer.py` | `client.rs` `repair_tool_call_arguments` + strict-API sanitizer (`message_safety::sanitize_tool_calls_for_strict_api`) | Covered. |
| `tools/daemon_pool.py` / `process_registry.py` | `process_registry.rs` | Covered. |
| `agent/replay_cleanup.py`, `stream_diag.py`, `lmstudio_reasoning.py` | Provider/transport-specific diagnostics; not loop-critical | N/A or low. |
| `agent/coding_context.py`, `subdirectory_hints.py` | `context_files.rs` (workspace context, default context files, `operant_context_files_*`) | Partially covered — auto-attach exists; per-directory hints N/A. |
| `agent/runtime_cwd.py`, `shell_hooks.py` | `runtime_adapter.rs`, `terminal_backend.rs` cwd handling | Covered. |
| `agent/battery.py`, `i18n.py`, `credits_tracker.py`, `account_usage.py` | `operant-hardware` (battery), no i18n (English-only product), `runtime_metrics.rs` | N/A or covered. |
| `agent/verify_hooks.py`, `verification_evidence.py` | G3 `verification_tool.rs` + evidence ledger | Covered. |

---

## 3. Drift / Dead-Config Observations

1. **`request_timeout_secs` is effectively dead in the loop** — defined in `config.rs:157` and
   `AgentConfig` (agent/mod.rs:109), but the only non-test users are the client's own HTTP
   timeout and test constructors (agent/mod.rs:4730/4773). The agentic loop never wraps
   `client.chat()`/streaming in `tokio::time::timeout(request_timeout)` — the field exists but
   the loop has no per-request budget of its own. Ties directly into R2: when we add a stale
   detector, wire this field as the ceiling.
2. **`HERMES_ENV_FILE` naming leak** (from Round 2, still present in `env_passthrough.rs`).
3. **Round-2 §3 items** (workspace doc drift in `AGENTS.md`, stale-process guard) remain open —
   infra, not loop.

---

## 4. Recommended Implementation Order

1. **R1 @-reference expansion** — new `context_references.rs` in core: parse `@file:` / `@folder:` /
   `@git:diff|staged` / `@url:` (quoted or bare, optional `:N-M` line range), refuse sensitive
   dirs (`.ssh/.aws/.gnupg/.kube/.docker/.azure`), inline content into the user message before
   context assembly; hook into `turn_context::build_turn_context` (the single user-input
   ingestion point — `agent/mod.rs:1206`). Highest daily-value gap; self-contained; CLI/TUI both
   benefit automatically.
2. **R3 bounded error-body reads** — small, surgical: replace `response.text().await?` at
   `client.rs:379` and `client.rs:443` with a capped read (e.g. 64 KiB byte cap + 10s wall-clock)
   mirroring `read_streaming_error_body` semantics; one helper + tests.
3. **R2 reasoning stale-timeout floor + guidance** — add `get_reasoning_stale_timeout_floor`
   (model-name list) + per-request budget applying `max(default, floor)` on the streaming read
   path, and a distinct "thinking phase exceeded idle timeout" error message for
   pre-first-token transport errors; wire `request_timeout_secs` as the ceiling.
4. **R4 tool guardrails** — new `tool_guardrails.rs`: per-turn repeated (name+args-hash) call
   counter → 3rd repeat yields a warning appended to the model feed; classify no-effect tools
   for interruption cleanup (reuse `message_safety`).
5. **G9/G10, R5/R6** — opportunistic (registry of protected files, env probe report, turn tally
   display, activity heartbeat).

---

## Status — 2026-08-16 (all Tier-1 + Tier-2 implemented, live-verified)

| Gap | Status | Evidence |
|-----|--------|----------|
| **R1** @-reference expansion | ✅ Implemented (`context_references.rs`, wired into `turn_context::build_turn_context`) | 9 unit tests; live: `@file:/tmp/r3-live/sample.txt:2-4` → `beta gamma delta` verbatim (laguna-s-2.1-free) |
| **R2** reasoning stale-timeout floor | ✅ Implemented (`reasoning_timeouts.rs` + per-request timeout raise in `client.rs` chat/chat_streaming + escape-point guidance annotation) | 4 unit tests |
| **R3** bounded error-body reads | ✅ Implemented (`read_bounded_body` in `client.rs`, both error sites) | 2 unit tests |
| **R4** tool-call guardrails | ✅ Implemented (`tool_guardrails.rs` + `observe()` in `execute_tools` pre-flight + `guardrail_skips` metric) | 7 unit tests; live: 2 identical env_probe calls allowed (spec: side-effect threshold=3) |
| **G9** credential-file registry | ✅ Implemented (`credential_files.rs` + wired into `file_tools::validate_path`) | 4 unit tests; live: model refuses to read `~/.operant/.env` and routes to `env_probe` |
| **G10** env-probe exposure audit | ✅ Implemented (`env_probe.rs` + registered as agent tool) | 7 unit tests; live: `env_probe` called in-loop, reports 2 exposures with names+lengths, values redacted |
| **Fallback wiring (new)** | ✅ hermes `fallback_providers` parity: `[providers]` loaded into run path, `FallbackModelClient` + `ProviderRegistry` constructed (`create_model_client_with_fallback`) | 5 CLI tests + 1 config round-trip; live: real 401 on primary → `switching to fallback provider, to_provider: zen-alt` → request to fallback endpoint |

Commits: `40ebfc5a` (R1–R4 + G9/G10), `54710da9` (fallback wiring), (R4 no-effect fix).
