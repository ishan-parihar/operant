//! `prompting` — method-group impl block extracted verbatim from agent/mod.rs.

use crate::client::{Message, ToolCall, ToolCallFunction};
use crate::config::runtime_config;
use crate::context_files::{load_default_context_files, load_workspace_context};
use crate::distillation::distill_session_to_memory;
use crate::tools::ToolContext;
use tracing::{debug, info, warn};

use super::*;

/// How many trajectory files are kept.
///
/// A trajectory is written per RUN, and the file name is
/// `<session_id>_<unix-timestamp>.json`, so a long-lived session that runs
/// many times accumulates one file per run. Nothing pruned them: the only
/// cleanup was `operant trajectory clear`, which is manual and deletes
/// everything. On a machine that records every run (a Holon run always
/// records — `is_holon_run()` forces it on), the directory grows without
/// bound.
///
/// The cap follows the checkpoint store's precedent (`max_snapshots`, also
/// 20): keep the newest N, drop the oldest. It IS a config key
/// (`[agent] max_trajectories`, 0 = keep all) rather than a constant,
/// because `operant trajectory export` exists to collect these files as
/// prompt-completion pairs for fine-tuning — so silently deleting the
/// oldest ones destroys exactly the corpus that command is for. The
/// default is 0 (keep all): the operator opts into a bound.
#[cfg(test)]
const MAX_TRAJECTORIES_KEPT: usize = 200;

/// What a completed review should SURFACE, given what it did.
///
/// A pure function because the arm it decides has no test seam: the completion
/// path lives inside an async daemon, and a guard written against live state
/// there cannot be driven from a test.
///
/// The rule has two halves, and the second is the one that was missing. A review
/// that wrote something reports what it wrote. A review that wrote NOTHING still
/// reports what it SAID, because the memory lane cannot write at all:
/// `MEMORY_REVIEW_PROMPT` instructs it to return a `Memory candidate: <fact>` line
/// for a foreground turn to act on, so its final text IS its output. Only an
/// empty report on both sides surfaces nothing.
/// Which iteration should carry the "you are out of budget, report now" message.
///
/// Extracted because the arm that uses it lives inside an async daemon and has no
/// test seam -- and it shipped WRONG for exactly that reason: it fired on the LAST
/// iteration, whose request is built and then never sent, so the model never saw
/// the instruction and the "one turn to report" was zero turns.
///
/// The message must ride the SECOND-TO-LAST iteration, so the last iteration's
/// request carries it and the model answers after being told to stop. A cap of 0 or
/// 1 has no such index, and `None` is the honest answer there: with a single
/// iteration there is no earlier request to carry a warning, and warning the model
/// in the same breath as the work it is already doing cannot change that turn.
pub(crate) fn finalize_iteration(iter: usize, max_iterations: usize) -> bool {
    max_iterations >= 2 && iter + 1 == max_iterations - 1
}

pub(crate) fn review_notification(actions: &[String], last_round_text: &str) -> Option<String> {
    // BOTH halves, not one or the other. The first version returned the actions
    // and dropped the text whenever `actions` was non-empty, which lost the
    // memory candidate on the COMBINED lane: its prompt says memory writes are
    // refused exactly as the memory-only lane's does, so it writes a SKILL (which
    // populates `actions`) and still returns a `Memory candidate:` line. The
    // candidate is the only output a foreground turn can act on, so dropping it
    // on the busier lane is the worse half to lose.
    //
    // Order: the writes first (what the review DID), then the report (what it
    // wants the operator to act on).
    let actions_text = if actions.is_empty() {
        String::new()
    } else {
        actions.join(" · ")
    };
    let report = last_round_text.trim();

    match (actions_text.is_empty(), report.is_empty()) {
        (false, false) => Some(format!(
            "💾 Self-improvement review: {actions_text}\n{report}"
        )),
        (false, true) => Some(format!("💾 Self-improvement review: {actions_text}")),
        (true, false) => Some(format!("💾 Self-improvement review: {report}")),
        (true, true) => None,
    }
}

/// Token ceiling on the transcript snapshot one background review may carry.
///
/// The review's snapshot was bounded by a message COUNT (40), which is a poor
/// proxy for cost: forty one-line answers are nothing and forty pasted files
/// overflow the review model's window, and the failure is silent in the same way
/// the advisor's was -- the review is spawned, the call fails, and the only trace
/// is a log line on a path whose whole output is optional.
///
/// Matches the advisor's and recap's ceilings: the three are the same class of
/// disposable side call over the same class of transcript, and a review that
/// needs more than this to read the recent conversation is not reading the
/// recent conversation.
pub(crate) const REVIEW_VIEW_TOKEN_BUDGET: usize = 8_000;

/// One review tool result as it enters the review's own transcript.
///
/// Bounded by the SAME cap the main agent applies to the same tool from the same
/// registry ([`crate::agent::tool_result_cap`]), because the alternative was no
/// bound at all. The review loop pushed `result.content` verbatim into
/// `review_messages` on every one of up to five iterations: a `memory_search`
/// returns as many as 50 rows and a `skill_view` a whole `SKILL.md`, so a
/// review that reads three of them is already past a small auxiliary model's
/// window. The provider then rejects the request, the loop logs a warning and
/// BREAKS, and the review yields nothing — which is indistinguishable in the log
/// from a model that simply found nothing worth saving.
///
/// Pure so the bound has a test seam. The loop that calls it lives inside an
/// async daemon, and a rule written inline there cannot be driven from a test;
/// that missing seam is precisely why the lane shipped unbounded.
pub(crate) fn review_tool_result(
    name: &str,
    success: bool,
    content: &str,
    error: Option<&str>,
) -> String {
    if !success {
        return format!(
            "{{\"success\": false, \"error\": \"{}\"}}",
            error.unwrap_or("Unknown error")
        );
    }
    crate::agent::truncate_tool_result_capped(content, crate::agent::tool_result_cap(name))
}

/// Drop the oldest trajectory files so at most `keep` remain.
///
/// `keep` of 0 means keep them all — the default.
///
/// Best-effort: a file that cannot be read or removed is skipped, and the
/// prune never fails the run that triggered it. A trajectory is a diagnostic
/// artefact; losing the pruning of one is not worth failing a turn over.
fn prune_old_trajectories(dir: &std::path::Path, keep: usize) {
    if keep == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(std::time::SystemTime, std::path::PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        // The mtime orders the files. The name also carries the run's
        // timestamp, but parsing it would mean trusting a format; the
        // mtime is what the filesystem says and needs no parsing.
        let Ok(modified) = meta.modified() else {
            continue;
        };
        files.push((modified, path));
    }
    if files.len() <= keep {
        return;
    }
    // Oldest first, so the tail is what survives.
    files.sort_by_key(|(modified, _)| *modified);
    let excess = files.len() - keep;
    for (_, path) in files.into_iter().take(excess) {
        if let Err(e) = std::fs::remove_file(&path) {
            warn!(error = %e, path = ?path, "Failed to prune an old trajectory");
        }
    }
}

/// Build the dedicated client for an auxiliary background-review route.
///
/// An endpoint override never inherits the primary credential: a secret is
/// issued for one host and must not cross to another.
fn review_route_client(
    route: &self::background_review::ReviewRoute,
    cfg: &crate::config::AppConfig,
    primary_base_url: Option<&str>,
    primary_api_key: Option<&str>,
) -> Arc<dyn crate::agent::ModelClient> {
    let mut base = crate::client::ClientConfig::from(&cfg.client);
    if let Some(url) = primary_base_url {
        base.base_url = url.to_string();
    }
    if let Some(key) = primary_api_key {
        base.api_key = Some(key.to_string());
    }
    if route.base_url.is_some() && route.api_key.is_none() {
        base.api_key = None;
    }
    let mut slot = cfg.auxiliary_models.memory.clone().unwrap_or_default();
    slot.model = Some(route.model.clone());
    slot.base_url = route.base_url.clone();
    slot.api_key = route.api_key.clone();
    Arc::new(crate::agent::clients::openai::OpenAIModelClient::new(
        crate::moa::slot_client(&base, &slot),
    ))
}

impl OperantAgent {
    pub(crate) fn load_context_file_prompt(&self) -> String {
        // R-07 (AUDIT-2026-10-16-capability-layer): in a Holon run the kernel's
        // brief is the only org guidance this body gets. Both sources below
        // are the owner's, not the organization's — `dirs::config_dir()` is
        // outside the run's HERMES_HOME (the owner's personal context), and
        // the workspace walk picks up whatever AGENTS.md sits in the legacy
        // tree the workdir lives in, a second stale instruction set beside
        // the brief. The kernel names no workspace context file in brief.json
        // today, so a Holon run gets none.
        if crate::platform::is_holon_run() {
            return String::new();
        }

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
        // AD-001: a Holon body's operant-side trajectory names its run and
        // employee. The kernel's trajectory.json drops metadata by design; the
        // kernel already holds the run row.
        if crate::platform::is_holon_run() {
            if let Ok(run_id) = std::env::var("HOLON_RUN_ID") {
                trajectory = trajectory.set_metadata("holon_run_id", run_id);
            }
            if let Some(employee) = crate::platform::holon_employee() {
                trajectory = trajectory.set_metadata("holon_employee", employee);
            }
        }
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
                    let keep = self.config.max_trajectories;
                    prune_old_trajectories(&trajectories_dir, keep);
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
        let session_id = session_id.to_string();

        // ── Resolve the auxiliary review route ─────────────────────────
        // Model, endpoint, and credential form ONE route contract. A
        // model-only slot rides the primary client; a pinned endpoint or
        // credential gets a dedicated client, so the request does not leak
        // the auxiliary model onto the primary route.
        let cfg = runtime_config();
        let primary_model = self.effective_model();
        let route = crate::agent::background_review::resolve_review_route(
            cfg.auxiliary_models.memory.as_ref(),
            &primary_model,
            self.client.base_url().as_deref(),
            self.client.current_api_key().as_deref(),
        );
        let (review_model, is_routed, client) = if let Some(route) = route {
            let client: Arc<dyn crate::agent::ModelClient> =
                if route.base_url.is_some() || route.api_key.is_some() {
                    review_route_client(
                        &route,
                        &cfg,
                        self.client.base_url().as_deref(),
                        self.client.current_api_key().as_deref(),
                    )
                } else {
                    Arc::clone(&self.client)
                };
            (route.model, route.routed, client)
        } else {
            (primary_model, false, Arc::clone(&self.client))
        };

        // ── Snapshot the conversation ─────────────────────────────────
        // Bounded TWICE, for two different reasons.
        //
        // A message COUNT (40) bounds how much history the review reads, and it
        // is a poor proxy for cost: forty one-line answers are nothing, and forty
        // pasted files are a context-window overflow. So the count stays as the
        // policy ("the recent conversation") and a TOKEN ceiling is applied to it
        // with the shared helper -- the same guard the advisor got in iter-2398
        // and recap has had since iter-2345.
        //
        // The count is applied first so a huge transcript does not allocate a
        // token estimate over its whole length; the token fit then cuts the
        // tail.
        let start = messages.len().saturating_sub(40);
        let snapshot = crate::context_management::fit_messages_to_token_budget(
            messages[start..].to_vec(),
            REVIEW_VIEW_TOKEN_BUDGET,
        );
        if snapshot.is_empty() {
            warn!(
                "background review snapshot emptied by the token budget — the newest \
                 message alone exceeds it; review skipped rather than run on nothing"
            );
            return;
        }

        // ── Tool whitelist: only memory + skill tools ─────────────────
        // The review agent should ONLY have access to memory and skill
        // management tools. Never terminal, file_write, browser, etc.
        // This matches hermes-agent's `set_thread_tool_whitelist` pattern.
        // (plan 2026-10-26 phase 2) memory_recall retired: the review's
        // toolset is the consolidated pair; the alias layer still
        // resolves the old name if a model calls it, but the review
        // prompt names what is actually on the wire.
        let review_tool_names: Vec<String> = vec![
            "memory_store".to_string(),
            "memory_search".to_string(),
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

        // Task-local read marks: concurrent reviews cannot inherit one
        // another's `skill_view` history.
        let review_reads = Arc::new(std::sync::RwLock::new(std::collections::HashSet::new()));

        crate::daemon_pool::spawn(
            "probe-extra",
            crate::write_origin::scope_background_review_with_reads(
                async move {
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
You have access to memory_store, memory_search, \
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
You have access to memory_store, memory_search, \
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
                    // Iterations actually run, so the completion log can tell a
                    // review that looked and found nothing from one that hit the
                    // cap. `review_iter` is out of scope by then.
                    let mut iterations_run = 0usize;
                    // The last non-empty assistant text the review produced. The
                    // loop discards it when it breaks, and the memory lane's whole
                    // output lives there.
                    let mut last_round_text = String::new();

                    for review_iter in 0..MAX_REVIEW_ITERATIONS {
                        iterations_run = review_iter + 1;
                        debug!(
                            iteration = review_iter + 1,
                            max = MAX_REVIEW_ITERATIONS,
                            session_id = %session_id,
                            "Background review iteration"
                        );

                        // Create the review chat request (non-streaming for background)
                        let request =
                            ChatRequest::new(review_model.clone(), review_messages.clone())
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
                        let tool_calls_deltas =
                            assistant_msg.tool_calls.clone().unwrap_or_default();
                        let content = assistant_msg.content.clone().unwrap_or_default();
                        // Kept for the completion path: a review that wrote
                        // nothing may still have SAID something worth surfacing.
                        if !content.trim().is_empty() {
                            last_round_text = content.clone();
                        }

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

                        // ── The staged budget: "finalize now", then stop ─────
                        //
                        // The iteration cap was a CLIFF: at iteration 5 the loop
                        // simply ended, so a review mid-way through a tool
                        // sequence lost whatever it had done and produced no
                        // report at all. The reference implementation's dreamer
                        // does this in two stages instead -- first a soft message
                        // that says the budget is spent and asks for the result
                        // now, and only then, if the agent ignores it, a refusal.
                        //
                        // One stage is enough here, because the loop already has a
                        // hard stop: this is the soft half. It fires on the
                        // PENULTIMATE iteration so a review that would have
                        // finished anyway is never interrupted, and a review that
                        // was going to be cut off gets one turn to report.
                        //
                        // The message is appended as a USER turn so the model sees
                        // it as an instruction from the host rather than as its own
                        // reasoning.
                        //
                        // NOT the last iteration. This comment used to say it fired
                        // there and the code agreed: it tested
                        // `review_iter + 1 == MAX_REVIEW_ITERATIONS`, which pushed the
                        // message onto the LAST request — the one that is built, run
                        // through its tools, and then never sent. The instruction
                        // reached zero turns, which is the exact thing it was written
                        // to fix.
                        //
                        // At the penultimate index the message rides the LAST
                        // iteration's request, which is the one the model answers
                        // after being told to stop. `MAX_REVIEW_ITERATIONS` must be
                        // at least 2 for there to be a penultimate index; enforcing
                        // that bound is `finalize_iteration`'s job, and it is the one
                        // place the rule is written down.
                        if finalize_iteration(review_iter, MAX_REVIEW_ITERATIONS) {
                            info!(
                                session_id = %session_id,
                                "Background review is nearly out of iterations — asking for the result now"
                            );
                            review_messages.push(Message::user(
                                "You are out of iteration budget. Do not make any more tool \
                                 calls. Report what you changed, or say 'Nothing to save.'"
                                    .to_string(),
                            ));
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
                                review_messages
                                    .push(Message::tool(tool_id, error_result.to_string()));
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
                                    // Bounded by the main agent's own cap — see
                                    // `review_tool_result` for why this lane had no
                                    // bound at all and what that cost. The cap is a
                                    // READ-BACK budget, not a truncation of what the
                                    // tool did: the write already happened, and the
                                    // model is told the tail is in the session store.
                                    let result_str = review_tool_result(
                                        tool_name,
                                        result.success,
                                        &result.content,
                                        result.error.as_deref(),
                                    );

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

                    // ── Surface what the review did, or what it SAID ─────────────
                    //
                    // Two outcomes, one rule (`review_notification`): a review
                    // that wrote reports its writes; a review that wrote NOTHING
                    // still reports its final text, because the memory lane
                    // CANNOT write and its only output is a
                    // `Memory candidate:` line for a foreground turn to act on.
                    //
                    // That candidate used to be dropped here. `actions_taken`
                    // counts WRITES; a read-and-propose lane that works correctly
                    // performs none; so its reply always took the empty branch and
                    // was discarded. The prompt named a contract no code honoured.
                    //
                    // The empty case is `info!` with the iteration count, not
                    // `debug!`: "looked and found nothing" (few iterations, clean
                    // exit) and "thrashed and gave up" (hit the cap) are different
                    // outcomes, and only the count separates them.
                    match review_notification(&actions_taken, &last_round_text) {
                        Some(notification) => {
                            info!(
                                session_id = %session_id,
                                actions = %actions_taken.join(" · "),
                                action_count = actions_taken.len(),
                                "Background review completed with updates"
                            );
                            // Deliver via callback (TUI/Gateway wired via
                            // with_background_review_callback) AND via AgentEvent,
                            // so TUI/CLI surfaces it without the callback wired.
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
                        }
                        None => {
                            info!(
                                session_id = %session_id,
                                iterations = iterations_run,
                                "Background review completed with no changes and no report"
                            );
                        }
                    }

                    debug!(session_id = %session_id, "Background review daemon finished");
                },
                review_reads,
            ),
        );

        // ── (plan 2026-11-05 RSI, iter-1744) the curator rides the same ──
        // cadence: the fork edits skills, the curator owns their lifecycle
        // afterwards (archive-after-idle, staleness — deterministic, no
        // LLM). maybe_run_curator gates by its own interval (24h default,
        // seeded on first call), so this is cheap per fire and a no-op
        // between intervals. Detached: a curator miss is a log line, never
        // a turn failure. Skills lane only — that is the lane that creates
        // the agent-authored skills the curator manages.
        if review_skills {
            // The operator's CONFIGURED skills tree — the same one the skills
            // tools write and the agent reads. The platform default is only
            // the config's default value: with `[skills] root_dir` set, the
            // curator used to manage the lifecycle of a tree the operator
            // does not use (RT-265).
            let skills_dir = crate::skills::configured_skills_dir();
            // The LIVE cron db: the curator's "never archive a cron-referenced
            // skill" rule must read the file the cron scheduler writes. If it
            // cannot be opened, the pass is SKIPPED rather than run with an
            // empty protection set — an unreadable protection set must never
            // become "nothing is protected" (RT-267).
            let cron_db = match crate::cronjobs::CronDb::init(crate::cronjobs::db_path(
                &crate::config::runtime_config().database_path,
            )) {
                Ok(db) => Some(db),
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "curator auto-run skipped — the cron db could not be opened"
                    );
                    return;
                }
            };
            tokio::spawn(async move {
                let curator_dir = skills_dir.join(".curator");
                let tracker = std::sync::Arc::new(crate::skill_usage::SkillUsageTracker::new(
                    curator_dir.join("usage.json"),
                ));
                let engine = crate::curator::CuratorEngine::new(
                    skills_dir,
                    curator_dir.join("state.json"),
                    tracker,
                );
                if engine.load_state().await.is_err() {
                    debug!("curator auto-run skipped — state unreadable");
                    return;
                }
                if engine.maybe_run_curator().await {
                    match engine.apply_automatic_transitions(cron_db.as_ref()).await {
                        Ok(counts) => {
                            info!(?counts, "curator auto-run applied lifecycle transitions");
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "curator auto-run failed");
                        }
                    }
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::background_review::resolve_review_route;
    use crate::config::{AppConfig, AuxiliaryModelConfig};

    /// Changing the endpoint without supplying a credential must NOT send
    /// the primary secret to the auxiliary host.
    #[test]
    fn an_auxiliary_endpoint_never_inherits_the_primary_credential() {
        let mut cfg = AppConfig::default();
        cfg.auxiliary_models.memory = Some(AuxiliaryModelConfig {
            provider: None,
            model: Some("aux-model".to_string()),
            base_url: Some("http://127.0.0.1:9/v1".to_string()),
            api_key: None,
        });
        let route = resolve_review_route(
            cfg.auxiliary_models.memory.as_ref(),
            "primary-model",
            Some("https://primary.example/v1"),
            Some("primary-secret"),
        )
        .expect("route");
        let client = review_route_client(
            &route,
            &cfg,
            Some("https://primary.example/v1"),
            Some("primary-secret"),
        );
        assert_eq!(
            client.base_url().as_deref(),
            Some("http://127.0.0.1:9/v1"),
            "the auxiliary endpoint must win"
        );
        assert_eq!(
            client.current_api_key(),
            None,
            "the primary credential must not cross to the auxiliary host"
        );
    }

    /// The auxiliary slot's own credential reaches its own endpoint.
    #[test]
    fn an_auxiliary_credential_reaches_the_auxiliary_endpoint() {
        let mut cfg = AppConfig::default();
        cfg.auxiliary_models.memory = Some(AuxiliaryModelConfig {
            provider: None,
            model: Some("aux-model".to_string()),
            base_url: Some("http://127.0.0.1:9/v1".to_string()),
            api_key: Some("aux-secret".to_string()),
        });
        let route = resolve_review_route(
            cfg.auxiliary_models.memory.as_ref(),
            "primary-model",
            Some("https://primary.example/v1"),
            Some("primary-secret"),
        )
        .expect("route");
        let client = review_route_client(
            &route,
            &cfg,
            Some("https://primary.example/v1"),
            Some("primary-secret"),
        );
        assert_eq!(client.current_api_key().as_deref(), Some("aux-secret"));
    }

    /// The trajectory directory must not grow without bound.
    ///
    /// A trajectory is written per run, and the file name carries the run's
    /// timestamp, so a long-lived session accumulates one file per run. A
    /// Holon run ALWAYS records (`is_holon_run()` forces it on), and
    /// nothing pruned the directory: the only cleanup was the manual
    /// `operant trajectory clear`, which deletes everything.
    #[test]
    fn the_oldest_trajectories_are_pruned() {
        let dir = std::env::temp_dir().join(format!(
            "operant_traj_prune_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");

        // Write more than the cap, with distinct mtimes so the order is
        // unambiguous.
        let total = MAX_TRAJECTORIES_KEPT + 5;
        for i in 0..total {
            let path = dir.join(format!("sess_{i}.json"));
            std::fs::write(&path, b"{}").expect("write trajectory");
            // Set the mtime explicitly: a filesystem with a coarse
            // timestamp granularity would otherwise give several files the
            // same mtime and make the order a tie.
            let stamp =
                std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(i as u64);
            filetime_set(&path, stamp);
        }
        // A file that is not a trajectory must survive the prune.
        let other = dir.join("notes.txt");
        std::fs::write(&other, b"keep me").expect("write other file");

        prune_old_trajectories(&dir, MAX_TRAJECTORIES_KEPT);

        let remaining = std::fs::read_dir(&dir)
            .expect("read the dir")
            .flatten()
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json"))
            .count();
        assert_eq!(
            remaining, MAX_TRAJECTORIES_KEPT,
            "the directory must be capped, not merely trimmed"
        );
        assert!(
            other.exists(),
            "a file that is not a trajectory must not be pruned"
        );
        // The NEWEST survive: the file written last is still there.
        assert!(
            dir.join(format!("sess_{}.json", total - 1)).exists(),
            "the newest trajectory must survive"
        );

        // The OLDEST are gone.
        assert!(
            !dir.join("sess_0.json").exists(),
            "the oldest trajectory must be pruned"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Set a file's modification time, so the prune's order is unambiguous.
    /// `FileTimes::set_modified` is an inherent method (stable 1.75+), so
    /// this needs no dependency.
    fn filetime_set(path: &std::path::Path, stamp: std::time::SystemTime) {
        let ft = std::fs::FileTimes::new().set_modified(stamp);
        let _ = std::fs::File::options()
            .write(true)
            .open(path)
            .map(|f| f.set_times(ft));
    }

    /// `keep = 0` means KEEP THEM ALL.
    ///
    /// `operant trajectory export` exists to collect these files as
    /// prompt-completion pairs for fine-tuning, so a prune that deletes
    /// them silently destroys the corpus that command is for. The bound is
    /// the operator's choice, and the default is no bound at all.
    #[test]
    fn a_cap_of_zero_keeps_every_trajectory() {
        let dir = std::env::temp_dir().join(format!(
            "operant_traj_keepall_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");

        let total = MAX_TRAJECTORIES_KEPT + 5;
        for i in 0..total {
            std::fs::write(dir.join(format!("sess_{i}.json")), b"{}").expect("write trajectory");
        }

        prune_old_trajectories(&dir, 0);

        let remaining = std::fs::read_dir(&dir)
            .expect("read the dir")
            .flatten()
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json"))
            .count();
        assert_eq!(remaining, total, "a cap of zero must keep every trajectory");

        let _ = std::fs::remove_dir_all(&dir);
    }
    /// A review that WROTE reports what it wrote.
    #[test]
    fn a_review_that_wrote_reports_its_writes() {
        // This test used to assert that the final text is DROPPED whenever there
        // are actions ("ignored final text"), which pinned the defect rather than
        // the rule: the combined lane writes a skill and returns its
        // `Memory candidate:` line in that same text, so dropping it lost the
        // only output a foreground turn can act on. The writes still lead.
        let actions = vec!["memory: saved a preference".to_string()];
        let note = review_notification(&actions, "Memory candidate: a fact.");
        assert_eq!(
            note.as_deref(),
            Some(
                "💾 Self-improvement review: memory: saved a preference\n\
                 Memory candidate: a fact."
            )
        );
    }

    /// A review that wrote NOTHING still reports what it SAID.
    ///
    /// This is the memory lane: `MEMORY_REVIEW_PROMPT` tells the model it cannot
    /// write and must return a `Memory candidate:` line. `actions_taken` counts
    /// writes, so a lane that works exactly as designed has an EMPTY actions list
    /// -- and its report used to be discarded with it. The candidate is the lane's
    /// only output, so it must survive.
    #[test]
    fn a_memory_candidate_survives_a_lane_that_cannot_write() {
        let note = review_notification(
            &[],
            "Memory candidate: the operator prefers terse diffs with rationale.",
        );
        assert_eq!(
            note.as_deref(),
            Some(
                "💾 Self-improvement review: Memory candidate: the operator prefers \
                 terse diffs with rationale."
            )
        );
    }

    /// Nothing written AND nothing said surfaces nothing -- the only silent case.
    #[test]
    fn an_empty_review_surfaces_nothing() {
        assert_eq!(review_notification(&[], ""), None);
        assert_eq!(review_notification(&[], "   \n  "), None);
    }
    /// BOTH halves reach the operator when a lane writes AND reports.
    ///
    /// The first version returned the actions and dropped the text whenever
    /// `actions` was non-empty. The COMBINED lane writes skills (populating
    /// `actions`) and still returns a `Memory candidate:` line, because its prompt
    /// refuses memory writes exactly as the memory-only lane's does -- so the
    /// candidate was lost on the busier lane, which is the worse half to lose.
    #[test]
    fn a_candidate_survives_a_lane_that_also_wrote() {
        let actions = vec!["skill_manage: updated the skill \"plan\"".to_string()];
        let note = review_notification(
            &actions,
            "Memory candidate: the operator prefers terse diffs.",
        );
        let note = note.expect("both halves are present, so there is something to surface");
        assert!(
            note.contains("skill_manage"),
            "the writes must still be reported: {note:?}"
        );
        assert!(
            note.contains("Memory candidate"),
            "the candidate must survive a lane that also wrote: {note:?}"
        );
    }

    /// The existing three cases are unchanged by the concatenation.
    #[test]
    fn the_notification_still_covers_writes_only_text_only_and_neither() {
        assert_eq!(
            review_notification(&["memory_store: saved".to_string()], "").as_deref(),
            Some("💾 Self-improvement review: memory_store: saved")
        );
        assert_eq!(
            review_notification(&[], "Memory candidate: a fact.").as_deref(),
            Some("💾 Self-improvement review: Memory candidate: a fact.")
        );
        assert_eq!(review_notification(&[], "   "), None);
    }

    /// A zero budget means the built-in ceiling, not "drop everything".
    ///
    /// The sibling `fit_to_token_budget` documents 0 as unset; this one used to
    /// return an EMPTY view for a 0 budget, because `total <= 0` is false for any
    /// non-empty view and the keep loop then broke on the first message.
    #[test]
    fn a_zero_budget_fits_to_the_builtin_ceiling() {
        let view = vec![
            Message::user("a short message"),
            Message::assistant("a short answer"),
        ];
        // `Message` has no PartialEq, so assert the PROPERTY rather than the
        // value: a 0 budget must keep everything, not empty the view. An empty
        // result is the defect being guarded.
        let zero = crate::context_management::fit_messages_to_token_budget(view.clone(), 0);
        assert_eq!(
            zero.len(),
            view.len(),
            "0 must mean the built-in ceiling, not 'drop everything'"
        );
    }
    /// The finalize message rides the SECOND-TO-LAST iteration, so a request is
    /// still built after it and the model can answer.
    ///
    /// It shipped on the LAST iteration first: that request is built and then the
    /// loop exits, so the model never saw the instruction and the fix delivered
    /// ZERO turns while reading as if it gave one. This test is the seam that was
    /// missing, which is why the bug survived a green suite.
    #[test]
    fn the_finalize_message_rides_an_iteration_that_is_still_sent() {
        // A five-iteration cap: the message goes on iteration 3 (0-based), and
        // iteration 4 is the one that answers it.
        for iter in 0..5 {
            assert_eq!(
                finalize_iteration(iter, 5),
                iter == 3,
                "iteration {iter} of a 5-cap must carry the message only at the penultimate index"
            );
        }
    }

    /// The LAST iteration must never be the carrier: its request is built and then
    /// the loop ends, so the message would never reach the model.
    #[test]
    fn the_last_iteration_never_carries_the_finalize_message() {
        for max in [2usize, 3, 5, 8] {
            assert!(
                !finalize_iteration(max - 1, max),
                "the last iteration of a {max}-cap must not carry the message"
            );
        }
    }

    /// A cap with no earlier iteration has no carrier, and says so rather than
    /// firing on the only iteration it has.
    #[test]
    fn a_single_iteration_cap_has_no_finalize_turn() {
        assert!(
            !finalize_iteration(0, 1),
            "one iteration cannot warn itself"
        );
        assert!(!finalize_iteration(0, 0), "no iterations, no carrier");
    }

    /// THE BOUND THIS LANE DID NOT HAVE.
    ///
    /// A review that calls `memory_search` (up to 50 rows) and `skill_view` (a
    /// whole SKILL.md) once per iteration pushes five rounds of that into ONE
    /// request. Unbounded, the request outgrows a small auxiliary model's window,
    /// the provider rejects it, the loop warns and breaks, and the review returns
    /// nothing. The result is capped so the request stays inside the window the
    /// review model was chosen for.
    #[test]
    fn a_review_tool_result_is_capped_like_the_main_agents() {
        let huge = "x".repeat(200_000);
        let capped = review_tool_result("memory_search", true, &huge, None);

        assert!(
            capped.len() < huge.len(),
            "an oversized review tool result is cut: {} -> {}",
            huge.len(),
            capped.len()
        );
        assert!(
            capped.len() <= crate::agent::tool_result_cap("memory_search") + 128,
            "cut to the main agent's own cap for the same tool, plus its stated \
             truncation footer: {} bytes",
            capped.len()
        );
        assert!(
            capped.contains("truncated") || capped.contains("withheld"),
            "the cut is STATED, so the model knows the tail exists: {}",
            &capped[capped.len().saturating_sub(160)..]
        );
    }

    /// A result that already fits is passed through untouched — the cap is a
    /// ceiling, not a rewrite.
    #[test]
    fn a_review_tool_result_that_fits_is_untouched() {
        let small = "{\"results\": [{\"id\": \"abc\", \"text\": \"terse\"}]}";
        assert_eq!(
            review_tool_result("memory_search", true, small, None),
            small
        );
    }

    /// The failure arm keeps its wire shape: the model reads `success: false`
    /// and stops, rather than seeing a truncation of an error string.
    #[test]
    fn a_failed_review_tool_result_keeps_its_error_shape() {
        let err = review_tool_result("skill_view", false, "ignored", Some("no such skill"));
        assert!(err.contains("\"success\": false"), "{err}");
        assert!(err.contains("no such skill"), "{err}");
        assert!(
            !err.contains("ignored"),
            "a failure's content is not carried: {err}"
        );

        let unknown = review_tool_result("skill_view", false, "ignored", None);
        assert!(
            unknown.contains("Unknown error"),
            "a missing error still names itself: {unknown}"
        );
    }

    /// The master switch is ONE key, not the two intervals an operator had to
    /// find in `[agent]`. `Some(false)` wins over any cadence.
    #[test]
    fn the_master_switch_disables_both_lanes_whatever_the_intervals_say() {
        let off = crate::agent::background_review::BackgroundReviewSettings {
            enabled: Some(false),
            instructions: String::new(),
        };
        assert!(!off.permits(10, 10), "off beats a live cadence");
        assert!(!off.permits(0, 0), "off beats an already-silent one");
    }

    /// Omitting the key is the historical behaviour: the intervals decide, and
    /// a zero in BOTH is the only thing that means off.
    #[test]
    fn an_unset_switch_follows_the_intervals_exactly_as_before() {
        let unset = crate::agent::background_review::BackgroundReviewSettings::default();
        assert!(unset.permits(10, 10));
        assert!(unset.permits(10, 0), "one live lane is enough");
        assert!(unset.permits(0, 10));
        assert!(!unset.permits(0, 0), "no cadence, no review");
    }

    /// The operator's instructions extend the lane contract rather than
    /// replacing it, and an empty setting reproduces the old prompt byte for
    /// byte.
    #[test]
    fn review_instructions_are_appended_after_the_lane_contract() {
        use crate::agent::background_review::build_review_prompt_with_instructions;

        for (memory, skills) in [(true, false), (false, true), (true, true)] {
            let bare = build_review_prompt_with_instructions(memory, skills, "");
            assert!(
                !bare.contains("Additional instructions from the operator"),
                "empty instructions add nothing"
            );

            let tailed =
                build_review_prompt_with_instructions(memory, skills, "  Prefer root causes.  ");
            assert!(
                tailed.starts_with(&bare),
                "the built-in contract comes first, unchanged"
            );
            assert!(
                tailed.contains("Prefer root causes."),
                "the operator's text is carried"
            );
            assert!(
                !tailed.contains("  Prefer"),
                "the text is trimmed before it is appended"
            );
        }
    }

    /// A whitespace-only setting is the same as no setting.
    #[test]
    fn blank_review_instructions_are_ignored() {
        use crate::agent::background_review::build_review_prompt_with_instructions;
        assert_eq!(
            build_review_prompt_with_instructions(true, true, "   \n\t "),
            build_review_prompt_with_instructions(true, true, "")
        );
    }
}
