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

        // Phase D (docs/agentic-os/, H4): a Holon run hands the kernel what the
        // gold standard carries — trajectory.json in ITS shape (steps of
        // thought/tool/args/outcome; the kernel compacts to 12) and, when the
        // agent already wrote its result.json, the usage only the driver
        // truthfully knows (tokens estimate, tool_calls, duration). The kernel's
        // set_run_usage coalesces: this measurement wins over the envelope's
        // claim. A personal run (no HOLON_RUN_DIR) keeps today's file untouched.
        if let Some(run_dir) = crate::platform::holon_run_dir() {
            self.write_holon_run_files(run_dir, &trajectory, messages);
        }

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

    /// Phase D (H4): write the kernel-facing run files. Never fatal — the
    /// kernel drops a malformed trajectory and coalesces usage from its own
    /// measurement when these are missing; the result envelope still decides
    /// the run.
    fn write_holon_run_files(
        &self,
        run_dir: std::path::PathBuf,
        trajectory: &crate::trajectory::Trajectory,
        _messages: &[Message],
    ) {
        // trajectory.json in the kernel's shape (one definition:
        // crate::trajectory::holon_trajectory_json — also unit-tested there)
        let traj = crate::trajectory::holon_trajectory_json(trajectory);
        if let Ok(text) = serde_json::to_string(&traj)
            && let Err(e) = std::fs::write(run_dir.join("trajectory.json"), text)
        {
            warn!(error = %e, "failed to write trajectory.json");
        }

        // usage (O-21): the driver's measurement always lands in the run
        // dir FIRST — the kernel reads usage.json on every stage (H-04),
        // including a body that submitted through the kernel and never
        // wrote result.json. duration_s stays the kernel's own run-row
        // measurement; the driver leaves it null.
        // O-19: the numbers are the PROVIDER's, metered per call in
        // emit_usage_and_cost (prompt/completion reported by the API),
        // with cost from the session accumulator (models_dev pricing when
        // the catalog has the model, else null) — never the chars/4
        // estimate the audit flagged (input undercounted by ~turns).
        let tokens_in = self
            .holon_tokens_in
            .load(std::sync::atomic::Ordering::Relaxed);
        let tokens_out = self
            .holon_tokens_out
            .load(std::sync::atomic::Ordering::Relaxed);
        let cost = self
            .session_cost_usd
            .read()
            .map(|c| if *c > 0.0 { Some(*c) } else { None })
            .unwrap_or(None);
        let usage = serde_json::json!({
            "tokens_in": tokens_in, "tokens_out": tokens_out,
            "cost": cost,
            "tool_calls": trajectory.tool_calls,
            "build": crate::BUILD_STAMP,
        });
        if let Err(e) = std::fs::write(
            run_dir.join("usage.json"),
            serde_json::to_string(&usage).unwrap_or_default(),
        ) {
            warn!(error = %e, "failed to write usage.json");
        }

        // then merge usage into the agent's result.json when it wrote one
        // (set_run_usage prefers the driver either way)
        // O-17: the recorded refusals merge into the result — the
        // kernel treats them as blockers, not breaches (H-03).
        let refusals = crate::org::read_refusals(&run_dir);

        let result_path = run_dir.join("result.json");
        let Ok(existing) = std::fs::read_to_string(&result_path) else {
            return;
        };
        let Ok(mut env) = serde_json::from_str::<serde_json::Value>(&existing) else {
            warn!(path = %result_path.display(), "result.json is not JSON — usage left unpatched");
            return;
        };
        let Some(obj) = env.as_object_mut() else {
            return;
        };
        if !refusals.is_empty() {
            obj.insert("refusals".to_string(), serde_json::Value::Array(refusals));
        }
        // W6: learning annotations (skill_changes / learnings / proposals)
        // merge the same way — O-22's contract is the result envelope.
        let learning: Vec<serde_json::Value> = crate::org::read_learning_annotations(&run_dir);
        if !learning.is_empty() {
            obj.insert("learnings".to_string(), serde_json::Value::Array(learning));
        }

        obj.insert(
            "usage".to_string(),
            serde_json::json!({
                "tokens_in": tokens_in,
                "tokens_out": tokens_out,
                "cost": cost,
                "tool_calls": trajectory.tool_calls,
                "duration_s": null,
            }),
        );
        if let Ok(text) = serde_json::to_string_pretty(&env)
            && let Err(e) = std::fs::write(&result_path, text)
        {
            warn!(error = %e, "failed to patch result.json usage");
        } else {
            info!(path = %result_path.display(), "driver usage patched into result.json");
        }
    }

    pub(crate) fn spawn_session_distillation(&self, history: Vec<Message>) {
        // (plan 11C3) no memory_manager gate anymore — distillation writes
        // through the process's memory-wire provider (checked inside).
        //
        // A Holon body does not distill (W6/O-22's sixth learner — caught
        // by the multi-turn e2e, which saw 3 provider calls and 2 metered):
        // the organization's memory is Holon's (boundary S1/S3 — the same
        // rule that gates skill_manage, the curator, memory_store and the
        // review nudges), and the post-turn distillation LLM call would be
        // run spend that neither usage.json nor the budget cap ever sees.
        // The run's learnings ride the result envelope (W6's capture), not
        // the personal memory store.
        if crate::platform::holon_run_dir().is_some() {
            return;
        }
        let client = self.client.clone();
        let model = self.config.model.clone();
        tokio::spawn(async move {
            if let Err(error) = distill_session_to_memory(client, model, history).await {
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

        crate::daemon_pool::spawn(
            "probe-extra",
            crate::write_origin::scope_background_review(async move {
                debug!(
                    session_id = %session_id,
                    review_model = %review_model,
                    is_routed,
                    review_skills,
                    review_memory,
                    "Background review daemon started"
                );

                // ── Write origin context (Phase 2) ────────────────
                // The write origin is "background_review" for THIS TASK ONLY
                // (task-local scope, R39-10 residual closed): the skills_tool
                // write guards know this is a review session, so the review
                // agent cannot modify protected (bundled) or hub-installed
                // skills — while a concurrent main-agent turn (a different
                // Tokio task) keeps reading its own origin and is not
                // write-guarded. Matches hermes-agent's
                // _memory_write_origin = "background_review" pattern.
                crate::tools::skills_tool::reset_review_read_marks();

                // ── Prompt cache parity (Phase 2) ─────────────────────
                // When parent_frozen_prefix is available (same model, not routed),
                // use the parent's EXACT system prompt bytes so the outbound HTTP
                // request hits the same provider prefix cache. The review-specific
                // instructions go in a USER message, not the system message — this
                // ensures the system prompt bytes stay byte-identical.
                // Matches hermes-agent's `_cached_system_prompt` pinning pattern.
                let (system_prompt_str, review_harness) = if let Some(ref frozen) =
                    parent_frozen_prefix
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
                // S3 no-op early exit: consecutive rounds whose successful
                // tool calls wrote NOTHING (reads only — memory_search /
                // memory_recall / skill_view) count toward an early stop;
                // any successful write resets the streak. The cap above
                // stays the outer bound — a daemon that writes every round
                // runs to it unchanged.
                let mut no_write_rounds = 0usize;

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
                    let mut round_wrote = false;
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
                                    if background_review::is_review_write_tool(tool_name) {
                                        round_wrote = true;
                                    }
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
                                review_messages
                                    .push(Message::tool(tool_id, error_result.to_string()));
                            }
                        }
                    }

                    // ── S3 no-op early exit ────────────────────────────
                    // A round with zero successful WRITES counts toward the
                    // early stop; any write resets the streak. Two
                    // consecutive read-only rounds means the daemon found
                    // nothing to act on — stop before the iteration cap
                    // instead of burning another provider call (the
                    // `{"count":0}` specimen).
                    if round_wrote {
                        no_write_rounds = 0;
                    } else {
                        no_write_rounds += 1;
                    }
                    if background_review::stop_after_no_write_rounds(no_write_rounds) {
                        info!(
                            session_id = %session_id,
                            iteration = review_iter + 1,
                            no_write_rounds,
                            "Background review exiting early — consecutive rounds with no writes"
                        );
                        break;
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
                                session_id: session_id.clone(),
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
            }),
        );
    }
}
