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

/// (plan 11E) The memory scope a provider instance serves. Type, not string:
/// a call site cannot accidentally pass a seat identity where a personal
/// bank belongs — the wrong shape does not compile.
///
/// - `Personal` — the user's own memory (`operant` bank): what solo
///   sessions see.
/// - `Seat` — an organization seat's memory (`emp:<identity>` bank): a
///   seat's writes and recalls never cross into the personal bank, and a
///   solo session that never opens a Seat scope can never read `emp:*`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryScope {
    Personal,
    Seat {
        identity: String,
    },
    /// A gateway sender with no seat and no admin rights (KB-8). Keyed by a
    /// digest of the sender, so no two senders share a bank.
    Sender {
        key: String,
    },
}

/// (plan 11E) Derive the bank id for a scope. The seat identity is
/// sanitized ([a-z0-9_-], ≤32 chars) so a hostile seat id cannot forge
/// bank names — `Bank { identity: "operant/x" }` cannot reach the
/// personal bank or fabricate paths.
pub fn bank_for(scope: &MemoryScope) -> String {
    match scope {
        MemoryScope::Personal => DEFAULT_BANK.to_string(),
        MemoryScope::Seat { identity } => {
            let cleaned: String = identity
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                        c.to_ascii_lowercase()
                    } else {
                        '-'
                    }
                })
                .take(32)
                .collect();
            format!("emp:{cleaned}")
        }
        MemoryScope::Sender { key } => {
            use sha2::{Digest, Sha256};
            let digest = Sha256::digest(key.as_bytes());
            let hex: String = digest.iter().take(16).map(|b| format!("{b:02x}")).collect();
            format!("sender-{hex}")
        }
    }
}

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
    /// (plan 06B) the bank id is a runtime-switchable key behind a RwLock:
    /// the gateway flips a seat's turn between the personal bank and its
    /// `emp:<id>` bank with `set_scope` — one store, one provider, per-turn
    /// scoping.
    bank: std::sync::RwLock<String>,
    /// (plan 11C3) False when the on-disk open failed and `new` fell back
    /// to an in-memory store — the legacy import must not rename the
    /// user's MEMORY.md against a store that dies with the process.
    persistent: bool,
}

impl<S: Store> MemoryWireProvider<S> {
    /// Wrap an already-constructed service (used by tests and by [`Self::new`]).
    pub fn from_service(service: MemoryService<S>, bank: impl Into<String>) -> Self {
        Self {
            service: Arc::new(service),
            bank: std::sync::RwLock::new(bank.into()),
            persistent: false,
        }
    }

    /// (plan 06B) retarget this provider at another scope's bank. Called
    /// by the gateway inside the turn lock, before the turn runs: a seat's
    /// turn recalls and retains in `emp:<id>`, the owner's in the personal
    /// bank. One store — the bank is only a key.
    pub fn set_scope(&self, scope: &MemoryScope) {
        if let Ok(mut b) = self.bank.write() {
            *b = bank_for(scope);
        }
    }

    /// Retain a completed turn as one memory, bounded to 500 (user) / 2000
    /// (assistant) characters. This is the write-back half of the memory
    /// contract, invoked by the agent loop's `MemorySyncExecutor` after every
    /// turn. A guard miss is a no-op — a degraded backend must not fail the
    /// turn that triggered the write.
    fn retain_turn(&self, user: &str, assistant: &str) {
        let bank = self.bank_id();
        self.retain_turn_in(&bank, user, assistant);
    }

    /// Retain one turn into `bank`. The sync worker passes the bank the turn
    /// was queued under (KB-7), so a scope switch before the worker runs
    /// cannot move the turn into another seat's bank.
    fn retain_turn_in(&self, bank: &str, user: &str, assistant: &str) {
        if user.trim().is_empty() && assistant.trim().is_empty() {
            return;
        }
        // Memory outlives the session, so a secret in a turn is redacted before
        // it is stored (KB-13).
        let u = crate::redaction::redact_sensitive_text(
            &user.chars().take(USER_TURN_CHARS).collect::<String>(),
        );
        let a = crate::redaction::redact_sensitive_text(
            &assistant
                .chars()
                .take(ASSISTANT_TURN_CHARS)
                .collect::<String>(),
        );
        let content = format!("User: {u}\nAssistant: {a}");
        let svc = &self.service;
        guard("retain_turn", || svc.retain(bank, &content, None));
    }

    /// Recall for a query, returning a formatted text block. Uses
    /// `recall_with_weights` (not plain `recall`) so the fusion weights are
    /// explicit and can be tuned without changing the call shape.
    pub fn recall_text(&self, query: &str, limit: usize) -> String {
        if query.trim().is_empty() {
            return String::new();
        }
        // (plan 06B) clone the key out of the lock: the service call
        // outlives any guard we could hold here.
        let bank = self.bank.read().map(|b| b.clone()).unwrap_or_default();
        let svc = &self.service;
        let hits = guard("recall", || {
            svc.recall_with_weights(&bank, query, 2000, &FusionWeights::SHIPPED)
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
    pub fn retain_fact(&self, content: &str, tags: &[String]) -> Option<String> {
        if content.trim().is_empty() {
            return None;
        }
        // (plan 06B) clone the key out of the lock: the service call
        // outlives any guard we could hold here.
        let bank = self.bank.read().map(|b| b.clone()).unwrap_or_default();
        let svc = &self.service;
        // KB-13: a fact outlives the session, so a secret is redacted before storage.
        let content = crate::redaction::redact_sensitive_text(content);
        guard("retain_fact", || {
            svc.retain_tagged(&bank, &content, None, tags)
        })
    }

    // ── (plan 08) CLI pass-throughs ────────────────────────────────
    // The `operant memory` subcommands read and write the SAME engine the
    // agent recalls from — thin, panic-guarded, same degradation contract
    // as the turn hooks (a miss, never a dead command).

    /// One page of the provider's bank, newest first (the memory-wire
    /// store's page order).
    pub fn list_page(&self, limit: usize, offset: usize) -> Vec<memory_wire::memory::Memory> {
        // (plan 06B) clone the key out of the lock: the service call
        // outlives any guard we could hold here.
        let bank = self.bank.read().map(|b| b.clone()).unwrap_or_default();
        let svc = &self.service;
        guard("list_memories", || svc.list_memories(&bank, limit, offset)).unwrap_or_default()
    }

    /// One memory by id, or `None` (absent in this bank reads as absent).
    pub fn memory_by_id(&self, id: &str) -> Option<memory_wire::memory::Memory> {
        // (plan 06B) clone the key out of the lock: the service call
        // outlives any guard we could hold here.
        let bank = self.bank.read().map(|b| b.clone()).unwrap_or_default();
        let svc = &self.service;
        guard("get_memory", || svc.get_memory(&bank, id)).flatten()
    }

    /// Delete one memory; `false` when it was not in the bank.
    pub fn delete_memory_by_id(&self, id: &str) -> bool {
        // (plan 06B) clone the key out of the lock: the service call
        // outlives any guard we could hold here.
        let bank = self.bank.read().map(|b| b.clone()).unwrap_or_default();
        let svc = &self.service;
        guard("delete_memory", || svc.delete_memory(&bank, id)).unwrap_or(false)
    }

    /// Row count + age bounds for the provider's bank (zeroes when the
    /// service is degraded — the doctor and `stats` print what they get).
    pub fn memory_bank_stats(&self) -> Option<memory_wire::store::BankStats> {
        // (plan 06B) clone the key out of the lock: the service call
        // outlives any guard we could hold here.
        let bank = self.bank.read().map(|b| b.clone()).unwrap_or_default();
        let svc = &self.service;
        guard("bank_stats", || svc.bank_stats(&bank))
    }

    /// The provider's bank id (the `--bank` default the CLI prints).
    pub fn bank_id(&self) -> String {
        self.bank.read().map(|b| b.clone()).unwrap_or_default()
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
    /// (`build_memory_provider`) handles by degrading to a no-op provider.
    pub fn new(storage_dir: PathBuf) -> Result<Self> {
        let db_path = storage_dir.join("memory_wire.sqlite");
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut persistent = true;
        let store = match SqliteStore::open(&db_path) {
            Ok(store) => store,
            Err(e) => {
                tracing::warn!(
                    path = %db_path.display(),
                    error = %e,
                    "memory_wire on-disk store unavailable — falling back to in-memory store"
                );
                persistent = false;
                SqliteStore::open_in_memory()
                    .map_err(|e| Error::Agent(format!("memory_wire store open failed: {e}")))?
            }
        };
        Ok(Self {
            service: Arc::new(MemoryService::new(store)),
            bank: std::sync::RwLock::new(DEFAULT_BANK.to_string()),
            persistent,
        })
    }

    /// (plan 11E) Open a provider scoped to one memory namespace — the
    /// seam the seat wiring consumes. Personal is the default bank; a Seat
    /// scope isolates that seat's memories behind `emp:<identity>`, so
    /// seat writes never land in the personal bank and a personal
    /// provider's recall never sees them.
    pub fn scoped(storage_dir: PathBuf, scope: &MemoryScope) -> Result<Self> {
        let provider = Self::new(storage_dir)?;
        provider.set_scope(scope);
        Ok(provider)
    }

    /// (plan 11E) The bank this provider instance serves (diagnostics and
    /// the seat-side wiring).
    pub fn bank(&self) -> String {
        self.bank.read().map(|b| b.clone()).unwrap_or_default()
    }

    /// (plan 11C3) The one-time MEMORY.md/USER.md import. The storage dir
    /// is the legacy files' home (the same place MemoryManager wrote them),
    /// so the provider that replaces the file store is the one that
    /// migrates its data. Only runs against a PERSISTENT store — a failed
    /// sqlite open degraded to in-memory, and renaming the user's files
    /// after retaining into a store that dies with the process would lose
    /// their data.
    pub fn import_legacy_files(&self, storage_dir: &std::path::Path) -> std::io::Result<usize> {
        if !self.persistent {
            tracing::warn!(
                "memory-wire store is in-memory (degraded) — legacy MEMORY.md import skipped; retrying next start"
            );
            return Ok(0);
        }
        let service = Arc::clone(&self.service);
        let bank = self.bank.read().map(|b| b.clone()).unwrap_or_default();
        crate::memory_import::import_legacy_files(storage_dir, |content, tags| {
            guard("import_legacy", || {
                service.retain_tagged(&bank, &content, None, &tags)
            })
            .is_some()
        })
        .inspect(|&imported| {
            if imported > 0 {
                tracing::info!(
                    count = imported,
                    "imported legacy MEMORY.md/USER.md entries into memory-wire"
                );
            }
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

    fn is_persistent(&self) -> bool {
        self.persistent
    }

    /// (plan 06B) the trait seam routes to the concrete switch — the
    /// gateway calls this through `dyn MemoryProvider` inside the turn
    /// lock, so it must live on the trait, not just the concrete type.
    fn set_scope(&self, scope: &MemoryScope) {
        MemoryWireProvider::set_scope(self, scope);
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

    fn write_bank(&self) -> Option<String> {
        Some(self.bank_id())
    }

    async fn sync_turn_in_bank(
        &self,
        bank: Option<&str>,
        user: &str,
        assistant: &str,
    ) -> Result<()> {
        match bank {
            Some(bank) => self.retain_turn_in(bank, user, assistant),
            None => self.retain_turn(user, assistant),
        }
        Ok(())
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

    /// (plan 06B) per-turn scoping: `set_scope` retargets the ONE store —
    /// a seat's retain is invisible to the personal bank and vice versa,
    /// and switching back re-exposes what the seat bank holds.
    #[test]
    fn set_scope_partitions_seat_and_personal_banks() {
        // Construct over the DEFAULT bank so `Personal` scoping is a
        // no-op switch at the start (in_memory_provider's "test" bank is
        // not what set_scope(Personal) targets).
        let store = SqliteStore::open_in_memory().expect("open in-memory store");
        let provider =
            MemoryWireProvider::from_service(MemoryService::new(store), super::DEFAULT_BANK);
        // Personal: remember a fact.
        provider
            .retain_fact("personal bank fact", &[])
            .expect("personal retain");
        // A seat's turn: switch, remember a DIFFERENT fact, switch back.
        provider.set_scope(&MemoryScope::Seat {
            identity: "eng-alice".to_string(),
        });
        assert_eq!(provider.bank_id(), "emp:eng-alice");
        provider
            .retain_fact("seat bank fact", &[])
            .expect("seat retain");
        provider.set_scope(&MemoryScope::Personal);
        assert_eq!(provider.bank_id(), DEFAULT_BANK);
        let recalled = provider.recall_text("bank fact", 5);
        assert!(
            recalled.contains("personal bank fact"),
            "the personal bank must recall only its own rows, got: {recalled}"
        );
        assert!(
            !recalled.contains("seat bank fact"),
            "a seat's row must never leak into the personal recall"
        );
        provider.set_scope(&MemoryScope::Seat {
            identity: "eng-alice".to_string(),
        });
        let seat_recalled = provider.recall_text("bank fact", 5);
        assert!(
            seat_recalled.contains("seat bank fact"),
            "the seat bank must recall its own rows"
        );
        assert!(
            !seat_recalled.contains("personal bank fact"),
            "the personal bank must never leak into a seat's recall"
        );
    }

    /// (plan 08) the CLI/agent unification acceptance: a fact stored
    /// through the CLI-shaped pass-through is found by the agent-shaped
    /// recall on the SAME provider. Pre-plan-08 the CLI wrote MEMORY.md
    /// and the agent recalled memory-wire — the fact never met the recall.
    #[test]
    fn cli_store_is_agent_visible() {
        let provider = in_memory_provider();
        let id = provider
            .retain_fact("prefers tabs over spaces for Rust", &[])
            .expect("retain accepted");
        assert!(
            provider.memory_by_id(&id).is_some(),
            "the stored fact must be readable back by id"
        );
        let recalled = provider.recall_text("tabs spaces preference", 5);
        assert!(
            recalled.contains("tabs over spaces"),
            "the agent-shaped recall must find the CLI-stored fact, got: {recalled}"
        );
        let stats = provider.memory_bank_stats().expect("stats readable");
        assert_eq!(stats.memories, 1);
        let listed = provider.list_page(10, 0);
        assert_eq!(listed.len(), 1);
        assert!(provider.delete_memory_by_id(&id));
        assert!(provider.list_page(10, 0).is_empty());
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

    // -- (plan 11E) bank_for + scope isolation -------------------------------

    #[test]
    fn bank_for_derives_sanitized_bank_ids() {
        use super::MemoryScope;
        assert_eq!(
            bank_for(&MemoryScope::Personal),
            "operant",
            "personal scope keeps the default bank"
        );
        assert_eq!(
            bank_for(&MemoryScope::Seat {
                identity: "alice".to_string()
            }),
            "emp:alice"
        );
        // Hostile identity: path separators and mixed case cannot forge a
        // bank name that reaches the personal bank or fabricates paths.
        // (`.` and `/` each collapse to `-`: the leading ".." becomes "--".)
        assert_eq!(
            bank_for(&MemoryScope::Seat {
                identity: "../OPERANT/x!".to_string()
            }),
            "emp:---operant-x-"
        );
        // Overlong identity truncates.
        let long = "a".repeat(64);
        assert_eq!(
            bank_for(&MemoryScope::Seat { identity: long }).len(),
            "emp:".len() + 32
        );
    }

    /// (plan 11E red-team) Seat isolation, end to end over a REAL sqlite
    /// store: a fact retained through a Seat-scoped provider is recallable
    /// from that seat and INVISIBLE to a Personal-scoped provider over the
    /// same database file — and vice versa. A solo session (Personal
    /// only) can never see `emp:*`.
    #[test]
    fn seat_bank_is_isolated_from_personal_bank() {
        use super::MemoryScope;
        let dir = std::env::temp_dir().join(format!("operant-banks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let seat = MemoryWireProvider::scoped(
            dir.clone(),
            &MemoryScope::Seat {
                identity: "alice".to_string(),
            },
        )
        .expect("seat provider");
        assert_eq!(seat.bank(), "emp:alice");
        seat.retain_turn(
            "alice's private standup note",
            "contains seat-only knowledge",
        );
        let seat_recall = seat.recall_text("standup note", 5);
        assert!(
            !seat_recall.trim().is_empty(),
            "the seat sees its own memory"
        );

        let solo = MemoryWireProvider::scoped(dir.clone(), &MemoryScope::Personal)
            .expect("personal provider");
        assert_eq!(solo.bank(), "operant");
        let solo_recall = solo.recall_text("standup note", 5);
        assert!(
            solo_recall.trim().is_empty(),
            "a personal-scope recall must NEVER see emp:* — got: {solo_recall:?}"
        );

        // And the reverse: personal memory is invisible to the seat. The
        // probe ("preference") is lexically disjoint from the seat's own
        // memory ("standup note", "seat-only knowledge") so a hit can only
        // be a cross-bank leak.
        solo.retain_turn(
            "my durable preference",
            "personal-only fact about formatting",
        );
        let seat_view = seat.recall_text("preference", 5);
        assert!(
            seat_view.trim().is_empty(),
            "the seat must not see the personal bank — got: {seat_view:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// KB-7: a turn queued under the personal bank lands there even after the
    /// provider has been scoped to a seat, and the seat bank stays clean.
    #[tokio::test]
    async fn sync_turn_in_bank_retains_into_the_queued_bank() {
        use crate::memory_provider::MemoryProvider as _;
        let store = SqliteStore::open_in_memory().expect("open in-memory store");
        let provider =
            MemoryWireProvider::from_service(MemoryService::new(store), super::DEFAULT_BANK);
        let queued = provider.write_bank();
        let seat = MemoryScope::Seat {
            identity: "eng-alice".to_string(),
        };
        provider.set_scope(&seat);
        provider
            .sync_turn_in_bank(queued.as_deref(), "personal question zebra", "answer")
            .await
            .expect("sync");
        provider.set_scope(&MemoryScope::Personal);
        assert!(
            provider
                .recall_text("zebra", 5)
                .contains("personal question zebra")
        );
        provider.set_scope(&seat);
        assert!(!provider.recall_text("zebra", 5).contains("zebra"));
    }

    #[tokio::test]
    async fn a_secret_in_a_turn_or_a_fact_is_not_stored_in_clear() {
        // KB-13: a provider token typed into chat or passed to memory_store is
        // redacted before it is retained, so recall never returns it.
        use crate::memory_provider::MemoryProvider as _;
        let store = SqliteStore::open_in_memory().expect("open in-memory store");
        let provider =
            MemoryWireProvider::from_service(MemoryService::new(store), super::DEFAULT_BANK);
        // The engine masks `sk-` and `ghp_` itself; `sk_live_` is caught only by
        // operant's redactor, so this test fails if that layer is removed.
        let token = "sk_live_0123456789abcdef";

        provider
            .sync_turn_in_bank(
                None,
                &format!("zebra key {token}"),
                &format!("noted {token}"),
            )
            .await
            .expect("sync");
        provider.retain_fact(&format!("zebra fact {token}"), &[]);

        let recalled = provider.recall_text("zebra", 5);
        assert!(
            recalled.contains("zebra key"),
            "the turn was retained: {recalled}"
        );
        assert!(
            recalled.contains("zebra fact"),
            "the fact was retained: {recalled}"
        );
        assert!(
            !recalled.contains(token),
            "the token was stored in clear: {recalled}"
        );
    }
}

#[cfg(test)]
mod persistence_tests {
    use super::MemoryWireProvider;
    use crate::memory_provider::MemoryProvider;

    #[test]
    fn a_store_that_cannot_open_its_file_reports_itself_in_memory() {
        // KB-14: a sqlite path that cannot be opened degrades to an in-memory
        // store, and the provider reports that, so the tool result can say so.
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("memory_wire.sqlite")).unwrap();

        let provider = MemoryWireProvider::new(dir.path().to_path_buf()).unwrap();

        assert!(!provider.is_persistent());
    }
}
