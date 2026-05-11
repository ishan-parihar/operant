# Hermes-RS Audit Report

**Date:** 2026-05-11 | **Branch:** main | **Version:** v0.1.3 (+uncommitted)

## 1. Project Structure

| Metric | Value |
|--------|-------|
| Workspace crates | 2 (`hermes-core`, `hermes-cli`) |
| Total Rust LOC | ~14,764 (core: ~11,964, cli: ~2,800) |
| Source files | ~68 (core: 61, cli: 7) |
| Registered tools | 41 (HEAD) / 49 (with uncommitted additions) |
| Dependencies | 90+ (core: serde, tokio, reqwest, tracing, sqlx, ratatui, etc.) |
| Git commits | 30 commits, HEAD pinned to v0.1.3 tag |

## 2. Test Results: 285 PASSED, 0 FAILED ✅

All three previously failing tests were diagnosed and fixed:

| Test | Issue | Fix Applied |
|------|-------|-------------|
| `agent::tests::test_agent_builder` | `build()` requires database but none was provided | Added `.database(db)` call with in-memory test DB |
| `tools::sub_agent_tool::tests::parse_args_rejects_empty_goal` | `parse_args()` didn't validate whitespace-only goals | Added `trim().is_empty()` validation for `goal` and task goals |
| `platform::tests::test_hermes_subdirs_are_children_of_home` | Race condition with `HERMES_HOME` env var mutation in parallel tests | Added `#[serial_test::serial]` to prevent parallel access |

## 3. Build Verification

| Check | Result |
|-------|--------|
| `cargo check --workspace` | Passes (58 warnings: unused imports, dead code, private types in public API) |
| `cargo test --workspace` | **285 passed, 0 failed** |
| `cargo clippy --all-targets` | Fails in test compilation (105 errors: espeak-rs-sys C stubs in test mode) |
| `cargo fmt` | Not verified |

## 4. Cargo Warnings Summary (58 total)

**Warnings by category:**
- Unused imports (~15): `Serialize`, `tracing::*`, `std::path::*`, `tokio::sync::*`, `std::collections::*`, etc.
- Dead code fields (~20+): struct fields never read (arises from serde-deserialized structs)
- Private type in public API (~2): `SubAgentRole` enum in `SubAgentTool::call` / `::call_batch` signatures
- Unused variables (~6): `output`, `summary`, `reason`, `body`, `args`, `tool`, `role_filter`, etc.
- Redundant semicolon: `scheduler.rs:57`
- Unused doc comment: `scanner.rs:41`
- Unused constants: `SECRET_VAR_RE`, `DELEGATE_BLOCKED_TOOLS`
- Static/function never used: `SESSION_SEARCH`, `SessionSearchState`, `get_session_search_state`

## 5. Tool Inventory (41 registered in HEAD, 49 with uncommitted)

| Category | Tools | Count |
|----------|-------|-------|
| **Core FS/Terminal** | `read`, `write`, `glob`, `terminal`, `patch` | 5 |
| **Web/HTTP** | `web_fetch`, `web_search`, `http` | 3 |
| **Code/Dev** | `execute_code`, `checkpoint`, `sub_agent` | 3 |
| **Memory/Knowledge** | `memory_search`, `memory_store`, `session_search`, `skill_view` | 4 |
| **DateTime** | `datetime`, `datetime_range` | 2 |
| **Productivity** | `todo`, `cron`, `kanban`, `clarify` | 4 |
| **Browser** | `browser`, `browser_cdp`, `browser_dialog`, `browser_download`, `vision` | 5 |
| **Multimedia/AI** | `image_generation`, `text_to_speech`, `video_analysis`, `moa` | 4 |
| **Communication** | `send_message`, `notify`, `approval_request`, `discord`, `discord_admin` | 5 |
| **Integration** | `delegate_task`, `skills`, `rl_training`, `spotify_*` (7) | 10 |
| **Enterprise** | `feishu_doc`, `feishu_drive`, `homeassistant`, `echo`, `calculate` | 5 |
| **Total** | | **49** |

## 6. Python Original vs Rust Port Coverage

### Fully Ported Tools
- `browser_tool`, `browser_cdp`, `browser_dialog`, `browser_downloader`
- `checkpoint_tool`, `clarify_tool`, `code_execution`, `computer_use` (CUA provider pattern)
- `cron_tool`, `datetime_tool`, `discord_tool`, `feishu_tool`
- `file_tools` (read/write/glob/list/search), `home_assistant_tool`
- `http_tool`, `image_generation_tool`, `kanban_tool`
- `mixture_of_agents_tool`, `notification_tool`, `patch_tool`
- `send_message_tool`, `skills_tool`, `sub_agent_tool`
- `terminal_tool`, `todo_tool`, `tts_tool`
- `video_analysis_tool`, `vision_tool`, `web_tools` (fetch + search)
- `memory_tools`, `session_search_tool`
- **New:** `rl_training_tool` (RL training tool), `spotify_tool` (7 Spotify actions)

### Missing/Partial Ports (6 P1 Tools)
| Python File | Lines | Rust Status | Notes |
|------------|-------|-------------|-------|
| `mcp_tool.py` | 3408 | **Not ported** | Expose MCP tools as agent-invocable tools |
| `skills_hub.py` | 3261 | **Not ported** | Skills hub management |
| `delegate_tool.py` | 2767 | **Partial** | Rust has basic sub-agent delegation, lacks orchestration features |
| `web_providers/` | ~500 | **Not ported** | Tavily, Exa, Searxng backends; Rust hardcodes DuckDuckGo |
| `transcription_tools.py` | 911 | **Not ported** | Whisper/audio transcription |
| `process_registry.py` | 1476 | **Not ported** | Long-running subprocess management |

### Missing Infrastructure (8 items)
| Feature | Status |
|---------|--------|
| MCP tool invocation via ToolRegistry | Pending |
| Web scrape/crawl (full extraction) | Pending |
| Audio/transcription tools | Pending |
| Computer use/UI interaction | Pending |
| File upload/download with limits | Pending |
| Gateway notification integration | Pending |
| Tool output size limits/truncation | Pending |
| Per-tool configurable timeouts | Pending |

## 7. Key Architecture Components

| Module | Lines | Description |
|--------|-------|-------------|
| `agent.rs` | 1317 | ReAct orchestration loop with self-healing |
| `config.rs` | 732 | TOML-first runtime configuration |
| `database.rs` | 454 | SQLite persistence (sessions, FTS5) |
| `memory.rs` | 1138 | Long-term memory store/search/distillation |
| `mcp.rs` | 876 | MCP client (HTTP + stdio transports) |
| `gateway.rs` | 751 | Multi-platform gateway (Telegram/Discord/Slack) |
| `parser.rs` | 590 | XML/tool call stream parser |
| `platform.rs` | 540 | Platform detection, paths, permissions |
| `trajectory.rs` | 395 | Session trajectory export |
| `skills.rs` | 613 | Skills system management |
| `tools.rs` | 501 | Tool registry with timeout |
| `context.rs` | 298 | Context window management |
| `context_files.rs` | 276 | Workspace context file auto-loading |
| `cronjobs/` | ~400 | Cron job DB, scheduler, injection scanner |
| `kanban/` | ~300 | Kanban board DB |
| CLI TUI | 1719 | Ratatui multi-panel terminal UI |
| Autonomous mode | 1719 | Self-improving coding loop |

## 8. Recommendations

1. **Address clippy warnings incrementally** — 58 dead-code/unused-import warnings add up. Focus on removing unused imports first.
2. **Fix `SubAgentRole` visibility** — making it `pub` would resolve 2 private-type-in-public-API warnings.
3. **Parallel test serialization** — ensure all `HERMES_HOME`-dependent tests use `#[serial_test::serial]`.
4. **Remove deprecated files** — `PORTING_PLAN.md` was deleted from filesystem but may still be tracked by git.
5. **CI for clippy** — currently blocked by espeak-rs-sys C stubs in test compilation; consider a build-script workaround or feature gate.
