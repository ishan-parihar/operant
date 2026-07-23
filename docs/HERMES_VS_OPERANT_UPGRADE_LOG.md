# Operant vs Hermes-Agent Upgrade Log

**Date:** July 23, 2026
**Scope:** Full-scale upgrade of operant's core agentic loop to match hermes-agent's functional backend

---

## Completed Phases

### Phase 1-3: MemoryProvider Trait + MemorySyncExecutor + Agent Loop Wiring
**Commit:** `66f1cf40` through `7e14e499`

| Feature | Status | Details |
|---------|--------|---------|
| MemoryProvider trait (15 methods) | ✅ Complete | 7 core + 8 lifecycle hooks |
| MemorySyncExecutor | ✅ Complete | FIFO ordering, graceful shutdown, job dropping warnings |
| Agent loop wiring | ✅ Complete | TOCTOU fix, UTF-8 safety, memory write mirroring |
| LearningMutationTool | ✅ Complete | Registered in tool system |
| TDG error logging | ✅ Complete | `on_session_end` errors surfaced instead of silently swallowed |

**Files modified:**
- `crates/operant-core/src/memory_provider.rs` — 8 lifecycle hooks, MemorySyncExecutor
- `crates/operant-core/src/agent/mod.rs` — Agent loop wiring, delegation observation

---

### Phase 4: Streaming Context Scrubber
**Commit:** `291d256f`

| Feature | Status | Details |
|---------|--------|---------|
| `strip_memory_context_tags()` | ✅ Complete | Strips `<long_term_memory>`, `<memory-context>`, `<workspace_context>` tags |
| Streaming path coverage | ✅ Complete | All 4 emission points covered |
| Non-streaming path coverage | ✅ Complete | `process_response` covered |

**Files modified:**
- `crates/operant-core/src/agent/mod.rs` — scrubber function + 4 emission points

---

### Phase 5a: Prompt Caching (system_and_3 Strategy)
**Commit:** `3e2ad5db`

| Feature | Status | Details |
|---------|--------|---------|
| `prompt_caching.rs` module | ✅ Complete | system_and_3 strategy, 4 breakpoints |
| Anthropic native layout | ✅ Complete | Wired into `convert_request()` |
| OpenRouter envelope layout | ✅ Complete | Wired into `build_chat_request()` |
| 5m/1h TTL support | ✅ Complete | Configurable via `CacheTtl` |
| Edge case handling | ✅ Complete | Empty-content messages skipped on envelope layout |
| Unit tests | ✅ Complete | 13 tests covering all scenarios |

**Files modified:**
- `crates/operant-core/src/agent/clients/prompt_caching.rs` — New module
- `crates/operant-core/src/agent/clients/anthropic.rs` — Wired into convert_request
- `crates/operant-core/src/client.rs` — Wired into build_chat_request for OpenRouter
- `crates/operant-core/src/agent/clients/mod.rs` — Registered new module

---

### Phase 5b: shutdown_memory_executor Arc Fix
**Commit:** `738ce17e`

| Feature | Status | Details |
|---------|--------|---------|
| Interior mutability | ✅ Complete | `Arc<std::sync::Mutex<Option<MemorySyncExecutor>>>` |
| `&self` signature | ✅ Complete | Works through `Arc<OperantAgent>` |
| All 5 access points | ✅ Complete | Updated with appropriate lock patterns |
| Debug logging | ✅ Complete | `try_lock()` failures logged |

**Files modified:**
- `crates/operant-core/src/agent/mod.rs` — Field type change + all access points

---

## Validation Results

| Check | Result |
|-------|--------|
| `cargo check --workspace` | ✅ Pass |
| `cargo test --workspace` | ✅ Pass (all tests) |
| `cargo clippy --workspace` | ⏳ Pending |
| Code review (each phase) | ✅ Pass |

---

## Remaining Gaps (from DEAD_CODE_GAP_ANALYSIS.md)

| Category | Priority | Status |
|----------|----------|--------|
| Learning graph mutations wiring | 🔴 High | Not started |
| MCP server completion | 🟡 Medium | Not started |
| TUI helper integration | 🟡 Medium | Not started |
| Background review wiring | 🟡 Medium | Not started |

---

## Commit History

```
738ce17e fix: make shutdown_memory_executor work through Arc<OperantAgent>
3e2ad5db feat: port Anthropic prompt caching from hermes-agent (system_and_3 strategy)
291d256f feat: add streaming context scrubber for memory tags
4ec826a1 fix: surface TDG on_session_end errors instead of silently swallowing
7e14e499 docs: clarify hook count in audit (15 trait methods, 8 lifecycle hooks)
66f1cf40 docs: update audit with final Phase 1-3 completion status
```
