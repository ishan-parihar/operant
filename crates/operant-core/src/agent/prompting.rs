//! `prompting` — method-group impl block extracted verbatim from agent/mod.rs.

use crate::client::{Message, ToolCall, ToolCallFunction};
use crate::config::runtime_config;
use crate::context_files::{load_default_context_files, load_workspace_context};
use crate::distillation::distill_session_to_memory;
use crate::tools::ToolContext;
use tracing::{debug, info, warn};

use super::*;

impl OperantAgent {
    pub(crate) fn load_context_file_prompt(&self) -> String {
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
    pub(crate) async fn save_trajectory(
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

    pub(crate) fn spawn_session_distillation(&self, history: Vec<Message>) {
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

    /// Spawn a background review daemon — a lightweight tokio task that
    /// replays the conversation snapshot through the LLM with a review
    /// prompt. Matches hermes-agent's `spawn_background_review_thread`
    /// pattern.
    ///
    /// The review agent:
    /// 1. Receives the conversation snapshot + a review prompt.
    /// 2. Gets tool schemas so it can call skill_manage / memory tools.
    /// 3. Writes go straight to stores; main conversation is untouched.
    /// 4. Results are logged but don't block the main loop.
    ///
    /// ## Tool Whitelist
    ///
    /// The review agent only gets memory and skill tools — never the full
    /// tool registry. This prevents the review from accidentally executing
    /// dangerous tools (terminal, file_write, etc.) and matches hermes-agent's
    /// `set_thread_tool_whitelist` pattern.
    ///
    /// ## Prompt Cache Reuse
    ///
    /// When running on the same model (not routed), the review agent shares
    /// the parent's warm cached system prompt so the outbound HTTP request
    /// hits the same Anthropic/OpenRouter prefix cache (~26% cost reduction).
    /// When routed to a different model, a compact digest replay minimizes
    /// cold-written tokens.
    ///
    /// ## Persistence Isolation
    ///
    /// The review agent does NOT write to the user's session database.
    /// All DB writes are skipped — the review only writes to memory and
    /// skill stores via its tools.
    pub(crate) async fn spawn_background_review(
        &self,
        messages: &[Message],
        session_id: &str,
        review_skills: bool,
        review_memory: bool,
    ) {
        use self::background_review::{build_review_prompt, digest_history};

        let prompt = build_review_prompt(review_memory, review_skills);
        let client = self.client.clone();
        let model = self.config.model.clone();
        let session_id = session_id.to_string();

        // ── Resolve auxiliary model for background review ──────────────
        // Check if the user configured an auxiliary model for background
        // reviews. If so, route the review to that model instead of the
        // main model. Different model = cold cache → use digest replay.
        let cfg = runtime_config();
        let (review_model, is_routed) = if let Some(aux) = cfg.auxiliary_models.memory.as_ref() {
            if let Some(ref aux_model) = aux.model {
                if aux_model != &model {
                    (aux_model.clone(), true)
                } else {
                    (model.clone(), false)
                }
            } else {
                (model.clone(), false)
            }
        } else {
            (model.clone(), false)
        };

        // ── Snapshot the conversation ─────────────────────────────────
        // Limit to last 40 messages to keep token usage reasonable.
        let start = messages.len().saturating_sub(40);
        let snapshot: Vec<Message> = messages[start..].to_vec();

        // ── Tool whitelist: only memory + skill tools ─────────────────
        // The review agent should ONLY have access to memory and skill
        // management tools. Never terminal, file_write, browser, etc.
        // This matches hermes-agent's `set_thread_tool_whitelist` pattern.
        let review_tool_names: Vec<String> = vec![
            "memory_store".to_string(),
            "memory_search".to_string(),
            "memory_recall".to_string(),
            "skill_manage".to_string(),
            "skill_view".to_string(),
        ];
        let tools = self
            .registry
            .get_available_schemas_filtered(&review_tool_names)
            .await;

        // ── Cache-aware replay selection ──────────────────────────────
        // Same model → full replay (warm cache reads, cheapest).
        // Different model → digest replay (cold cache, minimize tokens).
        let review_history = if is_routed {
            debug!(
                routed_model = %review_model,
                "Review routed to auxiliary model — using digest replay"
            );
            digest_history(&snapshot, 24)
        } else {
            snapshot.clone()
        };

        let registry_for_review = self.registry.clone();
        let callback = self.background_review_callback.clone();
        let event_tx = self.event_tx.clone();

        // ── Prompt cache parity (Phase 2) ─────────────────────────
        // When the review runs on the SAME model (not routed), share the
        // parent's frozen prefix (system prompt + skills) so the outbound
        // HTTP request hits the same provider prefix cache. The review
        // instructions go in a USER message, not the system message, so
        // the system prompt bytes stay byte-identical to the parent's.
        // Matches hermes-agent's `_cached_system_prompt` pinning pattern.
        //
        // When routed to a different model, the cache key differs anyway,
        // so no benefit to sharing — use None.
        let parent_frozen_prefix: Option<String> = if !is_routed {
            Some(self.build_frozen_prefix())
        } else {
            None
        };

        tokio::spawn(async move {
            debug!(
                session_id = %session_id,
                review_model = %review_model,
                is_routed,
                review_skills,
                review_memory,
                "Background review daemon started"
            );

            // ── Write origin context (Phase 2) ────────────────────
            // Set the write origin to "background_review" so the
            // skills_tool write guards know this is a review session.
            // This prevents the review agent from modifying protected
            // (bundled) or hub-installed skills. Matches hermes-agent's
            // _memory_write_origin = "background_review" pattern.
            let _origin_token = crate::write_origin::set_write_origin("background_review");
            crate::tools::skills_tool::reset_review_read_marks();

            // ── Prompt cache parity (Phase 2) ─────────────────────
            // When parent_frozen_prefix is available (same model, not routed),
            // use the parent's EXACT system prompt bytes so the outbound HTTP
            // request hits the same provider prefix cache. The review-specific
            // instructions go in a USER message, not the system message — this
            // ensures the system prompt bytes stay byte-identical.
            // Matches hermes-agent's `_cached_system_prompt` pinning pattern.
            let (system_prompt_str, review_harness) = if let Some(ref frozen) = parent_frozen_prefix
            {
                (
                    frozen.clone(),
                    format!(
                        "[Background review context]\n\nYou are a background review agent. Your job is to evaluate the \
conversation above and update skills and/or memory as needed. \
You have access to memory_store, memory_search, memory_recall, \
skill_manage, and skill_view tools only — do not attempt other tools. \
NEVER continue, execute, or complete the user's task from the conversation — \
scheduling, coding, messaging and similar actions belong exclusively to the main agent. \
Your only outputs are memory/skill updates. \
Be ACTIVE — most sessions produce at least one update. \
If nothing needs updating, say 'Nothing to save.' and stop.\n\n{}",
                        prompt
                    ),
                )
            } else {
                (
                    "You are Operant, a helpful AI assistant.".to_string(),
                    format!(
                        "You are a background review agent. Your job is to evaluate the \
conversation above and update skills and/or memory as needed. \
You have access to memory_store, memory_search, memory_recall, \
skill_manage, and skill_view tools only — do not attempt other tools. \
NEVER continue, execute, or complete the user's task from the conversation — \
scheduling, coding, messaging and similar actions belong exclusively to the main agent. \
Your only outputs are memory/skill updates. \
Be ACTIVE — most sessions produce at least one update. \
If nothing needs updating, say 'Nothing to save.' and stop.\n\n{}",
                        prompt
                    ),
                )
            };

            // Build messages: identical system prompt + review harness as user msg + snapshot
            let mut review_messages = Vec::new();
            review_messages.push(Message::system(&system_prompt_str));
            review_messages.push(Message::user(&review_harness));
            review_messages.extend(review_history);

            // ── Multi-turn tool execution loop ────────────────────────
            // Run up to MAX_REVIEW_ITERATIONS iterations to allow the review
            // agent to execute tools and see their results. This matches
            // hermes-agent's forked AIAgent.run_conversation() pattern.
            const MAX_REVIEW_ITERATIONS: usize = 5;
            let mut actions_taken: Vec<String> = Vec::new();

            for review_iter in 0..MAX_REVIEW_ITERATIONS {
                debug!(
                    iteration = review_iter + 1,
                    max = MAX_REVIEW_ITERATIONS,
                    session_id = %session_id,
                    "Background review iteration"
                );

                // Create the review chat request (non-streaming for background)
                let request = ChatRequest::new(review_model.clone(), review_messages.clone())
                    .with_tools(tools.clone())
                    .with_stream(false);

                let response = match client.chat(request).await {
                    Ok(r) => r,
                    Err(e) => {
                        warn!(
                            error = %e,
                            session_id = %session_id,
                            iteration = review_iter + 1,
                            "Background review agent failed at LLM call"
                        );
                        break;
                    }
                };

                // Extract assistant message from response
                let assistant_msg = match response.choices.first() {
                    Some(choice) => choice.message.clone(),
                    None => {
                        warn!("Background review: no choices in response");
                        break;
                    }
                };

                // Check if the model wants to stop (no tool calls)
                let tool_calls_deltas = assistant_msg.tool_calls.clone().unwrap_or_default();
                let content = assistant_msg.content.clone().unwrap_or_default();

                // If the model says "Nothing to save" or has no tool calls, we're done
                if content.contains("Nothing to save") || tool_calls_deltas.is_empty() {
                    if content.contains("Nothing to save") {
                        debug!("Background review: nothing to save");
                    } else if !content.is_empty() {
                        // Model provided a summary without tool calls
                        let preview: String = content.chars().take(200).collect();
                        info!(
                            session_id = %session_id,
                            response_preview = %preview,
                            "Background review completed with summary"
                        );
                    }
                    break;
                }

                // Add assistant message to review conversation
                let mut assistant_message = Message::assistant(&content);
                if !tool_calls_deltas.is_empty() {
                    // Convert ToolCallDelta to ToolCall for the message
                    let tool_calls: Vec<ToolCall> = tool_calls_deltas
                        .iter()
                        .filter_map(|delta| {
                            let function = delta.function.as_ref()?;
                            let id = delta.id.clone().unwrap_or_else(|| {
                                format!("bg-review-{}-{}", review_iter, delta.index)
                            });
                            Some(ToolCall {
                                id,
                                function: ToolCallFunction {
                                    name: function.name.clone(),
                                    arguments: function.arguments.clone(),
                                },
                            })
                        })
                        .collect();
                    assistant_message = assistant_message.with_tool_calls(tool_calls);
                }
                review_messages.push(assistant_message);

                // ── Execute whitelisted tools ─────────────────────────
                // Only execute tools that are in our whitelist. This matches
                // hermes-agent's set_thread_tool_whitelist pattern.
                for tool_call_delta in &tool_calls_deltas {
                    // Extract function info from the delta
                    let function = match &tool_call_delta.function {
                        Some(f) => f,
                        None => continue,
                    };
                    let tool_name = &function.name;
                    let args_str = &function.arguments;
                    let tool_id = tool_call_delta.id.as_deref().unwrap_or("unknown");

                    // Check if tool is in whitelist
                    if !review_tool_names.contains(tool_name) {
                        warn!(
                            tool = %tool_name,
                            "Background review attempted non-whitelisted tool"
                        );
                        let error_result = serde_json::json!({
                            "success": false,
                            "error": format!("Tool '{}' is not allowed in background review. Only memory and skill tools are permitted.", tool_name)
                        });
                        review_messages.push(Message::tool(tool_id, error_result.to_string()));
                        continue;
                    }

                    debug!(
                        tool = %tool_name,
                        args = %args_str,
                        "Background review executing tool"
                    );

                    // Parse arguments
                    let args: serde_json::Value = serde_json::from_str(args_str)
                        .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

                    // Execute the tool using the registry
                    let tool_result = registry_for_review
                        .execute(tool_name, tool_id, args, ToolContext::default())
                        .await;

                    match tool_result {
                        Ok(result) => {
                            let result_str = if result.success {
                                result.content.clone()
                            } else {
                                format!(
                                    "{{\"success\": false, \"error\": \"{}\"}}",
                                    result.error.unwrap_or_else(|| "Unknown error".to_string())
                                )
                            };

                            // Track actions taken for summary
                            if result.success {
                                let action_summary = format!(
                                    "{}: {}",
                                    tool_name,
                                    result_str.chars().take(100).collect::<String>()
                                );
                                actions_taken.push(action_summary);
                            }

                            review_messages.push(Message::tool(tool_id, &result_str));
                        }
                        Err(e) => {
                            warn!(
                                tool = %tool_name,
                                error = %e,
                                "Background review tool execution failed"
                            );
                            let error_result = serde_json::json!({
                                "success": false,
                                "error": format!("Tool execution failed: {}", e)
                            });
                            review_messages.push(Message::tool(tool_id, error_result.to_string()));
                        }
                    }
                }
            }

            // ── Summarize actions taken ──────────────────────────────
            // Surface a compact summary to the user via tracing AND callback.
            // Matches hermes-agent's _safe_print + background_review_callback pattern.
            if !actions_taken.is_empty() {
                let summary = actions_taken.join(" · ");
                let notification = format!("💾 Self-improvement review: {}", summary);
                info!(
                    session_id = %session_id,
                    actions = %summary,
                    action_count = actions_taken.len(),
                    "Background review completed with updates"
                );
                // Deliver via callback (TUI/Gateway wired via with_background_review_callback)
                // AND via AgentEvent so TUI/CLI surfaces it without needing the callback wired.
                if let Some(ref cb) = callback {
                    cb(notification.clone());
                }
                if let Some(ref tx) = event_tx {
                    let _ = tx
                        .send(AgentEvent::BackgroundReview {
                            summary: notification,
                        })
                        .await;
                }
            } else {
                debug!(
                    session_id = %session_id,
                    "Background review completed — no actions taken"
                );
            }

            debug!(session_id = %session_id, "Background review daemon finished");
        });
    }
}
