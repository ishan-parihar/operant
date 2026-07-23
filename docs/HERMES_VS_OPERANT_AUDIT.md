# Operant vs Hermes-Agent: Core Agentic Loop Audit

**Date:** July 23, 2026
**Scope:** Core agentic-loop backend, self-evolving/self-learning infrastructure, memory (TDG), browser (Obscura), dead code
**Method:** File-level comparison of `hermes-agent/agent/` vs `operant/crates/operant-core/src/agent/`

---

## 1. Architecture Comparison

### hermes-agent (Python)
The agentic loop lives in `run_agent.py` (~12k LOC) + `agent/conversation_loop.py` (~4k LOC extracted). It is a **single-threaded async loop** with rich lifecycle hooks:

```
build_turn_context() → while(iteration < max) → LLM call → tool dispatch → post-turn hooks
```

Key lifecycle points:
- `build_turn_context()` — per-turn setup (session restore, message sanitization, preflight compression, memory prefetch)
- `run_conversation()` — the main loop body (LLM call, tool dispatch, retries, fallbacks, compression)
- `finalize_turn()` — post-loop: memory sync, skill nudge, background review, session distillation
- `MemoryManager` — orchestrates prefetch/sync/session hooks across providers
- `IterationBudget` — thread-safe consume/refund with grace call

### operant (Rust)
The agentic loop lives in `crates/operant-core/src/agent/mod.rs` (~1500 LOC visible). It follows the same ReAct pattern but with **async Rust (Tokio)**:

```
build_turn_context() → while(iteration < max) → LLM call → tool dispatch → post-turn hooks
```

Key lifecycle points:
- `turn_context::build_turn_context()` — per-turn setup
- `OperantAgent::run()` — the main loop body
- `turn_finalizer::check_and_advance_evolution_triggers()` — post-loop evolution check
- `MemoryProvider` trait + `TdgMemoryProvider` — graph memory via tdg-rust
- `IterationBudget` — thread-safe consume/refund with grace call

---

## 2. Self-Evolving / Self-Learning Infrastructure Gaps

This is the most critical area. hermes-agent has a **deeply integrated self-evolution pipeline** that operant has partially ported but has significant gaps.

### 2.1 What hermes-agent Has (Complete Pipeline)

| Component | File | Purpose |
|-----------|------|---------|
| **Turn Finalizer** | `agent/turn_finalizer.py` | Post-loop: check skill nudge, memory review, background review triggers |
| **Learning Graph** | `agent/learning_graph.py` | Skills + memory as graph nodes with edges (lexical overlap, related_skills) |
| **Learning Mutations** | `agent/learning_mutations.py` | User-initiated edit/delete for journey nodes (skills + memories) |
| **Memory Provider** | `agent/memory_provider.py` | Abstract ABC with 12 lifecycle hooks (initialize, prefetch, sync_turn, on_turn_start, on_session_end, on_session_switch, on_pre_compress, on_memory_write, on_delegation, etc.) |
| **Memory Manager** | `agent/memory_manager.py` | Orchestrates builtin + one external provider, background sync executor, prefetch timeout, streaming context scrubber |
| **Background Review** | `agent/background_review.py` | Autonomous skill/memory improvement after each turn — spawns a review thread |
| **Iteration Budget** | `agent/iteration_budget.py` | Thread-safe consume/refund with grace call support |
| **Prompt Caching** | `agent/prompt_caching.py` | Anthropic cache_control injection for multi-turn cost reduction |
| **Conversation Compression** | `agent/conversation_compression.py` | LLM-based summarization of old turns before context overflow |
| **Steer Injection** | `run_agent.py` | `/steer` directive injection between iterations for real-time guidance |

### 2.2 What operant Has (Partially Ported)

| Component | File | Status |
|-----------|------|--------|
| **Turn Finalizer** | `agent/turn_finalizer.rs` | ✅ Ported — evolution trigger checks (skill nudge + memory review) |
| **Learning Graph** | `agent/learning_graph.rs` | ✅ Ported — build_learning_graph(), delete_node(), edit_node() |
| **Learning Mutations** | `agent/learning_graph.rs` (same file) | ⚠️ Implemented but **NOT WIRED UP** — functions exist but no tool or command calls them |
| **Memory Provider** | `memory_provider.rs` | ⚠️ Trait defined but **missing 8 of 12 lifecycle hooks** (see gap table below) |
| **Memory Manager** | `memory.rs` (MemoryManager) | ⚠️ Basic — missing background executor, prefetch timeout, streaming scrubber |
| **Background Review** | `agent/background_review.rs` | ✅ Ported — spawn_background_review() |
| **Iteration Budget** | `agent/iteration_budget.rs` | ✅ Ported — consume/refund/grace |
| **Prompt Caching** | (not present) | ❌ **MISSING** — no cache_control injection for Anthropic/OpenRouter |
| **Conversation Compression** | `agent/llm_compressor.rs` | ✅ Ported — LLM-based summarization |
| **Steer Injection** | `agent/mod.rs` | ✅ Ported — drain_steers() between iterations |

### 2.3 Memory Provider Hook Gaps (Critical)

hermes-agent's `MemoryProvider` ABC defines **12 lifecycle hooks**. operant's `MemoryProvider` trait only implements **7**:

| Hook | hermes-agent | operant | Gap |
|------|-------------|---------|-----|
| `initialize()` | ✅ | ✅ | — |
| `system_prompt_block()` | ✅ | ✅ | — |
| `prefetch()` | ✅ | ✅ | — |
| `sync_turn()` | ✅ | ✅ | — |
| `get_tool_schemas()` | ✅ | ✅ | — |
| `handle_tool_call()` | ✅ | ✅ | — |
| `shutdown()` | ✅ | ✅ | — |
| `on_turn_start()` | ✅ | ❌ | **MISSING** — per-turn tick with runtime context |
| `on_session_end()` | ✅ | ❌ | **MISSING** — end-of-session extraction |
| `on_session_switch()` | ✅ | ❌ | **MISSING** — mid-process session_id rotation |
| `on_pre_compress()` | ✅ | ❌ | **MISSING** — extract before context compression |
| `on_memory_write()` | ✅ | ❌ | **MISSING** — mirror built-in memory writes |
| `on_delegation()` | ✅ | ❌ | **MISSING** — parent-side observation of subagent work |
| `queue_prefetch()` | ✅ | ❌ | **MISSING** — background recall for next turn |
| `backup_paths()` | ✅ | ❌ | **MISSING** — extra on-disk paths for backup |

**Impact:** Without `on_session_end`, `on_session_switch`, and `on_pre_compress`, the TDG memory provider cannot extract end-of-session insights, handle session rotation, or preserve knowledge during context compression. This means TDG loses context at session boundaries and during compression — the graph doesn't learn from completed sessions.

### 2.4 Memory Manager Gaps

| Feature | hermes-agent | operant | Gap |
|---------|-------------|---------|-----|
| **Background sync executor** | ✅ Single-worker ThreadPoolExecutor with FIFO ordering | ❌ | **MISSING** — sync_turn runs inline, can block the agent loop |
| **Prefetch timeout** | ✅ 8s timeout with thread join | ❌ | **MISSING** — prefetch can block indefinitely |
| **Streaming context scrubber** | ✅ StreamingContextScrubber state machine | ❌ | **MISSING** — memory context can leak into streaming UI |
| **Skill scaffolding stripping** | ✅ _strip_skill_scaffolding() | ❌ | **MISSING** — skill prompts pollute memory stores |
| **External provider limit** | ✅ One external provider max | ❌ | **MISSING** — no guard against tool schema bloat |
| **Context fencing** | ✅ `<memory-context>` XML tags + sanitize | ❌ | **MISSING** — injected context can be treated as user input |
| **Shutdown drain** | ✅ 5s bounded drain with abandon tracking | ❌ | **MISSING** — shutdown can lose in-flight writes |
| **Memory write mirroring** | ✅ notify_memory_tool_write() | ❌ | **MISSING** — built-in memory writes don't propagate to TDG |

---

## 3. TDG-Rust Memory Wiring

### 3.1 Current State

TDG is **wired up** as the primary memory backend:

- `memory_provider.rs`: `TdgMemoryProvider` implements the `MemoryProvider` trait
- `agent/mod.rs`: TDG hook fires after each turn (`provider.sync_turn()`)
- `tools/tdg_tools.rs`: `tdg_search`, `tdg_create`, `tdg_connect`, `tdg_get_related` tools
- `config.rs`: `memory.provider = "tdg"` selects TDG backend
- Feature-gated: `#[cfg(feature = "tdg")]` — falls back to BuiltinProvider when off
- Pool sharing: Tools share the provider's connection pool (fixed dual-database bug)

### 3.2 What Works

| Feature | Status |
|---------|--------|
| TDG initialization (SQLite + FTS5 + migrations) | ✅ |
| Entity extraction from turns | ✅ |
| Auto-wiring edges from extracted entities | ✅ |
| HybridRetriever (FTS5 + trust + recency scoring) | ✅ |
| Tool schemas (tdg_search, tdg_create, tdg_connect, tdg_get_related) | ✅ |
| Post-turn sync_turn hook (entity extraction + auto-wiring) | ✅ |
| Graceful fallback to BuiltinProvider on init failure | ✅ |

### 3.3 What's Missing

| Gap | Impact | Priority |
|-----|--------|----------|
| **No `on_session_end` hook** | End-of-session insights not extracted to graph | 🔴 High |
| **No `on_session_switch` hook** | Session rotation doesn't rebind TDG state | 🔴 High |
| **No `on_pre_compress` hook** | Knowledge lost during context compression | 🔴 High |
| **No `on_memory_write` mirror** | Built-in MEMORY.md writes don't propagate to TDG graph | 🟡 Medium |
| **No `on_delegation` hook** | Subagent results not observed by parent's TDG | 🟡 Medium |
| **No `on_turn_start` hook** | No per-turn metadata (remaining tokens, model, platform) | 🟢 Low |
| **No background sync executor** | sync_turn blocks the agent loop | 🟡 Medium |
| **No prefetch timeout** | TDG search can block indefinitely | 🟡 Medium |
| **No backup_paths()** | TDG database not included in hermes backup | 🟢 Low |

---

## 4. Obscura Browser Wiring

### 4.1 Current State

Obscura is **listed as a browser provider** but the wiring is incomplete:

- `browser_provider.rs`: `ObscuraProvider` listed in the provider enum
- `config.rs`: `browser.provider = "obscura"` selectable
- `browser_downloader.rs`: Auto-download to `~/.operant/bin/obscura`
- `browser_tool.rs`: `BrowserTool` dispatches to the active provider

### 4.2 What Works

| Feature | Status |
|---------|--------|
| Browser provider trait | ✅ |
| Lightpanda provider (auto-download, CDP) | ✅ |
| Browserbase cloud provider | ✅ |
| Browser Use cloud provider | ✅ |
| Camofox anti-detection provider | ✅ |
| Firecrawl scrape provider | ✅ |
| Browser downloader | ✅ |
| Browser CDP tool | ✅ |
| Browser dialog tool | ✅ |

### 4.3 What's Missing for Obscura

| Gap | Impact | Priority |
|-----|--------|----------|
| **ObscuraProvider implementation incomplete** | Provider listed but methods may not be fully wired | 🔴 High |
| **No Obscura-specific CDP integration** | CDP connection to Obscura binary not implemented | 🔴 High |
| **No browser session persistence** | Browser sessions not saved across turns | 🟡 Medium |
| **No browser vision integration** | Screenshot + AI analysis not wired | 🟡 Medium |
| **No browser console evaluation** | JavaScript execution not available | 🟢 Low |

---

## 5. Dead Code & Redundancies in Operant

### 5.1 Quantified Summary

| Category | Count | Risk | Action |
|----------|-------|------|--------|
| Tool argument structs (`#[allow(dead_code)]`) | ~60 | 🟢 Low | Keep — serde runtime usage |
| Learning graph mutations (unwired) | 7 | 🔴 High | Wire up as tools |
| MCP infrastructure (incomplete) | ~45 | 🟡 Medium | Complete or remove |
| TUI helpers (unwired) | ~30 | 🟡 Medium | Wire up or remove |
| Agent infrastructure (unwired) | ~15 | 🟡 Medium | Wire up or remove |
| Provider-specific (conditional) | ~13 | 🟢 Low | Keep |
| `#[allow(unused_*)]` | ~45 | 🟢 Low | Clean up |

### 5.2 Critical Dead Code (Wire Up or Remove)

| Item | File | Why It's Dead | Recommendation |
|------|------|---------------|----------------|
| `learning_graph::delete_node()` | `agent/learning_graph.rs:127` | Never called from any tool or command | 🔴 Wire up as `learning_manage` tool |
| `learning_graph::edit_node()` | `agent/learning_graph.rs:137` | Never called from any tool or command | 🔴 Wire up as `learning_manage` tool |
| `learning_graph::delete_skill_node()` | `agent/learning_graph.rs:150` | Internal function, never called | 🔴 Wire up via `delete_node` |
| `learning_graph::edit_skill_node()` | `agent/learning_graph.rs:168` | Internal function, never called | 🔴 Wire up via `edit_node` |
| `learning_graph::delete_memory_node()` | `agent/learning_graph.rs:282` | Internal function, never called | 🔴 Wire up via `delete_node` |
| `learning_graph::edit_memory_node()` | `agent/learning_graph.rs:338` | Internal function, never called | 🔴 Wire up via `edit_node` |
| `MutationResult` struct | `agent/learning_graph.rs:215` | Return type for unwired functions | 🔴 Wire up |
| `learning_mutation_tool.rs` | `tools/learning_mutation_tool.rs` | Tool exists but `LearningMutationTool` not registered in builtin tools | 🔴 Register in `register_builtin_tools()` |

### 5.3 Herme-Agent Features Completely Missing in Operant

| Feature | hermes-agent File | operant Status |
|---------|-------------------|----------------|
| **Prompt caching (Anthropic cache_control)** | `agent/prompt_caching.py` | ❌ Not implemented |
| **Streaming context scrubber** | `agent/memory_manager.py:StreamingContextScrubber` | ❌ Not implemented |
| **Context fencing (`<memory-context>` tags)** | `agent/memory_manager.py:build_memory_context_block` | ❌ Not implemented |
| **Skill scaffolding stripping** | `agent/memory_manager.py:_strip_skill_scaffolding` | ❌ Not implemented |
| **External memory provider limit (one max)** | `agent/memory_manager.py:add_provider` | ❌ Not implemented |
| **Memory write mirroring** | `agent/memory_manager.py:notify_memory_tool_write` | ❌ Not implemented |
| **Shutdown drain with abandon tracking** | `agent/memory_manager.py:_drain_sync_executor` | ❌ Not implemented |
| **Background sync executor (single-worker)** | `agent/memory_manager.py:_submit_background` | ❌ Not implemented |
| **Prefetch timeout (8s)** | `agent/memory_manager.py:_prefetch_provider` | ❌ Not implemented |
| **on_session_end hook** | `agent/memory_provider.py` | ❌ Not implemented |
| **on_session_switch hook** | `agent/memory_provider.py` | ❌ Not implemented |
| **on_pre_compress hook** | `agent/memory_provider.py` | ❌ Not implemented |
| **on_memory_write hook** | `agent/memory_provider.py` | ❌ Not implemented |
| **on_delegation hook** | `agent/memory_provider.py` | ❌ Not implemented |
| **queue_prefetch (background recall)** | `agent/memory_provider.py` | ❌ Not implemented |
| **backup_paths()** | `agent/memory_provider.py` | ❌ Not implemented |
| **node_detail() (inspect before edit)** | `agent/learning_mutations.py` | ❌ Not implemented |

---

## 6. Priority Action Plan

### Phase 1: Memory Provider Hooks (Critical — TDG Parity)
Wire up the missing 8 lifecycle hooks in `MemoryProvider` trait:
1. `on_turn_start()` — per-turn metadata
2. `on_session_end()` — end-of-session extraction
3. `on_session_switch()` — session rotation rebinding
4. `on_pre_compress()` — preserve knowledge during compression
5. `on_memory_write()` — mirror built-in writes to TDG
6. `on_delegation()` — observe subagent results
7. `queue_prefetch()` — background recall for next turn
8. `backup_paths()` — include TDG in backup

### Phase 2: Memory Manager Infrastructure (Critical)
Port the missing memory manager infrastructure:
1. Background sync executor (single-worker, FIFO)
2. Prefetch timeout (8s)
3. Streaming context scrubber
4. Skill scaffolding stripping
5. Context fencing (`<memory-context>` tags)
6. External provider limit (one max)
7. Memory write mirroring
8. Shutdown drain with abandon tracking

### Phase 3: Learning Graph Wiring (High)
Wire up the existing learning graph mutation functions:
1. Register `LearningMutationTool` in `register_builtin_tools()`
2. Connect `delete_node`, `edit_node` to the tool system
3. Add `node_detail()` for edit prefill
4. Add `_clear_skill_cache()` equivalent

### Phase 4: Prompt Caching (Medium)
Implement Anthropic cache_control injection:
1. Port `apply_anthropic_cache_control()` from hermes-agent
2. Inject cache breakpoints at system prompt + last 3 messages
3. Support both native Anthropic and OpenRouter cache layouts

### Phase 5: Obscura Browser (Medium)
Complete the Obscura browser integration:
1. Finish `ObscuraProvider` implementation
2. Wire CDP connection to Obscura binary
3. Add browser session persistence
4. Add browser vision (screenshot + AI analysis)

### Phase 6: Dead Code Cleanup (Low)
Clean up suppressed warnings:
1. Remove unused `warn` import in operant-core
2. Remove unused `old_significator` variable
3. Audit 45 `#[allow(unused_*)]` annotations for removal

---

## 7. Summary

| Area | operant Coverage | Gap Severity |
|------|-----------------|--------------|
| Core ReAct loop | ~90% | 🟢 Low — same pattern, good parity |
| Self-evolution pipeline | ~60% | 🟡 Medium — turn_finalizer + background_review ported, but memory hooks missing |
| Memory provider hooks | ~40% | 🔴 High — 7 of 15 hooks implemented |
| Memory manager infrastructure | ~30% | 🔴 High — missing background executor, timeout, scrubber |
| TDG graph memory | ~70% | 🟡 Medium — works but loses context at boundaries |
| Learning graph mutations | ~80% | 🟡 Medium — implemented but not wired to tools |
| Prompt caching | 0% | 🔴 High — no cache_control injection |
| Browser (Obscura) | ~50% | 🟡 Medium — provider trait exists, implementation incomplete |
| Dead code | ~242 instances | 🟡 Medium — mostly tool arg structs (keep), some unwired features |

**Bottom line:** operant's core ReAct loop is solid and well-ported. The critical gaps are in the **memory provider lifecycle hooks** (which prevent TDG from learning at session boundaries and during compression) and the **memory manager infrastructure** (which can cause the agent to block on slow providers). The learning graph mutations are implemented but need wiring. Prompt caching is entirely absent.
