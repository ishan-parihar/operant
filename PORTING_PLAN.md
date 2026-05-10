# Hermes-RS Porting Plan

**Version:** 1.0  
**Date:** 2026-05-10  
**Status:** Active Roadmap  

---

## Executive Summary

This document defines the roadmap for completing the hermes-rs implementation - a Rust-based reimplementation of the hermes-agent Python codebase. The plan advocates a **hybrid architecture** where hermes-rs serves as the high-performance core engine while hermes-agent's Python code handles platform-specific integrations.

**Current State:** ~15-20% complete  
**Target State:** Functionally complete core engine  

---

## Architecture Decision: Hybrid Core

### Core Principle
> hermes-rs = embeddable agent engine  
> hermes-agent = integration layer (gateway, CLI, platform)

This follows the architecture pattern used by modern AI tools (Cursor, Zed, Claude Code) where:
- **Rust/Go/C++** handles performance-critical agent orchestration
- **Python/TypeScript** handles platform integrations

### What Stays in Python (DO NOT PORT)

| Module | Count | Reason |
|--------|-------|--------|
| Gateway platform adapters | 32 | Platform-specific, constant maintenance |
| Browser automation | 1 | Use MCP for Playwright instead |
| Voice/TTS/STT | 3 | Platform audio APIs |
| CLI dispatcher | 1 | Keep simple |
| Plugin system | 5 types | Can add later |

### What Goes in Rust (PORT)

See **Phase Breakdown** below.

---

## Phase Breakdown

### Phase 1: Tool Parity ⭐ HIGHEST PRIORITY

**Goal:** 17 → 73+ built-in tools

**Rationale:** Tools are the core value proposition. Without tool parity, hermes-rs cannot function as a drop-in replacement for agent workloads.

#### P0 - Critical Tools (Port First)

| Tool | Source File | Priority | Dependencies |
|------|-------------|----------|---------------|
| Browser (Playwright) | `browser_tool.py` | P0 | None |
| Vision/Image Analysis | `vision_tools.py` | P0 | None |
| Checkpoint Manager | `checkpoint_manager.py` | P0 | SQLite integration |
| Delegate Task (full) | `delegate_tool.py` | P0 | Already partial |
| Skills Management (full) | `skills_tool.py` | P0 | Already partial |
| Session Search | `session_search_tool.py` | P0 | SQLite |

#### P1 - Important Tools

| Tool | Source File | Priority | Dependencies |
|------|-------------|----------|---------------|
| Cron Jobs | `cronjob_tools.py` | P1 | Scheduler impl |
| Kanban | `kanban_tools.py` | P1 | SQLite |
| Home Assistant | `homeassistant_tool.py` | P1 | HTTP client |
| Send Message | `send_message_tool.py` | P1 | Platform adapters |
| Discord | `discord_tool.py` | P1 | HTTP client |
| Feishu Doc | `feishu_doc_tool.py` | P1 | HTTP client |
| Feishu Drive | `feishu_drive_tool.py` | P1 | HTTP client |
| Image Generation | `image_generation_tool.py` | P1 | HTTP client |

#### P2 - Nice to Have

| Tool | Source File | Priority | Dependencies |
|------|-------------|----------|---------------|
| TTS | `tts_tool.py` | P2 | Audio APIs |
| STT/Transcription | `transcription_tools.py` | P2 | Audio APIs |
| Voice Mode | `voice_mode_tool.py` | P2 | Audio APIs |
| Memory (all providers) | `memory_tool.py` + plugins/ | P2 | Already partial |
| Approval | `approval.py` | P2 | None |
| Interrupt | `interrupt.py` | P2 | Already partial |

#### Phase 1 Deliverables

- [ ] 73+ registered tools in Rust tool registry
- [ ] All tools functional with proper error handling
- [ ] Tool documentation auto-generated

---

### Phase 2: MCP Server Implementation ⭐ HIGH PRIORITY

**Goal:** hermes-rs exposes all functionality via MCP protocol

**Rationale:** MCP is the integration layer. Making hermes-rs consumable via MCP allows:
- hermes-agent Python to call into hermes-rs for agent work
- Any other client to use hermes-rs
- Language-agnostic consumption

#### Tasks

| Task | Description | Dependencies |
|------|-------------|--------------|
| MCP Server Core | HTTP + stdio server implementation | None |
| Tool Exposure | Register all tools as MCP resources | Phase 1 |
| Session Management | MCP methods for session CRUD | None |
| Memory Operations | MCP methods for memory CRUD | None |
| Skills Operations | MCP methods for skill loading | None |
| Gateway Proxy | Python → Rust via MCP | None |

#### Phase 2 Deliverables

- [ ] `hermes-rs serve` command runs MCP server
- [ ] All tools accessible via MCP
- [ ] Session management via MCP
- [ ] Memory management via MCP

---

### Phase 3: State & Persistence

**Goal:** Full session state management

#### Tasks

| Task | Description | Dependencies |
|------|-------------|--------------|
| SQLite Integration | rusqlite for session storage | None |
| Session Persistence | Save/restore conversation state | SQLite |
| Checkpoints | Named conversation snapshots | Session persistence |
| Trajectory Export | JSONL export for RL training | None |
| Config Persistence | User preferences in SQLite | None |

#### Phase 3 Deliverables

- [ ] SQLite-backed session database
- [ ] Checkpoint save/restore
- [ ] Trajectory export (RL-ready format)
- [ ] Config persistence

---

### Phase 4: Control Systems

**Goal:** Tool execution governance

#### Tasks

| Task | Description | Dependencies |
|------|-------------|--------------|
| Approval System | Approve/deny tool execution | Phase 2 |
| Budget Tracking | Cost and iteration limits | None |
| Rate Limiting | Per-tool/per-session limits | None |
| Tool Guardrails | Input validation per tool | Phase 1 |

#### Phase 4 Deliverables

- [ ] Approval system for dangerous tools
- [ ] Budget enforcement
- [ ] Tool execution guards

---

### Phase 5: Advanced Features (Optional)

**Goal:** Complete feature parity (if needed)

| Feature | Rationale | Effort |
|---------|-----------|--------|
| Full Plugin System | Memory providers, model providers | Medium |
| Cron Scheduler | Background job execution | Medium |
| Kanban Board | Multi-agent coordination | Medium |
| Curator | Skill lifecycle automation | Low |

---

## Implementation Guidelines

### Coding Standards

Follow `hermes-rs/AGENTS.md` guidelines:
- `cargo fmt --all` before commits
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace` for all changes

### Tool Porting Pattern

Each tool should follow this structure:

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

### Testing Strategy

1. **Unit tests** - Each tool has basic happy-path tests
2. **Integration tests** - Tool works in full agent loop
3. **Round-trip tests** - Compare Python tool output with Rust tool output

### Dependency Management

Prefer Rust ecosystem crates over reimplementation:
- `rusqlite` - SQLite
- `reqwest` - HTTP client
- `tokio` - Async runtime
- `serenity` (for Discord) - Or HTTP only
- `telegram-bot` - Or HTTP only

---

## File Organization

```
hermes-rs/
├── PORTING_PLAN.md          # This document
├── TODO.md                  # Autonomous mode task ledger
├── crates/
│   └── hermes-core/
│       └── src/
│           ├── tools/       # All tool implementations
│           │   ├── mod.rs   # Re-exports
│           │   ├── file_tools.rs
│           │   ├── terminal_tool.rs
│           │   └── ... (new tools go here)
│           ├── mcp.rs       # Expand for server
│           └── ...
│   └── hermes-cli/
│       └── src/
│           └── main.rs      # Add `serve` command
```

---

## Progress Tracking

Update `TODO.md` for autonomous mode progress. Each phase item should be tracked.

Update this `PORTING_PLAN.md` when:
- Phase completes
- New tools identified
- Architecture changes

---

## Dependencies & Prerequisites

### Before Starting Phase 1

- [x] Agent loop implementation (`agent.rs`) ✅ DONE
- [x] Tool registry (`tools.rs`) ✅ DONE  
- [x] MCP client (`mcp.rs`) ✅ DONE
- [x] Memory system (`memory.rs`) ✅ DONE
- [x] Skills system (`skills.rs`) ✅ DONE
- [x] Context management (`context.rs`) ✅ DONE
- [x] Config system (`config.rs`) ✅ DONE

### Required for Phase 2 (MCP Server)

- [ ] `axum` or `tokio` for HTTP server
- [ ] JSON-RPC implementation
- [ ] MCP protocol types

### Required for Phase 3 (Persistence)

- [ ] `rusqlite` or `sqlx` for SQLite

---

## Success Criteria

### Phase 1 Success
- [ ] 73+ tools registered and functional
- [ ] All tools have schema definitions
- [ ] Tool tests pass

### Phase 2 Success
- [ ] `hermes serve` runs MCP server
- [ ] Python client can call Rust agent
- [ ] Full session flow works end-to-end

### Phase 3 Success
- [ ] Sessions persist across restarts
- [ ] Checkpoints work
- [ ] Trajectory export produces valid JSONL

### Phase 4 Success
- [ ] Approval flow blocks dangerous tools
- [ ] Budget limits enforced

---

## Notes

- **Gateway adapters stay in Python** - This is not a gap, it's intentional
- **Browser tool** - Use Playwright MCP server, don't reimplement browser automation
- **Keep Rust lean** - Don't add features that aren't needed for core agent functionality
- **Autonomous mode** - Already working, ensure it continues to work as tools are added

---

*Last Updated: 2026-05-10*
*Maintainer: Development Team*