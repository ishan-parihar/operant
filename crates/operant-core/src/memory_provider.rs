//! Memory provider system for Operant-RS.
//!
//! The active provider is selected by `config.memory.provider`:
//!
//! | Value         | Backend                                                           |
//! |---------------|-------------------------------------------------------------------|
//! | `"agentmemory"` | Hybrid semantic memory via the agentmemory server (REST + MCP)  |
//! | `"builtin"`   | File-backed MEMORY.md / USER.md (zero-dependency fallback)        |
//! | other         | Silently falls back to `"builtin"`                                |
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
}

enum SyncJob {
    SyncTurn {
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
        let (tx, rx) = mpsc::channel(256);
        let handle = tokio::spawn(Self::run_loop(provider, rx));
        Self {
            tx,
            handle: Some(handle),
        }
    }

    /// Submit a sync_turn job (non-blocking).
    pub fn submit_sync_turn(&self, user: &str, assistant: &str) {
        if self
            .tx
            .try_send(SyncJob::SyncTurn {
                user: user.to_string(),
                assistant: assistant.to_string(),
            })
            .is_err()
        {
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
    async fn run_loop(provider: Arc<dyn MemoryProvider>, mut rx: mpsc::Receiver<SyncJob>) {
        while let Some(job) = rx.recv().await {
            match job {
                SyncJob::SyncTurn { user, assistant } => {
                    if let Err(e) = provider.sync_turn(&user, &assistant).await {
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
    /// Short identifier, e.g. `"builtin"`, `"agentmemory"`.
    fn name(&self) -> &str;

    /// True when credentials / deps are present (no network calls).
    fn is_available(&self) -> bool;

    /// Connect, create tables, warm up.  Called once at startup.
    async fn initialize(&self, session_id: &str) -> Result<()>;

    /// Probe whether the provider's backing service is currently reachable.
    /// Default: the cached availability flag (file-backed providers are
    /// always available). Providers with a live service (agentmemory)
    /// override with a real health probe that updates the cached flag.
    /// (iter-326 — lets the /mcp reconnect path report backend state.)
    async fn check_health(&self) -> bool {
        self.is_available()
    }

    /// Ensure the provider's backing service is reachable, spawning it when
    /// the provider supports managed auto-spawn (e.g. agentmemory's REST
    /// server). Returns true when the service is (or just became) ready.
    /// Default: always ready — file-backed providers have no external
    /// service. (iter-326 — lets the /mcp reconnect path warm the
    /// agentmemory backend BEFORE connecting its MCP server, so the MCP
    /// initialize handshake completes in <1s instead of minutes.)
    async fn ensure_server(&self) -> bool {
        true
    }

    /// Static text for the system prompt (instructions / status line).
    fn system_prompt_block(&self) -> String {
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
// ---------------------------------------------------------------------------

pub struct BuiltinProvider {
    manager: crate::memory::MemoryManager,
}

impl BuiltinProvider {
    pub fn new(manager: crate::memory::MemoryManager) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl MemoryProvider for BuiltinProvider {
    fn name(&self) -> &str {
        "builtin"
    }
    fn is_available(&self) -> bool {
        true
    }

    async fn initialize(&self, session_id: &str) -> Result<()> {
        self.manager.load_from_disk().await?;
        self.manager.start_session(session_id).await;
        Ok(())
    }

    fn system_prompt_block(&self) -> String {
        "Built-in file memory active (MEMORY.md / USER.md).".to_string()
    }

    async fn prefetch(&self, _query: &str) -> String {
        self.manager.build_memory_context(2000).await
    }

    async fn sync_turn(&self, user: &str, _assistant: &str) -> Result<()> {
        self.manager
            .add_message(crate::client::Message::user(user))
            .await;
        Ok(())
    }

    fn on_turn_start(&self, turn_number: usize, _message: &str) {
        tracing::debug!(turn_number, "BuiltinProvider: turn started");
    }

    fn on_session_end(&self, _messages: &[crate::client::Message]) {
        tracing::debug!("BuiltinProvider: session ended — no extraction needed");
    }

    fn on_session_switch(&self, _new_session_id: &str, _parent_session_id: &str, _reset: bool) {
        tracing::debug!("BuiltinProvider: session switched");
    }

    fn on_memory_write(&self, action: &str, target: &str, content: &str) {
        tracing::debug!(
            action,
            target,
            content_len = content.len(),
            "BuiltinProvider: memory write mirrored"
        );
    }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Construct the memory provider from the config `provider` string.
///
/// The `"agentmemory"` provider (hybrid semantic memory via the agentmemory
/// server) is the default and is constructed when the `agentmemory` feature
/// is compiled in. All unrecognized provider names (including the removed
/// `"tdg"`, `"hindsight"`, `"retaindb"`, `"mem0"`, `"local-vector"`) fall
/// back to `BuiltinProvider` (file-backed MEMORY.md / USER.md) — the agent
/// stays functional with a degraded memory backend rather than dying on
/// startup.
pub fn build_memory_provider(
    provider_name: &str,
    storage_dir: std::path::PathBuf,
) -> Arc<dyn MemoryProvider> {
    match provider_name {
        // "agentmemory" → AgentMemoryProvider (REST client + auto-spawn).
        // If the server can't be reached at startup it degrades gracefully
        // at call time (see agent_memory.rs) rather than panicking.
        #[cfg(feature = "agentmemory")]
        "agentmemory" => match crate::agent_memory::AgentMemoryProvider::new(storage_dir.clone()) {
            Ok(provider) => Arc::new(provider),
            Err(e) => {
                tracing::error!(
                    error = %e,
                    "agentmemory provider init failed; falling back to BuiltinProvider"
                );
                Arc::new(BuiltinProvider::new(
                    crate::memory::MemoryManager::with_storage_dir(storage_dir),
                ))
            }
        },
        // "builtin", "disabled", and any other value → BuiltinProvider.
        // Old provider names (tdg/hindsight/retaindb/mem0/local-vector) also
        // land here — they're treated as unknown and silently downgraded to
        // BuiltinProvider with no error, since the user's config may still
        // reference them after upgrading.
        _ => Arc::new(BuiltinProvider::new(
            crate::memory::MemoryManager::with_storage_dir(storage_dir),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_builtin() {
        let p = build_memory_provider("builtin", std::path::PathBuf::from("/tmp"));
        assert_eq!(p.name(), "builtin");
    }

    #[test]
    fn test_build_unknown_falls_back_to_builtin() {
        let p = build_memory_provider("unknown", std::path::PathBuf::from("/tmp"));
        assert_eq!(p.name(), "builtin");
    }

    // Old provider names that were removed should silently fall back to
    // builtin, not error or panic.
    #[test]
    fn test_build_removed_providers_fall_back_to_builtin() {
        for old in &[
            "tdg",
            "hindsight",
            "retaindb",
            "mem0",
            "local-vector",
            "local_vector",
        ] {
            let p = build_memory_provider(old, std::path::PathBuf::from("/tmp"));
            assert_eq!(
                p.name(),
                "builtin",
                "old provider '{}' should fall back to builtin",
                old
            );
        }
    }

    #[test]
    fn test_build_disabled_falls_back_to_builtin() {
        let p = build_memory_provider("disabled", std::path::PathBuf::from("/tmp"));
        assert_eq!(p.name(), "builtin");
    }

    #[tokio::test]
    async fn test_trait_defaults_check_health_and_ensure_server() {
        // Providers without a managed external service must report
        // availability via the cached flag (check_health default) and be
        // always-ready for the /mcp reconnect warm-up (ensure_server
        // default). (iter-326 — native agent-memory lifecycle.)
        let p = build_memory_provider("builtin", std::path::PathBuf::from("/tmp"));
        assert!(
            p.check_health().await,
            "builtin default check_health must report is_available()"
        );
        assert!(
            p.ensure_server().await,
            "builtin default ensure_server must be always-ready"
        );
    }
}
