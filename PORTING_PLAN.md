# Hermes-RS Porting Plan

**Version:** 1.1  
**Date:** 2026-05-11  
**Status:** Active Roadmap  

---

## Executive Summary

This document defines the roadmap for completing hermes-rs — a Rust-based reimplementation of the hermes-agent Python codebase. The plan advocates a **hybrid architecture** where hermes-rs serves as the high-performance core engine while hermes-agent's Python code handles platform-specific integrations.

**Current State:** ~50% complete (up from 15-20% in v1.0)  
**Target State:** Functionally complete core engine  

### What Changed Since v1.0

Version 1.0 was written before deep codebase investigation. Since then:

- **P0 tools are all done** and tested (Browser, Vision, Checkpoint, Delegate, Skills, Session Search)
- **Core infrastructure is ~90% complete** (agent loop, config, database, memory, skills, MCP client, context management, CLI/TUI)
- **P1 tools Cron, Kanban, and Image Generation** are already implemented
- **P2 tools TTS and Approval** are already implemented
- **New gaps discovered**: MCP tool invocation, web scrape/crawl, process management, web search abstraction, gateway notification tools, transcription/STT
- **Test suite**: 54/54 hermes-cli ✅, 173/175 hermes-core ✅, 31 P0-specific tests

---

## Architecture Decision: Hybrid Core (UPDATED)

### Core Principle (Unchanged)
> hermes-rs = embeddable agent engine  
> hermes-agent = integration layer (gateway, CLI, platform)

### Architecture Corrections from Investigation

| Original Plan | Actual | Impact |
|--------------|--------|--------|
| Browser uses Playwright MCP server | Browser uses **Lightpanda CLI** native binary | Lightpanda works well, no MCP dependency needed |
| TTS stays in Python ("platform audio APIs") | TTS **partially ported** via kokoro-tiny + espeak-ng | Rust TTS works for basic TTS; Python still needed for multi-voice/multi-provider |
| CLI stays simple in Python | CLI is **fully in Rust** with ratatui TUI, 5 subcommands | Rust CLI is mature and feature-rich |
| Gateway "can be added later" | **3 gateway adapters exist**: Telegram, Discord, Slack via PlatformAdapter trait | Gateway framework exists but notification tooling needs wiring |

### What Stays in Python (DO NOT PORT)

These are intentionally excluded due to platform-specific maintenance burden:

| Module | Count | Reason |
|--------|-------|--------|
| Gateway platform adapters | 30+ | Platform-specific, constant maintenance (Teams, IRC, Google Chat, Webhook, etc.) |
| Full voice mode / STT | 2 | Platform audio APIs, multiple providers |
| Plugin system (all 5 types) | 5 types | Memory providers, model providers — can bridge later via MCP |
| Home Assistant | 1 | IoT-specific, HTTP-only bridge via MCP |
| Discord/Feishu/Message gateways | 4 | Redundant with existing gateway adapters |
| Computer use | 1 | Vision-based, can bridge via MCP |
| Browser (alternative backends) | 5 | Browserbase, Camofox, Browser Use, Firecrawl — Lightpanda covers primary use |

### What Goes in Rust (PORT)

See **Phase Breakdown** below.

---

## Phase Breakdown

### Phase 1: Tool Parity ⭐ HIGHEST PRIORITY (UPDATED)

**Goal:** 30 registered tools → complete tool coverage

**Status:** 30 tools registered in builtin.rs. Target was 73+ but many Python tools are platform adapters intentionally excluded. Realistic target: ~40-45 Rust-native tools + dynamic MCP tools.

#### P0 — Critical Tools ✅ ALL DONE

| Tool | Rust File | Status | Test Coverage |
|------|-----------|--------|---------------|
| Browser (Lightpanda) | `browser_tool.rs` | ✅ 5 commands (navigate/snapshot/click/type/scroll) | 9 tests |
| Vision/Image Analysis | `vision_tool.rs` | ✅ image_url + user_prompt | Pre-existing |
| Checkpoint Manager | `checkpoint_tool.rs` | ✅ git-based, SQLite-backed, list/restore/diff | 6 tests |
| Delegate Task | `sub_agent_tool.rs` | ✅ Leaf/Orchestrator/Batch roles, depth-limited | Pre-existing |
| Skills Management | `skills_tool.rs` | ✅ SkillsList + SkillView, YAML frontmatter | 9 tests |
| Session Search | `session_search_tool.rs` | ✅ FTS5 full-text, role_filter, limit clamping | 7 tests |

#### P1 — Important Tools

| Tool | Rust File | Priority | Status | Dependencies |
|------|-----------|----------|--------|---------------|
| Cron Jobs | `cron_tool.rs` | P1 | ✅ **DONE** — create/list/get/update/delete/pause/resume | CronDb (SQLite), scheduler.rs |
| Kanban | `kanban_tool.rs` | P1 | ✅ **DONE** — show/create/update/complete/assign/block/heartbeat/comment/link | KanbanDb (SQLite) |
| Image Generation | `image_generation_tool.rs` | P1 | ✅ **DONE** — FAL API | HTTP client |
| Home Assistant | — | P1 | ❌ NOT PORTED — bridge via MCP/Python | HTTP client |
| Send Message | — | P1 | ❌ NOT PORTED — gateway adapter tool | Gateway framework |
| Discord | — | P1 | ❌ NOT PORTED — gateway adapter tool | Gateway framework |
| Feishu Doc | — | P1 | ❌ NOT PORTED — gateway adapter tool | HTTP client |
| Feishu Drive | — | P1 | ❌ NOT PORTED — gateway adapter tool | HTTP client |

#### P2 — Nice to Have

| Tool | Rust File | Priority | Status | Dependencies |
|------|-----------|----------|--------|---------------|
| TTS | `tts_tool.rs` | P2 | ✅ **DONE** — kokoro-tiny + espeak | espeak-ng, kokoro-tiny |
| Approval | `notification_tool.rs` | P2 | ✅ **DONE** — `approval_request` tool | None |
| STT/Transcription | — | P2 | ❌ NOT PORTED | Audio capture |
| Voice Mode | — | P2 | ❌ NOT PORTED — full multimodal voice | TTS + STT |
| Memory Providers | — | P2 | ❌ NOT PORTED — base MemoryStore works | None |
| Interrupt | — | P2 | ❌ NOT PORTED | Agent loop |

#### 🔍 New Items Discovered During Investigation

These gaps were not in the original plan but are needed for production readiness:

| Tool/Feature | Priority | Why Needed | Dependencies |
|-------------|----------|------------|--------------|
| MCP tool invocation | **P1** | McpManager tools exist but aren't invocable by agent — McpTool implements HermesTool, needs registry wiring | mcp.rs (876 lines, ready) |
| Web scrape/crawl | **P1** | Python has web_scrape; Rust has only basic WebFetch (GET/POST) | HTTP client |
| Web search abstraction | **P1** | Currently hardcoded to DDG Lite; needs Tavily/Exa/ Searxng backends | HTTP client per provider |
| Process management | **P1** | Python has process_registry for long-running subprocess tracking | None |
| Gateway notification tool | **P1** | Send message through Telegram/Discord/Slack gateway adapters | Gateway framework (exists) |
| File upload/download | **P2** | Binary file operations with size limits, MIME-type handling | None |
| Video download tool | **P2** | Python has browser tool for video — basic file download | HTTP client |

#### Phase 1 Deliverables

- [ ] 40+ built-in tools registered in Rust tool registry (30 done)
- [ ] All tools functional with proper error handling
- [ ] Tool documentation auto-generated

---

### Phase 2: MCP Server Implementation ⭐ HIGH PRIORITY (UPDATED)

**Goal:** hermes-rs exposes all functionality via MCP protocol

**Status:** MCP **client** fully implemented (876 lines, HTTP + stdio, dynamic tool loading). MCP **server** still needs implementation.

#### What's Done
- [x] MCP client with HTTP and stdio transport
- [x] McpManager for server lifecycle management
- [x] McpTool implements HermesTool trait (ready for registration)
- [x] Dynamic tool loading from MCP servers

#### What Remains

| Task | Description | Dependencies |
|------|-------------|--------------|
| MCP Server Core | HTTP + stdio server via `hermes serve` command | axum dependency |
| Tool Exposure | Register all tools as MCP resources | Phase 1 |
| Session Management | MCP methods for session CRUD | Database (done) |
| Memory Operations | MCP methods for memory CRUD | Memory (done) |
| Skills Operations | MCP methods for skill loading | Skills (done) |
| Gateway Proxy | Python → Rust via MCP | None |

#### Phase 2 Deliverables

- [ ] `hermes-rs serve` command runs MCP server
- [ ] All tools accessible via MCP
- [ ] Session management via MCP
- [ ] Memory management via MCP

---

### Phase 3: State & Persistence (UPDATED — MOSTLY DONE)

**Goal:** Full session state management

**Status:** Largely complete. SQLite integration, session persistence, and checkpoints all work. Trajectory export is pending.

#### What's Done
- [x] SQLite integration via rusqlite with FTS5
- [x] Session persistence (save/restore conversation)
- [x] Checkpoint save/restore/diff via CheckpointManager
- [x] 4 database tables: sessions, messages, checkpoints, messages_fts
- [x] Thread-safe via Arc\<Mutex\<Connection\>\>

#### What Remains

| Task | Description | Dependencies |
|------|-------------|--------------|
| Trajectory Export | JSONL export for RL training | Session data |
| Config Persistence | User preferences in SQLite | Config system |
| Database migrations | Schema versioning for future changes | None |

#### Phase 3 Deliverables

- [x] SQLite-backed session database ✅
- [x] Checkpoint save/restore ✅
- [ ] Trajectory export (RL-ready format)
- [ ] Config persistence

---

### Phase 4: Control Systems (UPDATED — PARTIALLY DONE)

**Goal:** Tool execution governance

**Status:** Approval tool exists. Budget tracking, rate limiting, and guardrails remain.

#### What's Done
- [x] Approval system — `approval_request` tool registered
- [x] Tool schema definitions for all 30 tools

#### What Remains

| Task | Description | Dependencies |
|------|-------------|--------------|
| Approval Flow UI | User-facing approve/deny in TUI and CLI | None |
| Budget Tracking | Cost and iteration limits | Config |
| Rate Limiting | Per-tool/per-session rate limits | None |
| Tool Guardrails | Input validation, path safety, URL safety | Phase 1 |

#### Phase 4 Deliverables

- [ ] Approval flow blocks dangerous tools
- [ ] Budget enforcement
- [ ] Tool execution guards

---

### Phase 5: Advanced Features (UPDATED — PARTIALLY DONE)

**Goal:** Complete feature parity (if needed)

**Status:** Cron scheduler and Kanban board done. Plugin system not ported (intentional).

| Feature | Rust File | Status | Rationale | Effort |
|---------|-----------|--------|-----------|--------|
| Cron Scheduler | `scheduler.rs` | ✅ **DONE** | Background job execution | N/A |
| Kanban Board | `kanban/` | ✅ **DONE** | Multi-agent coordination | N/A |
| Full Plugin System | — | ❌ Deferred | Memory providers, model providers | Medium |
| Curator | — | ❌ Deferred | Skill lifecycle automation | Low |

---

## Infrastructure Assessment (NEW)

Beyond tools, the core engine modules are in good shape:

| Module | File | Lines | Status |
|--------|------|-------|--------|
| Agent Loop | `agent.rs` | 1317 | ✅ ReAct with XML parsing, streaming, self-healing, memory injection |
| Config | `config.rs` | 732 | ✅ TOML + env overrides, 9 sections |
| Database | `database.rs` | 455 | ✅ SQLite + FTS5, 4 tables |
| Memory | `memory.rs` | 1138 | ✅ MEMORY.md/USER.md format, importance scoring |
| MCP Client | `mcp.rs` | 876 | ✅ HTTP + stdio, dynamic tool loading |
| Skills | `skills.rs` | 613 | ✅ SKILL.md parsing, platform/command validation |
| Context | `context.rs` | 298 | ✅ Token estimation, compression |
| Gateway | `gateway.rs` | — | ⚠️ Framework exists, 3 adapters, notification wiring needed |
| CLI/TUI | `main.rs` + tui/ | 681+ | ✅ 5 subcommands, ratatui interface |
| Autonomous | `autonomous.rs` | 1627 | ✅ TODO.md-driven coding with git automation |

### Infrastructure Hardening Needs

| Item | Priority | Description |
|------|----------|-------------|
| Configurable tool timeouts | **P1** | Currently global timeout only |
| Tool output size limits | **P1** | Prevent OOM on large responses |
| Schema review | **P2** | Ensure JSON schema accurately reflects tool params |
| Error message improvement | **P2** | Actionable suggestions in all tool errors |
| CI `cargo test` | **P1** | Currently blocked by espeak-rs-sys linker issue; espeak_audio_stubs.c is workaround |

---

## Test Results (Current)

| Suite | Count | Status |
|-------|-------|--------|
| hermes-cli | 54/54 | ✅ All pass |
| hermes-core | 173/175 | ✅ (2 pre-existing test env failures) |
| P0 tool tests | 31 | ✅ browser(9), checkpoint(6), skills(9), session_search(7) |
| Linker | — | ✅ Fixed via espeak_audio_stubs.c + cc build dep |

---

## Implementation Guidelines

### Coding Standards (Unchanged)

Follow `hermes-rs/AGENTS.md` guidelines:
- `cargo fmt --all` before commits
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace` for all changes

### Tool Porting Pattern (Unchanged)

```rust
// crates/hermes-core/src/tools/your_tool.rs

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;

pub struct YourTool;

#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ToolArgs {
    // fields matching Python tool
}

#[async_trait]
impl HermesTool for YourTool {
    fn name(&self) -> &str { "your_tool" }
    fn description(&self) -> &str { "..." }
    
    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<ToolArgs>(self.name(), self.description())
    }
    
    async fn execute(&self, args: Value, context: ToolContext) -> ToolResult {
        // implementation
    }
}
```

### Testing Strategy (UPDATED)

1. **Unit tests** — Each tool has basic happy-path tests (31 added for P0)
2. **Integration tests** — Tool works in full agent loop
3. **Round-trip tests** — Compare Python tool output with Rust tool output

### Dependency Management (UPDATED)

Current dependencies in use:
- `rusqlite` — SQLite ✅
- `reqwest` — HTTP client ✅
- `tokio` — Async runtime ✅
- `schemars` — JSON schema generation ✅
- `serde` / `serde_json` — Serialization ✅
- `ratatui` — TUI framework ✅
- `serenity` — Discord gateway ✅
- `teloxide` — Telegram gateway ✅

---

## File Organization (UPDATED)

```
hermes-rs/
├── PORTING_PLAN.md          # This document
├── TODO.md                  # Autonomous mode task ledger
├── crates/
│   └── hermes-core/
│       └── src/
│           ├── tools/       # 24 tool files + mod.rs + builtin.rs
│           │   ├── mod.rs   # Module declarations + ToolRegistry
│           │   ├── builtin.rs  # 30-tool registration
│           │   ├── browser_tool.rs
│           │   ├── checkpoint_tool.rs
│           │   ├── cron_tool.rs
│           │   ├── file_tools.rs
│           │   ├── kanban_tool.rs
│           │   ├── memory_tools.rs
│           │   ├── skills_tool.rs
│           │   ├── sub_agent_tool.rs
│           │   ├── web_tools.rs
│           │   └── ... (18 more tool files)
│           ├── cronjobs/    # CronDb + scheduler
│           ├── kanban/      # KanbanDb
│           ├── mcp.rs       # MCP client (server pending)
│           ├── agent.rs     # ReAct loop
│           ├── config.rs    # TOML config
│           ├── database.rs  # SQLite
│           └── ...
│   └── hermes-cli/
│       └── src/
│           ├── main.rs      # 5 subcommands + TUI
│           └── autonomous.rs
```

---

## Progress Tracking (UPDATED)

Update `TODO.md` for autonomous mode progress. Currently:
- **Implemented**: 26 items documented
- **Pending**: Phase 1 P1 tools + infrastructure hardening (8 items)

Update this `PORTING_PLAN.md` when:
- Phase completes
- New tools identified
- Architecture changes
- Current state estimate changes significantly

---

## Dependencies & Prerequisites (UPDATED)

### Phase 1 Prerequisites (ALL DONE ✅)

- [x] Agent loop implementation (`agent.rs`) ✅
- [x] Tool registry (`tools.rs`) ✅  
- [x] MCP client (`mcp.rs`) ✅
- [x] Memory system (`memory.rs`) ✅
- [x] Skills system (`skills.rs`) ✅
- [x] Context management (`context.rs`) ✅
- [x] Config system (`config.rs`) ✅

### Required for Phase 2 (MCP Server)

- [ ] `axum` for HTTP server
- [ ] `tower` for middleware
- [ ] JSON-RPC types (or use rmcp crate)

### Required for Phase 3 (Persistence)

- [x] `rusqlite` for SQLite ✅

---

## Success Criteria (UPDATED)

### Phase 1 Success
- [ ] 40+ tools registered and functional (30 done)
- [ ] All tools have schema definitions (done)
- [ ] Tool tests pass (31 P0 tests done)
- [ ] MCP tool invocation wired (pending)
- [ ] Web search provider abstraction (pending)

### Phase 2 Success
- [ ] `hermes serve` runs MCP server
- [ ] Python client can call Rust agent
- [ ] Full session flow works end-to-end

### Phase 3 Success
- [x] Sessions persist across restarts ✅
- [x] Checkpoints work ✅
- [ ] Trajectory export produces valid JSONL

### Phase 4 Success
- [ ] Approval flow blocks dangerous tools
- [ ] Budget limits enforced

---

## Notes (UPDATED)

- **Gateway adapters stay in Python** — This is not a gap, it's intentional. 30+ Telegram/Discord/Slack/Teams/IRC/etc. adapters would be constant churn.
- **Browser tool** — Uses Lightpanda CLI native binary (not Playwright MCP as originally proposed). Works well, no MCP dependency needed.
- **Keep Rust lean** — Don't add features that aren't needed for core agent functionality. TTS port was done because it enables autonomous voice features.
- **Autonomous mode** — Already working and validated. Ensure it continues to work as tools are added.
- **espeak-rs-sys linker** — Known issue on systems without espeak-ng audio backend. Workaround: `espeak_audio_stubs.c` provides stub symbols. CI should set `SQLX_OFFLINE=true` equivalent pattern.

---

*Last Updated: 2026-05-11*  
*Maintainer: Development Team*
