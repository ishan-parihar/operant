# TDG Memory Infrastructure Integration Plan

**Date**: 2026-06-19
**Target**: operant-rs (Rust agent runtime)
**Source**: tdg-rust (graph memory infrastructure)
**Status**: Planning

---

## Executive Summary

Integrate TDG-rust as a pluggable memory provider in operant-rs, completing the Rust port of the Python operant-agent → TDGMemoryProvider integration. TDG provides graph-based memory (entity relationships, temporal edges, drive propagation) as the 6th backend in operant-rs's existing `MemoryProvider` trait system.

---

## Architecture

### Current State

```
operant-rs (Rust)
├── MemoryProvider trait (5 backends)
│   ├── builtin (MEMORY.md files)
│   ├── local-vector (SQLite FTS5)
│   ├── hindsight (Cloud API)
│   ├── retaindb (Cloud API)
│   └── mem0 (Cloud API)
└── MemoryManager (flat HashMap, file persistence)
```

### Target State

```
operant-rs (Rust)
├── MemoryProvider trait (6 backends)
│   ├── builtin (MEMORY.md files)
│   ├── local-vector (SQLite FTS5)
│   ├── hindsight (Cloud API)
│   ├── retaindb (Cloud API)
│   ├── mem0 (Cloud API)
│   └── tdg (Graph memory via tdg-rust library)
└── TDGMemoryProvider
    └── tdg-rust library (SQLite + FTS5 + event sourcing)
```

---

## Integration Points

### 1. MemoryProvider Trait Implementation

**File**: `crates/operant-core/src/memory_provider.rs`

Add `TdgMemoryProvider` struct implementing the 8-method `MemoryProvider` trait:

```rust
pub struct TdgMemoryProvider {
    pool: tdg_rust::ConnectionPool,
    lean: bool,
}

impl TdgMemoryProvider {
    pub fn new(storage_dir: PathBuf) -> Self {
        let db_path = storage_dir.join("tdg").join("graph.db");
        let pool = tdg_rust::ConnectionPool::new(
            db_path.to_str().unwrap(),
            5,  // max_connections
            30_000,  // busy_timeout_ms
        ).unwrap();
        // Initialize schema
        pool.with_connection(|conn| {
            tdg_rust::init_schema(conn)?;
            tdg_rust::init_fts(conn)?;
            tdg_rust::run_migrations(conn)?;
            Ok(())
        }).unwrap();
        Self { pool, lean: false }
    }
}
```

### 2. Trait Methods Mapping

| MemoryProvider Method | TDG Implementation |
|----------------------|-------------------|
| `name()` | `"tdg"` |
| `is_available()` | Check if tdg-rust is compiled in (always true) |
| `initialize(session_id)` | Init schema, start session context |
| `system_prompt_block()` | "TDG graph memory active. Entities, relationships, and temporal context available." |
| `prefetch(query)` | FTS5 hybrid search → format top 5 nodes |
| `sync_turn(user, assistant)` | Entity extraction → create observation node → link to session |
| `tool_schemas()` | Expose `tdg_search`, `tdg_create`, `tdg_connect`, `tdg_get_related` |
| `handle_tool_call(name, args)` | Delegate to tdg-rust CRUD operations |

### 3. Cargo.toml Dependency

**Option A: Path dependency (development)**
```toml
[dependencies]
tdg-rust = { path = "../../tdg-rust" }
```

**Option B: Git dependency (production)**
```toml
[dependencies]
tdg-rust = { git = "https://github.com/ishan-parihar/tdg-rust.git", tag = "v0.2.0" }
```

### 4. Config Extension

**File**: `crates/operant-core/src/config.rs`

Extend `MemorySettings`:

```rust
pub struct MemorySettings {
    pub provider: String,
    pub enabled: bool,
    pub hindsight_api_url: Option<String>,
    pub hindsight_bank_id: Option<String>,
    pub hindsight_budget: Option<String>,
    pub tdg_db_path: Option<String>,  // NEW: custom TDG database path
    pub tdg_lean: Option<bool>,       // NEW: lean mode for TDG
}
```

### 5. Factory Registration

**File**: `crates/operant-core/src/memory_provider.rs`

```rust
pub fn build_memory_provider(
    provider_name: &str,
    storage_dir: PathBuf,
) -> Arc<dyn MemoryProvider> {
    match provider_name {
        "hindsight" => Arc::new(HindsightProvider::new()),
        "local-vector" | "local_vector" => Arc::new(LocalVectorProvider::new()),
        "retaindb" => Arc::new(RetainDbProvider::new()),
        "mem0" => Arc::new(Mem0Provider::new()),
        "tdg" => Arc::new(TdgMemoryProvider::new(storage_dir)),  // NEW
        _ => Arc::new(BuiltinProvider::new(
            crate::memory::MemoryManager::with_storage_dir(storage_dir),
        )),
    }
}
```

### 6. Tool Integration

Expose TDG graph operations as agent tools:

| Tool | Description | TDG Operation |
|------|-------------|---------------|
| `tdg_search` | Search graph memory | FTS5 hybrid search |
| `tdg_create` | Create entity node | `add_node()` |
| `tdg_connect` | Create relationship | `add_edge()` |
| `tdg_get_related` | Get connected nodes | `get_edges()` |
| `tdg_observe` | Record observation | Create observation node + auto-link |
| `tdg_reflect` | Synthesize from graph | LLM-powered cross-memory synthesis |

---

## Implementation Phases

### Phase 1: Core Integration (1-2 days)

| Task | File | Effort |
|------|------|--------|
| Add tdg-rust dependency | `Cargo.toml` | 1 hour |
| Implement `TdgMemoryProvider` | `memory_provider.rs` | 4 hours |
| Register in factory | `memory_provider.rs` | 30 min |
| Add config options | `config.rs` | 1 hour |
| Unit tests | `memory_provider.rs` | 2 hours |

### Phase 2: Tool Exposure (1-2 days)

| Task | File | Effort |
|------|------|--------|
| Create `tdg_tools.rs` module | `tools/tdg_tools.rs` | 4 hours |
| Implement 6 TDG tools | `tools/tdg_tools.rs` | 4 hours |
| Register tools in agent | `tools.rs` | 1 hour |
| Integration tests | `tests/` | 2 hours |

### Phase 3: Entity Extraction (1-2 days)

| Task | File | Effort |
|------|------|--------|
| Wire entity extraction on `sync_turn` | `memory_provider.rs` | 3 hours |
| Create observation nodes from conversations | `memory_provider.rs` | 3 hours |
| Auto-link entities via `tdg_connect` | `memory_provider.rs` | 2 hours |

### Phase 4: Advanced Features (2-3 days)

| Task | File | Effort |
|------|------|--------|
| Drive propagation on memory store | `memory_provider.rs` | 4 hours |
| Temporal context in prefetch | `memory_provider.rs` | 3 hours |
| Consolidation engine integration | `memory_provider.rs` | 3 hours |
| Diagnostic engine wiring | `memory_provider.rs` | 2 hours |

---

## File Changes Summary

| File | Change Type | Description |
|------|-------------|-------------|
| `crates/operant-core/Cargo.toml` | Modify | Add tdg-rust dependency |
| `crates/operant-core/src/memory_provider.rs` | Modify | Add TdgMemoryProvider + factory registration |
| `crates/operant-core/src/config.rs` | Modify | Add TDG config options to MemorySettings |
| `crates/operant-core/src/tools/tdg_tools.rs` | Create | 6 TDG agent tools |
| `crates/operant-core/src/tools.rs` | Modify | Register tdg_tools module |
| `crates/operant-core/src/lib.rs` | Modify | Add tdg_tools module |
| `operant.example.toml` | Modify | Add TDG config example |
| `tests/tdg_integration.rs` | Create | Integration tests |

---

## Configuration Example

```toml
[memory]
provider = "tdg"
enabled = true
tdg_db_path = "~/.operant/tdg/graph.db"
tdg_lean = false
```

---

## Risk Assessment

| Risk | Mitigation |
|------|------------|
| **Binary size increase** | tdg-rust is ~9MB static; operant-rs already bundles SQLite |
| **Compile time** | tdg-rust has many deps; consider feature gating |
| **Memory usage** | TDG adds ~50MB RSS; offset by removing other providers |
| **Complexity** | Follow existing provider patterns exactly |
| **Testing** | Use in-memory SQLite for tests, no external deps |

---

## Success Criteria

1. **Functional**: `provider = "tdg"` in config activates graph memory
2. **Tool exposure**: Agent can call `tdg_search`, `tdg_create`, `tdg_connect`
3. **Entity extraction**: Conversations auto-create observation nodes
4. **Temporal context**: Prefetch includes graph relationships
5. **No regression**: All 626 existing tests pass
6. **Performance**: <10ms overhead per turn

---

## Next Steps

1. Add tdg-rust as path dependency
2. Implement `TdgMemoryProvider` struct
3. Register in factory
4. Write unit tests
5. Implement tool exposure
6. Integration testing
7. Update documentation
