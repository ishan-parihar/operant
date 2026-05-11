# Hermes-RS Audit Report

**Date:** 2026-05-11 | **Branch:** main | **Version:** v0.1.3 (+uncommitted)

## 1. Project Structure

| Metric | Value |
|--------|-------|
| Workspace crates | 2 (`hermes-core`, `hermes-cli`) |
| Total Rust LOC | ~32,413 |
| Source files | ~70+ (core: 60+, cli: 7) |
| Registered tools | 58 `HermesTool` implementations (53 core, 2 examples, 2 CLI, 1 test) |
| Dependencies | 90+ (core: serde, tokio, reqwest, tracing, sqlx, ratatui, etc.) |
| Git commits | 30+ commits, HEAD at ed763e5 |

## 2. Test Results: 291 PASSED, 0 FAILED ✅

Previous test fixes (285 baseline):
- `agent::tests::test_agent_builder` — Added `.database(db)` call with in-memory test DB
- `tools::sub_agent_tool::tests::parse_args_rejects_empty_goal` — Added whitespace validation
- `platform::tests::test_hermes_subdirs_are_children_of_home` — Added `#[serial_test::serial]`

6 additional tests added with uncommitted tool implementations (MCP management, process, transcription, web providers).

## 3. Build Verification

| Check | Result |
|-------|--------|
| `cargo check --workspace` | Passes |
| `cargo test --workspace` | **291 passed, 0 failed** |
| `cargo clippy --all-targets` | Fails in test compilation (espeak-rs-sys C stubs in test mode) |
| `cargo fmt` | Passes |

## 4. Uncommitted Changes (17 files)

```
 M Cargo.lock                M crates/hermes-core/src/config.rs
 M Cargo.toml                M crates/hermes-core/src/lib.rs
 M crates/hermes-cli/src/autonomous.rs  M crates/hermes-core/src/mcp.rs
 M crates/hermes-cli/src/main.rs        M crates/hermes-core/src/tools.rs
 M crates/hermes-cli/src/tui/app.rs     M crates/hermes-core/src/tools/builtin.rs
 M hermes.example.toml       M crates/hermes-core/src/tools/web_tools.rs
?? crates/hermes-core/src/process_registry.rs
?? crates/hermes-core/src/tools/mcp_tool.rs
?? crates/hermes-core/src/tools/process_tool.rs
?? crates/hermes-core/src/tools/transcription_tool.rs
?? crates/hermes-core/src/tools/web_providers/
```

6 previously-claimed P1 tools are already implemented but uncommitted:
- **McpManagementTool** (`mcp_tool.rs`): Add/remove/list MCP servers
- **ProcessTool + ProcessRegistry** (`process_tool.rs`, `process_registry.rs`): Long-running subprocess management
- **TranscriptionTool** (`transcription_tool.rs`): Groq/OpenAI Whisper transcription
- **Web providers** (`web_providers/`): Tavily, Exa, SearXNG, Brave, DuckDuckGo backends
- **Computer use** (`computer_use_tool.rs`): Already committed, 13-action CUA provider pattern

## 5. Tool Inventory (58 HermesTool implementations)

| Category | Tools | Count |
|----------|-------|-------|
| **Core FS/Terminal** | `read`, `write`, `glob`, `terminal`, `patch` | 5 |
| **Web/HTTP** | `web_fetch`, `web_search`, `http` | 3 |
| **Code/Dev** | `execute_code`, `checkpoint`, `sub_agent`, `process` | 4 |
| **Memory/Knowledge** | `memory_search`, `memory_store`, `session_search`, `skill_view` | 4 |
| **DateTime** | `datetime`, `datetime_range` | 2 |
| **Productivity** | `todo`, `cron`, `kanban`, `clarify` | 4 |
| **Browser** | `browser`, `browser_cdp`, `browser_dialog`, `browser_download`, `vision`, `computer_use` | 6 |
| **Multimedia/AI** | `image_generation`, `text_to_speech`, `video_analysis`, `moa`, `transcription` | 5 |
| **Communication** | `send_message`, `notify`, `approval_request`, `discord`, `discord_admin` | 5 |
| **Integration** | `delegate_task`, `skills`, `rl_training`, `mcp_management`, `spotify_*` (7) | 11 |
| **Enterprise** | `feishu_doc`, `feishu_drive`, `homeassistant`, `echo`, `calculate` | 5 |
| **Total** | | **58** |

## 6. Python Original vs Rust Port Coverage

### Fully Ported Tools (committed + uncommitted)
- `browser_tool`, `browser_cdp`, `browser_dialog`
- `checkpoint_tool`, `clarify_tool`, `code_execution`, `computer_use` (CUA provider pattern)
- `cron_tool`, `datetime_tool`, `discord_tool`, `feishu_tool`
- `file_tools` (read/write/glob/list/search), `home_assistant_tool`
- `http_tool`, `image_generation_tool`, `kanban_tool`
- `mcp_tool` (management) via `mcp_tool.rs`
- `mixture_of_agents_tool`, `notification_tool`, `patch_tool`
- `process_registry` + `process_tool` via `process_registry.rs`, `process_tool.rs`
- `send_message_tool`, `skills_tool`, `sub_agent_tool`
- `terminal_tool`, `todo_tool`, `transcription_tools` via `transcription_tool.rs`
- `tts_tool`, `video_analysis_tool`, `vision_tool`
- `web_tools` + `web_providers` (Tavily, Exa, SearXNG, Brave, DDG)
- `memory_tools`, `session_search_tool`
- **New:** `rl_training_tool`, `spotify_tool` (7 actions)

### Not Ported (remaining Python tools)
| Python File | LOC | Priority | Notes |
|------------|-------|----------|-------|
| `skills_hub.py` | 3,261 | High | Skills community/hub management |
| `mcp_oauth.py` + `mcp_oauth_manager.py` | 1,239 | High | MCP OAuth flow (blocker for auth'd MCP servers) |
| `voice_mode.py` | 1,017 | Medium | Voice CLI mode |
| `skills_guard.py` | 932 | Medium | Skills security policy |
| `yuanbao_tools.py` | 736 | Low | Tencent Yuanbao (China-market) |
| `fuzzy_match.py` | 704 | Low | Fuzzy matching util |
| `tirith_security.py` | 691 | Medium | Security scanning tool |
| `skill_usage.py` | 609 | Low | Skill usage analytics |
| `microsoft_graph_auth.py` + `client.py` | 653 | Low | Microsoft Graph (enterprise) |
| `credential_files.py` | 436 | Low | Credential file mgmt |
| `skills_sync.py` | 431 | Low | Skills sync |
| `schema_sanitizer.py` | 370 | **N/A** | Unneeded in Rust (compiler-enforced) |
| `url_safety.py` | 327 | Medium | URL safety checks |
| `website_policy.py` | 282 | Low | Website policy |
| `tool_result_storage.py` | 232 | Low | Result persistence |
| `browser_camofox.py` | 603 | Low | Alternative browser provider |
| `environments/*` | ~5,100 | Low | Sandboxing layer (Docker, SSH, Modal, etc.) |
| **Total unported** | **~17,600** | | |

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
