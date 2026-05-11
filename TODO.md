# Hermes-RS TODO

## Implemented

- ReAct agent orchestration loop through `HermesAgent::run()`
- Shared TOML runtime configuration across `hermes-core` and `hermes-cli`
- Ratatui prompt-first TUI with conversation, reasoning, activity, MCP, skills, and behavior panels
- Streaming and non-streaming LLM request handling with tolerant reasoning/tool-call parsing
- Built-in file, patch, terminal, code execution, web, memory, and TODO tools
- GitHub Actions build, test, coverage, and release workflows with changelog-driven release notes
- Autonomous coding mode entrypoints: `hermes autonomous` and `hermes run --autonomous`
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
- Fixed 3 flaky/failing tests: test_agent_builder (missing DB), parse_args_rejects_empty_goal (whitespace validation), test_hermes_subdirs_are_children_of_home (env var race condition)
- Comprehensive audit report generated (REPORT.md) with 291/291 tests passing
- AUDIT.md generated with complete port mapping (58 HermesTool impls)
- MCP management tool (`mcp_tool.rs`): add/remove/list MCP servers
- Process management (`process_registry.rs`, `process_tool.rs`): long-running subprocess lifecycle
- Transcription tool (`transcription_tool.rs`): Groq/OpenAI Whisper audio transcription
- Web providers (`web_providers/`): Tavily, Exa, SearXNG, Brave, DuckDuckGo backends
- McpManager→ToolRegistry auto-registration wiring (mcp.rs:754 McpTool)

## Pending

### Phase 2 — High Priority Unported Tools

- **Skills ecosystem**: `skills_hub.py` (3,261 LOC) — community skills hub, the largest unported module
- **MCP OAuth**: `mcp_oauth.py` + `mcp_oauth_manager.py` (1,239 LOC) — required for MCP servers with OAuth
- **Security tools**: `tirith_security.py` (691 LOC) + `url_safety.py` (327 LOC) + `osv_check.py` (155 LOC)
- **Voice mode**: `voice_mode.py` (1,017 LOC) — CLI voice interaction mode
- **Skills guard**: `skills_guard.py` (932 LOC) — skill execution security policy
- **Wire McpManager→ToolRegistry**: Bridge auto-discovery of MCP server tools into ToolRegistry

### Phase 3 — Medium Priority

- **Credential files**: `credential_files.py` (436 LOC)
- **Skill usage tracking**: `skill_usage.py` (609 LOC)
- **Skills sync**: `skills_sync.py` (431 LOC)
- **Tool result storage**: `tool_result_storage.py` (232 LOC)
- **Website policy**: `website_policy.py` (282 LOC)
- **Browser Camofox**: `browser_camofox.py` (603 LOC)
- **Managed tool gateway**: `managed_tool_gateway.py` (167 LOC)
- **Gateway notification integration**: wire Telegram/Discord/Slack delivery into tools

### Phase 4 — Environments & Enterprise

- **Environments** (Python ~5,100 LOC): Docker, SSH, Modal, Daytona, Vercel Sandbox, Singularity sandboxing layer
- **Microsoft Graph**: `microsoft_graph_auth.py` + `microsoft_graph_client.py` (653 LOC)
- **Yuanbao**: `yuanbao_tools.py` (736 LOC) — Tencent integration

### Infrastructure & Hardening

- Configurable tool timeouts per tool (currently global only)
- Tool output size limits and truncation
- Schema generation review: ensure JSON schema accurately represents all tool parameters
- Improve error messages across all tools with actionable suggestions
- Add `cargo test` to CI (currently blocked by espeak-rs-sys audio backend linker issue on some platforms — see espeak_audio_stubs.c workaround)
