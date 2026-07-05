//! Operant Agent orchestration loop with self-healing
//!
//! Implements the ReAct (Reason + Act) pattern for LLM-driven tool execution.

use std::sync::Arc;
use std::time::Duration;

use futures::stream::BoxStream;
use futures::StreamExt;
use tokio::sync::{mpsc, RwLock};
use tokio::time::timeout;
use tracing::{debug, error, info, instrument, warn};

use crate::client::{ChatResponse, Message, Role, ToolCall};
use crate::config::{runtime_config, BehaviorSettings};
use crate::context_files::{load_default_context_files, load_workspace_context};
use crate::database::Database;
use crate::distillation::distill_session_to_memory;
use crate::error::{Error, Result};
use crate::memory::MemoryManager;
use crate::parser::{ToolCallParser, ToolCallStreamParser};
use crate::skills::SkillManager;
use crate::tools::{ToolContext, ToolRegistry, ToolResult};

/// Response from the user for tool permission requests
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolPermissionResponse {
    /// Allow this tool call once
    AllowOnce,
    /// Allow this tool call and all subsequent calls to this tool in the session
    AllowSession,
    /// Deny this tool call
    Deny,
}

/// Configuration for the Operant agent
#[derive(Debug, Clone)]
pub struct AgentConfig {
    /// Model to use (e.g., "gpt-4", "gpt-3.5-turbo")
    pub model: String,
    /// Maximum iterations before giving up
    pub max_iterations: usize,
    /// Timeout for tool execution
    pub tool_timeout: Duration,
    /// Timeout for LLM requests
    pub request_timeout: Duration,
    /// System prompt for the agent
    pub system_prompt: Option<String>,
    /// Whether to stream responses
    pub stream: bool,
    /// Context window size for truncation
    pub context_window: usize,
    /// Max self-healing attempts on tool errors
    pub max_healing_attempts: usize,
    /// Ordered list of fallback models for automatic failover on retryable errors.
    pub fallback_models: Vec<String>,
    /// Whether automatic fallback to fallback_models is enabled.
    pub fallback_on_errors: bool,
    /// Approval mode for tool execution: "smart" (default, pattern-based),
    /// "manual" (prompt for every tool), or "off" (no checks).
    pub approval_mode: String,
    /// Whether to record trajectories (ReAct steps + messages) for each run.
    /// Saved to ~/.operant/trajectories/<session_id>.json.
    pub record_trajectories: bool,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self::from(&runtime_config().agent)
    }
}

impl From<&BehaviorSettings> for AgentConfig {
    fn from(settings: &BehaviorSettings) -> Self {
        Self {
            model: settings.model.clone(),
            max_iterations: settings.max_iterations,
            tool_timeout: Duration::from_secs(settings.tool_timeout_secs),
            request_timeout: Duration::from_secs(settings.request_timeout_secs),
            system_prompt: settings.system_prompt.clone(),
            stream: settings.stream,
            context_window: settings.context_window,
            max_healing_attempts: settings.max_healing_attempts,
            fallback_models: settings.fallback_models.clone(),
            fallback_on_errors: settings.fallback_on_errors,
            approval_mode: "smart".to_string(),
            record_trajectories: false,
        }
    }
}

/// Events emitted by the agent
#[derive(Debug, Clone)]
pub enum AgentEvent {
    /// Thinking/reasoning step
    Thinking { content: String },
    /// Model reasoning content
    Reasoning { text: String },
    /// Tool execution started
    ToolStart { tool_call_id: String, name: String, arguments: String },
    /// Tool execution completed
    ToolComplete { result: ToolResult },
    /// Tool execution failed
    ToolError { tool_call_id: String, name: String, error: String },
    /// Response content received
    Content { text: String },
    /// Agent finished with final response
    Done { message: Message },
    /// Agent iteration completed
    IterationComplete { iteration: usize },
    /// Agent error
    Error { error: String },
    /// API usage statistics from the last completed request
    Usage {
        input_tokens: u32,
        output_tokens: u32,
        total_tokens: u32,
    },
    /// Tool requires permission before execution
    ToolPermissionRequest {
        tool_name: String,
        tool_id: String,
        description: String,
        danger_explanation: String,
        input_preview: Option<String>,
    },
}

/// Operant Agent for tool orchestration
pub struct OperantAgent {
    config: AgentConfig,
    client: Arc<dyn ModelClient>,
    registry: ToolRegistry,
    conversation: Arc<RwLock<Vec<Message>>>,
    event_tx: Option<mpsc::Sender<AgentEvent>>,
    permission_tx: Option<mpsc::Sender<ToolPermissionRequest>>,
    memory_manager: Option<MemoryManager>,
    skill_manager: Option<SkillManager>,
    database: Arc<Database>,
    /// TDG memory provider for graph memory hooks. When set, the agent
    /// calls `sync_turn(user, assistant)` after each completed turn so
    /// the graph self-organizes (entity extraction + auto-wiring).
    /// This is the native equivalent of the hermes-agent Python adapter's
    /// TDG hooks — no manual tdg_create/tdg_connect needed.
    memory_provider: Option<Arc<dyn crate::memory_provider::MemoryProvider>>,
    /// Hook registry for lifecycle events (AgentStart, AgentEnd, etc.).
    /// When set, the agent emits events at key lifecycle points.
    hook_registry: Option<Arc<crate::gateway_pipeline::HookRegistry>>,
    /// /steer directive queue (iter-65). When the user sends a steer
    /// message during a multi-iteration tool-calling loop, it's queued
    /// here. The run() loop drains pending steers between iterations
    /// and injects them into the conversation so the model sees the
    /// user's real-time guidance without restarting the turn.
    steer_queue: Arc<tokio::sync::Mutex<Vec<String>>>,
    /// Stable session ID for DB persistence across multiple `run()` calls.
    /// When set, all messages are persisted under this ID instead of generating
    /// a fresh one each call.  Set by the TUI at startup.
    persistent_session_id: Option<String>,
    /// Shared interrupt flag for graceful Ctrl-C cancellation.
    /// When triggered, the agent loop exits at the next iteration boundary
    /// and tool execution is aborted via `flag.check()`.
    interrupt_flag: crate::interrupt::InterruptFlag,
    /// Whether to record trajectories (ReAct steps + messages) for each run.
    /// When true, a trajectory JSON is saved to ~/.operant/trajectories/
    /// on run() completion. Set via AgentConfig::record_trajectories.
    record_trajectories: bool,
}

/// A pending permission request sent from the agent to the TUI
#[derive(Debug)]
pub struct ToolPermissionRequest {
    pub tool_name: String,
    pub tool_id: String,
    pub description: String,
    pub danger_explanation: String,
    pub input_preview: Option<String>,
    pub response_tx: tokio::sync::oneshot::Sender<ToolPermissionResponse>,
}

impl OperantAgent {
    /// Create a new Operant agent
    pub fn new(
        config: AgentConfig,
        client: Box<dyn ModelClient>,
        registry: ToolRegistry,
        database: Arc<Database>,
    ) -> Self {
        Self {
            config,
            client: Arc::from(client),
            registry,
            conversation: Arc::new(RwLock::new(Vec::new())),
            event_tx: None,
            permission_tx: None,
            memory_manager: None,
            memory_provider: None,
            hook_registry: None,
            steer_queue: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            skill_manager: None,
            database,
            persistent_session_id: None,
            interrupt_flag: crate::interrupt::InterruptFlag::new(),
            record_trajectories: false,
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
        Self {
            config,
            client: Arc::from(client),
            registry,
            conversation: Arc::new(RwLock::new(Vec::new())),
            event_tx: Some(event_tx),
            permission_tx: None,
            memory_manager: None,
            memory_provider: None,
            hook_registry: None,
            steer_queue: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            skill_manager: None,
            database,
            persistent_session_id: None,
            interrupt_flag: crate::interrupt::InterruptFlag::new(),
            record_trajectories: false,
        }
    }

    /// Attach a memory manager for long-term memory injection and session distillation.
    pub fn with_memory_manager(mut self, memory_manager: MemoryManager) -> Self {
        self.memory_manager = Some(memory_manager);
        self
    }

    /// Attach a TDG memory provider for graph memory hooks. When set,
    /// the agent calls `sync_turn(user, assistant)` after each completed
    /// turn so the graph self-organizes (entity extraction + auto-wiring).
    /// This is the native equivalent of the hermes-agent Python adapter's
    /// TDG hooks.
    pub fn with_memory_provider(
        mut self,
        memory_provider: Arc<dyn crate::memory_provider::MemoryProvider>,
    ) -> Self {
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
    pub fn steer_queue_handle(&self) -> Arc<tokio::sync::Mutex<Vec<String>>> {
        Arc::clone(&self.steer_queue)
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
            if msg.role == Role::Tool {
                if let Some(ref id) = msg.tool_call_id {
                    completed_ids.insert(id.clone());
                }
            }
        }

        // Second pass: find assistant messages with tool_calls for subagent tools.
        for msg in conv.iter() {
            if msg.role == Role::Assistant {
                if let Some(ref tool_calls) = msg.tool_calls {
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
        }
        result
    }

    /// Drain pending steer directives. Returns the steers as a single
    /// concatenated string, or None if no steers are pending.
    async fn drain_steers(&self) -> Option<String> {
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

    /// Send an event to the channel
    async fn emit(&self, event: AgentEvent) {
        if let Some(ref tx) = self.event_tx {
            let _ = tx.send(event).await;
        }
    }

    /// Add a message to the conversation history
    pub async fn add_message(&self, message: Message) {
        let mut conv = self.conversation.write().await;
        conv.push(message);
    }

    /// Add a user message
    pub async fn user_message(&self, content: impl Into<String>) {
        self.add_message(Message::user(content)).await;
    }

    /// Get current conversation
    pub async fn conversation(&self) -> Vec<Message> {
        self.conversation.read().await.clone()
    }

    /// Clear conversation history
    pub async fn clear_history(&self) {
        let mut conv = self.conversation.write().await;
        conv.clear();
    }

    /// Get a reference to the database
    pub fn db(&self) -> &Database {
        &self.database
    }

    /// Run the agent with a user query
    #[instrument(skip(self), fields(model = % self.config.model))]
    pub async fn run(&self, user_query: String) -> Result<Message> {
        info!("Starting agent run");

        // Emit AgentStart hook
        if let Some(ref hooks) = self.hook_registry {
            let ctx = crate::gateway_pipeline::HookContext::new()
                .with_session(self.persistent_session_id.as_deref().unwrap_or(""));
            hooks.emit(
                crate::gateway_pipeline::HookEvent::AgentStart,
                ctx,
            ).await;
        }

        // Reset interrupt flag from any previous run() call. Without this,
        // a Ctrl-C in run #1 permanently breaks run #2+ (the flag stays
        // triggered and the loop exits immediately).
        self.interrupt_flag.reset();

        let session_id = self
            .persistent_session_id
            .clone()
            .unwrap_or_else(|| format!("sess_{}", uuid::Uuid::new_v4()));

        // Add user message — but skip if the last message is already this
        // exact query (happens when run_with_healing retries run() — without
        // this check, N retries produce N duplicate user messages).
        {
            let conv = self.conversation.read().await;
            let already_added = conv
                .last()
                .is_some_and(|last| last.role == Role::User && last.content == user_query);
            if !already_added {
                drop(conv);
                self.add_message(Message::user(&user_query)).await;
            }
        }

        // Save session first (must exist before messages can reference it)
        self.database
            .save_session(
                &session_id,
                None,
                "agent",
                &chrono::Utc::now().to_rfc3339(),
                &chrono::Utc::now().to_rfc3339(),
            )
            .map_err(|e| {
                warn!(error = %e, "Failed to save session metadata");
                e
            })?;

        // Persist user message
        self.database
            .save_message(
                &session_id,
                "user",
                &user_query,
                &chrono::Utc::now().to_rfc3339(),
            )
            .map_err(|e| {
                warn!(error = %e, "Failed to persist user message");
                e
            })?;

        // Build initial messages including system prompt
        let mut messages = self.build_messages().await?;
        let mut iteration = 0;
        let mut total_tool_calls: usize = 0;

        loop {
            iteration += 1;
            debug!(iteration, "Agent iteration");

            // ── Graceful interrupt check (Ctrl-C) ──
            // If the interrupt flag has been triggered (e.g. by a Ctrl-C
            // signal handler in the TUI/CLI), exit the loop cleanly instead
            // of starting another LLM round-trip + tool execution cycle.
            if self.interrupt_flag.is_triggered() {
                info!("Agent loop interrupted by user (Ctrl-C)");
                if self.record_trajectories {
                    self.save_trajectory(
                        &session_id,
                        &messages,
                        iteration,
                        total_tool_calls,
                        false,
                        None,
                    )
                    .await;
                }
                self.emit(AgentEvent::Error {
                    error: "Interrupted by user".to_string(),
                })
                .await;
                return Err(Error::Agent("Interrupted by user".to_string()));
            }

            if iteration > self.config.max_iterations {
                // ── Grace call (iter-57) ────────────────────────────────
                // When max_iterations is exceeded, hermes-agent makes one
                // extra "grace call" with tools stripped, asking the model
                // to summarize what it has so far. This gives the user a
                // partial answer instead of a hard error.
                warn!(max = self.config.max_iterations, "Max iterations exceeded — attempting grace call");

                // Build a grace-call request: same messages but no tools
                let grace_request = ChatRequest::new(&self.config.model, messages.clone())
                    .with_stream(self.config.stream);

                let grace_result = if self.config.stream {
                    let stream = self.client.chat_streaming(grace_request).await?;
                    let (text, reasoning, _tcs, _extra) = self.process_stream(stream).await?;
                    Ok((text, reasoning))
                } else {
                    let response = self.client.chat(grace_request).await?;
                    self.process_response(response).await.map(|(t, r, _)| (t, r))
                };

                match grace_result {
                    Ok((text, _reasoning)) => {
                        let result = Message::assistant(&text);
                        if self.record_trajectories {
                            self.save_trajectory(
                                &session_id, &messages, iteration, total_tool_calls,
                                false, Some(&result),
                            ).await;
                        }
                        self.emit(AgentEvent::Done { message: result.clone() }).await;
                        // Emit AgentEnd hook
                        if let Some(ref hooks) = self.hook_registry {
                            hooks.emit(crate::gateway_pipeline::HookEvent::AgentEnd,
                                crate::gateway_pipeline::HookContext::new()
                                    .with_session(&session_id)).await;
                        }
                        return Ok(result);
                    }
                    Err(e) => {
                        warn!(error = %e, "Grace call failed — returning hard error");
                    }
                }

                if self.record_trajectories {
                    self.save_trajectory(
                        &session_id, &messages, iteration, total_tool_calls,
                        false, None,
                    ).await;
                }
                return Err(Error::MaxIterationsExceeded {
                    max: self.config.max_iterations,
                });
            }

            // Emit thinking event
            self.emit(AgentEvent::Thinking {
                content: format!(
                    "Iteration {}/{}: Requesting LLM response...",
                    iteration, self.config.max_iterations
                ),
            })
            .await;

            // Get tool schemas
            let tools = self.registry.get_schemas().await;

            let request = ChatRequest::new(&self.config.model, messages.clone())
                .with_tools(tools)
                .with_stream(self.config.stream);

            let mut stream_extra_content = None;
            let response = if request.stream {
                let stream = match self.client.chat_streaming(request).await {
                    Ok(s) => s,
                    Err(e) => {
                        // ── Context overflow auto-compression (iter-63) ───────
                        // When the provider returns a context_overflow error,
                        // compress the conversation using context_management
                        // and retry once. This prevents hard failures on long
                        // sessions that exceed the context window.
                        let classified = FallbackModelClient::classify_error(&e);
                        if classified.should_compress {
                            warn!(reason = %classified.reason, "Context overflow detected — compressing and retrying");
                            let budget = self.config.context_window;
                            messages = crate::context_management::manage_context(
                                messages, budget, 4096,
                            );
                            // Rebuild request with compressed messages
                            let tools = self.registry.get_schemas().await;
                            let retry_request = ChatRequest::new(&self.config.model, messages.clone())
                                .with_tools(tools)
                                .with_stream(self.config.stream);
                            self.client.chat_streaming(retry_request).await?
                        } else {
                            return Err(e);
                        }
                    }
                };
                let (text, reasoning, tcs, extra) = self.process_stream(stream).await?;
                stream_extra_content = extra;
                Ok((text, reasoning, tcs))
            } else {
                let response = match self.client.chat(request).await {
                    Ok(r) => r,
                    Err(e) => {
                        let classified = FallbackModelClient::classify_error(&e);
                        if classified.should_compress {
                            warn!(reason = %classified.reason, "Context overflow detected — compressing and retrying");
                            let budget = self.config.context_window;
                            messages = crate::context_management::manage_context(
                                messages, budget, 4096,
                            );
                            let tools = self.registry.get_schemas().await;
                            let retry_request = ChatRequest::new(&self.config.model, messages.clone())
                                .with_tools(tools)
                                .with_stream(self.config.stream);
                            self.client.chat(retry_request).await?
                        } else {
                            return Err(e);
                        }
                    }
                };
                self.process_response(response).await
            };

            match response {
                Ok((response_text, reasoning_text, tool_calls)) => {
                    // Add assistant message to conversation
                    // When tool calls are present, any text before them is typically
                    // model thinking/planning that shouldn't be shown to the user.
                    let effective_text = if !tool_calls.is_empty() {
                        String::new()
                    } else {
                        response_text.clone()
                    };
                    let mut assistant_msg = Message::assistant(&effective_text);
                    if !reasoning_text.is_empty() {
                        assistant_msg = assistant_msg.with_reasoning(reasoning_text);
                    }
                    if !tool_calls.is_empty() {
                        assistant_msg = assistant_msg.with_tool_calls(tool_calls.clone());
                    }
                    // Attach provider-specific extra content (e.g. Gemini thought_signature)
                    if let Some(ref extra) = stream_extra_content {
                        if !extra.is_null() {
                            assistant_msg = assistant_msg.with_extra_content(extra.clone());
                        }
                    }

                    messages.push(assistant_msg.clone());
                    self.add_message(assistant_msg.clone()).await;

                    // Persist assistant message — use save_message_full when
                    // the message has tool_calls so they're not lost on reload.
                    // Previously save_message (4-arg) dropped tool_calls, which
                    // meant reloaded sessions lost the assistant's tool-call
                    // context (the tool results became orphaned).
                    let timestamp = chrono::Utc::now().to_rfc3339();
                    if assistant_msg.tool_calls.is_some() {
                        let tool_calls_json = assistant_msg
                            .tool_calls
                            .as_ref()
                            .and_then(|tcs| serde_json::to_string(tcs).ok());
                        let msg_data = crate::database::MessageData {
                            id: 0,
                            session_id: session_id.clone(),
                            role: "assistant".to_string(),
                            content: Some(effective_text.clone()),
                            tool_call_id: None,
                            tool_calls: tool_calls_json,
                            tool_name: None,
                            timestamp,
                            token_count: None,
                            finish_reason: Some("tool_calls".to_string()),
                            reasoning: assistant_msg.reasoning.clone(),
                            reasoning_content: None,
                            reasoning_details: None,
                            codex_reasoning_items: None,
                            codex_message_items: None,
                            platform_message_id: None,
                            observed: None,
                            active: 1,
                        };
                        let _ = self.database.save_message_full(&msg_data);
                    } else {
                        let _ = self.database.save_message(
                            &session_id,
                            "assistant",
                            &effective_text,
                            &timestamp,
                        );
                    }
                    self.database
                        .save_session(
                            &session_id,
                            None,
                            "agent",
                            &chrono::Utc::now().to_rfc3339(),
                            &chrono::Utc::now().to_rfc3339(),
                        )
                        .ok();

                    // If no tool calls, we're done
                    if tool_calls.is_empty() {
                        let result = assistant_msg.clone();
                        self.spawn_session_distillation(messages.clone());

                        // Save trajectory if recording is enabled.
                        if self.record_trajectories {
                            self.save_trajectory(
                                &session_id,
                                &messages,
                                iteration,
                                total_tool_calls,
                                true,
                                Some(&result),
                            )
                            .await;
                        }

                        self.emit(AgentEvent::Done {
                            message: assistant_msg,
                        })
                        .await;

                        // TDG hook: sync this turn to graph memory. The
                        // provider extracts entities and auto-wires edges,
                        // so the graph self-organizes without the agent
                        // needing to call tdg_create/tdg_connect manually.
                        // This is the native equivalent of the hermes-agent
                        // Python adapter's post-turn TDG hook. Failures are
                        // logged but don't break the turn — the agent's
                        // response is already complete.
                        if let Some(provider) = &self.memory_provider {
                            let user_text = user_query.clone();
                            let assistant_text = result.content.clone();
                            let provider = provider.clone();
                            tokio::spawn(async move {
                                if let Err(e) = provider.sync_turn(&user_text, &assistant_text).await {
                                    tracing::warn!(error = %e, "TDG sync_turn hook failed");
                                }
                            });
                        }

                        // Emit AgentEnd hook
                        if let Some(ref hooks) = self.hook_registry {
                            hooks.emit(crate::gateway_pipeline::HookEvent::AgentEnd,
                                crate::gateway_pipeline::HookContext::new()
                                    .with_session(&session_id)).await;
                        }

                        return Ok(result);
                    }

                    total_tool_calls += tool_calls.len();

                    // Execute tools and add results
                    let tool_results = self.execute_tools(tool_calls).await?;

                    for result in &tool_results {
                        // Persist tool result
                        let _ = self.database.save_message(
                            &session_id,
                            "tool",
                            &result.content,
                            &chrono::Utc::now().to_rfc3339(),
                        );
                        self.database
                            .save_session(
                                &session_id,
                                None,
                                "agent",
                                &chrono::Utc::now().to_rfc3339(),
                                &chrono::Utc::now().to_rfc3339(),
                            )
                            .ok();
                        if result.success {
                            self.emit(AgentEvent::ToolComplete {
                                result: result.clone(),
                            })
                            .await;
                        } else {
                            self.emit(AgentEvent::ToolError {
                                tool_call_id: result.tool_call_id.clone(),
                                name: result.name.clone(),
                                error: result.error.clone().unwrap_or_default(),
                            })
                            .await;
                        }
                    }

                    // Add tool results to messages
                    for result in tool_results {
                        // Truncate large tool results (e.g. TTS base64 audio) to prevent
                        // context overflow. Keep a summary instead.
                        let content = if result.success {
                            truncate_tool_result(&result.name, &result.content)
                        } else {
                            result.error.as_deref().unwrap_or("Error").to_string()
                        };
                        messages.push(Message::tool(&result.tool_call_id, &content));
                        self.add_message(Message::tool(&result.tool_call_id, &content))
                            .await;
                    }
                }
                Err(e) => {
                    error!(error = %e, "Error processing stream");
                    self.emit(AgentEvent::Error {
                        error: e.to_string(),
                    })
                    .await;
                    if self.record_trajectories {
                        self.save_trajectory(
                            &session_id,
                            &messages,
                            iteration,
                            total_tool_calls,
                            false,
                            None,
                        )
                        .await;
                    }
                    return Err(e);
                }
            }

            self.emit(AgentEvent::IterationComplete { iteration }).await;

            // ── /steer directive drain (iter-65) ──────────────────────────
            // Between iterations, check if the user queued any steer
            // directives. If so, inject them as a user-role message so
            // the model sees the real-time guidance on the next iteration.
            // This mirrors hermes-agent's /steer drain which injects into
            // the last tool-role message to preserve role alternation.
            if let Some(steer_text) = self.drain_steers().await {
                info!(steer = %steer_text, "Injecting steer directive");
                let steer_msg = Message::user(&format!(
                    "[STEER] {}\n\nPlease adjust your approach based on this guidance.",
                    steer_text
                ));
                messages.push(steer_msg.clone());
                self.add_message(steer_msg).await;
            }
        }
    }

    /// Build messages including system prompt
    async fn build_messages(&self) -> Result<Vec<Message>> {
        let mut messages = Vec::new();

        // ── Prompt-cache stability (iter-39) ─────────────────────────────
        // Split the system prompt into TWO messages:
        //   1. FROZEN PREFIX: base system prompt + skills. These rarely
        //      change across turns, so keeping them byte-stable lets
        //      Anthropic's prompt cache hit (cache reads cost ~10x less
        //      than fresh prompt tokens).
        //   2. VOLATILE SUFFIX: memory context + workspace context. These
        //      change each turn (memory grows, workspace files change),
        //      so they go in a separate message AFTER the frozen prefix.
        //      The frozen prefix stays cache-stable; only the volatile
        //      suffix + conversation history are re-processed each turn.
        //
        // This is a simplified version of magic-context's m[0]/m[1]
        // cache layout. The full m[0]/m[1] scheme uses HARD/SOFT/SOFT+
        // pass taxonomy + byte-identical replay; this implementation
        // just splits into frozen vs volatile, which captures ~80% of
        // the cache benefit with ~10% of the complexity.

        let frozen_prefix = if let Some(ref system) = self.config.system_prompt {
            system.clone()
        } else {
            "You are Operant, a helpful AI assistant. You have access to tools that you can use to help users. \
                Use the provided tools when needed to accomplish tasks. \
                After receiving tool results, continue reasoning and either call more tools or provide your final response to the user."
                .to_string()
        };

        // Skills are stable within a session (they're loaded once at
        // startup), so they go in the frozen prefix.
        let mut frozen = frozen_prefix;
        if let Some(skill_manager) = &self.skill_manager {
            let skills = skill_manager.list();
            if !skills.is_empty() {
                frozen.push_str("\n\n<available_skills>\n");
                for (name, description) in &skills {
                    frozen.push_str(&format!(
                        "  <skill name=\"{}\">{}</skill>\n",
                        name, description
                    ));
                }
                frozen.push_str("</available_skills>");
            }
        }
        messages.push(Message::system(frozen));

        // Volatile suffix: memory context + workspace context. These
        // change each turn, so they're a separate message that doesn't
        // bust the frozen prefix's cache entry.
        let mut volatile_suffix = String::new();
        if let Some(memory_manager) = &self.memory_manager {
            let memory_context = memory_manager.build_memory_context(2048).await;
            let memory_context = memory_context.trim();
            if !memory_context.is_empty() {
                volatile_suffix.push_str("\n\n<long_term_memory>\n");
                volatile_suffix.push_str(memory_context);
                volatile_suffix.push_str("\n</long_term_memory>");
            }
        }

        let context_files = self.load_context_file_prompt();
        if !context_files.trim().is_empty() {
            volatile_suffix.push_str("\n\n<workspace_context>\n");
            volatile_suffix.push_str(context_files.trim());
            volatile_suffix.push_str("\n</workspace_context>");
        }

        if !volatile_suffix.trim().is_empty() {
            messages.push(Message::system(volatile_suffix.trim().to_string()));
        }

        // Add conversation history
        let conv = self.conversation.read().await;
        messages.extend(conv.clone());
        drop(conv);

        // Apply context management: decay-render old messages + evict
        // if over budget. Without this, any long-running session would
        // eventually exceed the context window and 400-error. The budget
        // is derived from the agent's context_window config; the reserve
        // leaves room for the model's response.
        let budget = self.config.context_window;
        let reserve = 4096; // tokens reserved for the model's response
        messages = crate::context_management::manage_context(messages, budget, reserve);

        Ok(messages)
    }

    fn load_context_file_prompt(&self) -> String {
        let mut blocks = Vec::new();

        let global_context = load_default_context_files();
        if !global_context.trim().is_empty() {
            blocks.push(global_context);
        }

        match std::env::current_dir() {
            Ok(cwd) => {
                if let Some(workspace_context) = load_workspace_context(&cwd) {
                    blocks.push(workspace_context);
                }
            }
            Err(error) => {
                warn!(error = %error, "Could not determine current directory for context files")
            }
        }

        blocks.join("\n\n")
    }

    /// Save a trajectory (ReAct steps + messages + metadata) for this run.
    ///
    /// Writes to `~/.operant/trajectories/<session_id>-<timestamp>.json`.
    /// Each trajectory captures: session ID, model, iteration count, tool
    /// call count, success status, full message history, and per-step
    /// thought/action/observation where extractable.
    async fn save_trajectory(
        &self,
        session_id: &str,
        messages: &[Message],
        iterations: usize,
        tool_calls: usize,
        success: bool,
        _final_response: Option<&Message>,
    ) {
        use crate::trajectory::{Trajectory, TrajectoryStep};

        let mut trajectory = Trajectory::new(
            format!("{}_{}", session_id, chrono::Utc::now().timestamp()),
            session_id,
            &self.config.model,
        );
        trajectory.iterations = iterations;
        trajectory.tool_calls = tool_calls;
        trajectory.success = success;

        // Build per-step records from the message history.
        // Each assistant message with tool calls → a reasoning step.
        // Each tool result → an observation step.
        // The final assistant message (no tool calls) → a response step.
        let mut step_idx = 0usize;
        for msg in messages {
            match msg.role.as_str() {
                "assistant" => {
                    let mut step = TrajectoryStep {
                        step: step_idx,
                        thought: Some(msg.content.clone()),
                        action: None,
                        action_args: None,
                        observation: None,
                        response: None,
                        success: true,
                    };
                    if let Some(tool_calls) = msg.tool_calls.as_ref() {
                        if let Some(first) = tool_calls.first() {
                            step.action = Some(first.function.name.clone());
                            step.action_args = Some(first.function.arguments.clone());
                        }
                    } else {
                        // No tool calls → this is a response step
                        step.response = Some(msg.content.clone());
                    }
                    trajectory.add_step(step);
                    step_idx += 1;
                }
                "tool" => {
                    // Attach as observation to the last step
                    if let Some(last) = trajectory.steps.last_mut() {
                        last.observation = Some(msg.content.clone());
                    }
                }
                _ => {}
            }
            trajectory.add_message(msg.clone());
        }

        // The final_response is the last assistant message; it's already
        // captured in the messages loop above, so no extra step needed.

        trajectory.calculate_tokens();

        // Write to ~/.operant/trajectories/
        let trajectories_dir = crate::platform::operant_home().join("trajectories");
        if let Err(e) = std::fs::create_dir_all(&trajectories_dir) {
            warn!(error = %e, "Failed to create trajectories dir");
            return;
        }
        let path = trajectories_dir.join(format!("{}.json", trajectory.id));
        match trajectory.to_json() {
            Ok(json) => {
                if let Err(e) = std::fs::write(&path, json) {
                    warn!(error = %e, path = ?path, "Failed to write trajectory");
                } else {
                    info!(path = %path.display(), "Trajectory saved");
                }
            }
            Err(e) => {
                warn!(error = %e, "Failed to serialize trajectory");
            }
        }
    }

    fn spawn_session_distillation(&self, history: Vec<Message>) {
        let Some(memory_manager) = self.memory_manager.clone() else {
            return;
        };

        let client = self.client.clone();
        let model = self.config.model.clone();
        tokio::spawn(async move {
            if let Err(error) =
                distill_session_to_memory(client, model, memory_manager, history).await
            {
                warn!(error = %error, "Session distillation failed");
            }
        });
    }

    /// Access the underlying model client (useful for tools needing direct
    /// access to the concrete provider client).
    pub fn client(&self) -> &Arc<dyn ModelClient> {
        &self.client
    }

    /// Process streaming response with early tool detection
    async fn process_stream(
        &self,
        mut stream: BoxStream<'static, Result<StreamChunk>>,
    ) -> Result<(String, String, Vec<ToolCall>, Option<serde_json::Value>)> {
        let mut accumulated_extra: Option<serde_json::Value> = None;
        let mut parser = ToolCallStreamParser::new().on_tool_call(|tc| {
            let tc_id = tc.id.clone();
            debug!(tool_call_id = %tc_id, name = %tc.function.name, "Early tool call detected");
        });
        let mut content_router = ThinkBlockRouter::default();
        let mut tool_call_router = ToolCallContentRouter::default();
        let mut accumulated_text = String::new();
        let mut accumulated_reasoning = String::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();
        // Capture the original stream error so we can surface it (instead of
        // the generic "Stream processing failed" string) and decide whether
        // to flush partials after the loop.
        let mut stream_error: Option<Error> = None;

        while let Some(chunk_result) = stream.next().await {
            match chunk_result {
                Ok(chunk) => {
                    // Process reasoning from StreamChunk
                    if let Some(reasoning) = chunk.reasoning {
                        let reasoning = strip_reasoning_tags(&reasoning);
                        if !reasoning.is_empty() {
                            accumulated_reasoning.push_str(&reasoning);
                            self.emit(AgentEvent::Reasoning { text: reasoning }).await;
                        }
                    }

                    // Capture provider-specific extra content (e.g. Gemini thought_signature)
                    if let Some(ref extra) = chunk.extra_content {
                        if !extra.is_null() {
                            accumulated_extra = Some(extra.clone());
                        }
                    }

                    // Process content from StreamChunk
                    if let Some(text) = chunk.content {
                        let (content_delta, reasoning_delta) = content_router.feed(&text);

                        if !content_delta.is_empty() {
                            let chunk_tool_calls = parser.process_chunk(&content_delta);
                            for tc in chunk_tool_calls {
                                if !tool_calls.iter().any(|existing| existing.id == tc.id) {
                                    tool_calls.push(tc);
                                }
                            }

                            let visible_text = tool_call_router.feed(&content_delta);
                            if !visible_text.is_empty() {
                                accumulated_text.push_str(&visible_text);
                                self.emit(AgentEvent::Content { text: visible_text }).await;
                            }
                        }

                        if !reasoning_delta.is_empty() {
                            accumulated_reasoning.push_str(&reasoning_delta);
                            self.emit(AgentEvent::Reasoning {
                                text: reasoning_delta,
                            })
                            .await;
                        }
                    }

                    // Merge native provider tool-call deltas
                    if let Some(chunk_tool_calls) = chunk.tool_calls {
                        for tc in chunk_tool_calls {
                            merge_stream_tool_call(&mut tool_calls, tc);
                        }
                    }
                }
                Err(e) => {
                    error!(error = %e, "Stream error");
                    // Capture the original error so we can surface it after
                    // flushing partials. Previously the error was swallowed
                    // and replaced with a generic "Stream processing failed"
                    // string, making debugging impossible.
                    stream_error = Some(e);
                    break;
                }
            }
        }

        // Flush any partial content/tool_calls still buffered in the routers
        // and parser. This runs on both success AND error paths so partial
        // tool calls (e.g. a tool_use block that started but didn't finish
        // before the stream broke) are still extracted and returned to the
        // caller. Previously the error path `break`ed before this flush,
        // dropping all partials.
        let (remaining_content, remaining_reasoning) = content_router.finish();
        if !remaining_content.is_empty() {
            let remaining_calls = parser.process_chunk(&remaining_content);
            for tc in remaining_calls {
                merge_stream_tool_call(&mut tool_calls, tc);
            }
            let visible = tool_call_router.feed(&remaining_content);
            if !visible.is_empty() {
                accumulated_text.push_str(&visible);
                // Emit the flushed partial so the TUI sees it even if we're
                // about to return Err — otherwise content streamed right
                // before the error would be silently lost.
                self.emit(AgentEvent::Content { text: visible }).await;
            }
        }
        let tail = tool_call_router.finish();
        if !tail.is_empty() {
            accumulated_text.push_str(&tail);
            self.emit(AgentEvent::Content { text: tail }).await;
        }
        if !remaining_reasoning.is_empty() {
            accumulated_reasoning.push_str(&remaining_reasoning);
            self.emit(AgentEvent::Reasoning {
                text: remaining_reasoning,
            })
            .await;
        }

        // Also try to extract any remaining tool calls from accumulated text.
        // On the error path we don't want a parser failure to mask the
        // original stream error, so fall back to an empty vec.
        let mut remaining_parser = ToolCallParser::new();
        let remaining_calls = if stream_error.is_some() {
            remaining_parser.parse(&accumulated_text).unwrap_or_default()
        } else {
            remaining_parser.parse(&accumulated_text)?
        };

        // Merge tool calls, avoiding duplicates
        for tc in remaining_calls {
            merge_stream_tool_call(&mut tool_calls, tc);
        }

        if let Some(err) = stream_error {
            // Surface the original stream error (not a generic "Stream
            // processing failed" string) so the caller can see what went
            // wrong. Trajectory saving is handled by the caller (run()) when
            // this error propagates up — see the Err(e) arm in the run() loop.
            return Err(Error::Agent(format!(
                "Stream processing failed: {err}"
            )));
        }

        Ok((
            accumulated_text,
            accumulated_reasoning,
            tool_calls,
            accumulated_extra,
        ))
    }

    async fn process_response(
        &self,
        response: ChatResponse,
    ) -> Result<(String, String, Vec<ToolCall>)> {
        let choice = response
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| Error::ParseResponse("response had no choices".to_string()))?;

        let message = choice.message;
        let raw_content = message.content.unwrap_or_default();
        let content = strip_tool_call_markup(&raw_content);
        let reasoning = message
            .reasoning_content
            .map(|value| strip_reasoning_tags(&value))
            .unwrap_or_default();
        let mut tool_calls = extract_tool_calls_from_choice(message.tool_calls);
        let mut xml_parser = ToolCallParser::new();
        if let Ok(xml_tool_calls) = xml_parser.parse(&raw_content) {
            for tool_call in xml_tool_calls {
                merge_stream_tool_call(&mut tool_calls, tool_call);
            }
        }

        if !content.is_empty() {
            self.emit(AgentEvent::Content {
                text: content.clone(),
            })
            .await;
        }
        if !reasoning.is_empty() {
            self.emit(AgentEvent::Reasoning {
                text: reasoning.clone(),
            })
            .await;
        }

        self.emit(AgentEvent::Usage {
            input_tokens: response.usage.prompt_tokens,
            output_tokens: response.usage.completion_tokens,
            total_tokens: response.usage.total_tokens,
        })
        .await;

        Ok((content, reasoning, tool_calls))
    }

    /// Execute tools and handle self-healing
    async fn execute_tools(&self, tool_calls: Vec<ToolCall>) -> Result<Vec<ToolResult>> {
        // ── Concurrent tool execution (iter-56) ──────────────────────────
        // Previously this was a sequential for-loop. Now it's two phases:
        //
        // Phase 1 (sequential): Pre-flight checks — interrupt flag, arg
        //   parsing, tool validation, approval gate, permission prompts.
        //   These MUST be sequential because permission prompts are
        //   interactive (the user sees one dialog at a time).
        //
        // Phase 2 (concurrent): Execute all approved tools concurrently
        //   using FuturesUnordered with a semaphore (max 8, matching
        //   hermes's _MAX_TOOL_WORKERS). Independent tool calls (e.g.
        //   4 web searches) now run in parallel instead of serially.
        //
        // Results are collected in the SAME ORDER as the input tool_calls
        // (the model expects results in the same order as the calls).

        use futures::stream::{self, StreamExt};
        use std::sync::Arc as StdArc;
        use tokio::sync::Semaphore;

        // ── Phase 1: Pre-flight (sequential) ────────────────────────────
        let mut pending: Vec<(usize, ToolCall, serde_json::Value)> = Vec::new();
        let mut early_results: Vec<Option<ToolResult>> = vec![None; tool_calls.len()];

        for (idx, tool_call) in tool_calls.into_iter().enumerate() {
            // Check interrupt flag
            if self.interrupt_flag.is_triggered() {
                early_results[idx] = Some(ToolResult::error(
                    &tool_call.id,
                    "Skipped: interrupted by user (Ctrl-C)".to_string(),
                ));
                continue;
            }

            let name = tool_call.function.name.clone();
            let raw_args = tool_call.function.arguments.clone();
            let trimmed = raw_args.trim();
            let args_str = if trimmed.is_empty() {
                "{}".to_string()
            } else {
                raw_args
            };

            debug!(tool = %name, args = %args_str, "Executing tool");
            self.emit(AgentEvent::ToolStart {
                tool_call_id: tool_call.id.clone(),
                name: name.clone(),
                arguments: args_str.clone(),
            })
            .await;

            // Parse arguments
            let args: serde_json::Value = match serde_json::from_str(&args_str) {
                Ok(a) => a,
                Err(e) => {
                    warn!(tool = %name, error = %e, "Failed to parse tool arguments");
                    early_results[idx] = Some(ToolResult::error(
                        &tool_call.id,
                        format!("Invalid JSON: {}", e),
                    ));
                    continue;
                }
            };

            // Validate tool exists
            if !self.registry.contains(&name).await {
                error!(tool = %name, "Tool not found");
                early_results[idx] = Some(ToolResult::error(
                    &tool_call.id,
                    format!("Tool '{}' not found", name),
                ));
                continue;
            }

            // Smart approval gate
            if self.config.approval_mode != "off" {
                let approval_result = crate::approval::check_tool_approval(
                    &name,
                    &args,
                    Some(&self.config.approval_mode),
                );
                match approval_result.verdict.as_str() {
                    "blocked" => {
                        warn!(tool = %name, "Tool call blocked by approval guard");
                        early_results[idx] = Some(ToolResult::error(
                            &tool_call.id,
                            format!("Blocked by security policy: {}", approval_result.reason.unwrap_or_else(|| "blocked".to_string())),
                        ));
                        continue;
                    }
                    "requires_approval" => {
                        warn!(tool = %name, "Tool call flagged — will prompt user");
                    }
                    _ => {}
                }
            }

            // Permission guard for dangerous tools (interactive — sequential)
            if let Some(ref permission_tx) = self.permission_tx {
                let needs_permission = matches!(
                    name.as_str(),
                    "bash" | "terminal" | "execute_command" | "code_execution"
                        | "file_read" | "file_write" | "file_edit" | "patch"
                        | "process" | "browser"
                );
                if needs_permission {
                    let (resp_tx, resp_rx) = tokio::sync::oneshot::channel();
                    let description = format!("Execute {} tool", name);
                    let danger = match name.as_str() {
                        "bash" | "terminal" | "execute_command" => "This runs a shell command on your system".to_string(),
                        "code_execution" => "This executes code in a sandbox".to_string(),
                        "file_read" => "This reads a file from your system".to_string(),
                        "file_write" => "This writes content to a file".to_string(),
                        "file_edit" | "patch" => "This modifies an existing file".to_string(),
                        "process" => "This manages background processes".to_string(),
                        "browser" => "This opens and interacts with a browser".to_string(),
                        _ => "This tool may modify your system".to_string(),
                    };
                    let input_preview = Some(args_str.clone());
                    let _ = permission_tx
                        .send(ToolPermissionRequest {
                            tool_name: name.clone(),
                            tool_id: tool_call.id.clone(),
                            description,
                            danger_explanation: danger,
                            input_preview,
                            response_tx: resp_tx,
                        })
                        .await;
                    let response = tokio::select! {
                        r = resp_rx => r.unwrap_or(ToolPermissionResponse::Deny),
                        _ = tokio::time::sleep(Duration::from_secs(120)) => ToolPermissionResponse::Deny,
                    };
                    match response {
                        ToolPermissionResponse::AllowOnce | ToolPermissionResponse::AllowSession => {}
                        ToolPermissionResponse::Deny => {
                            early_results[idx] = Some(ToolResult::error(
                                &tool_call.id,
                                "Permission denied by user".to_string(),
                            ));
                            continue;
                        }
                    }
                }
            }

            // Tool passed all pre-flight checks — queue for concurrent execution
            pending.push((idx, tool_call, args));
        }

        // ── Phase 2: Concurrent execution ───────────────────────────────
        // Use a semaphore to limit concurrency to 8 (matching hermes).
        // If only 1 tool is pending, skip the overhead and execute directly.
        if pending.is_empty() {
            // All tools were handled in pre-flight (errors/blocked/denied)
            let results = early_results.into_iter().map(|r| r.unwrap()).collect();
            return Ok(results);
        }

        if pending.len() == 1 {
            // Single tool — no concurrency overhead
            let (idx, tool_call, args) = pending.into_iter().next().unwrap();
            let name = tool_call.function.name.clone();
            let result = timeout(
                self.config.tool_timeout,
                self.registry.execute(&name, &tool_call.id, args, ToolContext::default()),
            ).await;
            early_results[idx] = Some(match result {
                Ok(Ok(r)) => r,
                Ok(Err(e)) => ToolResult::error(&tool_call.id, e.to_string()),
                Err(_) => ToolResult::error(&tool_call.id, format!("Tool timed out after {:?}", self.config.tool_timeout)),
            });
        } else {
            // Multiple tools — execute concurrently with semaphore
            let semaphore = StdArc::new(Semaphore::new(8));
            let tool_timeout = self.config.tool_timeout;

            let futures: Vec<_> = pending
                .into_iter()
                .map(|(idx, tool_call, args)| {
                    let sem = semaphore.clone();
                    let registry = &self.registry;
                    let interrupt_flag = &self.interrupt_flag;
                    async move {
                        // Acquire semaphore permit (limits to 8 concurrent)
                        let _permit = sem.acquire().await.unwrap();

                        // Check interrupt flag before execution
                        if interrupt_flag.is_triggered() {
                            return (idx, ToolResult::error(&tool_call.id, "Skipped: interrupted".to_string()));
                        }

                        let name = tool_call.function.name.clone();
                        let result = timeout(
                            tool_timeout,
                            registry.execute(&name, &tool_call.id, args, ToolContext::default()),
                        ).await;

                        (idx, match result {
                            Ok(Ok(r)) => r,
                            Ok(Err(e)) => ToolResult::error(&tool_call.id, e.to_string()),
                            Err(_) => ToolResult::error(&tool_call.id, format!("Tool timed out after {:?}", tool_timeout)),
                        })
                    }
                })
                .collect();

            // Execute all futures concurrently and collect results
            let results = stream::iter(futures)
                .buffer_unordered(8)
                .collect::<Vec<_>>()
                .await;

            // Place results in the correct position
            for (idx, result) in results {
                early_results[idx] = Some(result);
            }
        }

        // Collect results in original order
        let results = early_results.into_iter().map(|r| r.unwrap()).collect();
        Ok(results)
    }

    /// Run agent and handle self-healing on tool errors
    pub async fn run_with_healing(&self, user_query: String) -> Result<Message> {
        let mut iteration = 0;
        let max_healing_attempts = self.config.max_healing_attempts;

        loop {
            iteration += 1;

            match self.run(user_query.clone()).await {
                Ok(response) => return Ok(response),
                Err(e) if e.is_self_healing() && iteration <= max_healing_attempts => {
                    warn!(iteration, error = %e, "Self-healing: re-prompting LLM");

                    // Add error context as a system message
                    let error_msg = format!(
                        "Note: The previous attempt encountered an error: {}. \
                        Please correct your approach and try again.",
                        e.user_message()
                    );

                    self.add_message(Message::system(&error_msg)).await;
                }
                Err(e) => {
                    error!(error = %e, "Agent run failed");
                    return Err(e);
                }
            }
        }
    }
}

#[derive(Debug, Default)]
struct ThinkBlockRouter {
    pending: String,
    inside_reasoning: bool,
}

impl ThinkBlockRouter {
    fn feed(&mut self, chunk: &str) -> (String, String) {
        self.pending.push_str(chunk);
        self.drain_ready()
    }

    fn finish(&mut self) -> (String, String) {
        let (mut content, mut reasoning) = self.drain_ready();
        if !self.pending.is_empty() {
            if self.inside_reasoning {
                reasoning.push_str(&self.pending);
                if content.trim().is_empty() {
                    content.push_str(&self.pending);
                }
            } else {
                content.push_str(&self.pending);
            }
            self.pending.clear();
        }
        (content, reasoning)
    }

    fn drain_ready(&mut self) -> (String, String) {
        const MAX_TAG_LEN: usize = 23;
        let mut content = String::new();
        let mut reasoning = String::new();

        loop {
            let lowered = self.pending.to_ascii_lowercase();
            let tag = if self.inside_reasoning {
                find_first_tag(&lowered, CLOSE_REASONING_TAGS)
            } else {
                find_first_tag(&lowered, OPEN_REASONING_TAGS)
            };

            if let Some((index, marker)) = tag {
                let segment = self.pending[..index].to_string();
                if self.inside_reasoning {
                    reasoning.push_str(&segment);
                } else {
                    content.push_str(&segment);
                }
                self.pending.drain(..index + marker.len());
                self.inside_reasoning = !self.inside_reasoning;
                continue;
            }

            let keep = self.pending.len().min(MAX_TAG_LEN.saturating_sub(1));
            let flush_len =
                floor_char_boundary(&self.pending, self.pending.len().saturating_sub(keep));
            if flush_len == 0 {
                break;
            }

            let segment = self.pending[..flush_len].to_string();
            if self.inside_reasoning {
                reasoning.push_str(&segment);
            } else {
                content.push_str(&segment);
            }
            self.pending.drain(..flush_len);
        }

        (content, reasoning)
    }
}

const OPEN_REASONING_TAGS: &[&str] = &[
    "<think>",
    "<thinking>",
    "<reasoning>",
    "<thought>",
    "<reasoning_scratchpad>",
];

const CLOSE_REASONING_TAGS: &[&str] = &[
    "</think>",
    "</thinking>",
    "</reasoning>",
    "</thought>",
    "</reasoning_scratchpad>",
];

fn find_first_tag<'a>(haystack: &str, tags: &'a [&'a str]) -> Option<(usize, &'a str)> {
    tags.iter()
        .filter_map(|tag| haystack.find(tag).map(|index| (index, *tag)))
        .min_by_key(|(index, _)| *index)
}

fn floor_char_boundary(text: &str, index: usize) -> usize {
    let mut boundary = index.min(text.len());
    while boundary > 0 && !text.is_char_boundary(boundary) {
        boundary -= 1;
    }
    boundary
}

/// Truncate tool results that are too large for context (e.g. base64 audio).
/// Keeps a JSON summary with metadata but strips the bulk data.
const MAX_TOOL_RESULT_LEN: usize = 4096;

fn truncate_tool_result(tool_name: &str, content: &str) -> String {
    if content.len() <= MAX_TOOL_RESULT_LEN {
        return content.to_string();
    }
    // Try to parse as JSON and strip large fields
    if let Ok(mut val) = serde_json::from_str::<serde_json::Value>(content) {
        if let Some(obj) = val.as_object_mut() {
            // Remove known large fields
            let had_audio = obj.remove("audio").is_some();
            let had_data = obj.remove("data").is_some();
            if had_audio {
                obj.insert(
                    "audio".to_string(),
                    serde_json::json!("[audio data delivered to user]"),
                );
            }
            if had_data {
                obj.insert(
                    "data".to_string(),
                    serde_json::json!("[large data truncated]"),
                );
            }
            let serialized = serde_json::to_string(&val).unwrap_or_default();
            if serialized.len() <= MAX_TOOL_RESULT_LEN {
                return serialized;
            }
            // Serialized JSON is still too long — fall through to safe truncate
            return format!(
                "{}... [truncated, tool: {}]",
                safe_truncate_str(&serialized, MAX_TOOL_RESULT_LEN),
                tool_name
            );
        }
    }
    // Fallback: hard truncate (char-boundary-safe to avoid panic on CJK/emoji)
    format!(
        "{}... [truncated, tool: {}]",
        safe_truncate_str(content, MAX_TOOL_RESULT_LEN),
        tool_name
    )
}

/// Truncate a string to at most `max_bytes` bytes, ending at a UTF-8 char
/// boundary. Without this, `&s[..N]` panics if N falls in the middle of a
/// multi-byte character (common with CJK text or emoji in tool output).
fn safe_truncate_str(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn strip_reasoning_tags(text: &str) -> String {
    let mut cleaned = text.to_string();
    for tag in OPEN_REASONING_TAGS
        .iter()
        .chain(CLOSE_REASONING_TAGS.iter())
    {
        cleaned = cleaned.replace(tag, "");
        cleaned = cleaned.replace(&tag.to_uppercase(), "");
    }
    cleaned
}

fn extract_tool_calls_from_choice(
    deltas: Option<Vec<crate::client::ToolCallDelta>>,
) -> Vec<ToolCall> {
    deltas
        .unwrap_or_default()
        .into_iter()
        .filter_map(|delta| {
            let function = delta.function?;
            Some(ToolCall {
                id: delta
                    .id
                    .unwrap_or_else(|| format!("call_choice_{}_{}", delta.index, function.name)),
                function,
            })
        })
        .collect()
}

pub(crate) fn merge_stream_tool_call(tool_calls: &mut Vec<ToolCall>, tool_call: ToolCall) {
    if let Some(existing) = tool_calls
        .iter_mut()
        .find(|existing| existing.id == tool_call.id)
    {
        if existing.function.name.is_empty() {
            existing.function.name = tool_call.function.name;
        }
        if !tool_call.function.arguments.is_empty() {
            existing
                .function
                .arguments
                .push_str(&tool_call.function.arguments);
        }
    } else {
        tool_calls.push(tool_call);
    }
}

#[derive(Default)]
struct ToolCallContentRouter {
    pending: String,
    inside_tool_call: bool,
}

impl ToolCallContentRouter {
    fn feed(&mut self, chunk: &str) -> String {
        self.pending.push_str(chunk);
        self.drain_ready(false)
    }

    fn finish(&mut self) -> String {
        self.drain_ready(true)
    }

    fn drain_ready(&mut self, flush_all: bool) -> String {
        const OPEN: &str = "<tool_call";
        const CLOSE: &str = "</tool_call";
        let mut content = String::new();

        loop {
            if self.inside_tool_call {
                if let Some(index) = find_ascii_case_insensitive(&self.pending, CLOSE) {
                    let close_end = self.pending[index..]
                        .find('>')
                        .map(|offset| index + offset + 1);
                    if let Some(close_end) = close_end {
                        self.pending.drain(..close_end);
                        self.inside_tool_call = false;
                        continue;
                    }
                }

                if flush_all {
                    self.pending.clear();
                }
                break;
            }

            if let Some(index) = find_ascii_case_insensitive(&self.pending, OPEN) {
                content.push_str(&self.pending[..index]);
                if let Some(open_end) = self.pending[index..]
                    .find('>')
                    .map(|offset| index + offset + 1)
                {
                    self.pending.drain(..open_end);
                    self.inside_tool_call = true;
                    continue;
                }

                self.pending.drain(..index);
                break;
            }

            let keep = if flush_all {
                0
            } else {
                longest_suffix_prefix_match_case_insensitive(&self.pending, OPEN)
            };
            let flush_len = self.pending.len().saturating_sub(keep);
            if flush_len == 0 {
                break;
            }

            content.push_str(&self.pending[..flush_len]);
            self.pending.drain(..flush_len);
            break;
        }

        content
    }
}

fn longest_suffix_prefix_match(value: &str, marker: &str) -> usize {
    let max = value.len().min(marker.len().saturating_sub(1));
    for len in (1..=max).rev() {
        if value.ends_with(&marker[..len]) {
            return len;
        }
    }
    0
}

fn longest_suffix_prefix_match_case_insensitive(value: &str, marker: &str) -> usize {
    let lowered = value.to_ascii_lowercase();
    longest_suffix_prefix_match(&lowered, marker)
}

fn find_ascii_case_insensitive(value: &str, marker: &str) -> Option<usize> {
    value.to_ascii_lowercase().find(marker)
}

fn strip_tool_call_markup(content: &str) -> String {
    let mut router = ToolCallContentRouter::default();
    let mut visible = router.feed(content);
    visible.push_str(&router.finish());
    visible
}

/// Builder for creating a OperantAgent
pub struct OperantAgentBuilder {
    config: AgentConfig,
    client: Option<Box<dyn ModelClient>>,
    registry: Option<ToolRegistry>,
    memory_manager: Option<MemoryManager>,
    memory_provider: Option<Arc<dyn crate::memory_provider::MemoryProvider>>,
    hook_registry: Option<Arc<crate::gateway_pipeline::HookRegistry>>,
    database: Option<Arc<Database>>,
    steer_queue: Arc<tokio::sync::Mutex<Vec<String>>>,
}

impl OperantAgentBuilder {
    pub fn new() -> Self {
        Self {
            config: AgentConfig::default(),
            client: None,
            registry: None,
            memory_manager: None,
            memory_provider: None,
            hook_registry: None,
            steer_queue: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            database: None,
        }
    }

    /// Set the model
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.config.model = model.into();
        self
    }

    /// Set maximum iterations
    pub fn max_iterations(mut self, max: usize) -> Self {
        self.config.max_iterations = max;
        self
    }

    /// Set tool timeout
    pub fn tool_timeout(mut self, timeout: Duration) -> Self {
        self.config.tool_timeout = timeout;
        self
    }

    /// Set request timeout
    pub fn request_timeout(mut self, timeout: Duration) -> Self {
        self.config.request_timeout = timeout;
        self
    }

    /// Set system prompt
    pub fn system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.config.system_prompt = Some(prompt.into());
        self
    }

    /// Enable/disable streaming
    pub fn streaming(mut self, enabled: bool) -> Self {
        self.config.stream = enabled;
        self
    }

    /// Set the model client
    pub fn client(mut self, client: Box<dyn ModelClient>) -> Self {
        self.client = Some(client);
        self
    }

    /// Set the tool registry
    pub fn registry(mut self, registry: ToolRegistry) -> Self {
        self.registry = Some(registry);
        self
    }

    /// Set the database
    pub fn database(mut self, database: Arc<Database>) -> Self {
        self.database = Some(database);
        self
    }

    /// Set the long-term memory manager.
    pub fn memory_manager(mut self, memory_manager: MemoryManager) -> Self {
        self.memory_manager = Some(memory_manager);
        self
    }

    /// Set the TDG memory provider for graph memory hooks.
    pub fn memory_provider(
        mut self,
        memory_provider: Arc<dyn crate::memory_provider::MemoryProvider>,
    ) -> Self {
        self.memory_provider = Some(memory_provider);
        self
    }

    /// Set the hook registry for lifecycle events.
    pub fn hook_registry(
        mut self,
        hook_registry: Arc<crate::gateway_pipeline::HookRegistry>,
    ) -> Self {
        self.hook_registry = Some(hook_registry);
        self
    }

    /// Build the agent
    pub fn build(self) -> Result<OperantAgent> {
        let client: Box<dyn ModelClient> = self.client.unwrap_or_else(|| {
            let openai = crate::client::OpenAIClient::from_env().unwrap_or_else(|_| {
                crate::client::OpenAIClient::new(crate::client::ClientConfig::default())
            });
            Box::new(crate::agent::clients::openai::OpenAIModelClient::new(
                openai,
            ))
        });

        // Wrap with fallback logic if configured
        let client: Box<dyn ModelClient> = if !self.config.fallback_models.is_empty() {
            Box::new(FallbackModelClient::new(
                Arc::from(client),
                self.config.model.clone(),
                self.config.fallback_models.clone(),
                self.config.fallback_on_errors,
            ))
        } else {
            client
        };

        let registry = self
            .registry
            .unwrap_or_else(|| ToolRegistry::new(self.config.tool_timeout));

        let database = self.database.ok_or_else(|| {
            Error::Config("Database must be provided to the agent builder".to_string())
        })?;

        let mut agent = OperantAgent::new(self.config, client, registry, database);
        if let Some(memory_manager) = self.memory_manager {
            agent = agent.with_memory_manager(memory_manager);
        }
        if let Some(memory_provider) = self.memory_provider {
            agent = agent.with_memory_provider(memory_provider);
        }
        if let Some(hook_registry) = self.hook_registry {
            agent = agent.with_hook_registry(hook_registry);
        }
        // Steer queue is shared — set it directly
        agent.steer_queue = self.steer_queue;

        Ok(agent)
    }
}

impl Default for OperantAgentBuilder {
    fn default() -> Self {
        Self::new()
    }
}


mod model_client;
pub use model_client::{ChatRequest, ModelClient, StreamChunk};

mod fallback;
pub use fallback::{ClassifiedError, FallbackModelClient};

pub mod clients;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::ChatStreamEvent;
    use serial_test::serial;

    #[allow(dead_code)]
    fn extract_text_from_event(event: &ChatStreamEvent) -> Option<String> {
        let mut text = String::new();

        for choice in &event.choices {
            if let Some(content) = &choice.delta.content {
                text.push_str(content);
            }
        }

        if text.is_empty() {
            None
        } else {
            Some(text)
        }
    }

    #[allow(dead_code)]
    fn extract_reasoning_from_event(event: &ChatStreamEvent) -> Option<String> {
        let mut reasoning = String::new();

        for choice in &event.choices {
            if let Some(content) = &choice.delta.reasoning_content {
                reasoning.push_str(content);
            }
        }

        if reasoning.is_empty() {
            None
        } else {
            Some(reasoning)
        }
    }

    #[allow(dead_code)]
    fn extract_tool_calls_from_event(event: &ChatStreamEvent) -> Vec<ToolCall> {
        let mut tool_calls: Vec<ToolCall> = Vec::new();

        for choice in &event.choices {
            if let Some(delta_tool_calls) = &choice.delta.tool_calls {
                for delta in delta_tool_calls {
                    if let Some(ref function) = delta.function {
                        let id = delta.id.clone().unwrap_or_else(|| {
                            format!("call_stream_{}_{}", delta.index, function.name)
                        });

                        if let Some(last) = tool_calls.last_mut() {
                            if last.id == id {
                                last.function.arguments.push_str(&function.arguments);
                                continue;
                            }
                        }

                        tool_calls.push(ToolCall {
                            id: id.clone(),
                            function: crate::client::ToolCallFunction {
                                name: function.name.clone(),
                                arguments: function.arguments.clone(),
                            },
                        });
                    }
                }
            }
        }

        tool_calls
    }

    #[test]
    fn test_default_config() {
        let config = AgentConfig::default();
        assert_eq!(config.model, "gpt-4");
        assert_eq!(config.max_iterations, 90);
    }

    #[serial]
    #[tokio::test]
    async fn test_agent_builder() {
        let db =
            Arc::new(Database::init(std::path::PathBuf::from("test_agent_builder.db")).unwrap());
        let _agent = OperantAgentBuilder::new()
            .model("gpt-3.5-turbo")
            .max_iterations(10)
            .database(db)
            .build()
            .unwrap();

        // Clean up test database
        let _ = std::fs::remove_file("test_agent_builder.db");
        let _ = std::fs::remove_file("test_agent_builder.db-wal");
        let _ = std::fs::remove_file("test_agent_builder.db-shm");
    }

    #[serial]
    #[tokio::test]
    async fn build_messages_injects_long_term_memory() {
        use crate::agent::clients::openai::OpenAIModelClient;
        use crate::client::OpenAIClient;

        let memory_manager = MemoryManager::new();
        memory_manager
            .store(
                crate::memory::MemoryBlock::new("fact1", "fact", "User prefers concise answers")
                    .importance(80),
            )
            .await;

        let db = Database::init(std::path::PathBuf::from("test_db.sqlite")).unwrap();
        let agent = OperantAgent::new(
            AgentConfig::default(),
            Box::new(OpenAIModelClient::new(OpenAIClient::new(
                crate::client::ClientConfig::default(),
            ))),
            ToolRegistry::new(Duration::from_secs(1)),
            Arc::new(db),
        )
        .with_memory_manager(memory_manager);

        let messages = agent.build_messages().await.unwrap();
        // iter-39: the system prompt is now split into a frozen prefix
        // (base prompt + skills) and a volatile suffix (memory + workspace
        // context). Long-term memory lands in the second system message,
        // not the first. Concatenate all system message content to check.
        let system: String = messages
            .iter()
            .filter(|m| m.role == crate::client::Role::System)
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(system.contains("<long_term_memory>"));
        assert!(system.contains("[fact] User prefers concise answers"));
        assert!(system.contains("</long_term_memory>"));
    }

    #[test]
    fn test_extract_text_from_event() {
        let event = ChatStreamEvent {
            id: "test".to_string(),
            object: "chat.completion.chunk".to_string(),
            created: 0,
            model: "test".to_string(),
            choices: vec![crate::client::StreamChoice {
                index: 0,
                delta: crate::client::StreamingMessageDelta {
                    role: None,
                    content: Some("Hello ".to_string()),
                    reasoning_content: None,
                    tool_calls: None,
                    extra_content: None,
                },
                finish_reason: None,
            }],
        };

        let text = extract_text_from_event(&event);
        assert_eq!(text, Some("Hello ".to_string()));
    }

    #[test]
    fn think_router_splits_inline_think_blocks() {
        let mut router = ThinkBlockRouter::default();
        let (content_a, reasoning_a) = router.feed("Hello<think>plan");
        let (content_b, reasoning_b) = router.feed(" more</think> world");
        let (content_c, reasoning_c) = router.finish();

        assert_eq!(content_a, "Hello");
        assert_eq!(reasoning_a, "");
        assert_eq!(content_b, "");
        assert_eq!(reasoning_b, "plan more");
        assert_eq!(content_c, " world");
        assert_eq!(reasoning_c, "");
    }

    #[test]
    fn strip_reasoning_tags_removes_supported_markers() {
        assert_eq!(
            strip_reasoning_tags(
                "<think>abc</think><REASONING_SCRATCHPAD>def</REASONING_SCRATCHPAD>"
            ),
            "abcdef"
        );
    }

    #[test]
    fn think_router_does_not_split_multibyte_characters() {
        let mut router = ThinkBlockRouter::default();
        let (_content, _reasoning) = router.feed("Halo! 🧑‍💻 Senang bertemu");
        let (_content, _reasoning) = router.finish();
    }

    #[test]
    fn think_router_falls_back_to_content_for_unclosed_reasoning() {
        let mut router = ThinkBlockRouter::default();
        let (content, reasoning) = router.feed("<think>Visible answer");
        let (rest_content, rest_reasoning) = router.finish();

        assert_eq!(content, "");
        assert_eq!(reasoning, "");
        assert_eq!(rest_content, "Visible answer");
        assert_eq!(rest_reasoning, "Visible answer");
    }

    #[test]
    fn tool_call_router_hides_xml_from_visible_content() {
        let mut router = ToolCallContentRouter::default();

        let first = router.feed("Before <tool_call>{\"name\":\"datetime\"}");
        let second = router.feed("{\"arguments\":{}}</tool_call> after");
        let rest = router.finish();

        assert_eq!(first, "Before ");
        assert_eq!(second, " after");
        assert_eq!(rest, "");
    }

    #[test]
    fn tool_call_router_keeps_plain_text_streaming() {
        let mut router = ToolCallContentRouter::default();

        let first = router.feed("Halo ");
        let second = router.feed("operant!");
        let rest = router.finish();

        assert_eq!(first, "Halo ");
        assert_eq!(second, "operant!");
        assert_eq!(rest, "");
    }

    #[test]
    fn extract_tool_calls_from_choice_handles_non_streaming_calls() {
        let tool_calls = extract_tool_calls_from_choice(Some(vec![crate::client::ToolCallDelta {
            index: 0,
            id: Some("call_1".to_string()),
            call_type: Some("function".to_string()),
            function: Some(crate::client::ToolCallFunction {
                name: "datetime".to_string(),
                arguments: "{\"timezone\":\"UTC\"}".to_string(),
            }),
        }]));

        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].id, "call_1");
        assert_eq!(tool_calls[0].function.name, "datetime");
    }

    #[test]
    fn extract_tool_calls_from_choice_ignores_empty_entries() {
        let tool_calls = extract_tool_calls_from_choice(Some(vec![crate::client::ToolCallDelta {
            index: 0,
            id: None,
            call_type: None,
            function: None,
        }]));

        assert!(tool_calls.is_empty());
    }

    #[test]
    fn merge_stream_tool_call_appends_incremental_arguments() {
        let mut tool_calls = vec![ToolCall {
            id: "call_0_datetime".to_string(),
            function: crate::client::ToolCallFunction {
                name: "datetime".to_string(),
                arguments: "{\"format\":".to_string(),
            },
        }];

        merge_stream_tool_call(
            &mut tool_calls,
            ToolCall {
                id: "call_0_datetime".to_string(),
                function: crate::client::ToolCallFunction {
                    name: "datetime".to_string(),
                    arguments: "\"%Y-%m-%d\"}".to_string(),
                },
            },
        );

        assert_eq!(tool_calls.len(), 1);
        assert_eq!(
            tool_calls[0].function.arguments,
            "{\"format\":\"%Y-%m-%d\"}"
        );
    }

    #[test]
    fn tool_call_router_hides_split_tool_call_open_tag() {
        let mut router = ToolCallContentRouter::default();

        let first = router.feed("Before <tool_ca");
        let second = router.feed("ll>{\"name\":\"datetime\"}</tool_call> after");
        let rest = router.finish();

        assert_eq!(first, "Before ");
        assert_eq!(second, " after");
        assert_eq!(rest, "");
    }

    #[serial]
    #[tokio::test]
    async fn process_response_parses_xml_tool_calls_in_non_stream_mode() {
        use crate::agent::clients::openai::OpenAIModelClient;
        use crate::client::OpenAIClient;

        let db = Database::init(std::path::PathBuf::from("test_db_resp.sqlite")).unwrap();
        let agent = OperantAgent::new(
            AgentConfig::default(),
            Box::new(OpenAIModelClient::new(OpenAIClient::new(
                crate::client::ClientConfig::default(),
            ))),
            ToolRegistry::new(Duration::from_secs(1)),
            Arc::new(db),
        );

        let response = ChatResponse {
            id: "resp_1".to_string(),
            object: "chat.completion".to_string(),
            created: 0,
            model: "demo".to_string(),
            choices: vec![crate::client::Choice {
                index: 0,
                message: crate::client::MessageDelta {
                    role: Some(crate::client::Role::Assistant),
                    content: Some(
                        "<tool_call>{\"name\":\"datetime\",\"arguments\":\"{}\"}</tool_call>"
                            .to_string(),
                    ),
                    reasoning_content: Some("need tool".to_string()),
                    tool_calls: None,
                },
                finish_reason: Some("stop".to_string()),
            }],
            usage: crate::client::Usage {
                prompt_tokens: 1,
                completion_tokens: 1,
                total_tokens: 2,
            },
        };

        let (content, reasoning, tool_calls) = agent.process_response(response).await.unwrap();

        assert_eq!(content, "");
        assert_eq!(reasoning, "need tool");
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].function.name, "datetime");
    }

    #[test]
    fn tool_permission_response_variants() {
        let allow_once = ToolPermissionResponse::AllowOnce;
        let allow_session = ToolPermissionResponse::AllowSession;
        let deny = ToolPermissionResponse::Deny;

        assert_eq!(allow_once, ToolPermissionResponse::AllowOnce);
        assert_eq!(allow_session, ToolPermissionResponse::AllowSession);
        assert_eq!(deny, ToolPermissionResponse::Deny);
        assert_ne!(allow_once, deny);
    }

    #[test]
    fn agent_event_tool_permission_request_variant() {
        let event = AgentEvent::ToolPermissionRequest {
            tool_name: "terminal".to_string(),
            tool_id: "call_1".to_string(),
            description: "Execute terminal tool".to_string(),
            danger_explanation: "This runs a shell command".to_string(),
            input_preview: Some("ls -la".to_string()),
        };

        match event {
            AgentEvent::ToolPermissionRequest {
                tool_name, tool_id, ..
            } => {
                assert_eq!(tool_name, "terminal");
                assert_eq!(tool_id, "call_1");
            }
            _ => panic!("Expected ToolPermissionRequest variant"),
        }
    }
}
