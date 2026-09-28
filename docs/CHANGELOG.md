# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

Parity work against the `jcode` reference agent (iters 347-409). The TUI
gained roughly 3,000 lines; 1,146 lines of long-dead code were deleted.

### Fixed

- **`/steer` was a guaranteed no-op.** Submitting it required `Enter` while not
  streaming, but the queue path rejected exactly that state — the two halves
  were mutually exclusive, so the command had never worked despite being
  documented. The core loop's steering was fully implemented and unreachable.
- **A compaction cut could orphan a `tool_use`/`tool_result` pair.** Eviction
  keeps a head and a recency tail, so a cut could land between a request and its
  answer; providers reject either half alone, and an unanswered request leaves
  the model waiting so it reissues the call and burns a turn. A shape-independent
  repair now runs at the single chokepoint both eviction paths pass through.
- **`/keys` advertised two chords that never worked.** `Ctrl+H`/`Ctrl+L`
  ("previous/next history") had no dispatch arm anywhere, and cannot be bound
  anyway — `Ctrl+h` is ASCII 8 (backspace) and `Ctrl+l` is readline's
  clear-screen. The claims were removed and both chords added to the terminal
  conflict list that `/hotkeys` uses to explain unavailability.
- **`operant-core`'s `simple_agent` example did not compile** (a config field
  added in iter-357 was missing from its struct literal). Note that
  `cargo check --workspace` does NOT catch this — only `cargo test --workspace`
  builds example targets.

### Added

- **Turn lifecycle state machine** — a 9-state `TurnState` (idle, sending,
  connecting, thinking, streaming, running-tool, awaiting-approval,
  waiting-for-network, compacting) replaces the single `is_streaming` bool for
  display, plus per-turn wall-clock, tokens/sec, and input/output/cache deltas
  in the footer.
- **Tool-call grouping** — concurrent calls group with an `N/M done` meter and
  per-sub-call status, and a call waiting on the tool-pool semaphore now
  renders as queued rather than running.
- **Real markdown rendering** — `pulldown-cmark` drives lists (nested, ordered,
  task), italic, strikethrough, horizontal rules and footnotes. Previously
  `- item` printed literally and the renderer claimed italic support it did not
  have.
- **One clipboard path** with a five-mechanism fallback chain
  (arboard → wl-copy → xclip → xsel → OSC 52). Writes previously failed
  silently over plain SSH. Drag-select copy mode added.
- **Eight live themes** including a deuteranopia-safe palette, `reduce_motion`
  now honoured by the animation path, and Kitty/iTerm2/Sixel image rendering
  wired to paste.
- **Truecolor detection** with perceptual xterm-256 quantization, and
  grapheme-cluster + cell-width measurement (a CJK ideograph is one cluster but
  two cells, so cluster counting alone is still wrong).
- **Mermaid diagrams** render as images through the existing image stack, with
  text and raw-source fallbacks, on a background worker.
- **Provider factory registry** — a provider is now one trait impl plus one
  line, with a structural `RuntimeKey` (not a display string) and a typed
  `FailoverDecision` distinguishing a rate limit from a broken model.
- **OAuth PKCE + device-code login** with an account pool that stores only keys
  and order (never tokens) and a per-account single-flight refresh coordinator,
  so N concurrent requests cannot each spend the same single-use rotating
  refresh token.
- **Destructive-command gate** hooked into the existing approval path —
  structural blast-radius classification, a justification gate that measures
  substance rather than length, and system-path denial. Defense-in-depth, not a
  sandbox.
- **Cache-miss detection** — a prefix tracker over the stable prefix (system
  prompt + tools + skills), with proven and inferred verdicts reported
  separately. A detector reporting a hit it cannot know is worse than none.
- **Tool-name recovery** — a hallucinated or near-miss tool name is recovered by
  bounded Levenshtein, and the model is told the recovery happened. It refuses
  on an equidistant tie, because arbitrarily choosing is how you get a
  confidently wrong action.
- **Swarm workers** — real headless workers running their own agent loop, with a
  spawn guard that checks depth and a *global* breadth cap before spawning and
  releases its count on every exit path.
- **Keybinding registry wired** — `/keys` and `/hotkeys` read the catalogue and
  count real usage; input stash (`Ctrl+S`), burst undo (`Ctrl+Z`) and queued
  message recall.

### Changed

- **Oversized tool results are withheld, never silently truncated.** A result
  that would exceed `agent.max_tool_result_share` (default 5% of the context
  window) is replaced by a marker stating the kept head, the token count, the
  per-token price and the tokens removed.
- **1146 net lines of dead TUI deleted** — `voice_capture.rs`, `state.rs` and
  `terminal.rs` removed outright, `messages/cache.rs` superseded. Three sibling
  files had carried `#![allow(dead_code)] // wired in Phase 2I` with zero
  consumers.

### Known limitations

- **MCP turn-triggered materialisation is net-negative on short turns.** On a
  realistic 40-tool catalog a 4-token generic turn materialises 24 of them, so
  it *increases* the visible schema. Kept as a convenience (a relevant tool
  without a discovery turn), not a token optimisation; the number is pinned in
  a test.
- **Hybrid retrieval scoring is unproven and deliberately unwired.** Measured
  recall got *worse* (R@3 2/5 → 1/5). A mutation-proven gate fails the build if
  anything outside `retrieval.rs` calls it.
- **Themes still do not reach the whole TUI.** iter-414 migrated every
  foreground `Color::{White,Yellow,Cyan,Red,Green}` to a palette accessor
  (accessor calls 236 → 530), so the bulk of the named variants are now themed.
  Still unmigrated: 89 explicit `Color::Rgb`/`Color::Indexed` literals (~80
  distinct values, each a design decision rather than a refactor), plus
  `DarkGray` (194) and `Black` (28), which are context-dependent between
  border/muted/disabled and between bg and fg-on-selection. Five foreground sites
  are intentionally left hardcoded where the colour means a *syntax* or *tool
  category* role the palette does not model.
- **`operant acp` does not implement the Agent Client Protocol.** It is a
  hand-rolled JSON-RPC with four methods — `ping`, `status`, `command`, `stop`
  behind a version-validated envelope — and shares only a name with ACP. It has
  no `initialize`, `session/new`, `session/prompt`, `session/cancel` or streaming
  `session/update`, so it would not interoperate with any real ACP client.
- **There is no SDK server and no harness-api-server.** The `operant-harness`
  crate exposes no API surface; `operant status --json` and
  `architecture dump --live` are the only machine-readable surfaces today.


## [0.2.0] - 2026-09-27

### Added

- **Lossless Context Management (LCM) engine** (`agent.context_engine = "lcm"`) — hermes-lcm parity: an append-only SQLite DAG keeps every message verbatim (FTS5-indexed) while the fresh-tail (D0) window stays in context. Opt-in; the built-in `compact` engine remains the default.
- **LCM rollups (P1)** — on-demand day/week/month LLM summaries (`operant context rollup`), stored idempotently in `lcm_rollups`; over-budget contexts inject stored rollups into compaction (`context_lcm_rollups_inject`, default on) so the model reads condensed history while the DAG stays lossless.
- **LCM maintenance** — on-demand sweep (`operant context rollup-maintenance`) and a config-gated background scheduler (`context_lcm_rollup_interval_minutes`, 0 = off) that build missing rollups for all sessions; a bad LLM pass is logged and never aborts.
- **LCM agent tools (P2)** — `lcm_recall` (verbatim FTS recall), `lcm_stats` (engine diagnostics), `lcm_assert` (durable, conflict-preserving fact store with active-state resolution and contradiction reporting), and `lcm_recall_round` (multi-round evidence-gated retrieval with cumulative exact evidence and search leads).
- **LCM adaptive auto-recall (P3)** — one bounded retrieval round per assemble against the latest user message injects a system "pre-answer evidence" block (`context_lcm_auto_recall`, default on).
- **AFT tool bridge** — optional native integration that surfaces the AFT code-toolkit (`aft_read`/`aft_write`/etc.) to the agent when the `aft` binary is available, with natural fallback to operant-native tools.
- **Harness kernel surfaces** — `operant status --json` exposes `harness.metrics` (mount ok/pending/failed counters, best-effort boot) and `architecture dump --live` attaches the same `HarnessMetrics` block, so operators can see mount outcomes and alert on PENDING churn.
- **Configurable prompt-cache TTL** — `client.prompt_cache_ttl = "5m" | "1h"` (OpenRouter path honors it; Anthropic clients mark breakpoints with the ttl field).
- **WhatsApp markdown** — outbound WhatsApp messages optionally convert `*bold*`/`_italic_`/`~strike~`/fences when `parse_markdown` is set (shared `asterisk_dialect` with Slack mrkdwn).
- **Pinned agentmemory spawns** — every runtime `npx @agentmemory/*` spawn (auto-spawn + MCP entries) uses the pinned `DEFAULT_AGENTMEMORY_VERSION` (0.9.29), overridable via `[memory] agentmemory_version` / `AGENTMEMORY_VERSION`. A cold boot no longer fetches whatever npm serves at that moment.
- **Mount-cap enforcement** — `max_active_providers` is enforced per mount (config default 64) with a cheap `provider_count()` check, preventing OOM from runaway mounts at large pool counts.

### Fixed

- **web_search per-provider timeout** — every candidate engine is now bounded by `search_timeout_secs` (`run_provider_chain`), so a hung provider (e.g. a stalled igs subprocess whose own timeout can reach 60s) fails over to the next engine instead of killing the whole search at the agent-loop tool timeout. Timeouts/errors/empty results all fall through and the timeout is surfaced in the error.
- **agentmemory default alignment** — schema `MemoryConfig::default()` backend was `sqlite` while the core AppConfig default, docs, and `operant.example.toml` said `agentmemory`; the schema default is now `agentmemory` so the daemon/gateway path matches the CLI path (the injected MCP server stays deferred, and `ensure_backend` degrades gracefully with a warning when Node.js/npx is unavailable).
- **memory tool guidance** — `memory_store`/`memory_search`/`memory_recall` descriptions now state they target the builtin MEMORY.md store and point to `memory_save`/`memory_smart_search` for the agentmemory backend, so agents route to the correct memory surface.
- **`OPERANT_CONFIG_DIR` honored by every load path** — `default_config_paths()` (operant-core) now resolves `<OPERANT_CONFIG_DIR>/operant.toml` before the XDG/`~/.operant` fallbacks, matching the schema layer. Previously `operant run` (and the `config_manage` tool) silently ignored the env var and loaded the real `~/.operant/operant.toml`, so supposedly-isolated runs could mutate the user's real config.
- **Agentic self-management guidance** — `SKILLS_GUIDANCE` gains a Self-Management Protocol: consult the `operant` self-skill and `operant <cmd> --help` for CLI syntax, never read the Rust source to discover flags, trust command output (no re-runs), prefer the validating CLI over hand-edited TOML, and restore the baseline after management tasks.
- **Deployment-audit closeout (R39 series)** — doctor probes the configured endpoint rather than every provider's public default (custom gateways no longer report false `✗ (invalid API key)`); the clippy gate script survives deny violations and prints the offending site; background review no longer poisons the write-origin for the process lifetime (task-local scope); memory stats read the real session count from `database.db`; the doctor key scan can no longer send a base URL as a Bearer token.
- **Secret files created 0600 at open** — the memory master-key file and channel session stores are written with `OpenOptions::mode(0o600)` in one `open(2)` instead of a post-write `set_permissions` that left a umask-default window.
- **Dead Slack `_signing_secret` removed** — R14-4 withdrawn as a misread: Slack signature verification lives in the webhook adapter (verified against `webhooks_secret`), and the Socket-Mode adapter has no signed requests to verify.
- **Tagged-release pipeline repaired** — release workflow reads `docs/CHANGELOG.md` (was a nonexistent root path) and the dead `tdg-rust` clone step is gone from build; a `v*` tag push now produces notes and artifacts instead of exiting 1.

### Changed

- **Version 0.2.0** — workspace bumped from 0.1.4 so `operant --version` distinguishes freshly built binaries from stale installs (stale builds reject configs containing newer fields).

### Verified

- Live agentic-loop E2E across the full LCM tool surface (6/6 PASS) and cross-process durability (4/4 PASS): facts saved to the global assertion scope and DAG marker in one process are recalled verbatim by a fresh process.
- Fresh-install cold-start: bundled skills seeded (29), 84 tools registered, igs 1.0.3 + obscura 0.2.0 (stealth) provisioned, AFT v0.50.0 auto-downloaded; agentmemory path (memory_save → server, smart_search recall, session/start + observe hooks) and builtin path (store/search/recall) both PASS.

## [0.1.4] - 2026-07-19

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
- `/tasks` now actually works as the documented alias for `/agents` in the TUI instead of printing a "not yet wired" error
- Stabilized a pre-existing parallel-test flake in `osc8`'s URL-detection tests (was racing on an unsynchronized process-global env var)
- Stabilized a matching flake in `discord_tool`'s no-token test, which was missing the `#[serial_test::serial]` attribute its sibling tests already use
- `CredentialPool::refresh_async`/`refresh_oauth_async` no longer hold a lock across the network call that refreshes each OAuth credential, which previously blocked any writer for the whole refresh loop's duration
- `operant logs --follow` (and `operant logs`) could spin at 100% CPU forever if the log file hit a read error, instead of stopping — `Lines::filter_map(Result::ok)` skips errors and can loop on a repeating error per `std::io` docs; switched to `map_while(Result::ok)`, which stops at the first error
- The context-window warning subsystem was entirely dead: `check_token_warnings()` had full 80%/95%/100% threshold logic and its doc comment said to call it after updating `token_count`, but nothing ever did — users got no warning before hitting a full context window. Wired the call into the `AgentEvent::Usage` handler; also fixed a related latent bug the wiring would otherwise have shipped live — the threshold tracker only ever escalated, so after `/clear` or `/compact` shrank usage back down, warnings would never fire again for the rest of the session. Now resets once usage drops back below the last-shown threshold
- Pasted images (`Ctrl+V` clipboard paste) showed a thumbnail row implying they'd be sent, but silently vanished on submit — the core client's request path has no multi-part/image content support yet, and nothing drained `pending_images` or told the user. `App::drop_pending_images_with_notice()` now clears them on send and pushes a warning notification instead of a silent no-op; the underlying `PastedImage`/`clear_images` plumbing is left in place for a future dedicated session to wire up real image attachment

### Removed

- 96 `cargo clippy --fix`-applied style/idiom warnings in `operant-core` (124→28) across 39 files, unblocked by fixing the one invalid clippy suggestion (`PathBuf == &str`, not a valid comparison) that had been causing the whole-crate fix to silently roll back every prior session — behavior-preserving only
- 19 more manually-judged `operant-core` warnings (28→9): 3 dead struct fields, a duplicated attribute, a doc-indent nit, and a redundant always-`Caution` branch in `skills_guard.rs`'s verdict logic (see Fixed)
- 9 manually-judged `operant-cli` warnings (136→127): an orphaned doc comment for a deleted function, a merged if/else arm, a `loop`→`while let` simplification, a manual counter→`.enumerate()`, a manual `strip_prefix`, and 2 enum variants renamed to CamelCase
- ~110 `dead_code` warnings in `operant-cli` (127→55): unused functions, methods, fields, constants, enums, and structs removed across 24 files (adapter_types.rs, app.rs, dialogs.rs, diff_viewer.rs, and 20 others), each individually verified to have zero references anywhere in the workspace, including test-only usage that a binary-only clippy scan can't see. Deliberately-parked or actually-live features (device auth flow, event-bus variants pre-published for a future registry, the model picker's real entry points) were verified and kept, not deleted
- Duplicated wraparound list-selection arithmetic across 12 TUI overlay files (`agents_view`, `effort_picker`, `hooks_config_menu`, `mcp_view`, `memory_file_selector`, `model_picker`, `plugins_hub`, `session_branching`, `session_browser`, `skills_view`, `tasks_overlay`, `theme_screen`) — each `select_prev`/`select_next` reimplemented the same "decrement, wrap to `count - 1` at zero" / "increment, wrap to `0` at `count`" logic; replaced with shared `cycle_prev`/`cycle_next` helpers in `overlays.rs` (B2), net -116 lines. Files with genuinely different selection semantics (`ask_user_dialog`'s custom-input-row wrap, `bypass_permissions_dialog`/`settings_screen`'s non-wrapping toggle/clamp) were left untouched

- Dead TUI code with no reachable callers: the legacy `ToolPermissionDialog` cluster, the `render_message` renderer family, the `RenderContext.highlight` field, and five unused `App` fields (~1,400 lines total)
- Legacy `config.yaml`/`config.local.yaml` loading from `CliConfig::load()` — `operant.toml` is now the sole file-based config source; `.env` loading and `HERMES_*` env overrides are unaffected. Also removed the now-dead `deep_merge`/`expand_env_vars_in_value` YAML-merge helpers and the redundant `"gpt-4"` model precedence heuristic in `main.rs` (env-based `HERMES_MODEL` override already covers it)
- ~125 style/idiom clippy warnings in `operant-cli`'s bin target (derived-impl opportunities, redundant closures, unnecessary borrows, `to_string` in `format!` args, simplifiable `map_or`, manual `div_ceil`, and similar), via `cargo clippy --fix` — behavior-preserving only

Full Changelog: [v0.1.3...v0.1.4](https://github.com/ishan-parihar/operant/compare/v0.1.3...v0.1.4)

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
