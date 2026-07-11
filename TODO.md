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
- **Phase B (in progress)** — dialog unification. DONE: R13 dead ToolPermissionDialog cluster deleted (iter-229), R11 dead App fields deleted (iter-230), fixed `any_modal_open` missing effort_picker (iter-227). ALSO landed opportunistic residual-bug fixes: R4 ask_user word_wrap unicode (iter-231), R5/R6 status-height + indicator width (iter-232). REMAINING: `Modal` trait + registry (replaces the 35-gate key chain, `close_secondary_views`, `any_modal_open`) + publish `OverlayOpened/Closed` from it; shared `ListNav`/filter/scroll helpers; R14 dead `render_message` subtree (verified dead, ~500 lines, delete with its tests); R7 bypass dialog (wire `--dangerously-skip-permissions` or delete — security-sensitive, don't rush); R10 duplicate slash arms; R9 stub slash commands; investigate `/tasks` (no dispatch arm).
- **Phase C** — render pipeline: fix streaming full-transcript re-render, kill per-frame deep clones, one `text_util` wrap module (fixes R4/R5/R6), delete dead `render_message` subtree, split `prompt_input.rs`
- **Phase D** — app decomposition + residual bugs: shrink `handle_key_event`, delete dead App fields (R11), honest `SessionRecord` mapping (R1/R2), real cost from `AgentEvent::Cost` (R3), split `app.rs`
- **Phase E** — config consolidation: deprecate `config.yaml` + remove `"gpt-4"` precedence heuristic
- **Phase F** — hygiene: clippy 498 → 0 with `-D warnings` in verify path, author `workspace-lint.yaml`, archive stale `BUGS.md`

### Infrastructure & Hardening

- Configurable tool timeouts per tool (currently global only)
- Tool output size limits and truncation
- Schema generation review: ensure JSON schema accurately represents all tool parameters
- Improve error messages across all tools with actionable suggestions
- Add `cargo test` to CI (currently blocked by espeak-rs-sys audio backend linker issue on some platforms — see espeak_audio_stubs.c workaround)
