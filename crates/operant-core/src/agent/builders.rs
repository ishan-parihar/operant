//! `builders` — method-group impl block extracted verbatim from agent/mod.rs.

use self::background_review::SelfEvolutionState;
use self::iteration_budget::IterationBudget;
use crate::client::Role;
use crate::database::Database;
use crate::memory::MemoryManager;
use crate::observer::Observer;
use crate::skills::SkillManager;
use crate::tools::ToolRegistry;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use tracing::{debug, info, warn};

use super::*;

impl OperantAgent {
    /// True when `tool_name` is covered by the session or persistent allowlist
    /// (hermes `is_approved(session_key, pattern_key)` parity). Both sets are
    /// consulted so a session approval and a permanent approval behave
    /// identically at check time.
    pub(crate) fn tool_allowed_by_allowlist(&self, tool_name: &str) -> bool {
        let session = self
            .session_allowlist
            .read()
            .unwrap_or_else(|e| e.into_inner());
        let persistent = self
            .persistent_allowlist
            .read()
            .unwrap_or_else(|e| e.into_inner());
        let matched = session
            .iter()
            .chain(persistent.iter())
            .any(|pat| allowlist_pattern_matches(pat, tool_name));
        tracing::debug!(
            tool = %tool_name,
            session_count = session.len(),
            persistent_count = persistent.len(),
            persistent_items = ?persistent.iter().collect::<Vec<_>>(),
            matched,
            "allowlist check"
        );
        matched
    }

    /// Create a new Operant agent
    pub fn new(
        config: AgentConfig,
        client: Box<dyn ModelClient>,
        registry: ToolRegistry,
        database: Arc<Database>,
    ) -> Self {
        let max_iter = config.max_iterations;
        let nudge = config.skill_nudge_interval;
        let mem_interval = config.memory_review_interval;
        let persistent_allowlist = approval_allowlist_from_config(&config);
        Self {
            config,
            model_override: Arc::new(std::sync::RwLock::new(None::<String>)),
            client: Arc::from(client),
            registry,
            conversation: Arc::new(RwLock::new(Vec::new())),
            event_tx: None,
            permission_tx: None,
            session_allowlist: Arc::new(std::sync::RwLock::new(std::collections::HashSet::new())),
            persistent_allowlist: Arc::new(std::sync::RwLock::new(persistent_allowlist)),
            memory_manager: None,
            memory_provider: None,
            memory_sync_executor: Arc::new(std::sync::Mutex::new(None)),
            hook_registry: None,
            steer_queue: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            skill_manager: None,
            database,
            persistent_session_id: None,
            interrupt_flag: crate::interrupt::InterruptFlag::new(),
            thinking_timeout_hit: std::sync::atomic::AtomicBool::new(false),
            tool_guardrails: std::sync::Mutex::new(
                crate::tool_guardrails::ToolGuardrailTracker::new(),
            ),
            session_activity_last_stamp: std::sync::Mutex::new(std::collections::HashMap::new()),
            record_trajectories: false,
            session_cost_usd: Arc::new(std::sync::RwLock::new(0.0)),
            observer: None,
            evolution_state: std::sync::Mutex::new(SelfEvolutionState::new(
                &background_review::BackgroundReviewConfig {
                    skill_nudge_interval: nudge,
                    memory_review_interval: mem_interval,
                },
            )),
            iteration_budget: Arc::new(IterationBudget::new(max_iter)),
            metrics: Arc::new(crate::runtime_metrics::RuntimeMetrics::new()),
            llm_compressor: None,
            context_engine: None,
            credential_pool: None,
            active_credential_id: Arc::new(std::sync::RwLock::new(None)),
            rotation_cooldown_until: Arc::new(std::sync::RwLock::new(0.0)),
            rotation_count: Arc::new(std::sync::RwLock::new(0)),
            provider_registry: None,
            background_review_callback: None,
            last_prompt_tokens: std::sync::atomic::AtomicUsize::new(0),
            moa_guidance: Arc::new(std::sync::Mutex::new(None)),
            stream_emitted_tool_starts: std::sync::Mutex::new(std::collections::HashSet::new()),
        }
    }

    /// Create with event channel for streaming events
    pub fn with_events(
        config: AgentConfig,
        client: Box<dyn ModelClient>,
        registry: ToolRegistry,
        database: Arc<Database>,
        event_tx: mpsc::Sender<AgentEvent>,
    ) -> Self {
        let max_iter = config.max_iterations;
        let nudge = config.skill_nudge_interval;
        let mem_interval = config.memory_review_interval;
        let persistent_allowlist = approval_allowlist_from_config(&config);
        Self {
            config,
            model_override: Arc::new(std::sync::RwLock::new(None::<String>)),
            client: Arc::from(client),
            registry,
            conversation: Arc::new(RwLock::new(Vec::new())),
            event_tx: Some(event_tx),
            permission_tx: None,
            session_allowlist: Arc::new(std::sync::RwLock::new(std::collections::HashSet::new())),
            persistent_allowlist: Arc::new(std::sync::RwLock::new(persistent_allowlist)),
            memory_manager: None,
            memory_provider: None,
            memory_sync_executor: Arc::new(std::sync::Mutex::new(None)),
            hook_registry: None,
            steer_queue: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            skill_manager: None,
            database,
            persistent_session_id: None,
            interrupt_flag: crate::interrupt::InterruptFlag::new(),
            thinking_timeout_hit: std::sync::atomic::AtomicBool::new(false),
            tool_guardrails: std::sync::Mutex::new(
                crate::tool_guardrails::ToolGuardrailTracker::new(),
            ),
            session_activity_last_stamp: std::sync::Mutex::new(std::collections::HashMap::new()),
            record_trajectories: false,
            session_cost_usd: Arc::new(std::sync::RwLock::new(0.0)),
            observer: None,
            evolution_state: std::sync::Mutex::new(SelfEvolutionState::new(
                &background_review::BackgroundReviewConfig {
                    skill_nudge_interval: nudge,
                    memory_review_interval: mem_interval,
                },
            )),
            iteration_budget: Arc::new(IterationBudget::new(max_iter)),
            metrics: Arc::new(crate::runtime_metrics::RuntimeMetrics::new()),
            llm_compressor: None,
            context_engine: None,
            credential_pool: None,
            active_credential_id: Arc::new(std::sync::RwLock::new(None)),
            rotation_cooldown_until: Arc::new(std::sync::RwLock::new(0.0)),
            rotation_count: Arc::new(std::sync::RwLock::new(0)),
            provider_registry: None,
            background_review_callback: None,
            last_prompt_tokens: std::sync::atomic::AtomicUsize::new(0),
            moa_guidance: Arc::new(std::sync::Mutex::new(None)),
            stream_emitted_tool_starts: std::sync::Mutex::new(std::collections::HashSet::new()),
        }
    }

    /// Attach an observer for structured telemetry. When set, the agent
    /// emits ObserverEvent/ObserverMetric at key lifecycle points.
    pub fn with_observer(mut self, observer: Arc<dyn Observer>) -> Self {
        self.observer = Some(observer);
        self
    }

    /// Durable session activity heartbeat (R6 — hermes `session_activity.py`
    /// parity). Writes an observation-only activity stamp into
    /// `session_events` throttled to a ≥60s cadence per session, so the
    /// SessionDB write path is never hammered by a long agentic run.
    /// `force = true` bypasses the throttle (terminal stamps / shutdown).
    pub async fn touch_session_activity(&self, session_id: &str, description: &str) {
        const HEARTBEAT_MIN_INTERVAL_SECONDS: f64 = 60.0;
        let now_mono = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();

        let due = {
            let mut guard = self
                .session_activity_last_stamp
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let last = guard.get(session_id).copied().unwrap_or(0.0);
            let due = now_mono - last >= HEARTBEAT_MIN_INTERVAL_SECONDS;
            if due {
                guard.insert(session_id.to_string(), now_mono);
            }
            due
        };

        if due
            && let Ok(ts) =
                self.database
                    .record_session_activity(session_id, description, "agent.loop")
        {
            tracing::debug!(session_id, ts, "session activity heartbeat");
        }
    }

    /// Share an external runtime-metrics registry with the agent loop (and
    /// the memory sync executor it creates). The CLI/TUI passes the same
    /// `Arc` it renders a status pill from, so stream-drop retries and
    /// memory-sync failures become visible live. When unset, the agent
    /// keeps its own internal registry (metrics still increment, they just
    /// aren't observed externally).
    pub fn with_metrics(mut self, metrics: Arc<crate::runtime_metrics::RuntimeMetrics>) -> Self {
        self.metrics = metrics;
        self
    }

    /// The agent's shared runtime-metrics registry. The TUI reads this to
    /// render the retry/health pill; tests can assert counter increments.
    pub fn metrics(&self) -> Arc<crate::runtime_metrics::RuntimeMetrics> {
        Arc::clone(&self.metrics)
    }

    /// Attach a memory manager for long-term memory injection and session distillation.
    pub fn with_memory_manager(mut self, memory_manager: MemoryManager) -> Self {
        self.memory_manager = Some(memory_manager);
        self
    }

    #[expect(
        clippy::expect_used,
        reason = "poisoned lock: panic is the intended recovery"
    )]
    /// Attach a memory provider for long-term memory hooks. When set,
    /// the agent calls `sync_turn(user, assistant)` after each completed
    /// turn so the memory backend persists turn observations.
    /// This is the native equivalent of the hermes-agent Python adapter's
    /// memory hooks.
    pub fn with_memory_provider(
        mut self,
        memory_provider: Arc<dyn crate::memory_provider::MemoryProvider>,
    ) -> Self {
        // Create a background sync executor for this provider.
        // Single-worker FIFO ensures sync_turn, on_memory_write, and
        // other background writes happen in order without blocking the agent loop.
        *self
            .memory_sync_executor
            .lock()
            .expect("memory_sync_executor mutex poisoned — programmer error") = Some(
            crate::memory_provider::MemorySyncExecutor::new_with_metrics(
                memory_provider.clone(),
                Some(self.metrics.clone()),
            ),
        );
        self.memory_provider = Some(memory_provider);
        self
    }

    /// Attach a hook registry for lifecycle events. When set, the agent
    /// emits AgentStart/AgentEnd events at the beginning/end of each run().
    pub fn with_hook_registry(
        mut self,
        hook_registry: Arc<crate::gateway_pipeline::HookRegistry>,
    ) -> Self {
        self.hook_registry = Some(hook_registry);
        self
    }

    /// Attach a skill manager for available skills injection into the system prompt.
    pub fn with_skill_manager(mut self, skill_manager: SkillManager) -> Self {
        self.skill_manager = Some(skill_manager);
        self
    }

    /// Set the background review callback. When set, the agent calls this
    /// with a summary string after each background review completes,
    /// matching hermes-agent's `background_review_callback` pattern.
    pub fn with_background_review_callback(
        mut self,
        callback: Arc<dyn Fn(String) + Send + Sync>,
    ) -> Self {
        self.background_review_callback = Some(callback);
        self
    }

    pub fn with_permissions(mut self, permission_tx: mpsc::Sender<ToolPermissionRequest>) -> Self {
        self.permission_tx = Some(permission_tx);
        self
    }

    pub fn with_persistent_session(mut self, session_id: String) -> Self {
        self.persistent_session_id = Some(session_id);
        self
    }

    /// Inject an externally-managed `InterruptFlag` (e.g. wired to Ctrl-C by
    /// the TUI or CLI). When triggered, the agent loop exits gracefully at
    /// the next iteration boundary.
    pub fn with_interrupt_flag(mut self, flag: crate::interrupt::InterruptFlag) -> Self {
        self.interrupt_flag = flag;
        self
    }

    /// Get a clone of the agent's interrupt flag, so callers can trigger it
    /// (e.g. from a `tokio::signal::ctrl_c()` handler) without having kept
    /// their own copy.
    pub fn interrupt_flag(&self) -> crate::interrupt::InterruptFlag {
        self.interrupt_flag.clone()
    }

    /// Convenience check: has the interrupt flag been triggered?
    pub fn interrupt_triggered(&self) -> bool {
        self.interrupt_flag.is_triggered()
    }

    /// /steer directive (iter-65). Queue a steer message that will be
    /// injected into the conversation at the next iteration boundary.
    /// This allows real-time user guidance during a multi-iteration
    /// tool-calling loop — the model sees the steer without restarting
    /// the turn.
    ///
    /// The steer is injected as a user-role message appended to the
    /// conversation, so the model sees it as additional guidance from
    /// the user. Multiple steers can be queued; they're drained in order.
    pub async fn steer(&self, message: impl Into<String>) {
        let msg = message.into();
        debug!(steer = %msg, "Steer directive queued");
        self.steer_queue.lock().await.push(msg);
    }

    /// Get a clone of the steer queue handle so the TUI can push steers
    /// without holding a reference to the agent. The TUI stores this in an
    /// `Option<Arc<tokio::sync::Mutex<Vec<String>>>>` field and pushes to it
    /// when the user types while a turn is streaming. (iter-92 — closes the
    /// /steer parity gap.)
    /// Clone of the agent's live `ToolRegistry` handle.
    ///
    /// The registry shares its internal tool map via `Arc`, so a clone is a
    /// cheap handle: tools registered through it (e.g. `McpManager::
    /// sync_tools_to_registry` after a mid-session MCP reconnect) become
    /// visible to the agent on its next turn, since `get_schemas()` reads
    /// the shared map per iteration. (iter-93 reconnect parity — lets the
    /// TUI materialize deferred MCP tools without restarting operant.)
    pub fn registry(&self) -> ToolRegistry {
        self.registry.clone()
    }

    /// Return the attached long-term memory provider (if any). The TUI uses
    /// this to warm the agentmemory backend before a mid-session /mcp
    /// reconnect, so the MCP initialize handshake completes fast.
    /// (iter-326 — native agent-memory lifecycle management.)
    pub fn memory_provider(&self) -> Option<Arc<dyn crate::memory_provider::MemoryProvider>> {
        self.memory_provider.clone()
    }

    pub fn steer_queue_handle(&self) -> Arc<tokio::sync::Mutex<Vec<String>>> {
        Arc::clone(&self.steer_queue)
    }

    /// Inject per-turn MoA guidance (hermes `/moa` parity, G5). The
    /// guidance is drained by the next `build_messages` call, so it applies
    /// to exactly one `run()` turn. A `None`-returning
    /// `aggregate_moa_context` (no references configured) is a no-op.
    pub fn set_moa_guidance(&self, guidance: String) {
        if let Ok(mut slot) = self.moa_guidance.lock() {
            *slot = Some(guidance);
        }
    }

    /// Drain the pending MoA guidance for this turn (None when unset).
    pub(crate) fn drain_moa_guidance(&self) -> Option<String> {
        self.moa_guidance
            .lock()
            .ok()
            .and_then(|mut slot| slot.take())
    }

    /// List currently-active subagent tool calls. Returns a vec of
    /// (tool_call_id, status) pairs for every `delegate_task` / `spawn_subagent`
    /// tool call the agent has emitted in the current turn. Status is
    /// "running" for in-flight calls, "done" for completed. The TUI uses
    /// this to populate the /agents overlay and the subagent HUD pill.
    /// (iter-92 — closes the /agents parity gap.)
    ///
    /// This reads from the agent's conversation history, scanning for
    /// tool_calls (OpenAI format: assistant messages with `tool_calls` vec)
    /// whose function.name is "delegate_task" or "spawn_subagent", and
    /// matching them against tool-role messages (tool_call_id field) to
    /// determine status.
    pub async fn list_subagents(&self) -> Vec<(String, String)> {
        let conv = self.conversation.read().await;
        let mut result: Vec<(String, String)> = Vec::new();
        let mut completed_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

        // First pass: collect all tool_call_ids that have a matching tool-result message.
        for msg in conv.iter() {
            if msg.role == Role::Tool
                && let Some(ref id) = msg.tool_call_id
            {
                completed_ids.insert(id.clone());
            }
        }

        // Second pass: find assistant messages with tool_calls for subagent tools.
        for msg in conv.iter() {
            if msg.role == Role::Assistant
                && let Some(ref tool_calls) = msg.tool_calls
            {
                for tc in tool_calls {
                    let name = &tc.function.name;
                    if name == "delegate_task" || name == "spawn_subagent" {
                        let status = if completed_ids.contains(&tc.id) {
                            "done".to_string()
                        } else {
                            "running".to_string()
                        };
                        result.push((tc.id.clone(), status));
                    }
                }
            }
        }
        result
    }

    /// Drain pending steer directives. Returns the steers as a single
    /// concatenated string, or None if no steers are pending.
    pub(crate) async fn drain_steers(&self) -> Option<String> {
        let mut queue = self.steer_queue.lock().await;
        if queue.is_empty() {
            return None;
        }
        let steers: Vec<String> = queue.drain(..).collect();
        let combined = steers.join("\n");
        debug!(steer = %combined, "Draining steer directives");
        Some(combined)
    }

    /// Enable trajectory recording for this agent. When enabled, each `run()`
    /// call builds a `Trajectory` (ReAct steps + messages + metadata) and
    /// saves it to `~/.operant/trajectories/<session_id>.json`.
    pub fn with_trajectory_recording(mut self, enabled: bool) -> Self {
        self.record_trajectories = enabled;
        self
    }

    /// Attach an LLM-based context compressor for intelligent summarization.
    /// When set, context overflow errors trigger LLM summarization before
    /// falling back to deterministic decay/eviction.
    pub fn with_llm_compressor(mut self, config: llm_compressor::LlmCompressorConfig) -> Self {
        self.llm_compressor = Some(tokio::sync::Mutex::new(llm_compressor::LlmCompressor::new(
            config,
        )));
        self
    }

    /// Attach a pluggable context engine (hermes-lcm parity). When set,
    /// the engine assembles the per-call message list (lossless DAG + fresh
    /// tail) instead of the default lossy eviction.
    pub fn with_context_engine(
        mut self,
        engine: std::sync::Arc<dyn crate::context::ContextEngine>,
    ) -> Self {
        self.context_engine = Some(engine);
        self
    }

    /// Attach a credential pool for multi-key failover and rotation.
    /// When set, auth/rate-limit errors trigger automatic credential
    /// rotation via pool.invalidate() + pool.select(). Matches
    /// hermes-agent's `_credential_pool` pattern.
    pub fn with_credential_pool(
        mut self,
        pool: Arc<crate::credential_pool::CredentialPool>,
    ) -> Self {
        self.credential_pool = Some(pool);
        self
    }

    /// Attach a provider registry for cross-provider fallback.
    /// When set, auth/billing errors trigger provider switching.
    pub fn with_provider_registry(
        mut self,
        registry: Arc<provider_registry::ProviderRegistry>,
    ) -> Self {
        self.provider_registry = Some(registry);
        self
    }

    /// Try to rotate to the next available credential in the pool.
    ///
    /// Invalidates the current credential, selects the next one from the
    /// pool, and updates the client's API key via `set_api_key()`. Returns
    /// the new credential on success, or None if the pool is exhausted.
    pub fn try_rotate_credential(&self) -> Option<crate::credential_pool::PooledCredential> {
        let pool = self.credential_pool.as_ref()?;

        // Anti-thrash: check rotation cooldown to prevent burning through
        // the credential pool when multiple auth failures cascade.
        {
            let cooldown = self.rotation_cooldown_until.read().ok()?;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0);
            if now < *cooldown {
                warn!(
                    remaining_secs = (*cooldown - now) as u64,
                    "Rotation anti-thrash cooldown active — skipping rotation"
                );
                return None;
            }
        }

        // Single write lock for the entire rotation to avoid TOCTOU races.
        let mut active_id = self.active_credential_id.write().ok()?;

        // Invalidate the current credential
        if let Some(ref id) = *active_id {
            pool.invalidate(id, None, Some("rotated"), None, false);
        }

        // Select the next available credential
        let next = pool.select();
        if let Some(ref cred) = next {
            *active_id = Some(cred.id.clone());
            // Switch the client's API key to the new credential.
            self.client.set_api_key(&cred.value);
            // Arm rotation cooldown: 5s, 10s, 20s, capped at 60s.
            {
                let mut count = self.rotation_count.write().ok()?;
                *count += 1;
                let base = 5.0_f64;
                let delay = (base * 2.0_f64.powi(*count as i32 - 1)).min(60.0);
                let mut cooldown = self.rotation_cooldown_until.write().ok()?;
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs_f64())
                    .unwrap_or(0.0);
                *cooldown = now + delay;
                info!(
                    credential = %cred.name,
                    source = %cred.source,
                    rotation_count = *count,
                    cooldown_secs = delay,
                    "Credential rotated — client API key updated"
                );
            }
        } else {
            warn!("No available credentials in pool for rotation");
        }

        next
    }
}
