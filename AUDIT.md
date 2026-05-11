# Hermes-RS Port Audit

**Date**: 2026-05-11
**Status**: Phase 1 complete, ~80% core tools ported

---

## 1. Executive Summary

The Rust port has successfully replicated the core tooling from the Python `hermes-agent` codebase.
- **32,413 LOC** of Rust across 2 workspace crates (`hermes-core`, `hermes-cli`)
- **58 HermesTool implementations** (53 core, 2 examples, 2 CLI, 1 test)
- **291/291 tests passing**
- **12 modified + 5 untracked files** ready for commit

The port is in Phase 1 of a multi-phase plan. The core architecture (ToolRegistry, providers, config, CLI) is solid.
Approximately **12,500+ lines** of Python tool code (~25 tool modules) remain unported.

---

## 2. Test Results

```
test result: ok. 291 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

- All 291 tests pass across the workspace
- 0 warnings (beyond 1 minor trailing-semicolon lint)
- Previous REPORT.md claimed 285 — 6 new tests added with uncommitted tools

---

## 3. Workspace Structure

```
hermes-rs/
├── Cargo.toml                  # Workspace root
├── crates/
│   ├── hermes-core/            # Library: tools, config, providers, MCP, cron
│   │   └── src/
│   │       ├── tools/          # 30+ tool modules
│   │       ├── providers/      # LLM providers (OpenAI, Anthropic, etc.)
│   │       ├── mcp.rs          # MCP client + McpTool
│   │       ├── tool_registry/  # ToolRegistry + ToolSet
│   │       ├── config.rs       # Configuration
│   │       └── cronjobs/       # Scheduler
│   └── hermes-cli/             # Binary: main.rs, TUI, autonomous mode
```

---

## 4. Port Status: Python → Rust Mapping

### 4.1 COMMITTED — Ported and in git history

| Python File | LOC | Rust File | LOC | Notes |
|---|---|---|---|---|
| `delegate_tool.py` | 2,767 | `sub_agent_tool.rs` | 604 | SubAgentTool — smaller in Rust |
| `tool_result_storage.py` | 232 | (inlined in sub_agent_tool) | — | |
| `terminal_tool.py` | 2,344 | `terminal_tool.rs` | 832 | Core terminal execution |
| `file_operations.py` | 1,571 | `file_tools.rs` | 829 | FileRead/Write/Search/List |
| `code_execution_tool.py` | 1,781 | `code_execution.rs` | 343 | Sandboxed execution |
| `send_message_tool.py` | 1,883 | `send_message_tool.rs` | 188 | Multi-platform messaging |
| `checkpoint_manager.py` | 1,640 | `checkpoint_tool.rs` | 413 | Git checkpoint tool |
| `vision_tools.py` | 1,420 | `vision_tool.rs` | 530 | Vision analysis |
| `rl_training_tool.py` | 1,396 | `rl_training_tool.rs` | 467 | RL training |
| `browser_tool.py` | 3,600 | `browser_tool.rs` | 635 | Different arch (Playwright MCP vs CDP) |
| `browser_cdp_tool.py` | 563 | `browser_cdp_tool.rs` | 120 | CDP bridge |
| `computer_use_tool.py` | 1,428 | `computer_use_tool.rs` | 490 | 13-action CUA |
| `tts_tool.py` | 2,198 | `tts_tool.rs` | 824 | Text-to-speech |
| `image_generation_tool.py` | 1,058 | `image_generation_tool.rs` | 290 | Image gen |
| `discord_tool.py` | 959 | `discord_tool.rs` | 1,071 | Discord + Admin |
| `cronjob_tools.py` | 690 | `cron_tool.rs` | 1,307 | Cron + scheduler |
| `memory_tool.py` | 586 | `memory_tools.rs` | 300 | Memory store/search/recall |
| `session_search_tool.py` | 612 | `session_search_tool.rs` | 196 | Session search |
| `skills_tool.py` | 1,533 | `skills_tool.rs` | 295 | Skills/SkillView |
| `web_tools.py` | 2,279 | `web_tools.rs` | 395 | WebSearch/Fetch |
| `mcp_tool.py` (client) | 3,408 | `mcp.rs:754` (McpTool) | — | MCP dynamic tool |
| `todo_tool.py` | 277 | `todo_tool.rs` | 252 | Todo management |
| `kanban_tools.py` | 873 | `kanban_tool.rs` | 304 | Kanban board |
| `homeassistant_tool.py` | 513 | `home_assistant_tool.rs` | 924 | Home Assistant |
| `feishu_doc_tool.py` + `feishu_drive_tool.py` | 569 | `feishu_tool.rs` | 378 | Feishu/Lark |
| `mixture_of_agents_tool.py` | 541 | `mixture_of_agents_tool.rs` | 310 | MoA |
| `browser_dialog_tool.py` | 148 | `browser_dialog_tool.rs` | 111 | Dialog tool |
| `patch_parser.py` | 592 | `patch_tool.rs` | 245 | Patch tool |
| `registry.py` | 563 | `tools.rs` | 320 | ToolRegistry |
| `spotify_tool` (various) | — | `spotify_tool.rs` | 560 | 7 Spotify tools |

### 4.2 UNCOMMITTED — Implemented but not committed

| Python File | LOC | Rust File | LOC | Notes |
|---|---|---|---|---|
| `process_registry.py` | 1,476 | `process_registry.rs` | 300 | Process lifecycle |
| `process_tool.py` (subset) | — | `process_tool.rs` | 132 | Process management tool |
| `mcp_tool.py` (management) | — | `mcp_tool.rs` | 210 | McpManagementTool (add/remove/list MCP servers) |
| `transcription_tools.py` | 911 | `transcription_tool.rs` | 261 | Groq/OpenAI Whisper |
| `web_providers/` (various) | — | `web_providers/` | — | Tavily, Exa, SearXNG, Brave, DDG backends |

### 4.3 NOT PORTED — Python tools without Rust version

| Python File | LOC | Notes |
|---|---|---|
| `skills_hub.py` | 3,261 | Skills community/hub — LARGEST unported module |
| `mcp_oauth.py` | 632 | MCP OAuth flow |
| `mcp_oauth_manager.py` | 607 | MCP OAuth management |
| `voice_mode.py` | 1,017 | Voice mode CLI |
| `skills_guard.py` | 932 | Skills security guard policies |
| `yuanbao_tools.py` | 736 | Tencent Yuanbao integration |
| `fuzzy_match.py` | 704 | Fuzzy matching utilities |
| `tirith_security.py` | 691 | Security scanning tool |
| `skill_usage.py` | 609 | Skill usage tracking/analytics |
| `microsoft_graph_auth.py` | 245 | Microsoft Graph auth |
| `microsoft_graph_client.py` | 408 | Microsoft Graph API client |
| `credential_files.py` | 436 | Credential file management |
| `skills_sync.py` | 431 | Skills sync |
| `url_safety.py` | 327 | URL safety checks |
| `schema_sanitizer.py` | 370 | Schema sanitization (likely unneeded in Rust) |
| `website_policy.py` | 282 | Website policy enforcement |
| `tool_result_storage.py` | 232 | Tool result persistence |
| `browser_camofox.py` | 603 | Camofox browser provider |
| `managed_tool_gateway.py` | 167 | Managed tool gateway |
| `env_passthrough.py` | 145 | Environment passthrough |
| `tool_backend_helpers.py` | 144 | Tool backend utilities |
| `slash_confirm.py` | 162 | Slash confirm dialog |
| `osv_check.py` | 155 | OSV vulnerability check |
| `interrupt.py` | 98 | Interrupt handling |
| `tool_output_limits.py` | 92 | Output limit enforcement |
| `budget_config.py` | 52 | Budget config |
| `ansistrip.py`, `binary_extensions.py`, `debug_helpers.py` | ~190 | Utilities (replaced by Rust idioms) |
| `openrouter_client.py`, `xai_http.py` | 45 | Thin API clients |
| **Total not ported** | **~12,500+** | Across ~30 modules |

### 4.4 Environments (Python, not ported)

| Python File | LOC | Notes |
|---|---|---|
| `environments/base.py` | 843 | Base environment |
| `environments/docker.py` | 645 | Docker execution |
| `environments/local.py` | 581 | Local execution |
| `environments/ssh.py` | 290 | SSH execution |
| `environments/modal.py` | 460 | Modal cloud |
| `environments/daytona.py` | 259 | Daytona |
| `environments/vercel_sandbox.py` | 638 | Vercel Sandbox |
| `environments/...` (others) | 1,400+ | file_sync, managed_modal, singularity, modal_utils |
| **Total** | **~5,100+** | Environment sandboxing layer |

---

## 5. Architecture Assessment

### 5.1 Strengths
- **ToolRegistry pattern**: Well-designed with `ToolSet` composition, `ToolRouter`, and provider-based tool execution
- **HermesTool trait**: Clean abstraction with schema generation, async execution, and error handling
- **Provider model**: ProviderRegistry supports dynamic model routing, fallback, and retry
- **MCP integration**: Dual-mode — McpTool (dynamic from servers) + McpManagementTool (server lifecycle)
- **Config system**: `hermes.example.toml` is well-structured with environment variable interpolation
- **Tests**: 291 passing, good coverage on core tools

### 5.2 Gaps
- **McpManager→ToolRegistry wiring**: McpManager manages server lifecycle but auto-registration of MCP tools into ToolRegistry is not connected (mcp.rs:754 has McpTool implementing HermesTool, but the bridge isn't wired)
- **Skills ecosystem**: `skills_hub.py` (3,261 LOC), `skills_guard.py`, `skill_usage.py`, `skills_sync.py` are all absent
- **OAuth for MCP**: `mcp_oauth.py` + `mcp_oauth_manager.py` (1,239 LOC) — servers needing OAuth won't work
- **Security scanning**: `tirith_security.py`, `osv_check.py`, `url_safety.py` missing
- **Environments layer**: ~5,100 LOC of sandboxing code not ported (code_execution.rs covers basic execution only)
- **Voice mode**: Not ported (1,017 LOC, complex CLI interaction)

### 5.3 Already Replaced by Rust Idioms
These Python utilities are not needed in Rust:
- `schema_sanitizer.py` — Rust's type system ensures schema correctness
- `debug_helpers.py` — Replaced by `tracing` crate
- `ansi_strip.py` — Handled by terminal libraries
- `binary_extensions.py` — Rust's Path handles extensions natively
- `tool_backend_helpers.py` — Trait pattern replaces function composition

---

## 6. Uncommitted Changes (17 files)

```
 M Cargo.lock
 M Cargo.toml
 M crates/hermes-cli/src/autonomous.rs
 M crates/hermes-cli/src/main.rs
 M crates/hermes-cli/src/tui/app.rs
 M crates/hermes-core/src/config.rs
 M crates/hermes-core/src/lib.rs
 M crates/hermes-core/src/mcp.rs
 M crates/hermes-core/src/tools.rs
 M crates/hermes-core/src/tools/builtin.rs
 M crates/hermes-core/src/tools/web_tools.rs
 M hermes.example.toml
?? crates/hermes-core/src/process_registry.rs
?? crates/hermes-core/src/tools/mcp_tool.rs
?? crates/hermes-core/src/tools/process_tool.rs
?? crates/hermes-core/src/tools/transcription_tool.rs
?? crates/hermes-core/src/tools/web_providers/
```

- **12 modified files**: Config updates, tool registration wiring, MCP changes, CLI changes
- **5 new files**: `process_registry.rs`, `mcp_tool.rs`, `process_tool.rs`, `transcription_tool.rs`, `web_providers/`

---

## 7. Key Architecture Items

| Item | Status | Notes |
|---|---|---|
| ToolRegistry + ToolSet | ✅ Committed | Fully functional |
| HermesTool trait | ✅ Committed | 58 impls |
| LLM providers | ✅ Committed | OpenAI, Anthropic, Gemini, Groq, OpenRouter, Ollama, etc. |
| MCP client (McpTool) | ✅ Committed | Dynamic tool from MCP servers |
| MCP management | ⚠️ Uncommitted | mcp_tool.rs (add/remove/list servers) |
| MCP OAuth | ❌ Not ported | mcp_oauth.py (1,239 LOC) |
| Web providers | ⚠️ Uncommitted | web_providers/ (Tavily, Exa, SearXNG, Brave, DDG) |
| Process management | ⚠️ Uncommitted | process_registry.rs + process_tool.rs |
| Transcription | ⚠️ Uncommitted | transcription_tool.rs |
| Cron scheduling | ✅ Committed | scheduler.rs + cron_tool.rs |
| Skills | ❌ Not ported | skills_hub, skills_guard, skill_usage, skills_sync |
| Security | ❌ Not ported | tirith, osv_check, url_safety, website_policy |
| Environments | ❌ Not ported | Docker, SSH, Modal, etc. (5,100 LOC) |
| Voice mode | ❌ Not ported | 1,017 LOC |

---

## 8. Documentation Status

| Document | Status | Issue |
|---|---|---|
| `REPORT.md` | ❌ Outdated | Claims 285 tests (actual: 291), claims 6 P1 tools unported (all exist uncommitted) |
| `TODO.md` | ❌ Outdated | Pending section lists mcp_tool, process, transcription, web_providers as pending (all implemented) |
| `CHANGELOG.md` | ✅ Up to date | 4 releases documented |
| `AUDIT.md` | ✅ This file | Current |

---

## 9. Recommended Next Steps

### Immediate (before next phase)
1. **Commit all uncommitted work** — 17 files, 291 tests passing
2. **Update REPORT.md** — Fix test count (291), tool count (58 HermesTool impls), P1 status
3. **Update TODO.md** — Move implemented items to completed, add unported items with LOC estimates

### Phase 2 Priority (high-value unported modules)
1. **Skills ecosystem**: `skills_hub.py` (3,261 LOC) — largest unported module
2. **MCP OAuth**: `mcp_oauth.py` + `mcp_oauth_manager.py` (1,239 LOC) — needed for MCP servers with auth
3. **Security**: `tirith_security.py` (691 LOC) + `url_safety.py` (327 LOC) + `osv_check.py` (155 LOC)
4. **Wire McpManager→ToolRegistry** — Bridge MCP server tools into ToolRegistry

### Phase 3
1. **Environments** — Port Docker/SSH/Modal sandboxing (~5,100 LOC)
2. **Voice mode** — Port voice_cli integration (1,017 LOC)
3. **Credential files** — Port credential management (436 LOC)
4. **Skill tracking** — `skill_usage.py`, `skills_sync.py`, `skills_guard.py`
