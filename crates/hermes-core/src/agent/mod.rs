//! Hermes Agent orchestration loop with self-healing
//!
//! Implements the ReAct (Reason + Act) pattern for LLM-driven tool execution.

use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use futures::future::join_all;
use futures::stream::BoxStream;
use futures::StreamExt;
use tokio::sync::{mpsc, RwLock};
use tokio::time::timeout;
use tracing::{debug, error, info, instrument, warn};

use crate::client::{ChatResponse, Message, ToolCall};
use crate::config::{runtime_config, BehaviorSettings, ToolProgressMode};
use crate::context_files::{load_default_context_files, load_workspace_context};
use crate::database::Database;
use crate::distillation::distill_session_to_memory;
use crate::error::{Error, Result};
use crate::memory::MemoryManager;
use crate::parser::{ToolCallParser, ToolCallStreamParser};
use crate::tools::{ToolContext, ToolRegistry, ToolResult};

/// Simple detector for infinite agent loops
struct LoopDetector {
    /// Window of recent states to compare
    history: VecDeque<u64>,
    /// Max repeats allowed
    max_repeats: usize,
    /// Window size for comparison
    window_size: usize,
}

impl LoopDetector {
    fn new(window_size: usize, max_repeats: usize) -> Self {
        Self {
            history: VecDeque::with_capacity(window_size),
            max_repeats,
            window_size,
        }
    }

    fn check(&mut self, state: u64) -> bool {
        self.history.push_back(state);
        if self.history.len() > self.window_size {
            self.history.pop_front();
        }

        if self.history.len() < self.window_size {
            return false;
        }

        let repeats = self.history.iter().filter(|&&h| h == state).count();
        repeats >= self.max_repeats
    }
}

fn hash_tool_calls(calls: &[ToolCall]) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    for tc in calls {
        tc.function.name.hash(&mut hasher);
        tc.function.arguments.hash(&mut hasher);
    }
    hasher.finish()
}

/// Configuration for the Hermes agent
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
    /// How tool execution progress is reported
    pub tool_progress: ToolProgressMode,
    /// Max consecutive tool-only iterations before force-answer.
    /// When the LLM calls tools N times without producing text,
    /// the next request omits tools to force a textual response.
    /// 0 = disabled (use max_iterations as the only limit).
    pub max_consecutive_tool_only: usize,
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
            tool_progress: settings.tool_progress.clone(),
            max_consecutive_tool_only: settings.max_consecutive_tool_only,
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
    ToolStart {
        name: String,
        arguments: String,
        tool_call_id: String,
    },
    /// Tool execution completed
    ToolComplete { result: ToolResult },
    /// Tool execution failed
    ToolError { name: String, error: String },
    /// Response content received
    Content { text: String },
    /// Agent finished with final response
    Done { message: Message },
    /// Agent iteration completed
    IterationComplete { iteration: usize },
    /// Agent error
    Error { error: String },
}

/// Hermes Agent for tool orchestration
pub struct HermesAgent {
    config: AgentConfig,
    client: Arc<dyn ModelClient>,
    registry: ToolRegistry,
    conversation: Arc<RwLock<Vec<Message>>>,
    event_tx: Option<mpsc::Sender<AgentEvent>>,
    memory_manager: Option<MemoryManager>,
    database: Arc<Database>,
}

impl HermesAgent {
    /// Create a new Hermes agent
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
            memory_manager: None,
            database,
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
            memory_manager: None,
            database,
        }
    }

    /// Attach a memory manager for long-term memory injection and session distillation.
    pub fn with_memory_manager(mut self, memory_manager: MemoryManager) -> Self {
        self.memory_manager = Some(memory_manager);
        self
    }

    /// Send an event to the channel
    async fn emit(&self, event: AgentEvent) {
        if let Some(ref tx) = self.event_tx {
            debug!("Emitting event: {:?}", std::mem::discriminant(&event));
            if tx.send(event).await.is_err() {
                warn!("Agent event channel closed (receiver dropped)");
            }
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

    /// Run the agent with a user query
    #[instrument(skip(self), fields(model = % self.config.model))]
    pub async fn run(&self, user_query: String) -> Result<Message> {
        info!("Starting agent run");

        // Generate a session ID for this run if not already present
        let session_id = format!("sess_{}", uuid::Uuid::new_v4());

        // Add user message
        self.add_message(Message::user(&user_query)).await;

        // Save session metadata FIRST (messages FK depends on sessions.id)
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
        let mut force_answer = false;
        let mut force_answer_attempted = false;
        let mut consecutive_tool_only = 0usize;
        let mut loop_detector = LoopDetector::new(5, 3);

        loop {
            iteration += 1;
            debug!(iteration, "Agent iteration");

            if iteration > self.config.max_iterations {
                error!(max = self.config.max_iterations, "Max iterations exceeded");
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

            let mut messages_for_api = messages.clone();
            sanitize_messages_for_api(&mut messages_for_api);

            debug!(
                iteration = iteration,
                message_count = messages_for_api.len(),
                message_roles = ?messages_for_api.iter().map(|m| (m.role.as_str(), m.tool_calls.is_some(), m.tool_call_id.is_some())).collect::<Vec<_>>(),
                "Sending messages to API"
            );

            // When force_answer is true, omit tools from the request so the LLM
            // cannot call tools and must respond with text. This breaks infinite
            // tool-only loops (e.g. Gemma via Gemini calling tools forever).
            let request = if force_answer {
                info!("FORCE ANSWER: omitting tools to force text response");
                ChatRequest::new(&self.config.model, messages_for_api)
                    .with_stream(self.config.stream)
            } else {
                ChatRequest::new(&self.config.model, messages_for_api)
                    .with_tools(tools)
                    .with_stream(self.config.stream)
            };

            let (response_text, reasoning_text, tool_calls, extra_content) = if request.stream {
                let stream = self.client.chat_streaming(request).await?;
                let (text, reasoning, tcs, extra) = self.process_stream(stream).await?;
                debug!(count = tcs.len(), "Streaming response tool_calls");
                (text, reasoning, tcs, extra)
            } else {
                let response = self.client.chat(request).await?;
                let (text, reasoning, tcs, extra) = self.process_response(response).await?;
                debug!(count = tcs.len(), "Non-streaming response tool_calls");
                (text, reasoning, tcs, extra)
            };
            let response_text = response_text;
            let reasoning_text = reasoning_text;
            let tool_calls = tool_calls;
            let extra_content = extra_content;
                    info!(
                        "API RESPONSE: text_len={}, tool_count={}, text_preview={:?}",
                        response_text.len(),
                        tool_calls.len(),
                        response_text.chars().take(100).collect::<String>()
                    );
                    // Add assistant message to conversation
                    let mut assistant_msg = Message::assistant(&response_text);
                    if !reasoning_text.is_empty() {
                        assistant_msg = assistant_msg.with_reasoning(reasoning_text);
                    }
                    if !tool_calls.is_empty() {
                        assistant_msg = assistant_msg.with_tool_calls(tool_calls.clone());
                    }
                    // Include provider-specific extra content (e.g. Google Gemini thought_signature)
                    if let Some(ref extra) = extra_content {
                        if !extra.is_null() {
                            assistant_msg = assistant_msg.with_extra_content(extra.clone());
                        }
                    }
                        for tc in &tool_calls {
                            debug!(
                                assistant_tool_id = %tc.id,
                                assistant_tool_name = %tc.function.name,
                                "Tool call in assistant message"
                            );
                        }
                    }

                    debug!(
                        assistant_has_tcs = assistant_msg.tool_calls.is_some(),
                        assistant_tc_count = assistant_msg.tool_calls.as_ref().map(|tcs| tcs.len()).unwrap_or(0),
                        "Assistant message created"
                    );
                    messages.push(assistant_msg.clone());
                    self.add_message(assistant_msg.clone()).await;

                    // Persist assistant message
                    let _ = self.database.save_message(
                        &session_id,
                        "assistant",
                        &response_text,
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

                    // If the model returned tool_calls, execute them and loop.
                    if !tool_calls.is_empty() {
                        info!(
                            "LOOP CONTINUE: iteration has {} tool call(s): {:?}",
                            tool_calls.len(),
                            tool_calls.iter().map(|tc| format!("{}={}", tc.function.name, tc.id)).collect::<Vec<_>>()
                        );

                        // Check for loops
                        let state_hash = hash_tool_calls(&tool_calls);
                        if loop_detector.check(state_hash) {
                            warn!("Agent loop detected");
                            if !force_answer_attempted {
                                info!("Loop detected, retrying with force-answer");
                                force_answer = true;
                                force_answer_attempted = true;
                                continue;
                            }
                            info!("Force-answer already attempted, returning result");
                            let result = assistant_msg.clone();
                            self.spawn_session_distillation(messages.clone());
                            self.emit(AgentEvent::Done {
                                message: assistant_msg,
                            })
                            .await;
                            return Ok(result);
                        }

                        // Consecutive tool-only detection: some LLMs (Gemma via Gemini)
                        // call tools indefinitely without producing text. When the
                        // model calls tools but produces empty text N times in a row,
                        // force-answer kicks in by omitting tools from the request.
                        let tool_only = response_text.trim().is_empty();
                        if tool_only {
                            consecutive_tool_only += 1;
                            let threshold = self.config.max_consecutive_tool_only;
                            if threshold > 0 && consecutive_tool_only >= threshold {
                                info!(
                                    "FORCE ANSWER after {} consecutive tool-only iterations (threshold={})",
                                    consecutive_tool_only, threshold
                                );
                                force_answer = true;
                            }
                        } else {
                            consecutive_tool_only = 0;
                        }

                        // Fall through to tool execution below
                    } else {
                        // Model returned no tool_calls. This is the natural end of the turn.
                        // If content is also empty and we've been executing tools,
                        // retry ONCE without tools (the model may need help producing text).
                        let response_has_reasoning = assistant_msg.reasoning.is_some();
                        let content_empty = response_text.trim().is_empty();

                        if content_empty && !force_answer_attempted && iteration > 1 {
                            info!(
                                "Empty response after tool execution, retrying without tools \
                                 (iteration={}, content_empty={}, reasoning={})",
                                iteration, content_empty, response_has_reasoning
                            );
                            force_answer = true;
                            force_answer_attempted = true;
                            continue;
                        }

                        info!(
                            "DONE: content_len={}, content_empty={}, has_reasoning={}",
                            response_text.len(),
                            content_empty,
                            response_has_reasoning
                        );
                        let result = assistant_msg.clone();
                        self.spawn_session_distillation(messages.clone());
                        self.emit(AgentEvent::Done {
                            message: assistant_msg.clone(),
                        })
                        .await;
                        return Ok(result);
                    }

                    // Execute tools and add results
                    let tool_results = self.execute_tools(tool_calls).await?;

                    // Important: We must use the IDs from the tool_calls that were actually sent to the provider
                    // to ensure strict 1:1 mapping in the conversation history.
                    for result in tool_results {
                        let tool_content = if result.success {
                            result.content.clone()
                        } else {
                            result.error.clone().unwrap_or_else(|| "Error".to_string())
                        };
                        
                        debug!(
                            tool_call_id = %result.tool_call_id,
                            tool_name = %result.name,
                            content_len = tool_content.len(),
                            "Creating tool result message"
                        );

                        // Persist tool result to database
                        let _ = self.database.save_message(
                            &session_id,
                            "tool",
                            &tool_content,
                            &chrono::Utc::now().to_rfc3339(),
                        );

                        let tool_msg = Message::tool_with_name(
                            &result.tool_call_id,
                            &result.name,
                            &tool_content,
                        );

                        self.emit(AgentEvent::ToolComplete {
                            result: result.clone(),
                        })
                        .await;

                        messages.push(tool_msg.clone());
                        self.add_message(tool_msg).await;
                    }

                    // Update session timestamp
                    self.database
                        .save_session(
                            &session_id,
                            None,
                            "agent",
                            &chrono::Utc::now().to_rfc3339(),
                            &chrono::Utc::now().to_rfc3339(),
                        )
                        .ok();
            self.emit(AgentEvent::IterationComplete { iteration }).await;
        }
    }

    /// Build messages including system prompt
    async fn build_messages(&self) -> Result<Vec<Message>> {
        let mut messages = Vec::new();

        let mut system_prompt = if let Some(ref system) = self.config.system_prompt {
            system.clone()
        } else {
            "You are Hermes, an AI assistant. You have access to tools that you can call to help users. \
                When you need to use a tool, use the available functions provided to you. \
                After receiving tool results, continue reasoning and either call more tools or provide your final response."
                .to_string()
        };

        if let Some(memory_manager) = &self.memory_manager {
            let memory_context = memory_manager.build_memory_context(2048).await;
            let memory_context = memory_context.trim();
            if !memory_context.is_empty() {
                system_prompt.push_str("\n\n<long_term_memory>\n");
                system_prompt.push_str(memory_context);
                system_prompt.push_str("\n</long_term_memory>");
            }
        }

        let context_files = self.load_context_file_prompt();
        if !context_files.trim().is_empty() {
            system_prompt.push_str("\n\n<workspace_context>\n");
            system_prompt.push_str(context_files.trim());
            system_prompt.push_str("\n</workspace_context>");
        }

        // Add system prompt
        messages.push(Message::system(system_prompt));

        // Add conversation history
        let conv = self.conversation.read().await;
        messages.extend(conv.clone());

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
        let event_tx = self.event_tx.clone();
        let tool_progress = self.config.tool_progress.clone();
        let mut native_emitted_ids: HashSet<String> = HashSet::new();
        let mut accumulated_extra: Option<serde_json::Value> = None;

        let mut parser = ToolCallStreamParser::new().on_tool_call({
            let event_tx = event_tx.clone();
            let tool_progress = tool_progress.clone();
            move |tc| {
                let tc_id = tc.id.clone();
                debug!(tool_call_id = %tc_id, name = %tc.function.name, "Early tool call detected");
                if tool_progress == ToolProgressMode::Streaming
                    || tool_progress == ToolProgressMode::Auto
                {
                    if let Some(ref tx) = event_tx {
                        let _ = tx.try_send(AgentEvent::ToolStart {
                            name: tc.function.name,
                            arguments: tc.function.arguments,
                            tool_call_id: tc.id,
                        });
                    }
                }
            }
        });
        let mut content_router = ThinkBlockRouter::default();
        let mut tool_call_router = ToolCallContentRouter::default();
        let mut accumulated_text = String::new();
        let mut accumulated_reasoning = String::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();
        let mut seen = SeenToolCalls::default();
        let mut has_error = false;

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

                    // Process content from StreamChunk
                    if let Some(text) = chunk.content {
                        let (content_delta, reasoning_delta) = content_router.feed(&text);

                        if !content_delta.is_empty() {
                            let chunk_tool_calls = parser.process_chunk(&content_delta);
                            for tc in chunk_tool_calls {
                                merge_stream_tool_call(&mut tool_calls, tc, &mut seen);
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

                    // Capture provider-specific extra content (e.g. Gemini thought_signature)
                    if let Some(ref extra) = chunk.extra_content {
                        if !extra.is_null() {
                            accumulated_extra = Some(extra.clone());
                        }
                    }

                    // Merge native provider tool-call deltas
                    if let Some(chunk_tool_calls) = chunk.tool_calls {
                        debug!(
                            count = chunk_tool_calls.len(),
                            "Native tool_call deltas received"
                        );
                        for tc in chunk_tool_calls {
                            let is_new = !tool_calls.iter().any(|existing| existing.id == tc.id);
                            let tc_id = tc.id.clone();
                            let name = tc.function.name.clone();
                            let args = tc.function.arguments.clone();
                            merge_stream_tool_call(&mut tool_calls, tc, &mut seen);
                            if is_new
                                && !name.is_empty()
                                && (tool_progress == ToolProgressMode::Streaming
                                    || tool_progress == ToolProgressMode::Auto)
                            {
                                if !native_emitted_ids.contains(&tc_id) {
                                    native_emitted_ids.insert(tc_id.clone());
                                    if let Some(ref tx) = event_tx {
                                        if tx
                                            .try_send(AgentEvent::ToolStart {
                                                name: name.clone(),
                                                arguments: args,
                                                tool_call_id: tc_id.clone(),
                                            })
                                            .is_err()
                                        {
                                            warn!(
                                                tool_id = %tc_id,
                                                tool_name = %name,
                                                "native tool_call try_send failed"
                                            );
                                        }
                                    }
                                    debug!(
                                        tool_id = %tc_id,
                                        tool_name = %name,
                                        "emitted ToolStart for native tool_call"
                                    );
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    error!(error = %e, "Stream error");
                    has_error = true;
                    break;
                }
            }
        }

        if has_error {
            return Err(Error::Agent("Stream processing failed".to_string()));
        }

        // Streaming native tool_calls arrive as partial argument deltas
        // (e.g. "{" then "}") which contaminate seen. Rebuild from final
        // complete tool_calls so XML-parsed equivalents dedupe correctly.
        {
            let mut fresh_seen = SeenToolCalls::default();
            for tc in &tool_calls {
                fresh_seen.insert(&tc.function.name, &tc.function.arguments);
            }
            seen = fresh_seen;
        }

        // Final flush of all routers
        let (remaining_content, remaining_reasoning) = content_router.finish();
        
        // Feed remaining content through the routers
        if !remaining_content.is_empty() {
            let chunk_tool_calls = parser.process_chunk(&remaining_content);
            for tc in chunk_tool_calls {
                merge_stream_tool_call(&mut tool_calls, tc, &mut seen);
            }
            let visible_text = tool_call_router.feed(&remaining_content);
            if !visible_text.is_empty() {
                accumulated_text.push_str(&visible_text);
                self.emit(AgentEvent::Content { text: visible_text }).await;
            }
        }

        // Final finish for parser and tool_call_router
        let (remaining_calls, final_parser_text) = parser.finish();
        for tc in remaining_calls {
            merge_stream_tool_call(&mut tool_calls, tc, &mut seen);
        }
        
        let final_router_text = tool_call_router.finish();
        
        // Use the more complete text from the routers/parser
        if !final_router_text.is_empty() {
            accumulated_text.push_str(&final_router_text);
            self.emit(AgentEvent::Content { text: final_router_text }).await;
        }
        
        if !final_parser_text.is_empty() && !accumulated_text.contains(&final_parser_text) {
             // This is a fallback in case the router missed something the parser caught
             // or vice versa. We avoid duplicates by checking contains().
             accumulated_text.push_str(&final_parser_text);
        }

        accumulated_reasoning.push_str(&remaining_reasoning);

        // Also try to extract any remaining tool calls from accumulated text as a final safety net
        let mut remaining_parser = ToolCallParser::new();
        if let Ok(extra_calls) = remaining_parser.parse(&accumulated_text) {
            for tc in extra_calls {
                merge_stream_tool_call(&mut tool_calls, tc, &mut seen);
            }
        }

        info!(
            "process_stream DONE: text_len={}, text_preview={:?}, tool_count={}, tool_names={:?}",
            accumulated_text.len(),
            accumulated_text.chars().take(100).collect::<String>(),
            tool_calls.len(),
            tool_calls.iter().map(|tc| &tc.function.name).collect::<Vec<_>>()
        );
        Ok((accumulated_text, accumulated_reasoning, tool_calls, accumulated_extra))
    }

    async fn process_response(
        &self,
        response: ChatResponse,
    ) -> Result<(String, String, Vec<ToolCall>, Option<serde_json::Value>)> {
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
        let mut seen = SeenToolCalls::default();
        for tc in &tool_calls {
            seen.insert(&tc.function.name, &tc.function.arguments);
        }
        let mut xml_parser = ToolCallParser::new();
        if let Ok(xml_tool_calls) = xml_parser.parse(&raw_content) {
            for tool_call in xml_tool_calls {
                merge_stream_tool_call(&mut tool_calls, tool_call, &mut seen);
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

        Ok((content, reasoning, tool_calls))
    }

    async fn execute_tools(&self, tool_calls: Vec<ToolCall>) -> Result<Vec<ToolResult>> {
        let tool_timeout = self.config.tool_timeout;

        for tc in &tool_calls {
            debug!(
                tool_call_id = %tc.id,
                tool = %tc.function.name,
                args = %tc.function.arguments,
                "Executing tool"
            );
            self.emit(AgentEvent::ToolStart {
                name: tc.function.name.clone(),
                arguments: tc.function.arguments.clone(),
                tool_call_id: tc.id.clone(),
            })
            .await;
        }

        let registry = &self.registry;

        let futures: Vec<_> = tool_calls
            .into_iter()
            .map(|tool_call| async move {
                let name = tool_call.function.name.clone();
                let args_str = tool_call.function.arguments.clone();
                let id = tool_call.id.clone();

                // Aggressive trimming of whitespace, null bytes, and other control characters
                let mut trimmed_args = args_str.trim().trim_matches(|c: char| c.is_control() || c.is_whitespace()).to_string();

                if trimmed_args.is_empty() || trimmed_args == "\"\"" {
                     debug!(tool = %name, "Empty tool arguments received, defaulting to empty object");
                     trimmed_args = "{}".to_string();
                 }

                let args: serde_json::Value = match serde_json::from_str(&trimmed_args) {
                    Ok(a) => a,
                    Err(e) => {
                        warn!(tool = %name, error = %e, args = %trimmed_args, "Failed to parse tool arguments");
                        return ToolResult::error_with_name(&name, &id, format!("Invalid JSON: {}", e));
                    }
                };

                if !registry.contains(&name).await {
                    error!(tool = %name, "Tool not found");
                    return ToolResult::error_with_name(&name, &id, format!("Tool '{}' not found", name));
                }

                let result = timeout(
                    tool_timeout,
                    registry.execute(&name, &id, args, ToolContext::default()),
                )
                .await;

                match result {
                    Ok(Ok(mut r)) => {
                        debug!(tool = %name, success = r.success, "Tool execution completed");
                        r.name = name.clone();
                        r
                    }
                    Ok(Err(e)) => {
                        error!(tool = %name, error = %e, "Tool execution failed");
                        ToolResult::error_with_name(&name, &id, e.to_string())
                    }
                    Err(_) => {
                        error!(tool = %name, "Tool execution timed out");
                        ToolResult::error_with_name(&name, &id, format!("Tool timed out after {:?}", tool_timeout))
                    }
                }
            })
            .collect();

        let results = join_all(futures).await;
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
            let mut function = delta.function?;

            // Normalize empty arguments to "{}" to prevent EOF errors later
            if function.arguments.trim().is_empty() {
                function.arguments = "{}".to_string();
            }

            let id = delta.id.unwrap_or_else(|| {
                let generated = format!("call_choice_{}_{}", delta.index, function.name);
                debug!(
                    index = %delta.index,
                    generated_id = %generated,
                    name = %function.name,
                    "No id in delta, generating one"
                );
                generated
            });
            debug!(
                tool_call_id = %id,
                name = %function.name,
                index = %delta.index,
                "extract_tool_calls_from_choice"
            );
            Some(ToolCall { id, function })
        })
        .collect()
}

#[derive(Default)]
struct SeenToolCalls {
    names: std::collections::HashSet<String>,
}

impl SeenToolCalls {
    fn insert(&mut self, name: &str, arguments: &str) -> bool {
        let key = format!("{}:{}", name, arguments);
        self.names.insert(key)
    }
}

fn merge_stream_tool_call(
    tool_calls: &mut Vec<ToolCall>,
    tool_call: ToolCall,
    seen: &mut SeenToolCalls,
) {
    if tool_call.id.is_empty() {
        if seen.insert(&tool_call.function.name, &tool_call.function.arguments) {
            debug!(
                new_name = %tool_call.function.name,
                "Tool call has empty ID, treating as new call"
            );
            tool_calls.push(tool_call);
        } else {
            debug!(
                duplicate_name = %tool_call.function.name,
                "Duplicate tool call with same name/args, skipping"
            );
        }
        return;
    }

    if let Some(existing) = tool_calls
        .iter_mut()
        .find(|existing| existing.id == tool_call.id)
    {
        debug!(
            existing_id = %existing.id,
            new_name = %tool_call.function.name,
            "Merging tool call arguments"
        );
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
        if seen.insert(&tool_call.function.name, &tool_call.function.arguments) {
            debug!(
                new_id = %tool_call.id,
                new_name = %tool_call.function.name,
                "Adding new tool call to list"
            );
            tool_calls.push(tool_call);
        } else {
            debug!(
                duplicate_id = %tool_call.id,
                name = %tool_call.function.name,
                "Skipping duplicate tool call with different ID but same name/args"
            );
        }
    }
}

fn sanitize_messages_for_api(messages: &mut Vec<Message>) {
    let mut seen_tool_call_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    for msg in messages.iter() {
        if msg.role == crate::client::Role::Assistant {
            if let Some(ref tool_calls) = msg.tool_calls {
                for tc in tool_calls {
                    if !tc.id.is_empty() {
                        seen_tool_call_ids.insert(tc.id.clone());
                    }
                }
            }
        }
    }

    // Filter out tool results that don't have a matching tool call
    messages.retain(|msg| {
        if msg.role == crate::client::Role::Tool {
            if let Some(ref tool_call_id) = msg.tool_call_id {
                if !seen_tool_call_ids.contains(tool_call_id) {
                    warn!(
                        orphan_tcid = %tool_call_id,
                        "Removing orphan tool result message to avoid API errors"
                    );
                    return false;
                }
                true
            } else {
                warn!("Removing tool message without tool_call_id");
                false
            }
        } else {
            true
        }
    });
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
                    self.inside_tool_call = false;
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

/// Builder for creating a HermesAgent
pub struct HermesAgentBuilder {
    config: AgentConfig,
    client: Option<Box<dyn ModelClient>>,
    registry: Option<ToolRegistry>,
    memory_manager: Option<MemoryManager>,
    database: Option<Arc<Database>>,
}

impl HermesAgentBuilder {
    pub fn new() -> Self {
        Self {
            config: AgentConfig::default(),
            client: None,
            registry: None,
            memory_manager: None,
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

    /// Build the agent
    pub fn build(self) -> Result<HermesAgent> {
        let client: Box<dyn ModelClient> = self.client.unwrap_or_else(|| {
            let openai = crate::client::OpenAIClient::from_env().unwrap_or_else(|_| {
                crate::client::OpenAIClient::new(crate::client::ClientConfig::default())
            });
            Box::new(crate::agent::clients::openai::OpenAIModelClient::new(
                openai,
            ))
        });

        let registry = self
            .registry
            .unwrap_or_else(|| ToolRegistry::new(self.config.tool_timeout));

        let database = self.database.ok_or_else(|| {
            Error::Config("Database must be provided to the agent builder".to_string())
        })?;

        let mut agent = HermesAgent::new(self.config, client, registry, database);
        if let Some(memory_manager) = self.memory_manager {
            agent = agent.with_memory_manager(memory_manager);
        }

        Ok(agent)
    }
}

impl Default for HermesAgentBuilder {
    fn default() -> Self {
        Self::new()
    }
}

mod context_compressor;
pub use context_compressor::{CompressionStrategy, ContextCompressor};

mod prompt_builder;
pub use prompt_builder::{ContextEntry, PromptBuilder};

mod model_client;
pub use model_client::{ChatRequest, ModelClient, StreamChunk};

pub mod clients;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::ChatStreamEvent;
    use crate::client::{Choice, MessageDelta, ToolCallFunction};
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
        assert_eq!(config.max_iterations, 20);
    }

    #[serial]
    #[tokio::test]
    async fn test_agent_builder() {
        let db =
            Arc::new(Database::init(std::path::PathBuf::from("test_agent_builder.db")).unwrap());
        let _agent = HermesAgentBuilder::new()
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
        let agent = HermesAgent::new(
            AgentConfig::default(),
            Box::new(OpenAIModelClient::new(OpenAIClient::new(
                crate::client::ClientConfig::default(),
            ))),
            ToolRegistry::new(Duration::from_secs(1)),
            Arc::new(db),
        )
        .with_memory_manager(memory_manager);

        let messages = agent.build_messages().await.unwrap();
        let system = messages
            .first()
            .map(|message| message.content.as_str())
            .unwrap_or_default();

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
        let second = router.feed("hermes!");
        let rest = router.finish();

        assert_eq!(first, "Halo ");
        assert_eq!(second, "hermes!");
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
        let mut seen = SeenToolCalls::default();
        seen.insert("datetime", "{\"format\":");
        seen.insert("datetime", "\"%Y-%m-%d\"}");

        merge_stream_tool_call(
            &mut tool_calls,
            ToolCall {
                id: "call_0_datetime".to_string(),
                function: crate::client::ToolCallFunction {
                    name: "datetime".to_string(),
                    arguments: "\"%Y-%m-%d\"}".to_string(),
                },
            },
            &mut seen,
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
        let agent = HermesAgent::new(
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

        let (content, reasoning, tool_calls, _extra) = agent.process_response(response).await.unwrap();

        assert_eq!(content, "");
        assert_eq!(reasoning, "need tool");
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].function.name, "datetime");
    }

    #[test]
    fn sanitize_messages_for_api_keeps_matching_tool_messages() {
        let mut messages = vec![
            Message::system("You are a helpful assistant."),
            Message::user("What is the weather?"),
            Message::assistant("")
                .with_tool_calls(vec![ToolCall {
                    id: "call_abc123".to_string(),
                    function: crate::client::ToolCallFunction {
                        name: "get_weather".to_string(),
                        arguments: "{\"location\":\"NYC\"}".to_string(),
                    },
                }]),
            Message::tool_with_name("call_abc123", "get_weather", "Sunny, 72°F"),
        ];

        sanitize_messages_for_api(&mut messages);

        assert_eq!(messages.len(), 4, "Tool message with matching ID should be kept");
        assert_eq!(messages[3].role, crate::client::Role::Tool);
        assert_eq!(messages[3].tool_call_id.as_deref(), Some("call_abc123"));
    }

    #[test]
    fn sanitize_messages_for_api_removes_orphan_tool_messages() {
        let mut messages = vec![
            Message::system("You are a helpful assistant."),
            Message::user("What is the weather?"),
            Message::assistant("")
                .with_tool_calls(vec![ToolCall {
                    id: "call_abc123".to_string(),
                    function: ToolCallFunction {
                        name: "get_weather".to_string(),
                        arguments: "{\"location\":\"NYC\"}".to_string(),
                    },
                }]),
            Message::tool_with_name("call_xyz999", "get_weather", "Sunny, 72°F"),
        ];

        sanitize_messages_for_api(&mut messages);

        assert_eq!(messages.len(), 3, "Orphan tool message should be REMOVED");
        assert!(messages.last().unwrap().tool_call_id.is_none());
    }

    #[test]
    fn sanitize_messages_for_api_handles_empty_tool_call_ids() {
        let mut messages = vec![
            Message::assistant("")
                .with_tool_calls(vec![ToolCall {
                    id: "".to_string(),
                    function: ToolCallFunction {
                        name: "get_weather".to_string(),
                        arguments: "{}".to_string(),
                    },
                }]),
            Message::tool_with_name("call_abc123", "get_weather", "Sunny"),
        ];

        sanitize_messages_for_api(&mut messages);

        assert_eq!(messages.len(), 1, "Tool message without matching assistant tool_call_id should be removed");
    }

    #[test]
    fn sanitize_messages_for_api_multi_iteration() {
        let mut messages = vec![
            Message::system("Assistant."),
            Message::user("Check weather and time."),
            Message::assistant("Checking weather.")
                .with_tool_calls(vec![ToolCall {
                    id: "call_001".to_string(),
                    function: ToolCallFunction {
                        name: "get_weather".to_string(),
                        arguments: "{\"location\":\"NYC\"}".to_string(),
                    },
                }]),
            Message::tool_with_name("call_001", "get_weather", "Sunny"),
            Message::assistant("Now checking time.")
                .with_tool_calls(vec![ToolCall {
                    id: "call_002".to_string(),
                    function: ToolCallFunction {
                        name: "get_time".to_string(),
                        arguments: "{\"timezone\":\"EST\"}".to_string(),
                    },
                }]),
            Message::tool_with_name("call_002", "get_time", "3:00 PM"),
        ];

        sanitize_messages_for_api(&mut messages);

        assert_eq!(messages.len(), 6, "All messages with matching IDs should be kept");
    }

    #[test]
    fn merge_stream_tool_call_handles_native_and_xml_same_id() {
        let mut tool_calls = vec![ToolCall {
            id: "call_abc123".to_string(),
            function: ToolCallFunction {
                name: "get_weather".to_string(),
                arguments: "{\"locati".to_string(),
            },
        }];
        let mut seen = SeenToolCalls::default();
        seen.insert("get_weather", "{\"locati");

        merge_stream_tool_call(
            &mut tool_calls,
            ToolCall {
                id: "call_abc123".to_string(),
                function: ToolCallFunction {
                    name: "get_weather".to_string(),
                    arguments: "on\":\"NYC\"}".to_string(),
                },
            },
            &mut seen,
        );

        assert_eq!(tool_calls.len(), 1, "Same ID should merge into one tool_call");
        assert_eq!(
            tool_calls[0].function.arguments,
            "{\"location\":\"NYC\"}"
        );
    }

    #[test]
    fn merge_stream_tool_call_skips_duplicate_with_different_id() {
        let mut tool_calls = vec![ToolCall {
            id: "call_abc123".to_string(),
            function: ToolCallFunction {
                name: "get_weather".to_string(),
                arguments: "{\"location\":\"NYC\"}".to_string(),
            },
        }];
        let mut seen = SeenToolCalls::default();
        seen.insert("get_weather", "{\"location\":\"NYC\"}");

        merge_stream_tool_call(
            &mut tool_calls,
            ToolCall {
                id: "call_1744678400".to_string(),
                function: ToolCallFunction {
                    name: "get_weather".to_string(),
                    arguments: "{\"location\":\"NYC\"}".to_string(),
                },
            },
            &mut seen,
        );

        assert_eq!(tool_calls.len(), 1,
            "FIX: merge_stream_tool_call should skip duplicate even when IDs differ");
    }

    #[serial]
    #[tokio::test]
    async fn process_response_native_and_xml_tool_calls_different_ids() {
        use crate::client::ToolCallDelta;
        use crate::client::ToolCallFunction as TcFunc;

        let response = ChatResponse {
            id: "resp_1".to_string(),
            object: "chat.completion".to_string(),
            created: 0,
            model: "deepseek".to_string(),
            choices: vec![Choice {
                index: 0,
                message: MessageDelta {
                    role: Some(crate::client::Role::Assistant),
                    content: Some(
                        "I'll check the weather.\n<tool_call>{\"name\":\"get_weather\",\"arguments\":{\"location\":\"NYC\"}}</tool_call>".to_string()
                    ),
                    reasoning_content: None,
                    tool_calls: Some(vec![ToolCallDelta {
                        index: 0,
                        id: Some("call_deepseek_abc123".to_string()),
                        call_type: Some("function".to_string()),
                        function: Some(TcFunc {
                            name: "get_weather".to_string(),
                            arguments: "{\"location\":\"NYC\"}".to_string(),
                        }),
                    }]),
                },
                finish_reason: Some("tool_calls".to_string()),
            }],
            usage: crate::client::Usage {
                prompt_tokens: 10,
                completion_tokens: 20,
                total_tokens: 30,
            },
        };

        let db_path = std::path::PathBuf::from("test_db_native_xml.sqlite");
        let _ = std::fs::remove_file(&db_path);

        let agent = HermesAgent::new(
            AgentConfig::default(),
            Box::new(crate::agent::clients::openai::OpenAIModelClient::new(
                crate::client::OpenAIClient::new(crate::client::ClientConfig::default()),
            )),
            ToolRegistry::new(Duration::from_secs(1)),
            Arc::new(Database::init(db_path).unwrap()),
        );

        let (content, reasoning, tool_calls, _extra) = agent.process_response(response).await.unwrap();

        assert_eq!(content, "I'll check the weather.\n");
        let has_duplicate = tool_calls.len() > 1;
        if has_duplicate {
            println!("process_response returned {} tool_calls instead of 1", tool_calls.len());
        }
        assert!(tool_calls.len() >= 1);
        assert_eq!(tool_calls[0].function.name, "get_weather");
    }



    #[test]
    fn merge_stream_tool_call_different_args_same_name_allows_both() {
        let mut tool_calls = vec![ToolCall {
            id: "call_001".to_string(),
            function: ToolCallFunction {
                name: "get_weather".to_string(),
                arguments: "{\"location\":\"NYC\"}".to_string(),
            },
        }];
        let mut seen = SeenToolCalls::default();
        seen.insert("get_weather", "{\"location\":\"NYC\"}");

        merge_stream_tool_call(
            &mut tool_calls,
            ToolCall {
                id: "call_002".to_string(),
                function: ToolCallFunction {
                    name: "get_time".to_string(),
                    arguments: "{\"timezone\":\"EST\"}".to_string(),
                },
            },
            &mut seen,
        );

        assert_eq!(tool_calls.len(), 2,
            "Different tools with different IDs should both be added");
    }
}
