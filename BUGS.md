# BUGS.md — Operant Audit Fixes

## Round 2 (2026-08-06)

### R2-1 — LLM context compressor dead-wired (FIXED c394c517)
`with_llm_compressor` was never called in any binary (cli/runtime/gateway/core all zero callers). The compressor was always `None`, so `compress_context_overflow` always fell back to deterministic decay/evict.
- **Fix**: wired `with_llm_compressor` in both agent factories (`create_runtime_agent` + `create_agent_without_events`) in `operant-cli/src/main.rs`. Compressor self-guards on threshold/cooldown + deterministic fallback. Gated on `config.agent.context_compression` (R4 follow-up: see below).

### R2-2 — Real token usage never drives compression (FIXED c394c517)
Both compression gates used the char/4 `estimate_total_tokens` heuristic instead of actual API `usage.prompt_tokens`.
- **Fix**: added `last_reported_prompt_tokens: AtomicUsize` on `OperantAgent`, recorded in `emit_usage_and_cost`. Both gates now use `estimate_current_tokens` which prefers the reported value via `prefer_reported(reported, fallback)`.

### R2-3 — Memory bifurcation / dead JSON store (FIXED c394c517)
Agent-callable memory tools (`memory_store`/`memory_search`/`memory_recall`) wrote to a naive substring JSON store (`~/.operant/memory/tool_memories.json`) that was never injected into prompts. The real injected store (`MemoryManager`/MEMORY.md) was decoupled.
- **Fix**: added `ACTIVE_MEMORY_MANAGER` global hook in `memory_tools.rs`; tools now delegate to the agent's active `MemoryManager` via `set_active_memory_manager` (wired in `load_memory_manager` in main.rs). JSON store is fallback only when hook is unset (tests).
- **Live-verified**: `memory_store` writes land in MEMORY.md (2 hits for test key), `tool_memories.json` stays empty (0).

## Round 2 Follow-Up (2026-08-06)

### R2-1 follow-up — Compressor config-gate (FIXED 7960e614)
R2-1 wired `with_llm_compressor` unconditionally with `..Default::default()` (enabled=true), overriding user's `context_compression = false`.
- **Fix**: gate `enabled` on `config.agent.context_compression`, use `config.agent.context_compression_threshold` for `threshold_percent`.

## Round 4 (2026-08-06)

### R4-1 — Empty-response retry ladder (FIXED 7960e614)
Free-tier providers intermittently return empty assistant responses (no text, no reasoning, no tool calls). Operant silently accepted these as final answers instead of retrying.
- **Fix**: added `empty_content_retries` counter in the per-iteration tool loop (`operant-core/src/agent/mod.rs:1137`); retries up to `max_retries` (3) by appending the empty assistant turn as a nudge (mirrors hermes-agent `conversation_loop.py` empty-retry loop).
- **Live-verified**: `WARN Empty assistant response — retrying (1/3)` fired on a real empty turn, task recovered (10 files created).

## Round 3 (2026-08-06)

### R3 — Credential pool dead-wired (FIXED 1da115a9)
`with_credential_pool` had zero callers; the pool was never built/attached, so `try_rotate_credential` always returned `None`. Multi-key rotation (a hermes runtime feature) was unreachable even when `credential_pool.enabled=true`.
- **Fix**: shared attach helper in both agent factories — seeds from provider env var + `client.additional_api_keys`, attaches when `config.credential_pool.enabled` and pool non-empty.
- **Live-verified**: "Attached credential pool, provider: openai, creds: 2" + "Credential rotated — client API key updated, rotation_count: 1" on auth failure.

## Known/Deferred

### Bug #1 — Session aggregate counters dead (FIXED 9eb8f4a4)
`sessions` table counters (message_count, tool_call_count) stayed 0 despite persisted rows. `save_message`/`save_message_full` never incremented them.
- **Fix**: rewrote both writers to increment counters in a transaction mirroring hermes (`hermes_state.py:6361`). Live-verified: fresh session shows `message_count=2`.

### Bug #2 — `session_events` table fully dead (YAGNI-flagged)
`record_event` on `session_events` has zero runtime callers; never read by CLI; hermes has no such table. Dead scaffolding — removal skipped pending user direction.
