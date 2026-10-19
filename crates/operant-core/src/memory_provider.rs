//! Memory provider system for Operant-RS.
//!
//! The active provider is selected by `config.memory.provider`:
//!
//! | Value         | Backend                                                           |
//! |---------------|-------------------------------------------------------------------|
//! | `"agentmemory"` (also `"memory_wire"` / `"memory-wire"`) | In-process hybrid semantic memory via the `memory_wire` crate |
//! | `"builtin"`   | File-backed MEMORY.md / USER.md (zero-dependency fallback)        |
//! | other         | Silently falls back to `"builtin"`                                |
//!
//! The provider name `"agentmemory"` is retained for config compatibility: it
//! is served by the in-process [`crate::memory_wire::MemoryWireProvider`]. The
//! agentmemory REST server, its `npx` auto-spawn, and its deferred MCP server
//! were removed — two live engines is how double-engine conflicts start.
//!
//! Old provider names (`"tdg"`, `"hindsight"`, `"retaindb"`, `"mem0"`,
//! `"local-vector"`) were removed — configs that still reference them are
//! silently downgraded to `"builtin"`.

use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::error::Result;

// ---------------------------------------------------------------------------
// Background sync executor
// ---------------------------------------------------------------------------

/// Single-worker FIFO executor for memory provider operations.
/// Ensures sync_turn, on_memory_write, and other background writes
/// happen in order (no interleaving) and don't block the agent loop.
///
/// Ported from hermes-agent's MemoryManager._submit_background() pattern
/// where a dedicated ThreadPoolExecutor with FIFO ordering processes
/// memory writes sequentially.
pub struct MemorySyncExecutor {
    tx: mpsc::Sender<SyncJob>,
    handle: Option<tokio::task::JoinHandle<()>>,
    /// Shared runtime-metrics registry. When set, sync_turn failures and
    /// channel-full job drops are counted so the TUI status bar can surface
    /// memory-backend health. The agent passes its own registry via
    /// `with_metrics`; standalone usage (tests) leaves it unset.
    metrics: Option<Arc<crate::runtime_metrics::RuntimeMetrics>>,
}

enum SyncJob {
    SyncTurn {
        bank: Option<String>,
        user: String,
        assistant: String,
    },
    MemoryWrite {
        action: String,
        target: String,
        content: String,
    },
    Delegation {
        task: String,
        result: String,
    },
    SessionEnd {
        messages: Vec<crate::client::Message>,
    },
    Shutdown,
}

impl MemorySyncExecutor {
    /// Create a new executor that processes jobs sequentially on a
    /// dedicated background task. The channel capacity (256) bounds
    /// memory to ~256 pending writes before backpressure kicks in.
    pub fn new(provider: Arc<dyn MemoryProvider>) -> Self {
        Self::new_with_metrics(provider, None)
    }

    /// Create the executor with a shared runtime-metrics registry threaded
    /// into the worker. The agent uses this so sync_turn failures and
    /// channel-full drops are counted for the TUI status bar; standalone
    /// callers (tests) pass `None`.
    pub fn new_with_metrics(
        provider: Arc<dyn MemoryProvider>,
        metrics: Option<Arc<crate::runtime_metrics::RuntimeMetrics>>,
    ) -> Self {
        let (tx, rx) = mpsc::channel(256);
        let handle = tokio::spawn(Self::run_loop(provider, metrics.clone(), rx));
        Self {
            tx,
            handle: Some(handle),
            metrics,
        }
    }

    /// Submit a sync_turn job (non-blocking). `bank` is the bank the turn
    /// belongs to, read by the caller now, so the worker never reads it later.
    pub fn submit_sync_turn(&self, user: &str, assistant: &str, bank: Option<String>) {
        if self
            .tx
            .try_send(SyncJob::SyncTurn {
                bank,
                user: user.to_string(),
                assistant: assistant.to_string(),
            })
            .is_err()
        {
            if let Some(m) = &self.metrics {
                m.record_memory_jobs_dropped();
            }
            tracing::warn!("MemorySyncExecutor: sync_turn channel full — job dropped");
        }
    }

    /// Submit a memory write mirror job (non-blocking).
    pub fn submit_memory_write(&self, action: &str, target: &str, content: &str) {
        if self
            .tx
            .try_send(SyncJob::MemoryWrite {
                action: action.to_string(),
                target: target.to_string(),
                content: content.to_string(),
            })
            .is_err()
        {
            if let Some(m) = &self.metrics {
                m.record_memory_jobs_dropped();
            }
            tracing::warn!("MemorySyncExecutor: memory_write channel full — job dropped");
        }
    }

    /// Submit a delegation observation job (non-blocking).
    pub fn submit_delegation(&self, task: &str, result: &str) {
        if self
            .tx
            .try_send(SyncJob::Delegation {
                task: task.to_string(),
                result: result.to_string(),
            })
            .is_err()
        {
            if let Some(m) = &self.metrics {
                m.record_memory_jobs_dropped();
            }
            tracing::warn!("MemorySyncExecutor: delegation channel full — job dropped");
        }
    }

    /// Submit a session end extraction job (non-blocking).
    /// Limits to the last 5 assistant messages to avoid cloning large conversations.
    pub fn submit_session_end(&self, messages: &[crate::client::Message]) {
        let limited: Vec<_> = messages
            .iter()
            .filter(|m| m.role == crate::client::Role::Assistant)
            .take(5)
            .cloned()
            .collect();
        if self
            .tx
            .try_send(SyncJob::SessionEnd { messages: limited })
            .is_err()
        {
            if let Some(m) = &self.metrics {
                m.record_memory_jobs_dropped();
            }
            tracing::warn!("MemorySyncExecutor: session_end channel full — job dropped");
        }
    }

    /// Graceful shutdown: drain pending jobs (up to 5s) then abandon.
    /// Ported from hermes-agent's _drain_sync_executor() pattern.
    pub async fn shutdown(mut self) {
        // Send shutdown sentinel
        let _ = self.tx.send(SyncJob::Shutdown).await;
        // Drop the sender so the channel closes
        drop(self.tx);
        // Wait for the worker to finish (up to 5s)
        if let Some(handle) = self.handle.take() {
            let _ = tokio::time::timeout(std::time::Duration::from_secs(5), handle).await;
        }
    }

    /// Background worker loop: processes jobs in FIFO order.
    async fn run_loop(
        provider: Arc<dyn MemoryProvider>,
        metrics: Option<Arc<crate::runtime_metrics::RuntimeMetrics>>,
        mut rx: mpsc::Receiver<SyncJob>,
    ) {
        while let Some(job) = rx.recv().await {
            match job {
                SyncJob::SyncTurn {
                    bank,
                    user,
                    assistant,
                } => {
                    if let Err(e) = provider
                        .sync_turn_in_bank(bank.as_deref(), &user, &assistant)
                        .await
                    {
                        if let Some(m) = &metrics {
                            m.record_memory_sync_failure();
                        }
                        tracing::warn!(error = %e, "MemorySyncExecutor: sync_turn failed");
                    }
                }
                SyncJob::MemoryWrite {
                    action,
                    target,
                    content,
                } => {
                    provider.on_memory_write(&action, &target, &content);
                }
                SyncJob::Delegation { task, result } => {
                    provider.on_delegation(&task, &result);
                }
                SyncJob::SessionEnd { messages } => {
                    provider.on_session_end(&messages);
                }
                SyncJob::Shutdown => {
                    tracing::debug!("MemorySyncExecutor: shutdown received");
                    break;
                }
            }
        }
        tracing::debug!("MemorySyncExecutor: worker exited");
    }
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Lifecycle-matching the Python MemoryProvider ABC.
#[async_trait]
pub trait MemoryProvider: Send + Sync {
    /// Short identifier, e.g. `"builtin"`, `"memory_wire"`.
    fn name(&self) -> &str;

    /// True when credentials / deps are present (no network calls).
    fn is_available(&self) -> bool;

    /// False when writes land in a store that dies with the process (KB-14).
    fn is_persistent(&self) -> bool {
        true
    }

    /// Connect, create tables, warm up.  Called once at startup.
    async fn initialize(&self, session_id: &str) -> Result<()>;

    /// Probe whether the provider's backing service is currently reachable.
    /// Default: the cached availability flag (file-backed and in-process
    /// providers are always available). Providers with a live external service
    /// override with a real health probe that updates the cached flag.
    /// (iter-326 — lets the /mcp reconnect path report backend state.)
    async fn check_health(&self) -> bool {
        self.is_available()
    }

    /// Ensure the provider's backing service is reachable, spawning it when
    /// the provider supports managed auto-spawn. Returns true when the service
    /// is (or just became) ready. Default: always ready — file-backed and
    /// in-process providers have no external service. (iter-326 — lets the
    /// /mcp reconnect path warm an externally-hosted backend BEFORE connecting
    /// its MCP server, so the MCP initialize handshake completes in <1s instead
    /// of minutes.)
    async fn ensure_server(&self) -> bool {
        true
    }

    /// Static text for the system prompt (instructions / status line).
    /// NOTE: must be async — some providers fetch live context here, and
    /// build_messages runs inside the tokio runtime.
    async fn system_prompt_block(&self) -> String {
        String::new()
    }

    /// Recall relevant context before a turn. Returns formatted text or "".
    async fn prefetch(&self, query: &str) -> String {
        let _ = query;
        String::new()
    }

    /// Persist a completed turn asynchronously.
    async fn sync_turn(&self, user: &str, assistant: &str) -> Result<()> {
        let _ = (user, assistant);
        Ok(())
    }

    /// The bank this provider writes turns to right now. Read when a turn is
    /// queued, so a deferred sync keeps that bank (KB-7). `None` for providers
    /// without banks.
    fn write_bank(&self) -> Option<String> {
        None
    }

    /// Persist a completed turn into `bank`; `None` means the current bank.
    /// Default ignores the bank and calls [`Self::sync_turn`].
    async fn sync_turn_in_bank(
        &self,
        bank: Option<&str>,
        user: &str,
        assistant: &str,
    ) -> Result<()> {
        let _ = bank;
        self.sync_turn(user, assistant).await
    }

    /// Tool schemas this provider exposes to the model (OpenAI format).
    fn tool_schemas(&self) -> Vec<Value> {
        vec![]
    }

    /// Dispatch a tool call; return a JSON result string.
    async fn handle_tool_call(&self, name: &str, _args: Value) -> String {
        format!(
            r#"{{"error":"provider {} does not handle tool {}"}}"#,
            self.name(),
            name
        )
    }

    /// Flush queues and close connections.
    async fn shutdown(&self) {}

    /// (plan 06B) Retarget the provider at another scope's bank — the
    /// gateway calls this inside the turn lock so a seat's turn recalls
    /// and retains in `emp:<id>` and the owner's in the personal bank.
    /// Default no-op: providers without banks (and Holon runs, which
    /// never write operant memory — O-22) keep their construction scope.
    fn set_scope(&self, _scope: &crate::memory_wire::MemoryScope) {}

    // -- Optional lifecycle hooks (override to opt in) ----------------------

    /// Called at the start of each turn with the user message.
    /// Use for turn-counting, scope management, periodic maintenance.
    fn on_turn_start(&self, _turn_number: usize, _message: &str) {}

    /// Called when a session ends (explicit exit or timeout).
    /// Use for end-of-session fact extraction, summarization, etc.
    fn on_session_end(&self, _messages: &[crate::client::Message]) {}

    /// Called when the agent switches session_id mid-process.
    /// Fires on /resume, /branch, /reset, /new, and context compression.
    fn on_session_switch(&self, _new_session_id: &str, _parent_session_id: &str, _reset: bool) {}

    /// Called before context compression discards old messages.
    /// Use to extract insights from messages about to be compressed.
    /// Return text to include in the compression summary prompt.
    fn on_pre_compress(&self, _messages: &[crate::client::Message]) -> String {
        String::new()
    }

    /// Called when the built-in memory tool writes an entry.
    /// Use to mirror built-in memory writes to your backend.
    fn on_memory_write(&self, _action: &str, _target: &str, _content: &str) {}

    /// Called on the PARENT agent when a subagent completes.
    /// The parent's memory provider gets the task+result pair.
    fn on_delegation(&self, _task: &str, _result: &str) {}

    /// Queue a background recall for the NEXT turn.
    /// Called after each turn completes. The result will be consumed
    /// by prefetch() on the next turn.
    fn queue_prefetch(&self, _query: &str) {}

    /// Return extra on-disk paths this provider stores outside
    /// the operant home directory. Used by backup to include them.
    fn backup_paths(&self) -> Vec<std::path::PathBuf> {
        vec![]
    }
}

// ---------------------------------------------------------------------------
// Builtin — wraps existing MemoryManager (MEMORY.md / USER.md)

// (plan 11C3) BuiltinProvider is deleted — memory-wire is the only engine.

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Construct the memory provider from the config `provider` string.
///
/// The `"agentmemory"` provider name is the historical default. It is now
/// served by the in-process [`crate::memory_wire::MemoryWireProvider`] — the
/// agentmemory REST server, its `npx` auto-spawn, and its deferred MCP server
/// were removed, and `"memory_wire"` / `"memory-wire"` are accepted as explicit
/// aliases. All unrecognized provider names (including the removed
/// `"tdg"`, `"hindsight"`, `"retaindb"`, `"mem0"`, `"local-vector"`) fall
/// back to `BuiltinProvider` (file-backed MEMORY.md / USER.md) — the agent
/// stays functional with a degraded memory backend rather than dying on
/// startup.
pub fn build_memory_provider(
    provider_name: &str,
    storage_dir: std::path::PathBuf,
) -> Arc<dyn MemoryProvider> {
    // (plan 11C3) memory-wire is the ONLY provider. The name argument is
    // accepted so existing configs keep loading; anything other than the
    // memory-wire family warns once (the legacy `builtin` selection is the
    // important case — the user's own config may still say it).
    if !matches!(
        provider_name,
        "memory-wire" | "memory_wire" | "agentmemory" | "memory" | ""
    ) {
        eprintln!(
            "warning: [memory] provider = {provider_name:?} is ignored — memory-wire is the only memory engine now. Remove the key from operant.toml."
        );
    }
    match crate::memory_wire::MemoryWireProvider::new(storage_dir.clone()) {
        Ok(provider) => {
            // The one-time MEMORY.md/USER.md import rides construction: the
            // provider that replaces the file store migrates its data.
            // Failures are logged inside (never rename on a failed retain).
            let _ = provider.import_legacy_files(&storage_dir);
            Arc::new(provider)
        }
        Err(e) => {
            // No fallback anymore (BuiltinProvider is deleted): memory
            // degrades to unavailable for this session — the agent's
            // degrade-don't-die contract turns it into empty prefetches.
            tracing::error!(
                error = %e,
                "memory_wire provider init failed — memory disabled for this session"
            );
            Arc::new(NoopProvider)
        }
    }
}

/// (plan 11C3) The init-failure stand-in: every operation is a miss. This
/// keeps the `Arc<dyn MemoryProvider>` contract (the agent loop calls
/// prefetch/sync_turn unconditionally) without resurrecting a file store.
struct NoopProvider;

#[async_trait]
impl MemoryProvider for NoopProvider {
    fn name(&self) -> &str {
        "unavailable"
    }

    fn is_available(&self) -> bool {
        false
    }

    async fn initialize(&self, _session_id: &str) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (plan 11C3) the ONE-engine contract: every name — known aliases and
    /// unknown/retired values alike — builds the memory-wire provider (or
    /// the no-op stand-in on init failure). The builtin fallback is gone.
    #[test]
    fn test_build_unknown_lands_on_memory_wire() {
        let p = build_memory_provider("unknown", std::path::PathBuf::from("/tmp"));
        assert_eq!(p.name(), "memory_wire");
    }

    #[tokio::test]
    async fn test_trait_defaults_check_health_and_ensure_server() {
        // Providers without a managed external service must report
        // availability via the cached flag (check_health default) and be
        // always-ready for the /mcp reconnect warm-up (ensure_server
        // default). (iter-326 — native agent-memory lifecycle.)
        let p = build_memory_provider("memory-wire", std::path::PathBuf::from("/tmp"));
        assert!(
            p.check_health().await,
            "memory-wire check_health must report is_available()"
        );
        assert!(
            p.ensure_server().await,
            "memory-wire ensure_server must be always-ready"
        );
    }

    // The agentmemory name (and its memory_wire aliases) must route to the
    // in-process MemoryWireProvider, not to BuiltinProvider. Uses a real temp
    // dir because the provider opens a file-backed SQLite store on construct.
    #[test]
    fn test_agentmemory_and_aliases_build_the_memory_wire_provider() {
        let dir = tempfile::tempdir().expect("tempdir");
        for name in ["agentmemory", "memory_wire", "memory-wire"] {
            let p = build_memory_provider(name, dir.path().to_path_buf());
            assert_eq!(
                p.name(),
                "memory_wire",
                "provider name '{name}' should route to the memory_wire provider"
            );
        }
    }
}

#[cfg(test)]
mod bank_at_enqueue_tests {
    use super::*;
    use std::sync::Mutex;

    /// Records the bank each sync job arrives with.
    struct BankRecorder {
        current: Mutex<String>,
        written: Mutex<Vec<Option<String>>>,
    }

    #[async_trait]
    impl MemoryProvider for BankRecorder {
        fn name(&self) -> &str {
            "bank-recorder"
        }

        fn is_available(&self) -> bool {
            true
        }

        async fn initialize(&self, _session_id: &str) -> Result<()> {
            Ok(())
        }

        fn write_bank(&self) -> Option<String> {
            Some(self.current.lock().unwrap().clone())
        }

        async fn sync_turn_in_bank(
            &self,
            bank: Option<&str>,
            _user: &str,
            _assistant: &str,
        ) -> Result<()> {
            self.written.lock().unwrap().push(bank.map(str::to_string));
            Ok(())
        }
    }

    #[tokio::test]
    async fn queued_turn_keeps_the_bank_it_was_queued_under() {
        // KB-7 / GW-4: the bank is read at enqueue. A scope switch before the
        // worker drains must not move the turn into another seat's bank.
        let provider = Arc::new(BankRecorder {
            current: Mutex::new("personal".to_string()),
            written: Mutex::new(Vec::new()),
        });
        let exec = MemorySyncExecutor::new(provider.clone());
        exec.submit_sync_turn("hi", "hello", provider.write_bank());
        *provider.current.lock().unwrap() = "emp:alice".to_string();
        exec.shutdown().await;
        assert_eq!(
            *provider.written.lock().unwrap(),
            vec![Some("personal".to_string())]
        );
    }
}
