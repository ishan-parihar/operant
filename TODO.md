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
- Comprehensive audit report generated (REPORT.md) with 285/285 tests passing

## Pending

### Phase 1 — P1 Tools & Infrastructure

- MCP tool invocation: expose McpManager tools as an agent-invocable "mcp_tool" via ToolRegistry
- Web scrape/crawl tool: extract structured content from pages beyond basic fetch
- Web search provider abstraction: support Tavily, Exa, Searxng backends alongside hardcoded DDG
- Process management/registry: track and manage long-running subprocesses
- Transcription/audio tool: port from Python transcription_tools.py (whisper, etc.)
- Computer use / UI interaction tool: agent-driven desktop interaction
- File upload/download tool: binary file operations with size limits
- Gateway notification integration: wire Telegram/Discord/Slack delivery into tools

### Infrastructure & Hardening

- Configurable tool timeouts per tool (currently global only)
- Tool output size limits and truncation
- Schema generation review: ensure JSON schema accurately represents all tool parameters
- Improve error messages across all tools with actionable suggestions
- Add `cargo test` to CI (currently blocked by espeak-rs-sys audio backend linker issue on some platforms — see espeak_audio_stubs.c workaround)
