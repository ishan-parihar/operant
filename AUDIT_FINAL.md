# Final Port Audit: hermes-agent (Python) → hermes-rs (Rust)

**Date**: 2026-05-11
**Status**: Core tools & infrastructure ported. Full service layer pending.
**Tests**: 619 passed, 0 failed
**Rust LOC**: 52,419 across 106 .rs files
**HermesTool impls**: 62 (60 in core + 2 in examples)

---

## 1. What Has Been Ported ✓

### 1.1 Core Agent Loop
| Module | File | LOC | Status |
|--------|------|-----|--------|
| HermesAgent (ReAct loop) | `agent.rs` | 1,322 | Complete |
| OpenAIClient (streaming) | `client.rs` | 698 | Complete |
| ToolCallParser (XML) | `parser.rs` | 596 | Complete |
| Schema generation | `schema.rs` | 247 | Complete |
| Config (TOML + env) | `config.rs` | 762 | Complete |
| Error types | `error.rs` | 162 | Complete |

### 1.2 Tool System
| Module | File | LOC | Status |
|--------|------|-----|--------|
| HermesTool trait + ToolRegistry | `tools.rs` | 518 | Complete |
| Built-in tool registration | `tools/builtin.rs` | 233 | Complete |
| File tools (read/write/search/list) | `tools/file_tools.rs` | 405 | Complete |
| Terminal tool | `tools/terminal_tool.rs` | 212 | Complete |
| Code execution | `tools/code_execution.rs` | 409 | Complete |
| Browser tool | `tools/browser_tool.rs` | 216 | Complete |
| Browser CDP | `tools/browser_cdp_tool.rs` | 147 | Complete |
| Browser dialog | `tools/browser_dialog_tool.rs` | 118 | Complete |
| Browser downloader (Rust-only) | `tools/browser_downloader.rs` | 125 | Complete |
| CDP utils | `tools/cdp_utils.rs` | 72 | Complete |
| Computer use | `tools/computer_use_tool.rs` | 490 | Complete |
| Web search/fetch | `tools/web_tools.rs` | 390 | Complete |
| HTTP request (Rust-only) | `tools/http_tool.rs` | 159 | Complete |
| Memory tools (store/search/recall) | `tools/memory_tools.rs` | 230 | Complete |
| Datetime (Rust-only) | `tools/datetime_tool.rs` | 332 | Complete |
| Todo tool | `tools/todo_tool.rs` | 246 | Complete |
| Clarify tool | `tools/clarify_tool.rs` | 182 | Complete |
| Patch tool | `tools/patch_tool.rs` | 367 | Complete |
| MCP management | `tools/mcp_tool.rs` | 210 | Complete |
| Sub-agent delegation | `tools/sub_agent_tool.rs` | 605 | Complete |
| Skills list/view | `tools/skills_tool.rs` | 390 | Complete |
| Checkpoint tool | `tools/checkpoint_tool.rs` | 538 | Complete |
| Cron tool | `tools/cron_tool.rs` | 275 | Complete |
| Kanban tool | `tools/kanban_tool.rs` | 171 | Complete |
| Session search | `tools/session_search_tool.rs` | 297 | Complete |
| Process tool | `tools/process_tool.rs` | 132 | Complete |
| Notification + Approval | `tools/notification_tool.rs` | 107 | Complete |
| Vision analysis | `tools/vision_tool.rs` | 360 | Complete |
| Image generation | `tools/image_generation_tool.rs` | 214 | Complete |
| TTS tool | `tools/tts_tool.rs` | 678 | Complete |
| Transcription | `tools/transcription_tool.rs` | 261 | Complete |
| Video analysis (Rust-only) | `tools/video_analysis_tool.rs` | 104 | Complete |
| Send message | `tools/send_message_tool.rs` | 635 | Complete |
| Discord | `tools/discord_tool.rs` | 1,188 | Complete |
| Feishu/Lark | `tools/feishu_tool.rs` | 575 | Complete |
| Home Assistant | `tools/home_assistant_tool.rs` | 1,558 | Complete |
| Spotify (Rust-only, 7 tools) | `tools/spotify_tool.rs` | 514 | Complete |
| RL training | `tools/rl_training_tool.rs` | 571 | Complete |
| MoA tool | `tools/mixture_of_agents_tool.rs` | 215 | Complete |

### 1.3 Web Search Providers
| Provider | File | Status |
|----------|------|--------|
| DuckDuckGo | `web_providers/ddg.rs` | Complete |
| Exa (Rust-only) | `web_providers/exa.rs` | Complete |
| SearXNG | `web_providers/searxng.rs` | Complete |
| Tavily (Rust-only) | `web_providers/tavily.rs` | Complete |

### 1.4 Infrastructure Modules
| Module | File | LOC | Status |
|--------|------|-----|--------|
| Memory manager | `memory.rs` | 1,138 | Complete |
| Context manager | `context.rs` | 298 | Complete |
| Context files | `context_files.rs` | 276 | Complete |
| Database (SQLite) | `database.rs` | 454 | Complete |
| Trajectory | `trajectory.rs` | 395 | Complete |
| Distillation | `distillation.rs` | 188 | Complete |
| ANSI strip | `ansi_strip.rs` | 167 | Complete |
| Fuzzy match | `fuzzy_match.rs` | 605 | Complete |
| Budget config | `budget_config.rs` | 340 | Complete |
| Interrupt | `interrupt.rs` | 198 | Complete |
| Platform detection | `platform.rs` | 541 | Complete |
| Schema sanitizer | `schema_sanitizer.rs` | 466 | Complete |
| Security (path + tirith) | `security.rs` | 800 | Complete |
| Process registry | `process_registry.rs` | 339 | Complete |
| Browser camofox | `browser_camofox.rs` | 414 | Complete |
| Credential files | `credential_files.rs` | 238 | Complete |
| Credential pool | `credential_pool.rs` | 1,008 | Complete |
| Env passthrough | `env_passthrough.rs` | 220 | Complete |
| Tool result storage | `tool_result_storage.rs` | 335 | Complete |
| Website policy | `website_policy.rs` | 323 | Complete |
| MCP client + manager | `mcp.rs` | 970 | Complete |
| MCP OAuth | `mcp_oauth.rs` | 1,724 | Complete |
| Managed tool gateway | `managed_tool_gateway.rs` | 727 | Complete |
| Gateway types | `gateway.rs` | 751 | Stub (types only) |

### 1.5 Skills System
| Module | File | LOC | Status |
|--------|------|-----|--------|
| Skill manager | `skills.rs` | 613 | Complete |
| Skills hub | `skills_hub.rs` | 3,627 | Complete |
| Skills guard | `skills_guard.rs` | 2,046 | Complete |
| Skills sync | `skills_sync.rs` | 756 | Complete |
| Skill usage | `skill_usage.rs` | 488 | Complete |

### 1.6 Platform Integrations
| Module | File | LOC | Status |
|--------|------|-----|--------|
| Microsoft Graph | `ms_graph.rs` | 1,152 | Complete |
| Yuanbao/Tencent | `yuanbao.rs` | 683 | Complete |
| Voice/TTS | `voice.rs` | 2,549 | Complete |

### 1.7 Environment Backends
| Backend | File | LOC | Status |
|---------|------|-----|--------|
| Environment trait + pool | `environments/mod.rs` | 275 | Complete |
| Local subprocess | `environments/local.rs` | 82 | Complete |
| Docker | `environments/docker.rs` | 128 | Complete |
| SSH | `environments/ssh.rs` | 170 | Complete |
| Daytona | `environments/daytona.rs` | 66 | Complete |
| Modal | `environments/modal.rs` | 66 | Complete |
| Singularity | `environments/singularity.rs` | 149 | Complete |
| Vercel | `environments/vercel.rs` | 79 | Complete |

### 1.8 Cron Subsystem
| Module | File | LOC | Status |
|--------|------|-----|--------|
| Cron DB | `cronjobs/db.rs` | 444 | Complete |
| Cron scheduler | `cronjobs/scheduler.rs` | 139 | Complete |
| Prompt scanner | `cronjobs/scanner.rs` | 93 | Complete |

### 1.9 CLI/TUI
| Module | File | LOC | Status |
|--------|------|-----|--------|
| CLI entry (5 subcommands) | `hermes-cli/main.rs` | 689 | Complete |
| Autonomous mode | `hermes-cli/autonomous.rs` | 1,719 | Complete |
| TUI app | `hermes-cli/tui/app.rs` | 932 | Complete |
| TUI rendering | `hermes-cli/tui/render.rs` | 2,000 | Complete |
| TUI state | `hermes-cli/tui/state.rs` | 748 | Complete |
| TUI forms | `hermes-cli/tui/forms.rs` | 264 | Complete |
| TUI actions | `hermes-cli/tui/action.rs` | 194 | Complete |

---

## 2. What Is NOT Ported ✗

### 2.1 Python Tools (15 files, ~4,660 LOC)
| Python File | LOC | Rust Equivalent | Effort |
|-------------|-----|-----------------|--------|
| `approval.py` | 1,289 | None | High |
| `binary_extensions.py` | 42 | None | Trivial |
| `browser_camofox_state.py` | 47 | None | Trivial |
| `browser_supervisor.py` | 1,366 | None | High |
| `browser_providers/` (4 files) | 608 | None | Medium |
| `debug_helpers.py` | 105 | None | Low |
| `file_state.py` | 332 | None | Medium |
| `neutts_synth.py` | 104 | Partially (voice.rs) | Low |
| `openrouter_client.py` | 33 | None | Trivial |
| `osv_check.py` | 155 | None | Low |
| `slash_confirm.py` | 162 | None | Low |
| `tool_backend_helpers.py` | 144 | None | Low |
| `tool_output_limits.py` | 92 | None | Trivial |
| `url_safety.py` | 327 | None | Medium |
| `xai_http.py` | 12 | None | Trivial |

### 2.2 CRITICAL: Gateway Subsystem (52 files, ~70K LOC)
This is the single largest gap. Python's `gateway/` directory has **52 files, ~70K LOC**:

| Component | Python LOC | Rust |
|-----------|-----------|------|
| Gateway runner (`run.py`) | 16,046 | Stub (751 LOC) |
| Platform adapters (31 files) | ~48,000 | None |
| Session management | 1,387 | None |
| Config | 1,774 | None |
| Stream consumer | 1,018 | None |
| Delivery routing | 249 | None |
| Hook registry | 210 | None |
| Channel directory | 357 | None |
| Status/mirror/display | 500+ | None |
| **TOTAL** | **~70,000** | **751 LOC stub** |

**Platform adapters not ported** (31 files):
Telegram, Discord, Slack, Feishu, FeishuComment, DingTalk, WeChat, WeCom, WhatsApp, Signal, Matrix, Mattermost, BlueBubbles, Email, SMS, Webhook, QQBot (5 sub-files), HomeAssistant, API Server (SSE streaming), plus Yuanbao sub-adapters.

### 2.3 CRITICAL: CLI Subsystem (65 files, ~79K LOC)
Python's `hermes_cli/` has **65 files, ~79K LOC**. Rust has ~7 files, ~6.5K LOC.

Major unported CLI modules:
| Module | Python LOC | Rust |
|--------|-----------|------|
| Config (`config.py`) | 5,141 | Has config.rs (762) |
| Auth (`auth.py`) | 5,352 | None |
| Web server (`web_server.py`) | 4,259 | None |
| Setup wizard (`setup.py`) | 3,472 | None |
| Plugin manager (`plugins.py` + `plugins_cmd.py`) | 3,041 | None |
| Model management (`models.py`) | 3,555 | None |
| Tools config (`tools_config.py`) | 2,747 | None |
| Kanban CLI | 2,228 | Has kanban_tool.rs |
| System doctor (`doctor.py`) | 1,778 | None |
| Model switch | 1,773 | None |
| Skills hub CLI | 1,594 | Has skills_hub.rs |
| Profiles | 1,403 | None |
| Skin engine | 892 | None |
| Voice CLI | 846 | Has voice.rs |
| +50 more files | ~40,000 | None |

### 2.4 CRITICAL: Agent Subsystem (50+ files, ~31K LOC)
Python's `agent/` and `run_agent.py` contain the full agent architecture:

| Component | Python LOC | Rust |
|-----------|-----------|------|
| `run_agent.py` (AIAgent class) | 15,411 | agent.rs (1,322) |
| Auxiliary client | 4,520 | None |
| Anthropic adapter | 2,064 | None |
| Curator | 1,770 | None |
| Model metadata | 1,569 | None |
| Context compressor | 1,556 | None |
| Prompt builder | 1,448 | client.rs has basic prompt building |
| Bedrock adapter | 1,276 | None |
| Google OAuth | 1,061 | None |
| Codex Responses adapter | 1,050 | None |
| Gemini adapters (2) | 1,868 | None |
| Display/output | 1,008 | None |
| Insights/analytics | 930 | None |
| Usage pricing | 866 | None |
| Shell hooks | 836 | None |
| Context engine | 418 | None |
| Transport layer (5 files) | ~500 | None |
| Remaining agent files (30+) | ~12,000 | None |

### 2.5 Evaluation Environments (25 files, ~7,354 LOC)
| Component | Python LOC | Rust |
|-----------|-----------|------|
| Base env class | 714 | Environments trait exists |
| Agent loop harness | 473 | None |
| Tool context | 292 | None |
| Web research env | 719 | None |
| Agentic OPD env | 1,214 | None |
| Benchmarks (3 envs) | 1,983 | None |
| Tool call parsers (12 files) | ~1,400 | parser.rs exists |

### 2.6 Plugin System (~90 files, ~29K LOC)
| Component | Python LOC | Rust |
|-----------|-----------|------|
| Memory providers (9 providers) | ~8,000 | memory.rs built-in |
| Model providers (23 thin adapters) | ~1,500 | None (OpenAI client only) |
| Platform plugins | ~5,000 | None |
| Google Meet bot | ~2,500 | None |
| Spotify tools | ~500 | spotify_tool.rs exists |
| Langfuse observability | 874 | None |
| Disk cleanup | 496 | None |
| Teams pipeline | ~2,000 | None |
| Context engine plugin | 219 | None |

### 2.7 ACP Protocol (9 files, ~3,974 LOC)
| Component | Python LOC | Rust |
|-----------|-----------|------|
| Server | 1,714 | None |
| Tools | 1,180 | None |
| Session | 628 | None |
| Events | 194 | None |
| Permissions | 148 | None |

### 2.8 TUI Gateway (8 files, ~7,450 LOC)
| Component | Python LOC | Rust |
|-----------|-----------|------|
| TUI server (`server.py`) | 6,555 | None |
| Transport layer | 219 | None |
| WebSocket handler | 174 | None |
| Event publisher | 126 | None |

### 2.9 Cron System (~3K LOC)
| Component | Python LOC | Rust |
|-----------|-----------|------|
| Scheduler | 1,819 | cronjobs/scheduler.rs (139) |
| Job definitions | 1,115 | cronjobs/db.rs (444) |

### 2.10 State/Session DB (~2.9K LOC)
| Component | Python LOC | Rust |
|-----------|-----------|------|
| `hermes_state.py` (SQLite FTS5) | 2,863 | `database.rs` (454) — basic SQLite |

### 2.11 Additional Root Modules (~10K LOC)
| Component | Python LOC | Rust |
|-----------|-----------|------|
| `cli.py` | 12,967 | hermes-cli/main.rs (689) |
| `hermes_state.py` | 2,863 | database.rs (454) |
| `trajectory_compressor.py` | 1,508 | trajectory.rs (395) |
| `model_tools.py` | 867 | None |
| `toolsets.py` | 851 | None |
| `batch_runner.py` | 1,302 | None |
| `mcp_serve.py` | 897 | None |
| `mini_swe_runner.py` | 736 | None |

### 2.12 Web Frontend (non-Python, ~23K LOC)
Full React SPA with 12 pages, 15+ components, i18n (English + Chinese), and a 4.2K-line FastAPI backend server. Not a porting target for Rust.

---

## 3. Summary Statistics

| Metric | Python | Rust | Ported % |
|--------|--------|------|----------|
| **Tools** (tools/) | 86 files, 65,614 LOC | 50+ files | ~85% |
| **Tool implementations** (HermesTool) | ~80 tool functions | 62 impls | ~75% |
| **Tests** | ~470 files, ~115K LOC | 619 tests | — |
| **Agent core** (agent/ + run_agent) | 50+ files, ~47K LOC | agent.rs + client.rs (2K) | ~5% |
| **Gateway** (gateway/) | 52 files, ~70K LOC | gateway.rs (751 LOC) | ~1% |
| **CLI** (hermes_cli/) | 65 files, ~79K LOC | 8 files, ~6.5K LOC | ~8% |
| **Environments** | 25 files, 7,354 LOC | 8 files, ~1K LOC | ~15% |
| **Plugins** | ~90 files, ~29K LOC | None | 0% |
| **OAuth/Integrations** | Several thousand LOC | ms_graph, yuanbao | ~20% |
| **TOTAL** | **~360 files, ~278K LOC** | **106 files, 52K LOC** | **~18%** |

### Porting Progress by Layer

```
Tools Layer:        ████████████████████░░░░  80%
Core Infrastructure: ██████████████████░░░░░░  70%  
Agent Core:          ██░░░░░░░░░░░░░░░░░░░░░░   5%
Gateway:             ░░░░░░░░░░░░░░░░░░░░░░░░   1%
CLI:                 █░░░░░░░░░░░░░░░░░░░░░░░   8%
Plugins:             ░░░░░░░░░░░░░░░░░░░░░░░░   0%
Environments:        ██░░░░░░░░░░░░░░░░░░░░░░  15%
```

---

## 4. Recommendations

### 4.1 What to Port Next (Priority Order)

1. **Approval system** (`approval.py`) — blocking gap. Without it, interactive approval workflows don't work in Rust.

2. **Individual tool stubs** — `url_safety.py`, `osv_check.py`, `binary_extensions.py`, `slash_confirm.py`, `file_state.py`, `browser_camofox_state.py` — each is small (<350 LOC) and independent.

3. **State/session DB** (`hermes_state.py` → `database.rs` expansion) — needed for session persistence with FTS5 search.

4. **Gateway base** (`gateway/platforms/base.py` + `gateway/session.py`) — port the base platform adapter trait and session management so individual platform adapters can be added.

5. **CLI config** (`hermes_cli/config.py`) — port the YAML config system to complement existing TOML config.

### 4.2 What NOT to Port (Keep as Python or React)

- **Web frontend** (`web/`) — React/TypeScript. Run alongside Rust core via the web_server API.
- **TUI gateway** (`tui_gateway/`) — 6.5K LOC Python. Defer indefinitely.
- **Plugin system** (`plugins/`) — 90 files. The dynamic importlib-based plugin model doesn't map well to Rust. Design a statically-linked plugin system instead.
- **31 platform adapters** — each requires its own API integration. Port on-demand, when a user needs that platform in Rust mode.
- **Evaluation benchmarks** — use Python for evaluation; Rust can expose a runner API.

### 4.3 Architecture Decisions for Porting

| Python Pattern | Rust Equivalent |
|----------------|-----------------|
| `ABC` abstract classes | `trait` with default methods |
| `importlib` dynamic loading | Static linking or `#[cfg(feature = "...")]` feature gates |
| `asyncio` event loops | `tokio` shared runtime |
| `os.environ` for session state | `AppState` with `Arc<RwLock<>>` |
| `importlib.metadata.entry_points()` | Proc macro `#[plugin]` + build-time registration |
| AST-level `registry.register()` detection | Build script source analysis + `include!()` |
| `pydantic` schemas | `schemars::JsonSchema` derive |

### 4.4 Effort Estimates

| Work Item | Est. Effort | Dependencies |
|-----------|-------------|-------------|
| Port 15 remaining tools (~4.6K LOC) | 1-2 days | None |
| Port approval system (1.3K LOC) | 1 day | Gateway notif types |
| Expand state DB (2.8K LOC) | 2 days | database.rs |
| Port gateway base + session (4.9K LOC) | 3-4 days | Config, state DB |
| Port CLI config (5.1K LOC) | 2 days | None |
| Port agent context compressor (1.5K LOC) | 1 day | None |
| Port agent prompt builder (1.4K LOC) | 1 day | client.rs |
| Full agent subsystem (~31K LOC) | 4-6 weeks | Everything |
| Full gateway (~70K LOC) | 6-8 weeks | Agent core |
| Full CLI (~79K LOC) | 4-6 weeks | Config, auth |

---

## 5. File Inventory

### 5.1 Rust Port: 106 files, 52,419 LOC

```
crates/hermes-core/src/
├── lib.rs (127)
├── agent.rs (1,322)
├── client.rs (698)
├── parser.rs (596)
├── schema.rs (247)
├── schema_sanitizer.rs (466)
├── config.rs (762)
├── error.rs (162)
├── database.rs (454)
├── memory.rs (1,138)
├── context.rs (298)
├── context_files.rs (276)
├── trajectory.rs (395)
├── distillation.rs (188)
├── gateway.rs (751)
├── mcp.rs (970)
├── mcp_oauth.rs (1,724)
├── managed_tool_gateway.rs (727)
├── security.rs (800)
├── platform.rs (541)
├── interrupt.rs (198)
├── ansi_strip.rs (167)
├── fuzzy_match.rs (605)
├── budget_config.rs (340)
├── process_registry.rs (339)
├── browser_camofox.rs (414)
├── env_passthrough.rs (220)
├── credential_files.rs (238)
├── credential_pool.rs (1,008)
├── tool_result_storage.rs (335)
├── website_policy.rs (323)
├── ms_graph.rs (1,152)
├── yuanbao.rs (683)
├── voice.rs (2,549)
├── skills.rs (613)
├── skills_hub.rs (3,627)
├── skills_guard.rs (2,046)
├── skills_sync.rs (756)
├── skill_usage.rs (488)
├── tools.rs (518)
├── tools/ (35 tool files)
├── environments/ (8 files, ~1K LOC)
├── cronjobs/ (4 files, ~681 LOC)
├── kanban/ (2 files, ~608 LOC)
├── web_providers/ (5 files, ~133 LOC)
├── build.rs (11)
└── examples/simple_agent.rs (190)

crates/hermes-cli/src/
├── main.rs (689)
├── autonomous.rs (1,719)
└── tui/ (6 files, ~4,145 LOC)
```

### 5.2 Python Source (not ported): ~360 files, ~278K LOC

```
hermes-agent/
├── agent/           — 50 files, 31,695 LOC
├── gateway/         — 52 files, 69,988 LOC
├── hermes_cli/      — 65 files, 79,161 LOC
├── environments/    — 25 files, 7,354 LOC
├── plugins/         — ~90 files, ~29,000 LOC
├── cron/            — 3 files, 2,976 LOC
├── acp_adapter/     — 9 files, 3,974 LOC
├── tui_gateway/     — 8 files, 7,450 LOC
├── providers/       — 2 files, 356 LOC
├── scripts/         — 11 files, 4,844 LOC
├── optional-skills/ — 27 files, 10,272 LOC
├── website/         — 3 files, 1,392 LOC
├── Root *.py        — 15 files, 39,740 LOC
├── web/             — ~75 TS/JS files, not Python
└── tests/           — ~470 files, ~115K LOC
```

---

## 6. Rust-Specific Additions (No Python Equivalent)

These modules exist only in the Rust port (feature additions):

| Module | LOC | Description |
|--------|-----|-------------|
| `browser_downloader.rs` | 125 | File download via browser |
| `datetime_tool.rs` | 332 | Current datetime utility |
| `http_tool.rs` | 159 | Generic HTTP request tool |
| `notification_tool.rs` | 107 | Desktop notification + approval |
| `spotify_tool.rs` | 514 | 7 Spotify API tools |
| `video_analysis_tool.rs` | 104 | Video frame analysis |
| `web_providers/exa.rs` | 36 | Exa search engine |
| `web_providers/tavily.rs` | 36 | Tavily search engine |
| `schema.rs` | 247 | Shared JSON schema utilities |
| `cdp_utils.rs` | 72 | CDP protocol helpers |
| `sub_agent_tool.rs` | 605 | Sub-agent delegation (replaces `delegate_tool.py`) |

---

**Generated by**: Sisyphus — Final Port Audit
**Date**: 2026-05-11
**Rust Version**: 0.1.3
**Python Version**: Latest (unreleased commits)
