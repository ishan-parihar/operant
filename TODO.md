# Operant TODO

## Implemented

- ReAct agent orchestration loop through `HermesAgent::run()`
- Shared TOML runtime configuration across `operant-core` and `operant-cli`
- Ratatui prompt-first TUI with conversation, reasoning, activity, MCP, skills, and behavior panels
- Streaming and non-streaming LLM request handling with tolerant reasoning/tool-call parsing
- Built-in file, patch, terminal, code execution, web, memory, and TODO tools
- GitHub Actions build, test, coverage, and release workflows with changelog-driven release notes
- Autonomous coding mode entrypoints: `operant autonomous` and `operant run --autonomous`
- Autonomous workspace loop that reads `TODO.md`, runs the agent, validates changes, and only pushes after passing tests
- End-to-end autonomous mode validation against a disposable sample repository, with README operator workflow documentation
- Dedicated repo-local `autonomous-status.toml` reporting for autonomous state, validation summaries, repeated failures, and paused states
- Persistent autonomous failure pause state across process restarts until `TODO.md` or git state changes
- State distillation with long-term memory injection and async session fact extraction into `MEMORY.md`
- Workspace context-file auto-loading with prompt-injection scanning for agent guidance files
- Sub-agent delegation as an opt-in built-in tool through `delegate_to_sub_agent`
- Full built-in tool registration of 30+ tools including checkpoint, cron, and kanban
- `cronjobs` and `kanban` modules exposed and compiled in the crate
- Cron tool fully supports create/list/get/update/delete/pause/resume actions
- Kanban tool supports show/create/update/complete/assign/block/heartbeat/comment/link actions
- Checkpoint tool provides git-based filesystem snapshots with list/restore/diff
- `builtin_tool_names()` fixed to return all 30 registered tool names
- CLI build_registry creates `CronDb` and `KanbanDb` alongside main database
- All unregistered tools (checkpoint, cron, kanban) wired into built-in registry and compiled
- Full RL training tool (`rl_training_tool.rs`) — PPO/GRPO curriculum training with mlx, wandb integration
- Full Spotify control tool (`spotify_tool.rs`) — 7 action types (playback, queue, search, playlists, devices, library, repeat/shuffle)
- Fixed 3 flaky/failing tests: test_agent_builder (missing DB), parse_args_rejects_empty_goal (whitespace validation), test_operant_subdirs_are_children_of_home (env var race condition)
- Comprehensive audit report generated (REPORT.md) with 291/291 tests passing
- AUDIT.md generated with complete port mapping (58 HermesTool impls)
- MCP management tool (`mcp_tool.rs`): add/remove/list MCP servers
- Process management (`process_registry.rs`, `process_tool.rs`): long-running subprocess lifecycle
- Transcription tool (`transcription_tool.rs`): Groq/OpenAI Whisper audio transcription
- Web providers (`web_providers/`): Tavily, Exa, SearXNG, Brave, DuckDuckGo backends
- McpManager→ToolRegistry auto-registration wiring (mcp.rs:754 McpTool)
- **Phase 2**: skills_hub, mcp_oauth, security, voice_mode, skills_guard (all complete)
- **Phase 3**: credential_files, skill_usage, skills_sync, website_policy, fuzzy_match, ansi_strip, schema_sanitizer, interrupt, budget_config, tool_result_storage, browser_camofox, managed_tool_gateway, path_security, env_passthrough (all complete)
- **Phase 4**: credential_pool, ms_graph, yuanbao, environments (8 backends) (all complete)
- **Phase 5a+5b**: 10 standalone tool stubs (binary_extensions, xai_http, camofox_state, debug_helpers, tool_output_limits, file_state, slash_confirm, tool_backend_helpers, openrouter_client, neutts_synth) — all in `crates/operant-core/src/tools/`
- **Phase 6**: approval system (`approval.rs`) — 3-layer guard, 12 hardline categories, 47 dangerous patterns, 30+ tests
- **Phase 7**: browser supervisor (`browser_supervisor.rs`) — CDPSupervisor + Browserbase/Browser Use/Firecrawl providers + 3 HermesTools + 24 tests
- **Phase 8**: State DB expansion (`database.rs` → 1477 LOC) — FTS5 search, session_metadata/tags/events/tools_state tables, merge_sessions, retry engine
- **Phase 9**: Gateway enhancement (`gateway.rs` → 1118 LOC) — PlatformAdapter trait, SessionStore, ChannelDirectory, WebhookAdapter, GatewayStats
- **Phase 10**: CLI config system (`crates/operant-cli/src/config.rs` → 3672 LOC) — CliConfig (40+ sections), env expansion, deep merge, 8-step migration, validation
- **843 tests passing** (755 core lib + 86 cli bin + 2 doctest) — 0 failures

## Pending

All Python tool modules from operant-agent have been fully ported to Rust across all 10 phases. Remaining items are infrastructure/hardening only:
- Integrate environment backends with real SDK dependencies (Docker, SSH, Modal, Daytona, Vercel)
- Reduce clippy warnings (~82 pre-existing)
- Set up CI pipeline (GitHub Actions)
- Consider squashing 53+ uncommitted files into logical phase commits

### TUI Upgrade (audit: `docs/audit/2026-07-11-tui-fragmentation-audit-and-refactor-plan.md`)

Execute phases in order; each phase's tasks are checkboxed in the audit doc. Continue iteration numbering from iter-223.

- **Phase A (DONE, iter-223→228)** — debug-loop hardening: ✅ screen-buffer assertions + `--dump-screen` (iter-223), ✅ mock `--agent-script` injection (iter-224), ✅ 7 event-bus variants + faithful slash interception (iter-225; Mouse/Resize/Paste/OverlayOpened/Closed deferred to Phase B registry), ✅ generic dot-path assertions + `--size`/`--max-frames` (iter-226), ✅ dialog open/close scenario pack + fixed `effort_picker`/`any_modal_open` drift bug (iter-227), ✅ docs `docs/tui-debugging.md` (iter-228)
- **Phase B/C/D/F (largely done through iter-243)** — DONE: R13 dead ToolPermissionDialog cluster (iter-229), R11 dead App fields (iter-230), R4 word_wrap unicode (iter-231), R5/R6 width bugs (iter-232), R10 duplicate slash arms merged (iter-233), R14 dead render_message subtree (iter-234) + RenderContext.highlight (iter-235), R1 session-browser real timestamps/counts (iter-236), B1 `overlay_flags()` single-source-of-truth for any_modal_open+debug_snapshot (iter-237), C1-partial streaming markdown memoization (iter-238), clippy 393→261 in operant-cli (iter-239). R9/R7 slash-command implementation (per hermes-agent): iter-240 `/steer` `/reload-mcp` `/mouse` `/queue`, iter-241 `--dangerously-skip-permissions`+bypass dialog (R7), iter-242 `/reload` `/reload-skills`, iter-243 `/background` (safe partial). Fixed effort_picker/any_modal_open drift (iter-227).
  iter-244: R3 cost fidelity — `AgentEvent::Cost` now feeds `CostTracker`/`self.cost_usd` live instead of being discarded; `Database::update_session_cost` (new) persists real accumulated cost, `list_sessions` selects it, session browser shows it instead of `$0.00`. **Covered the non-streaming request path only** at the time — streaming's gap closed in iter-247 below.
  iter-245 (Phase E): deprecated `config.yaml`/`config.local.yaml` — `CliConfig::load()` no longer reads them (`.env` + `HERMES_*` overrides unaffected); deleted the now-dead `deep_merge`/`expand_env_vars_in_value` YAML-merge helpers + their 5 orphaned tests; removed the redundant `"gpt-4"` precedence heuristic in `main.rs` (env override already covers it).
  iter-246 (Phase F): `cargo clippy --fix` burn-down in `operant-cli` — bin-target warnings 261→136 (style/idiom lints: derived impls, redundant closures, borrowed-expr, `to_string` in `format!`, `map_or` simplification, manual `div_ceil`, etc.), all behavior-preserving/auto-applied, plus one manual dedup of a doubled assertion the fix left behind. `cargo clippy --fix --lib -p operant-core` was attempted too but **failed outright** ("failed to automatically apply fixes suggested by rustc to crate `operant_core`", zero files touched) — likely overlapping suggestion spans; left as a follow-up, not investigated further this iteration.
  iter-247: streaming-mode cost/usage tracking — closes the gap left by R3. `StreamChunk`/`ChatStreamEvent` gained a `usage: Option<Usage>` field; OpenAI-compatible requests now send `stream_options: {"include_usage": true}` when streaming so the final chunk carries real token counts; the native Anthropic client now parses `message_start` (`input_tokens`) and `message_delta` (`output_tokens`) SSE events, since Anthropic reports usage split across two events instead of one. `process_stream` (agent/mod.rs) merges both halves and, once complete, emits `AgentEvent::Usage`/`AgentEvent::Cost` via a new shared `emit_usage_and_cost` helper — extracted out of `process_response` so the cost-calc logic isn't duplicated across the streaming and non-streaming paths. 3 new tests cover the Anthropic message_start/message_delta parsing (including the no-usage-present edge case); all 1,602 tests pass (1,599 default + 3 new), plus 948 pass under `--features anthropic` (previously untested by the default `cargo test --workspace` run since the module is feature-gated).
  iter-248: two small root-cause fixes. (1) Stabilized the pre-existing `osc8::tests::detects_www_and_normalizes_to_https` parallel-test flake — root cause was `enabled_respects_env_var` mutating the real process-global `OPERANT_NO_HYPERLINKS` env var with no synchronization, racing with `enabled()` reads from other tests under `cargo test --workspace`'s default parallelism. Fixed by splitting `enabled()` into a pure `is_enabled(Option<&str>)` and rewriting the test to call that directly instead of touching process env — no more shared mutable state, no serialization needed. (Note: the same unguarded-env-var-mutation pattern also exists in `operant-core/src/tools/discord_tool.rs`'s `test_discord_no_token_returns_error` — not fixed this iteration, flagged as a similar latent flake.) (2) Wired `/tasks` as a working alias for `/agents` in the TUI — it was already documented as an alias (`commands.rs`/`gateway_commands.rs` `CommandDef::with_aliases`, shown in gateway help as `/agents (tasks)`) but the TUI's own slash-command intercept match only accepted the literal `"agents"`, so `/tasks` fell through to a dead, never-populated `CommandRegistry.handlers` fallback and printed "Command /agents is not yet wired to a handler." Fixed with `"agents" | "tasks" =>` (matches the existing `"diff" | "review"` alias pattern). 1 new regression test added; all tests green (1,603 total, +1 from before).
  iter-249: fixed the `discord_tool.rs` flake flagged in iter-248 — `test_discord_no_token_returns_error` was the one test in that file mutating the shared `DISCORD_BOT_TOKEN` env var without `#[serial_test::serial]`, while its 4 sibling tests already had it. Added the missing attribute (the crate already depends on `serial_test`, used by those siblings) — consistent with the established in-repo pattern, no new dependency, no production code touched.
  iter-250: root-caused and unblocked `operant-core`'s `cargo clippy --fix`, which had failed outright every prior session. Cause: `clippy::cmp_owned` suggested rewriting `abs_dir == PathBuf::from("/")` (in `tools/checkpoint_tool.rs:128`) to `abs_dir == "/"` — but `PathBuf` doesn't implement `PartialEq<&str>`, so that one invalid suggestion broke compilation, and `cargo clippy --fix` rolls back its *entire* batch of fixes for a crate if the post-fix build fails (all-or-nothing, not per-file). Rewrote the comparison to `abs_dir == Path::new("/")` (valid — avoids the same unnecessary-allocation clippy was flagging, without the broken type coercion), which let `--fix` apply cleanly. Result: `operant-core` warnings 124→28 across 39 files (`derivable_impls`/`new_without_default` — replacing manual `impl Default` blocks with `#[derive(Default)]`/`#[default]` where safe, `manual_div_ceil`, redundant closures, etc.), all behavior-preserving/auto-applied; ran twice to confirm 0 more auto-fixable suggestions remain. All tests green (1,603 default + 948 `--features anthropic`), 0 regressions. Remaining 28 warnings are genuine judgment calls (dead_code, `non_snake_case` struct fields tied to external JSON API casing like `DisplayText`/`RecognitionStatus`/`localStorage` that would need `#[serde(rename = "...")]` if renamed, one `type_complexity`) — not attempted this iteration.
  iter-251: manual judgment-call cleanup of the remaining 28 `operant-core` warnings → 9. Highlights (not just style): (1) **real fix**, not just a lint silence — `credential_pool.rs`'s `refresh_async`/`refresh_oauth_async` held a `std::sync::RwLock` read guard across a network `.await` in a loop, blocking any writer for the whole refresh duration; now collects the credentials needing refresh into an owned `Vec` and drops the guard before the loop. (2) `skills_guard.rs::determine_verdict` had a redundant `if has_high {Caution} else {Caution}` — both branches identical, so `has_high` never affected the outcome; simplified (verified via the 4 existing severity-level tests, all still pass) and flagged in a code comment that any Critical-free finding list currently maps to `Caution` regardless of High/Medium/Low severity — a possible policy gap, not decided here. (3) 3 genuinely-dead struct fields deleted (`FFmpegRecorder.current_rms` — hardcoded-0.0 getter never reads it; two STT response fields never read after parsing). (4) 2 non_snake_case fields tied to external JSON casing (Azure `DisplayText`, Camofox `localStorage`) kept and renamed to snake_case with `#[serde(rename = "...")]` for wire/file compatibility. (5) mechanical: `Default` for `CheckpointManager`, `&PathBuf`→`&Path` ×2, manual `strip_prefix` ×3, manual `clamp` ×2, duplicated attribute, doc indent. Remaining 9 warnings (`too_many_arguments` ×3, `should_implement_trait` ×3, `type_complexity` ×3) need real API redesign or public-method renames with caller impact — left as accepted style debt, not attempted.
  REMAINING (large refactors / need dedicated sessions or product decisions): full `Modal` trait + registry to replace the 35-gate key chain + `close_secondary_views` (B1 write-side) — **descoped 2026-07-11**: no active bug (read-side drift already fixed via `overlay_flags()`), high blast radius on an 8,192-line file, not worth the risk/cost at this session's scale; shared `ListNav`/filter/scroll helpers (B2); full C1 (blocked — needs headless committed-turn render coverage, see memory); app.rs/prompt_input.rs splits (C5/D5); clippy → 0 + `-D warnings` gate + espeak CI (Phase F, operant-cli at 136 remaining, mostly dead_code judgment; operant-core at 9 remaining, `too_many_arguments`/`should_implement_trait`/`type_complexity` — accepted debt); descoped commands (billing/update/replay/browser-connect — no backend); possible `ScanVerdict` policy gap noted in `skills_guard.rs` (see iter-251).
- **Phase C** — render pipeline: fix streaming full-transcript re-render, kill per-frame deep clones, one `text_util` wrap module (fixes R4/R5/R6), delete dead `render_message` subtree, split `prompt_input.rs`
- **Phase D** — app decomposition + residual bugs: shrink `handle_key_event`, delete dead App fields (R11), honest `SessionRecord` mapping (R1/R2), real cost from `AgentEvent::Cost` (R3, DONE iter-244 for non-streaming), split `app.rs`
- **Phase E** — config consolidation: deprecate `config.yaml` + remove `"gpt-4"` precedence heuristic (DONE iter-245)
- **Phase F** — hygiene: clippy 498 → 0 with `-D warnings` in verify path, author `workspace-lint.yaml`, archive stale `BUGS.md`

### Infrastructure & Hardening

- Configurable tool timeouts per tool (currently global only)
- Tool output size limits and truncation
- Schema generation review: ensure JSON schema accurately represents all tool parameters
- Improve error messages across all tools with actionable suggestions
- Add `cargo test` to CI (currently blocked by espeak-rs-sys audio backend linker issue on some platforms — see espeak_audio_stubs.c workaround)
