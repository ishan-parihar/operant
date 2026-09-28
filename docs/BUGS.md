# Operant Bugs & Issues

**Last Updated**: 2026-08-03 (triaged against current `main` @ `322e424a`)
**Status**: MVP Deployment Phase — legacy issue list triaged; see audit below

> **Triage note (2026-08-03):** The previous version of this file (2026-06-19)
> predated the TDG→agentmemory, Obscura→igs, prompt-caching, streaming-scrubber,
> session-resume, and gateway tool-execution work. Every item below was verified
> against the current codebase before being marked resolved. Cross-reference:
> [`docs/HERMES_VS_OPERANT_AUDIT_2026-08-03.md`](HERMES_VS_OPERANT_AUDIT_2026-08-03.md)
> and [`docs/RUST_BEST_PRACTICES_PLAN.md`](RUST_BEST_PRACTICES_PLAN.md).

---

## Resolved (verified 2026-08-03)

### Critical (Blocks Deployment) — all resolved

- [x] **Compilation errors in test_phases_expanded.rs** — file removed; workspace compiles clean (`cargo check --workspace`: 0 errors, 15 lib test suites ok).
- [x] **Provider.rs lifetime mismatch (`fetch_models`)** — API reshaped; `operant-providers` compiles and 105 `Provider` impls build.
- [x] **Skin.rs `.cloned()` on MutexGuard** — TUI skin code restructured; no such call remains.
- [x] **Gateway test failure (`test_gateway_start_stop_with_disabled_platforms`)** — test present in `gateway_runner.rs` and passing in CLI test suite.

### High (Affects Functionality) — all resolved

- [x] **Config schema test failure** — `schema_export_contains_expected_contract_shape` passes (1 passed).
- [x] **Memory provider defaults not set (TDG)** — TDG removed; builtin/sqlite default + agentmemory REST provider (`agent_memory.rs`) wired through `load_memory_manager`.
- [x] **TTS provider defaults not set** — `default_tts_provider()` / `default_tts_voice()` in config schema.
- [x] **Dashboard API endpoints missing** — 30+ handler fns in `gateway/src/api.rs` (sessions, cost, tools, cron, nodes, canvas, plugins).
- [x] **WebSocket support missing** — full WS agent chat in `gateway/src/ws.rs` with streaming chunks, tool calls, session resume.
- [x] **Session resume/switch missing** — `resumed` + `message_count` in WS `session_start`, `SessionBackend` persistence.
- [x] **File operations incomplete** — `file_edit.rs`, `file_write.rs`, `file_state.rs` (patch/dedup/staleness).
- [x] **Tool executor missing** — `tools.rs` dispatch with error handling; runtime `loop_.rs` turn loop with retries.
- [x] **Turn context missing** — `agent/turn_context.rs` role alternation + evolution-state tracking.
- [x] **Delivery router missing** — platform routing/truncation in runtime `loop_.rs` + channel adapters.
- [x] **Slash command dispatch missing** — `resolve_command`/`handle_command` in `gateway_commands.rs` (executed in gateway).

### Medium (Enhancement) — all resolved

- [x] **Prompt caching not implemented** — `agent/clients/prompt_caching.rs` (cache_control blocks).
- [x] **Context compressor partial** — `llm_compressor.rs` (LLM compaction) + `context_compressor.rs` (auto-compaction triggers).
- [x] **Conversation compression missing** — `/compress` command in `gateway_commands.rs`.
- [x] **Auxiliary client missing** — `background_review.rs`, `llm_compressor.rs` side-LLM paths.
- [x] **Platform adapters incomplete** — 26 channel adapters in `operant-channels` (telegram, whatsapp, slack, discord, signal, mattermost, iMessage, irc, dingtalk, qq, twitter, mochat, wecom, clawdtalk, notion, reddit, bluesky, linq, wati, nextcloud, webhook, gmail/email, mqtt).
- [x] **Memory provider implementations missing (Honcho/Mem0/SuperMemory)** — superseded by the agentmemory integration (per product decision) plus sqlite/qdrant/postgres/lucid/markdown backends.
- [x] **Gemini provider missing** — `gemini.rs` + `gemini_cli.rs`.
- [x] **Bedrock provider missing** — `bedrock.rs` (SigV4).
- [x] **Model catalog missing** — `resolve_default_model` + model listing in config providers.
- [x] **MCP catalog missing** — `mcp.rs` manager + `mcp_oauth.rs`.
- [x] **Credential persistence missing** — `oauth_refresh.rs`, `credential_pool.rs`, `mcp_oauth.rs`.
- [x] **TODO tool missing** — `core/src/tools/todo_tool.rs`.
- [x] **Clarify tool missing** — `tools/ask_user.rs`.
- [x] **URL safety missing (SSRF)** — `core/src/tools/web_tools.rs` guardrails.
- [x] **Tool guardrails missing** — `approval.rs` risk-based approval + `requires_approval`.
- [x] **Error sanitization missing** — `sanitize_api_error` in providers.
- [x] **Plugin hooks missing** — `HookRegistry` / `gateway_pipeline.rs` pre/post hooks.
- [x] **check_fn TTL missing** — replaced by `Tool::is_available()` in `tools.rs`; availability computed per loop pass.

---

## Still Open (2026-08-03)

### Low (Nice to Have)

- [x] **`embedded-web` never embedded anything — the path was wrong, not the artifact** (FIXED iter-439). Previously filed as "the SPA dist isn't committed". That diagnosis was wrong in the direction that matters: the built dashboard IS committed, 15 files under `crates/operant-cli/src/dashboard/`, which is exactly where `crates/operant-cli/web/vite.config.ts` writes (`outDir: "../src/dashboard"`). The real defect was that `crates/operant-gateway/build.rs` and the `include_dir!` in `crates/operant-gateway/src/static_files.rs` both pointed at `<repo-root>/web/dist`, a location that has never existed in this layout. Because the build script's existence check ran against a directory the build never writes, `embedded_web_dist_available` was never set, so the `include_dir!` was never even compiled and `embedded-web` silently degraded to the `gateway.web_dist_dir` filesystem fallback on every machine and every build — a feature that reads as wired and embeds nothing. Both paths now resolve to `crates/operant-cli/src/dashboard`, and each carries a comment pointing at the other so they cannot drift apart silently again. Mutation-proven: pointing the embed at a nonexistent directory makes `include_dir!` panic (exit 101), which is what proves the cfg is now live and the embed actually runs — a green `cargo check --features embedded-web` alone cannot distinguish "embedded correctly" from "never compiled".

### Cross-references (engineering hygiene, see audit doc)

- **G2** `#![deny(missing_docs)]` — 0 deny-attrs across 10 lib crates; Phase 6.
- **G3** ~104 justified `expect()` in gateway need `#[expect]` escapes; Phase 2b.
- **G4** `--all-features` broken: runtime observability (otel/prometheus deps never declared) + hardware `include_str!` firmware + embedded-web dist; wire or remove.
- **G5** (this file) — triaged.
- **Phase 7** — hermes parity: tool-planning crate, telemetry crate, eval harness, `node_detail()`.
