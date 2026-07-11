# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Autonomous coding mode through `operant autonomous` and the `operant run --autonomous` compatibility alias
- Shared `[autonomous]` runtime configuration for autonomous polling interval, TODO path, status report path, validation command, git target, commit message, command timeout, and repeated-failure pause threshold
- Repo-root `TODO.md` task ledger with `Implemented` and `Pending` sections for autonomous workspace planning
- Repo-local `autonomous-status.toml` status reports that capture autonomous state, validation results, failure summaries, and last push targets
- Disposable-repo autonomous validation coverage that exercises the full tick loop without a live model call
- Long-term memory injection into agent system prompts via `<long_term_memory>` context built from durable `MEMORY.md` facts
- Async state distillation that extracts durable session facts into repo-local `MEMORY.md` after completed agent runs
- Workspace context-file auto-loading for `AGENTS.md`, `CLAUDE.md`, `.operant.md`, `HERMES.md`, and `.cursorrules` with prompt-injection scanning and truncation
- `delegate_to_sub_agent` tool for opt-in isolated child-agent delegation from the parent ReAct loop
- Headless TUI simulator (`operant tui debug simulate`) that drives the real TUI loop against a test backend, with `--dump-screen`/`--assert-screen` (rendered-screen assertions), `--agent-script` (deterministic offline mock agent events), dot-path state assertions over the App debug snapshot, and `--size`/`--max-frames` guards; documented in `docs/tui-debugging.md`
- `--dangerously-skip-permissions` flag that shows a confirmation dialog at startup and, on accept, runs the session in permission-bypass mode

### Changed

- TUI slash commands `/steer`, `/queue`, `/reload-mcp`, `/reload`, `/reload-skills`, and `/mouse` now perform their real actions instead of printing placeholder status (steer injects mid-turn guidance, `/reload-mcp` reconnects MCP servers live, `/reload-skills` rescans the skills directory, `/mouse` reports live capture state)
- README, `AGENTS.md`, and `CLAUDE.md` now document the autonomous workflow, the role of `TODO.md` as the workspace task source of truth, and the disposable operator workflow for validating autonomous mode safely
- Repeated autonomous failure pauses now persist across process restarts until `TODO.md` or git state changes, using `autonomous-status.toml` as the durable state store
- CLI, TUI, and autonomous sessions now reload persisted long-term memory from the current workspace before constructing agents
- The TUI workspace now uses the desktop split at 120 columns and gracefully collapses secondary panels into popups below 65 columns or 20 rows
- TUI agent reasoning now renders with quote rails, and tool activity now renders as compact blocks for clearer tool-call scanning

### Fixed

- Autonomous command execution now runs in blocking isolation with strict exit-status checks so failed validation cannot fall through to git push
- Autonomous status tracking no longer dirties workspace fingerprints or staged commits with the runtime status file itself
- TUI layout rows now preserve the primary conversation area in cramped terminals instead of letting fixed chrome starve the workspace body
- The effort picker now registers as an open modal, so background keys no longer leak through while it is visible
- `/resume` session browser now shows real last-activity timestamps and message counts instead of placeholder "just now" / 0 values
- CJK/emoji text in the ask-user dialog, status row, and new-messages indicator now wraps and sizes by display width instead of byte length
- Live TUI cost display now uses the real model-aware per-request cost from `AgentEvent::Cost` instead of a flat-rate estimate; the `/resume` session browser now shows the real accumulated cost per session (persisted via a new `Database::update_session_cost`) instead of always `$0.00` (R3 — non-streaming request path)
- Streaming mode now reports real token usage and cost too, closing the R3 gap: OpenAI-compatible providers get `stream_options.include_usage` on streamed requests and a `usage` field on the final chunk; the native Anthropic client now parses `message_start`/`message_delta` usage events. Both merge in `process_stream` and emit `AgentEvent::Usage`/`AgentEvent::Cost` the same way the non-streaming path already did

### Removed

- Dead TUI code with no reachable callers: the legacy `ToolPermissionDialog` cluster, the `render_message` renderer family, the `RenderContext.highlight` field, and five unused `App` fields (~1,400 lines total)
- Legacy `config.yaml`/`config.local.yaml` loading from `CliConfig::load()` — `operant.toml` is now the sole file-based config source; `.env` loading and `HERMES_*` env overrides are unaffected. Also removed the now-dead `deep_merge`/`expand_env_vars_in_value` YAML-merge helpers and the redundant `"gpt-4"` model precedence heuristic in `main.rs` (env-based `HERMES_MODEL` override already covers it)
- ~125 style/idiom clippy warnings in `operant-cli`'s bin target (derived-impl opportunities, redundant closures, unnecessary borrows, `to_string` in `format!` args, simplifiable `map_or`, manual `div_ceil`, and similar), via `cargo clippy --fix` — behavior-preserving only

## [0.1.3] - 2026-04-20

### Added

- Prompt history in the TUI input box, with `Up` / `Down` navigation that replays recent prompts and restores the current draft when you leave history browsing
- New README screenshots for the landing screen and workspace chat flow in `assets/main.png` and `assets/chat.png`
- Project-context sections in `AGENTS.md` and `CLAUDE.md` so future coding agents can immediately understand the current config, TUI, and release workflow expectations

### Changed

- Conversation rendering now follows the newest assistant output by default while still allowing manual scrolling with `Up`, `Down`, `PageUp`, `PageDown`, `Home`, and `End`
- Prompt mode keeps chat scrolling available through paging keys, so long replies remain readable even while the input box is focused
- Workspace UI now labels active assistant output as `responding` when `stream = false` instead of incorrectly showing `streaming`
- README now documents the prompt history keys, conversation scrolling behavior, screenshots, and release-driven documentation sync expectations

### Fixed

- Streaming provider tool-call deltas now tolerate missing `index` fields, which prevented some NVIDIA NIM tool runs from completing in `stream = true` mode
- Streaming tool-call parsing now merges incremental argument chunks and strips split `<tool_call>` XML tags from visible conversation output more reliably
- Non-streaming mode now parses XML tool calls embedded in assistant content instead of leaving tool execution text stranded in the reasoning pane
- Final assistant replies and tool outputs now land in the conversation pane more consistently instead of leaving the workspace stuck on older chat content
- Non-Windows CI and tarpaulin coverage now pass the join-error TUI test by using terminal-free run-result assertions instead of any real TTY-backed terminal setup

Full Changelog: [v0.1.2...v0.1.3](https://github.com/eikarna/operant-rs/compare/v0.1.2...v0.1.3)

## [0.1.2] - 2026-04-20

### Added

- Workspace follow-up prompting now returns to prompt mode automatically after both completed and failed runs, so a user can continue the same session without clearing history
- Regression coverage for Windows key handling, landing prompt bootstrap, follow-up prompting after errors, and activity-pane failure rendering

### Changed

- Runtime and operational errors in the rich TUI are now summarized in the footer while their detailed text is rendered in the `Activity` pane
- Activity entries now render as compact single-line log rows so failures stay visible in narrow panel heights
- README now documents the TOML configuration model, `operant.example.toml`, and the current ratatui-based interactive workflow

### Fixed

- Windows and PowerShell landing screen now paints an explicit dark canvas instead of inheriting a gray terminal background
- Landing prompt entry now accepts immediate typing on the first screen while still preventing duplicated characters from key-release events
- Landing status/footer no longer duplicates `idle` or `run failed`
- Current chat sessions can accept a new prompt after runtime errors without conflicting with agent self-healing logic

Full Changelog: [v0.1.1...v0.1.2](https://github.com/eikarna/operant-rs/compare/v0.1.1...v0.1.2)

## [0.1.1] - 2026-04-19

### Added

- Shared TOML-backed `AppConfig` runtime configuration across `operant-cli` and `operant-core`
- Config discovery with precedence `defaults < operant.toml/.operant.toml/config.toml < env vars < CLI flags`
- Config sections for client, agent behavior, TUI, MCP, skills, gateway, and tool/runtime defaults
- Responsive ratatui application architecture with landing and workspace views split across dedicated state, action, form, render, and app modules
- In-TUI panels for Session, MCP, Skills, and Behavior management, including modal forms for MCP server creation, skill creation, and behavior editing
- Example-config parse coverage to keep `operant.example.toml` synchronized with the Rust config schema
- GitHub release workflow that extracts matching release notes from `CHANGELOG.md` and publishes tagged build artifacts to GitHub Releases

### Changed

- Replaced ad hoc CLI-only config parsing with shared core config loading and runtime installation
- Moved runtime-tunable defaults and provider/tool endpoints out of scattered literals in `client`, `agent`, `gateway`, `web_tools`, `http_tool`, `terminal_tool`, and `code_execution`
- Reworked rich `run` and `chat` flows to launch the new TUI instead of the previous single-screen live monitor
- Updated build workflow artifact naming so tag builds can be promoted directly into release assets
- Bumped crate versioning to `0.1.1`

### Fixed

- Reasoning, MCP, skills, and behavior state now render as dedicated TUI surfaces instead of raw merged text
- Missing or invalid TOML configuration files now fail with user-facing diagnostics instead of silently falling back

## [0.1.0] - 2026-04-17

### Added

- ReAct orchestration loop with streaming-first architecture
- Tolerant XML parser for tool call detection with early execution
- OpenAI API client with SSE streaming support
- Dynamic JSON Schema generation from Rust structs (`schemars`)
- Tool registry with 17 built-in tools:
  - `file_read`, `file_write`, `terminal`, `code_execution`
  - `web_search` (DuckDuckGo Lite scraper), `web_fetch`, `http_request`
  - `datetime` with timezone offsets and advanced formatting
  - `memory_store`, `memory_search`, `memory_profile`
  - `todo` (in-memory task list), `clarify` (agent-to-user questioning)
  - `patch` (find-and-replace file patching)
- MCP protocol client with HTTP and stdio transports
- Persistent file-backed memory (`MEMORY.md` / `USER.md`) matching Python agent format
- Skills system with YAML front matter parsing
- Gateway adapters for Telegram, Discord, and Slack
- Context window management with compression
- RL trajectory export
- Cross-platform utilities (`platform.rs`) for shell detection, config dirs, file permissions
- CLI with `run`, `chat`, `tools`, and `test` subcommands
- 99 unit and integration tests
- CI/CD pipelines: lint (rustfmt + clippy + docs), build (3 native + 6 cross-compiled targets), test (3 platforms + coverage)

### Changed

- Switched TLS backend from `native-tls` (OpenSSL) to `rustls-tls` (pure Rust) for cross-compilation support

### Dependencies

- Rust MSRV: 1.86
- `tokio` 1.36, `reqwest` 0.12 (rustls), `serde` 1.0, `clap` 4.5
- `schemars` 0.8, `tracing` 0.1, `anyhow` 1.0, `thiserror` 1.0

[0.1.0]: https://github.com/eikarna/operant-rs/releases/tag/v0.1.0
[0.1.1]: https://github.com/eikarna/operant-rs/releases/tag/v0.1.1
[0.1.2]: https://github.com/eikarna/operant-rs/releases/tag/v0.1.2
[0.1.3]: https://github.com/eikarna/operant-rs/releases/tag/v0.1.3
