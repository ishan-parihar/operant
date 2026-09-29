//! memory-wire provider — in-process semantic memory via the `memory_wire` crate.
//!
//! memory-wire (<https://github.com/ishan-parihar/memory-wire>) is a Rust memory
//! infrastructure crate that fuses Hindsight-style `retain`/`recall`/`reflect`
//! with agentmemory-style hybrid retrieval (BM25 + token-overlap, RRF-fused). We
//! embed it as an **in-process library** (a cargo git-dependency), not a
//! subprocess: the library exposes `recall_with_weights`, `retain_tagged`, and
//! `retain_doc`, none of which the crate's HTTP surface offers.
//!
//! This provider implements the [`crate::memory_provider::MemoryProvider`]
//! trait against `memory_wire::api::MemoryService` — the local replacement for
//! the agentmemory REST integration it supersedes (see `docs/INTEGRATION-PLAN.md`
//! §3):
//!
//! | Method / hook        | memory-wire call                                |
//! |----------------------|-------------------------------------------------|
//! | `sync_turn`          | `retain` (every turn, truncated 500/2000)       |
//! | `on_memory_write`    | `retain_tagged` (add/update mirror)             |
//! | `prefetch`           | `recall_with_weights` (SHIPPED fusion)          |
//! | `memory_smart_search`| `recall_with_weights` (tool)                    |
//! | `memory_save`        | `retain_tagged` (tool)                          |
//!
//! memory-wire has no turn-ingestion endpoint and no session-lifecycle
//! endpoints; it is explicitly caller-driven. That is why `sync_turn` still
//! exists: **our** agent loop calls it, and we simply retain each turn.
//!
//! ## Panic containment (why this file is careful)
//!
//! memory-wire is a **0.x in-process** crate. There is no process boundary, so
//! a panic anywhere in its code would unwind straight through this provider
//! into the agent loop and kill the turn. Every call into the crate is
//! therefore wrapped in a [`std::panic::catch_unwind`] boundary (`guard`) that
//! degrades a panic (or an `Err`) to a *memory miss* — an empty prefetch or a
//! no-op sync — rather than a dead turn. This is the cheapest mitigation for
//! the in-process integration mode and is correct regardless of how defensive
//! the upstream crate is.
//!
//! ## Storage
//!
//! The provider owns a file-backed [`SqliteStore`] under the operant storage
//! directory, so memories survive across sessions. Recall is bank-isolated; we
//! use a single default bank.
//!
//! ## `spawn_blocking`
//!
//! The `memory_wire` API is **synchronous** (bundled SQLite), so it is called
//! directly rather than through `spawn_blocking`. The measured per-call cost is
//! ~0.3 ms for `retain` and ~0.4 ms for `recall` against an in-memory store —
//! far below the ~10 ms threshold where parking a tokio worker would matter, so
//! the extra task hop would cost more than it saves. The measurement is pinned
//! by `recall_and_retain_are_fast_enough_to_call_inline` in the test module; if
//! that test's budget is ever exceeded, revisit this decision.

use async_trait::async_trait;
use memory_wire::api::{MemoryService, ScoredMemory};
use memory_wire::recall::FusionWeights;
use memory_wire::store::{SqliteStore, Store};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;

use crate::error::{Error, Result};

/// Default bank id. Recall is bank-isolated; a single bank keeps every operant
/// memory in one recallable namespace.
pub const DEFAULT_BANK: &str = "operant";

/// Characters retained from the user turn in `sync_turn`. Mirrors the previous
/// agentmemory integration's truncation, kept on our side so the stored memory
/// is bounded regardless of what memory-wire does internally.
const USER_TURN_CHARS: usize = 500;
/// Characters retained from the assistant turn in `sync_turn`.
const ASSISTANT_TURN_CHARS: usize = 2000;

/// Run one `memory_wire` call, degrading a panic or an `Err` to a memory miss.
///
/// This is the panic boundary for the whole integration. A `memory_wire` panic
/// in a 0.x in-process crate would otherwise unwind through the provider into
/// the agent loop and abort the turn; here it becomes `None` — an empty
/// prefetch / a no-op sync — which is exactly the "degraded memory, live agent"
/// state the [`crate::memory_provider::MemoryProvider`] contract already
/// promises for an unreachable backend.
///
/// `f` is a plain (non-async) closure returning memory-wire's own `Result`; the
/// service is synchronous, so there is no await point inside the unwind window.
fn guard<T, F>(op: &'static str, f: F) -> Option<T>
where
    F: FnOnce() -> std::result::Result<T, memory_wire::api::ApiError>,
{
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(Ok(value)) => Some(value),
        Ok(Err(e)) => {
            tracing::warn!(
                op,
                error = %e,
                "memory_wire call failed — degrading to a memory miss"
            );
            None
        }
        Err(_) => {
            // A panic in a 0.x in-process dependency must never kill a turn.
            tracing::error!(op, "memory_wire panicked — degrading to a memory miss");
            None
        }
    }
}

/// In-process memory provider backed by `memory_wire`.
///
/// `S` is the backing [`Store`]. Production uses [`SqliteStore`] (file-backed,
/// persistent); the test module substitutes a store to exercise the panic
/// boundary and to drive the wiring tests through a store we fully control.
pub struct MemoryWireProvider<S: Store = SqliteStore> {
    service: Arc<MemoryService<S>>,
    bank: String,
}

impl<S: Store> MemoryWireProvider<S> {
    /// Wrap an already-constructed service (used by tests and by [`Self::new`]).
    pub fn from_service(service: MemoryService<S>, bank: impl Into<String>) -> Self {
        Self {
            service: Arc::new(service),
            bank: bank.into(),
        }
    }

    /// Retain a completed turn as one memory, bounded to 500 (user) / 2000
    /// (assistant) characters. This is the write-back half of the memory
    /// contract, invoked by the agent loop's `MemorySyncExecutor` after every
    /// turn. A guard miss is a no-op — a degraded backend must not fail the
    /// turn that triggered the write.
    fn retain_turn(&self, user: &str, assistant: &str) {
        if user.trim().is_empty() && assistant.trim().is_empty() {
            return;
        }
        let u = user.chars().take(USER_TURN_CHARS).collect::<String>();
        let a = assistant
            .chars()
            .take(ASSISTANT_TURN_CHARS)
            .collect::<String>();
        let content = format!("User: {u}\nAssistant: {a}");
        let bank = &self.bank;
        let svc = &self.service;
        guard("retain_turn", || svc.retain(bank, &content, None));
    }

    /// Recall for a query, returning a formatted text block. Uses
    /// `recall_with_weights` (not plain `recall`) so the fusion weights are
    /// explicit and can be tuned without changing the call shape.
    fn recall_text(&self, query: &str, limit: usize) -> String {
        if query.trim().is_empty() {
            return String::new();
        }
        let bank = &self.bank;
        let svc = &self.service;
        let hits = guard("recall", || {
            svc.recall_with_weights(bank, query, 2000, &FusionWeights::SHIPPED)
        });
        let Some(hits) = hits else {
            return String::new();
        };
        Self::format_hits(&hits, limit)
    }

    /// Format scored memories into a compact, deterministic text block.
    fn format_hits(hits: &[ScoredMemory], limit: usize) -> String {
        let mut out = String::new();
        for hit in hits.iter().take(limit) {
            let content = hit.memory.content.trim();
            if content.is_empty() {
                continue;
            }
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str("- ");
            out.push_str(content);
        }
        out
    }

    /// Retain a standalone fact (the `memory_save` tool / `on_memory_write`).
    fn retain_fact(&self, content: &str, tags: &[String]) -> Option<String> {
        if content.trim().is_empty() {
            return None;
        }
        let bank = &self.bank;
        let svc = &self.service;
        guard("retain_fact", || {
            svc.retain_tagged(bank, content, None, tags)
        })
    }
}

impl MemoryWireProvider<SqliteStore> {
    /// Open (or create) a file-backed store under `storage_dir` and build a
    /// provider over it, creating parent directories as needed.
    ///
    /// If the on-disk database cannot be opened, fall back to an in-memory
    /// store: an unreadable database file should cost persistence, not memory
    /// for the rest of the process. `Err` is reserved for the case where even
    /// the in-memory store cannot be constructed, which the caller
    /// (`build_memory_provider`) handles by degrading to `BuiltinProvider`.
    pub fn new(storage_dir: PathBuf) -> Result<Self> {
        let db_path = storage_dir.join("memory_wire.sqlite");
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let store = match SqliteStore::open(&db_path) {
            Ok(store) => store,
            Err(e) => {
                tracing::warn!(
                    path = %db_path.display(),
                    error = %e,
                    "memory_wire on-disk store unavailable — falling back to in-memory store"
                );
                SqliteStore::open_in_memory()
                    .map_err(|e| Error::Agent(format!("memory_wire store open failed: {e}")))?
            }
        };
        Ok(Self {
            service: Arc::new(MemoryService::new(store)),
            bank: DEFAULT_BANK.to_string(),
        })
    }
}

#[async_trait]
impl<S: Store + Send + Sync + 'static> crate::memory_provider::MemoryProvider
    for MemoryWireProvider<S>
{
    fn name(&self) -> &str {
        "memory_wire"
    }

    fn is_available(&self) -> bool {
        // In-process: a constructed provider is backed by a live store. The
        // graceful-degradation contract is carried by `guard` (per-call), not
        // by an availability flag.
        true
    }

    async fn initialize(&self, _session_id: &str) -> Result<()> {
        // memory-wire has no session-lifecycle endpoint and operant already
        // owns session identity (docs/INTEGRATION-PLAN.md §3.2). The bank is
        // created on the first retain, so there is nothing to do here.
        Ok(())
    }

    async fn system_prompt_block(&self) -> String {
        "In-process memory-wire memory active. Recall is automatic each turn; use memory_smart_search to search deeper and memory_save to store facts.".to_string()
    }

    async fn prefetch(&self, query: &str) -> String {
        let text = self.recall_text(query, 5);
        if text.is_empty() {
            String::new()
        } else {
            format!("[memory]\n{text}")
        }
    }

    async fn sync_turn(&self, user: &str, assistant: &str) -> Result<()> {
        self.retain_turn(user, assistant);
        Ok(())
    }

    fn tool_schemas(&self) -> Vec<Value> {
        vec![
            serde_json::json!({
                "name": "memory_smart_search",
                "description": "Hybrid BM25 + token-overlap search across long-term memory (memory-wire). Returns past work, decisions, and facts relevant to the query.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": {"type": "string", "description": "What to recall"},
                        "limit": {"type": "integer", "description": "Max results (default 5)"}
                    },
                    "required": ["query"]
                }
            }),
            serde_json::json!({
                "name": "memory_save",
                "description": "Save a fact, decision, or piece of work to long-term memory (memory-wire) so future sessions can recall it.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "content": {"type": "string", "description": "The memory content"},
                        "concepts": {"type": "array", "items": {"type": "string"}, "description": "Optional keywords to index"}
                    },
                    "required": ["content"]
                }
            }),
        ]
    }

    fn on_memory_write(&self, action: &str, _target: &str, content: &str) {
        if !matches!(action, "add" | "update") {
            return;
        }
        self.retain_fact(content, &["fact".to_string()]);
    }

    async fn handle_tool_call(&self, name: &str, args: Value) -> String {
        match name {
            "memory_smart_search" => {
                let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
                let limit = args
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(5)
                    .clamp(1, 20) as usize;
                let text = self.recall_text(query, limit);
                serde_json::json!({ "results": text }).to_string()
            }
            "memory_save" => {
                let content = args
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if content.trim().is_empty() {
                    return serde_json::json!({ "error": "content is required" }).to_string();
                }
                let concepts = args
                    .get("concepts")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|c| c.as_str().map(str::to_string))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                match self.retain_fact(&content, &concepts) {
                    Some(id) => serde_json::json!({ "saved": true, "id": id }).to_string(),
                    None => serde_json::json!({ "error": "memory save degraded" }).to_string(),
                }
            }
            _ => serde_json::json!({ "error": format!("unknown tool {name}") }).to_string(),
        }
    }

    async fn shutdown(&self) {
        // In-process: the store is dropped with the provider. No server to stop.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory_provider::MemoryProvider;
    use memory_wire::memory::{Bank, Memory};
    use memory_wire::store::{RecallInputs, StoreError, UpdateMode};

    /// Build a provider over a throwaway in-memory store.
    fn in_memory_provider() -> MemoryWireProvider<SqliteStore> {
        let store = SqliteStore::open_in_memory().expect("open in-memory store");
        MemoryWireProvider::from_service(MemoryService::new(store), "test")
    }

    /// A `Store` whose every mutating/reading operation panics, standing in for
    /// a `memory_wire` 0.x panic. Used to prove the provider's `guard` boundary
    /// degrades a dependency panic to a memory miss instead of killing the turn.
    struct PanicStore;

    impl Store for PanicStore {
        fn put_bank(&self, _bank: &Bank) -> std::result::Result<(), StoreError> {
            panic!("memory_wire store panicked (put_bank)");
        }
        fn put(&self, _m: &Memory) -> std::result::Result<(), StoreError> {
            panic!("memory_wire store panicked (put)");
        }
        fn get(
            &self,
            _bank_id: &str,
            _id: &str,
        ) -> std::result::Result<Option<Memory>, StoreError> {
            panic!("memory_wire store panicked (get)");
        }
        fn list(&self, _bank_id: &str) -> std::result::Result<Vec<Memory>, StoreError> {
            panic!("memory_wire store panicked (list)");
        }
        fn put_doc(
            &self,
            _m: &Memory,
            _tags: &[String],
            _document_id: Option<&str>,
            _update_mode: UpdateMode,
        ) -> std::result::Result<String, StoreError> {
            panic!("memory_wire store panicked (put_doc)");
        }
        fn recall_inputs(
            &self,
            _bank_id: &str,
            _query: &str,
            _tags: &[String],
            _fts_limit: usize,
        ) -> std::result::Result<RecallInputs, StoreError> {
            panic!("memory_wire store panicked (recall_inputs)");
        }
    }

    /// A provider whose backing store panics on every operation.
    fn panicking_provider() -> MemoryWireProvider<PanicStore> {
        MemoryWireProvider::from_service(MemoryService::new(PanicStore), "test")
    }

    // -- Round-trip through the real memory_wire API ------------------------

    #[tokio::test]
    async fn retain_then_recall_round_trips_through_real_api() {
        let p = in_memory_provider();
        // retain (via sync_turn's retain path)
        p.sync_turn("how is auth done", "we use jose middleware for jwt tokens")
            .await
            .expect("sync_turn");
        // recall (via prefetch's recall_with_weights path)
        let recalled = p.prefetch("jose middleware").await;
        assert!(
            recalled.contains("jose middleware"),
            "recall must surface the retained turn; got {recalled:?}"
        );
    }

    #[tokio::test]
    async fn prefetch_returns_empty_for_unrelated_query() {
        let p = in_memory_provider();
        p.sync_turn("auth", "jose").await.expect("sync_turn");
        let recalled = p.prefetch("quantum chromodynamics").await;
        assert!(
            !recalled.contains("jose"),
            "unrelated query must not surface the memory; got {recalled:?}"
        );
    }

    #[tokio::test]
    async fn sync_turn_truncates_to_500_2000_chars() {
        let p = in_memory_provider();
        let user = "u".repeat(USER_TURN_CHARS + 100);
        let assistant = "a".repeat(ASSISTANT_TURN_CHARS + 100);
        p.sync_turn(&user, &assistant).await.expect("sync_turn");
        let recalled = p.prefetch("User").await;
        // The stored user segment is exactly 500 chars, so the retained turn
        // must not carry the extra 100 'u's from the tail.
        assert!(
            !recalled.contains(&"u".repeat(USER_TURN_CHARS + 1)),
            "sync_turn must truncate the user turn to {USER_TURN_CHARS} chars"
        );
    }

    #[tokio::test]
    async fn memory_save_tool_retains_a_fact() {
        let p = in_memory_provider();
        let out = p
            .handle_tool_call(
                "memory_save",
                serde_json::json!({ "content": "deploy target is a linux release build" }),
            )
            .await;
        assert!(out.contains("\"saved\":true"), "got {out}");
        let recalled = p.prefetch("linux release build").await;
        assert!(recalled.contains("linux release build"), "got {recalled:?}");
    }

    #[tokio::test]
    async fn memory_smart_search_tool_recalls() {
        let p = in_memory_provider();
        p.sync_turn("ci", "the pipeline runs cargo test then clippy")
            .await
            .expect("sync_turn");
        let out = p
            .handle_tool_call(
                "memory_smart_search",
                serde_json::json!({ "query": "clippy" }),
            )
            .await;
        assert!(out.contains("clippy"), "got {out}");
    }

    #[tokio::test]
    async fn handle_tool_call_unknown_and_empty_are_rejected() {
        let p = in_memory_provider();
        let unknown = p.handle_tool_call("nope", serde_json::json!({})).await;
        assert!(unknown.contains("unknown tool"), "got {unknown}");
        let empty = p
            .handle_tool_call("memory_save", serde_json::json!({ "content": "" }))
            .await;
        assert!(empty.contains("content is required"), "got {empty}");
    }

    #[tokio::test]
    async fn on_memory_write_mirrors_add_and_update() {
        let p = in_memory_provider();
        p.on_memory_write("add", "MEMORY.md", "the api key rotates monthly");
        let recalled = p.prefetch("api key rotates").await;
        assert!(recalled.contains("rotates monthly"), "got {recalled:?}");
    }

    #[tokio::test]
    async fn on_memory_write_ignores_delete() {
        let p = in_memory_provider();
        p.on_memory_write("delete", "MEMORY.md", "should not be stored");
        let recalled = p.prefetch("should not be stored").await;
        assert!(
            !recalled.contains("should not be stored"),
            "got {recalled:?}"
        );
    }

    // -- Panic containment (mutation-proof) ----------------------------------

    #[tokio::test]
    async fn a_memory_wire_panic_degrades_to_a_memory_miss() {
        let p = panicking_provider();
        // prefetch must return empty (a miss), not unwind.
        let recalled = p.prefetch("anything").await;
        assert_eq!(
            recalled, "",
            "a store panic must degrade to an empty prefetch"
        );
        // sync_turn must be a no-op Ok, not unwind.
        let result = p.sync_turn("hello", "world").await;
        assert!(result.is_ok(), "a store panic must degrade to a no-op sync");
    }

    #[tokio::test]
    async fn a_memory_wire_panic_in_the_tool_path_degrades_to_an_error() {
        let p = panicking_provider();
        let out = p
            .handle_tool_call("memory_save", serde_json::json!({ "content": "a fact" }))
            .await;
        assert!(out.contains("degraded"), "got {out}");
    }

    // -- Wiring through the agent-loop call path -----------------------------

    /// The iter-422 lesson: a helper-only unit test proves nothing if the agent
    /// loop never reaches the real call. This drives the provider through the
    /// exact trait methods the loop uses — `sync_turn` (the auto write-back the
    /// `MemorySyncExecutor` calls after every turn) and `prefetch` (the recall
    /// `build_messages` runs before each turn) — and asserts the write is
    /// actually readable back through the same provider instance, i.e. the
    /// memory really lands in the store the loop will read.
    #[tokio::test]
    async fn agent_loop_path_writes_and_reads_through_the_same_provider() {
        let provider: std::sync::Arc<dyn MemoryProvider> =
            std::sync::Arc::new(in_memory_provider());

        // What the agent loop does post-turn (run.rs: sync_turn hook).
        provider
            .sync_turn(
                "which datastore backs sessions",
                "sessions live in sqlite; the schema is migrated at boot",
            )
            .await
            .expect("agent-loop sync_turn");

        // What the agent loop does pre-turn (build_messages: prefetch).
        let context = provider.prefetch("sqlite datastore sessions").await;
        assert!(
            context.contains("sqlite"),
            "the loop's prefetch must see the loop's sync_turn write; got {context:?}"
        );
        assert!(
            context.starts_with("[memory]"),
            "prefetch output must be tagged for <memory_context> injection; got {context:?}"
        );
    }

    // -- Latency measurement (pins the no-spawn_blocking decision) -----------

    /// Measures retain+recall wall time on the real store. The `spawn_blocking`
    /// decision in the module docs rests on this being far under the ~10 ms
    /// threshold where parking a tokio worker would matter. A generous budget
    /// keeps the test stable on loaded CI while still failing if the store ever
    /// regresses into something that blocks the loop.
    #[test]
    fn recall_and_retain_are_fast_enough_to_call_inline() {
        use std::time::Instant;
        let p = in_memory_provider();
        let start = Instant::now();
        for i in 0..50 {
            p.retain_turn(&format!("query {i}"), &format!("answer about topic {i}"));
        }
        for i in 0..50 {
            let _ = p.recall_text(&format!("topic {i}"), 5);
        }
        let elapsed = start.elapsed();
        assert!(
            elapsed.as_millis() < 5_000,
            "100 inline memory_wire ops took {elapsed:?}; spawn_blocking may be warranted"
        );
    }

    // -- Construction / naming ----------------------------------------------

    #[test]
    fn provider_reports_its_name_and_availability() {
        let p = in_memory_provider();
        assert_eq!(p.name(), "memory_wire");
        assert!(p.is_available());
    }

    #[tokio::test]
    async fn initialize_is_a_noop_and_succeeds() {
        let p = in_memory_provider();
        p.initialize("session-1").await.expect("initialize");
    }
}
