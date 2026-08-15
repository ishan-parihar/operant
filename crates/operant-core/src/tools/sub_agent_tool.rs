//! Sub-agent delegation tool.
//!
//! This tool lets a parent agent delegate focused analysis to an isolated child
//! agent without changing the parent ReAct loop. Supports single-task and batch
//! (parallel) modes with role-based tool restrictions and spawn depth limits.

use std::error::Error as StdError;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use tokio::time::timeout;

use crate::agent::clients::openai::OpenAIModelClient;
use crate::agent::{AgentConfig, AgentEvent, ModelClient, OperantAgent};
use crate::client::{ClientConfig, OpenAIClient};
use crate::database::Database;
use crate::schema::ToolSchema;
use crate::tools::async_delegation;
use crate::tools::delegation_output_schema::{
    MAX_SCHEMA_RETRIES, append_output_contract, build_retry_message, coerce_output_schema,
    validate_output,
};
use crate::tools::{OperantTool, ToolContext, ToolRegistry, ToolResult};

const TOOL_NAME: &str = "delegate_task";

/// Maximum concurrent children (default from Python implementation)
const DEFAULT_MAX_CONCURRENT_CHILDREN: usize = 3;
/// Maximum concurrent background delegations (hermes
/// `_DEFAULT_MAX_ASYNC_CHILDREN = 3` parity).
const MAX_ASYNC_CHILDREN: usize = 3;
/// Default child timeout in seconds (10 minutes)
const DEFAULT_CHILD_TIMEOUT_SECONDS: u64 = 600;
/// Minimum spawn depth
const MIN_SPAWN_DEPTH: u32 = 1;
/// Maximum spawn depth cap
const MAX_SPAWN_DEPTH_CAP: u32 = 3;
/// Default max spawn depth (flat: parent -> child, no grandchildren)
const DEFAULT_MAX_SPAWN_DEPTH: u32 = 1;

type BoxedToolError = Box<dyn StdError + Send + Sync>;

/// Role of the sub-agent - determines capabilities
#[derive(Debug, Clone, Copy, PartialEq, Eq, JsonSchema, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SubAgentRole {
    /// Leaf agent - focused worker, cannot delegate further
    #[default]
    Leaf,
    /// Orchestrator - can spawn its own sub-agents
    Orchestrator,
}

/// A single task in batch mode
#[derive(Debug, Deserialize, JsonSchema)]
#[expect(
    dead_code,
    reason = "serde-argument struct: fields deserialized from tool-call JSON; optional fields kept for schema parity"
)]
struct DelegationTask {
    /// The focused task instruction for the child agent
    goal: String,
    /// Optional context to pass to the sub-agent
    #[serde(default)]
    context: Option<String>,
    /// Optional toolsets to enable for this task
    #[serde(default)]
    toolsets: Option<Vec<String>>,
    /// Optional JSON Schema object the child's final answer must validate
    /// against (hermes delegation_output_schema.py parity).
    #[serde(default)]
    output_schema: Option<Value>,
}

/// Arguments for delegated sub-agent work.
#[derive(Debug, Deserialize, JsonSchema)]
#[expect(
    dead_code,
    reason = "serde-argument struct: fields deserialized from tool-call JSON; optional fields kept for schema parity"
)]
struct SubAgentArgs {
    /// The focused task instruction for the child agent (single mode)
    #[serde(default, alias = "task")]
    goal: Option<String>,
    /// Optional context to pass to the sub-agent
    #[serde(default)]
    context: Option<String>,
    /// Optional toolsets to enable for the sub-agent
    #[serde(default)]
    toolsets: Option<Vec<String>>,
    /// Batch mode: array of tasks to run in parallel
    #[serde(default)]
    tasks: Option<Vec<DelegationTask>>,
    /// Role of the sub-agent: "leaf" (default) or "orchestrator"
    #[serde(default, alias = "agent_role")]
    role: Option<SubAgentRole>,
    /// Maximum iterations for the child agent
    #[serde(default)]
    max_iterations: Option<u32>,
    /// Timeout for child agent in seconds (default: 600)
    #[serde(default)]
    timeout_seconds: Option<u64>,
    /// Optional JSON Schema object the child's final answer must validate
    /// against. On failure the child gets exactly ONE bounded retry turn
    /// carrying the validation errors verbatim (hermes
    /// delegation_output_schema.py parity).
    #[serde(default)]
    output_schema: Option<Value>,
    /// Run the delegated task in the background and return a handle
    /// immediately (hermes async_delegation.py parity). Poll the result
    /// with `query`.
    #[serde(default)]
    background: Option<bool>,
    /// Poll the status/result of a previously dispatched background
    /// delegation by its `delegation_id`.
    #[serde(default)]
    query: Option<String>,
}

/// Runtime state for delegation
static MAX_SPAWN_DEPTH: AtomicU32 = AtomicU32::new(DEFAULT_MAX_SPAWN_DEPTH);
static ORCHESTRATOR_ENABLED: AtomicU32 = AtomicU32::new(1); // default true
static MAX_CONCURRENT_CHILDREN: AtomicU32 = AtomicU32::new(DEFAULT_MAX_CONCURRENT_CHILDREN as u32);

/// Tool that delegates a focused task to an isolated child OperantAgent.
pub struct SubAgentTool {
    client_config: ClientConfig,
    http_client: Client,
    model: String,
    /// Parent's current depth (0 = root agent)
    parent_depth: u32,
    /// Parent's enabled toolsets
    parent_toolsets: Vec<String>,
    /// Parent's explicitly disabled tools — inherited so children never gain
    /// tools the parent lacks (hermes delegate_tool.py: "subagent must not
    /// gain tools the parent lacks").
    parent_disabled_tools: std::collections::HashSet<String>,
    /// Parent's explicitly disabled toolsets — inherited for the same reason.
    parent_disabled_toolsets: std::collections::HashSet<String>,
    database: Arc<Database>,
    /// Optional event channel to forward child progress events to parent
    event_tx: Option<tokio::sync::mpsc::Sender<AgentEvent>>,
}

impl SubAgentTool {
    pub fn new(
        parent_client: &OpenAIClient,
        model: impl Into<String>,
        parent_depth: u32,
        parent_toolsets: Vec<String>,
        database: Arc<Database>,
        event_tx: Option<tokio::sync::mpsc::Sender<AgentEvent>>,
    ) -> Self {
        Self::with_parent_tool_policy(
            parent_client,
            model,
            parent_depth,
            parent_toolsets,
            std::collections::HashSet::new(),
            std::collections::HashSet::new(),
            database,
            event_tx,
        )
    }

    /// Construct with the parent's disabled tool/toolset policy so that
    /// spawned children inherit the exact same tool restrictions as the
    /// parent registry (hermes parity).
    #[expect(
        clippy::too_many_arguments,
        reason = "builder mirrors the SubAgentTool fields (hermes parity); callers construct it directly"
    )]
    pub fn with_parent_tool_policy(
        parent_client: &OpenAIClient,
        model: impl Into<String>,
        parent_depth: u32,
        parent_toolsets: Vec<String>,
        parent_disabled_tools: std::collections::HashSet<String>,
        parent_disabled_toolsets: std::collections::HashSet<String>,
        database: Arc<Database>,
        event_tx: Option<tokio::sync::mpsc::Sender<AgentEvent>>,
    ) -> Self {
        Self {
            client_config: parent_client.config_clone(),
            http_client: parent_client.http_client_clone(),
            model: model.into(),
            parent_depth,
            parent_toolsets,
            parent_disabled_tools,
            parent_disabled_toolsets,
            database,
            event_tx,
        }
    }

    /// Run a focused delegated task in an isolated child agent.
    ///
    /// When `output_schema` is provided, the child's system prompt gets an
    /// OUTPUT CONTRACT block and the final answer is validated against the
    /// schema (hermes delegation_output_schema.py parity): on failure the
    /// child receives exactly ONE bounded retry turn carrying the validation
    /// errors verbatim, and a persistent failure still returns the answer
    /// (flagged) so the parent keeps the data.
    pub async fn call(
        &self,
        goal: impl Into<String>,
        context: Option<impl Into<String>>,
        role: SubAgentRole,
        max_iterations: Option<u32>,
        timeout_seconds: u64,
        output_schema: Option<Value>,
    ) -> std::result::Result<String, BoxedToolError> {
        self.ensure_supported_model()?;

        let goal = goal.into();
        let goal = goal.trim();
        if goal.is_empty() {
            return Err("Sub-agent goal must not be empty".into());
        }

        // Check depth limits
        let child_depth = self.parent_depth + 1;
        let max_depth = MAX_SPAWN_DEPTH.load(Ordering::Relaxed);
        if child_depth > max_depth {
            return Err(format!(
                "Cannot spawn sub-agent at depth {}: max spawn depth is {}",
                child_depth, max_depth
            )
            .into());
        }

        // Determine effective role based on depth and orchestrator enabled
        let effective_role = if role == SubAgentRole::Orchestrator {
            let orchestrator_ok =
                ORCHESTRATOR_ENABLED.load(Ordering::Relaxed) == 1 && child_depth < max_depth;
            if orchestrator_ok {
                SubAgentRole::Orchestrator
            } else {
                SubAgentRole::Leaf
            }
        } else {
            SubAgentRole::Leaf
        };

        let schema = match coerce_output_schema(output_schema) {
            Ok(schema) => schema,
            Err(error) => return Err(error.into()),
        };
        let context: Option<String> = context.map(|c| c.into());

        let mut retry_errors: Vec<String> = Vec::new();
        let mut attempts: usize = 0;
        loop {
            // Build child system prompt based on role
            let mut system_prompt = build_child_system_prompt(
                goal,
                context.as_deref(),
                effective_role,
                child_depth,
                max_depth,
            );
            if let Some(schema) = schema.as_ref() {
                system_prompt.push_str(&format!("\n\n{}", append_output_contract(None, schema)));
            }
            if !retry_errors.is_empty() {
                system_prompt.push_str(&format!("\n\n{}", build_retry_message(&retry_errors)));
            }

            let answer = self
                .run_child(
                    system_prompt,
                    goal.to_string(),
                    effective_role,
                    max_iterations,
                    timeout_seconds,
                )
                .await?;

            let Some(schema_ref) = schema.as_ref() else {
                return Ok(answer);
            };
            let (valid, errors) = validate_output(&answer, schema_ref);
            if valid {
                return Ok(answer);
            }
            if attempts >= MAX_SCHEMA_RETRIES {
                // Persistent failure: keep the answer (the parent needs the
                // data) but flag the contract breach explicitly (hermes
                // returns the final text with the errors noted).
                return Ok(format!(
                    "{answer}\n\n[SCHEMA VALIDATION FAILED after {MAX_SCHEMA_RETRIES} \
                     retr{} — errors: {}]",
                    if MAX_SCHEMA_RETRIES == 1 { "y" } else { "ies" },
                    errors.join("; ")
                ));
            }
            retry_errors = errors;
            attempts += 1;
        }
    }

    /// Run a single child agent to completion with the given system prompt.
    /// Shared by the sync, batch, and background delegation paths so depth
    /// limits, tool inheritance, and result shaping stay in one place.
    async fn run_child(
        &self,
        system_prompt: String,
        goal: String,
        role: SubAgentRole,
        max_iterations: Option<u32>,
        timeout_seconds: u64,
    ) -> std::result::Result<String, BoxedToolError> {
        // Determine effective toolsets based on role and parent toolsets
        let child_toolsets = self.compute_child_toolsets(role);

        let raw_client = OpenAIClient::from_shared_http_client(
            self.client_config.clone(),
            self.http_client.clone(),
        );
        let client: Box<dyn ModelClient> = Box::new(OpenAIModelClient::new(raw_client));

        let max_iters: usize = max_iterations.unwrap_or(50) as usize;
        let config = AgentConfig {
            model: self.model.clone(),
            stream: false,
            system_prompt: Some(system_prompt),
            max_iterations: max_iters,
            ..AgentConfig::default()
        };

        let registry = ToolRegistry::new(config.tool_timeout);
        // Register tools based on child toolsets (filtered)
        self.register_child_tools(&registry, &child_toolsets).await;

        let agent = if let Some(ref parent_tx) = self.event_tx {
            let (child_tx, mut child_rx) = tokio::sync::mpsc::channel::<AgentEvent>(128);
            let parent_tx = parent_tx.clone();
            tokio::spawn(async move {
                while let Some(event) = child_rx.recv().await {
                    let _ = parent_tx.send(event).await;
                }
            });
            OperantAgent::with_events(config, client, registry, self.database.clone(), child_tx)
        } else {
            OperantAgent::new(config, client, registry, self.database.clone())
        };

        // Run with timeout
        let timeout_duration = Duration::from_secs(timeout_seconds.max(30));
        let result = timeout(timeout_duration, agent.run(goal)).await;

        match result {
            Ok(Ok(message)) => Ok(message.content),
            Ok(Err(error)) => Err(format!("Sub-agent error: {}", error).into()),
            Err(_) => Err(format!("Sub-agent timed out after {} seconds", timeout_seconds).into()),
        }
    }

    /// Run multiple tasks in parallel (batch mode)
    #[allow(clippy::type_complexity)]
    pub async fn call_batch(
        &self,
        tasks: Vec<(
            String,
            Option<String>,
            SubAgentRole,
            Option<u32>,
            u64,
            Option<Value>,
        )>,
    ) -> std::result::Result<String, BoxedToolError> {
        let max_concurrent = MAX_CONCURRENT_CHILDREN.load(Ordering::Relaxed) as usize;
        let max_concurrent = max_concurrent.clamp(1, 10);

        // Use semaphore to limit concurrency
        let semaphore = Arc::new(tokio::sync::Semaphore::new(max_concurrent));
        let mut handles = Vec::new();

        for (goal, context, role, max_iterations, timeout_seconds, output_schema) in tasks {
            let tool = self.clone_for_task();
            let permit = semaphore
                .clone()
                .acquire_owned()
                .await
                .map_err(|e| e.to_string())?;

            let handle = tokio::spawn(async move {
                let _permit = permit;
                tool.call(
                    goal,
                    context,
                    role,
                    max_iterations,
                    timeout_seconds,
                    output_schema,
                )
                .await
            });
            handles.push(handle);
        }

        // Wait for all tasks and collect results
        let mut results = Vec::new();
        let mut errors = Vec::new();

        for handle in handles {
            match handle.await {
                Ok(Ok(result)) => results.push(result),
                Ok(Err(e)) => errors.push(e.to_string()),
                Err(e) => errors.push(e.to_string()),
            }
        }

        if results.is_empty() && !errors.is_empty() {
            return Err(format!("All sub-agents failed: {}", errors.join("; ")).into());
        }

        // Format results as summary
        Ok(format_batch_results(&results, &errors))
    }

    /// Dispatch a delegation in the background and return a handle immediately
    /// (hermes `async_delegation.py` parity). The child runs on a spawned
    /// tokio task; on completion the record transitions to completed/failed
    /// and an `AgentEvent::AsyncDelegation` is pushed onto the parent's event
    /// channel (when one exists) so the CLI/TUI can surface the outcome. The
    /// agent polls progress with `delegate_task(query="<id>")`.
    async fn dispatch_background(
        &self,
        goal: String,
        context: Option<String>,
        role: SubAgentRole,
        max_iterations: Option<u32>,
        timeout_seconds: u64,
        output_schema: Option<Value>,
    ) -> std::result::Result<String, BoxedToolError> {
        if async_delegation::pending_count() >= MAX_ASYNC_CHILDREN {
            return Err(format!(
                "Too many background delegations in flight (max {MAX_ASYNC_CHILDREN}); \
                 wait for one to complete or use synchronous delegation."
            )
            .into());
        }
        let delegation_id = async_delegation::create_record(&goal, &self.model);

        let tool = self.clone_for_task();
        let event_tx = self.event_tx.clone();
        let spawn_goal = goal.clone();
        let spawn_id = delegation_id.clone();
        tokio::spawn(async move {
            let outcome = tool
                .call(
                    spawn_goal,
                    context,
                    role,
                    max_iterations,
                    timeout_seconds,
                    output_schema,
                )
                .await;
            match outcome {
                Ok(content) => {
                    async_delegation::mark_completed(&spawn_id, &content);
                    if let Some(tx) = event_tx {
                        let _ = tx
                            .send(AgentEvent::AsyncDelegation {
                                delegation_id: spawn_id.clone(),
                                status: "completed".to_string(),
                                summary: preview_of(&content, 200),
                            })
                            .await;
                    }
                }
                Err(error) => {
                    async_delegation::mark_failed(&spawn_id, &error.to_string());
                    if let Some(tx) = event_tx {
                        let _ = tx
                            .send(AgentEvent::AsyncDelegation {
                                delegation_id: spawn_id.clone(),
                                status: "failed".to_string(),
                                summary: error.to_string(),
                            })
                            .await;
                    }
                }
            }
        });

        Ok(serde_json::json!({
            "delegation_id": delegation_id,
            "status": "dispatched",
            "model": self.model,
            "goal": preview_of(&goal, 80),
            "poll": format!("delegate_task query=\"{delegation_id}\""),
        })
        .to_string())
    }

    fn clone_for_task(&self) -> Self {
        Self {
            client_config: self.client_config.clone(),
            http_client: self.http_client.clone(),
            model: self.model.clone(),
            parent_depth: self.parent_depth,
            parent_toolsets: self.parent_toolsets.clone(),
            parent_disabled_tools: self.parent_disabled_tools.clone(),
            parent_disabled_toolsets: self.parent_disabled_toolsets.clone(),
            database: self.database.clone(),
            event_tx: self.event_tx.clone(),
        }
    }

    fn compute_child_toolsets(&self, role: SubAgentRole) -> Vec<String> {
        // Whether the parent supplied an explicit toolset list at all. hermes
        // semantics: an EMPTY parent list means "use defaults" (DEFAULT_TOOLSETS
        // fallback), but a NON-EMPTY parent list means "intersect/strip only"
        // — if every supplied toolset gets stripped, the child gets ZERO
        // tools, never a silent re-addition of tools the parent withheld.
        let parent_supplied = !self.parent_toolsets.is_empty();

        // Start with the parent's toolsets, with the parent's explicit
        // disabled toolsets removed (hermes: children never gain tools the
        // parent lacks). Most core tools live in the "builtin" toolset.
        let mut toolsets: Vec<String> = self
            .parent_toolsets
            .iter()
            .filter(|ts| !self.parent_disabled_toolsets.contains(ts.as_str()))
            .cloned()
            .collect();

        // Hermes fallback: when the parent supplies NO toolset list, children
        // get the default toolset (builtin core tools), not nothing. Without
        // this, children whose parent passes an empty toolset list (the CLI
        // registers the tool with `vec![]`) would receive ZERO tools. Guarded
        // by parent_supplied so a parent that explicitly supplied toolsets
        // (even ones that all got stripped) never gets builtin re-added, and
        // never re-adds a toolset the parent explicitly disabled.
        if toolsets.is_empty()
            && !parent_supplied
            && !self.parent_disabled_toolsets.contains("builtin")
        {
            toolsets.push("builtin".to_string());
        }

        // Remove toolsets that are blocked for ALL children. Mirrors hermes
        // delegate_tool.py `_strip_blocked_tools` + `DELEGATE_BLOCKED_TOOLS`:
        // children never get delegation/clarify/memory/code_execution from the
        // parent's toolset inheritance.
        let blocked_toolsets: [&str; 4] = ["delegation", "clarify", "memory", "code_execution"];
        toolsets.retain(|ts| !blocked_toolsets.contains(&ts.as_str()));

        // Orchestrators retain the delegation toolset: the delegate_task tool
        // itself is re-registered for orchestrator children in
        // register_child_tools (hermes `_blocked_toolsets_for_role` discards
        // "delegate_task" from the blocklist when role == "orchestrator").
        if role == SubAgentRole::Orchestrator {
            toolsets.push("delegation".to_string());
        }

        toolsets
    }

    async fn register_child_tools(&self, registry: &ToolRegistry, toolsets: &[String]) {
        use super::browser_tool::BrowserTool;
        use super::datetime_tool::{DateTimeTool, TimestampTool};
        use super::file_state::FileStateTool;
        use super::file_tools::{FileListTool, FileReadTool, FileSearchTool, FileWriteTool};
        use super::http_tool::HttpRequestTool;
        use super::patch_tool::PatchTool;
        use super::terminal_tool::TerminalTool;
        use super::vision_tool::VisionTool;
        use super::web_tools::{WebFetchTool, WebSearchTool};

        let toolset_set: std::collections::HashSet<&str> =
            toolsets.iter().map(String::as_str).collect();

        // Only register a tool when BOTH the child's computed toolset list
        // allows its toolset AND the parent did not explicitly disable it.
        // The child's toolset list was already filtered by the parent's
        // disabled toolsets + the hermes child-blocked list, so a tool whose
        // toolset is absent here is deliberately withheld from children.
        // Most tools default to toolset "builtin", which is always present
        // unless explicitly disabled by the parent.
        let builtin = "builtin";
        self.register_if_allowed(registry, &toolset_set, "terminal", builtin, TerminalTool)
            .await;
        self.register_if_allowed(registry, &toolset_set, "file_read", builtin, FileReadTool)
            .await;
        self.register_if_allowed(registry, &toolset_set, "file_write", builtin, FileWriteTool)
            .await;
        self.register_if_allowed(
            registry,
            &toolset_set,
            "file_search",
            builtin,
            FileSearchTool,
        )
        .await;
        self.register_if_allowed(registry, &toolset_set, "file_list", builtin, FileListTool)
            .await;
        self.register_if_allowed(registry, &toolset_set, "file_state", builtin, FileStateTool)
            .await;
        self.register_if_allowed(registry, &toolset_set, "web_search", builtin, WebSearchTool)
            .await;
        self.register_if_allowed(registry, &toolset_set, "web_fetch", builtin, WebFetchTool)
            .await;
        self.register_if_allowed(registry, &toolset_set, "browser", builtin, BrowserTool)
            .await;
        self.register_if_allowed(
            registry,
            &toolset_set,
            "http_request",
            builtin,
            HttpRequestTool,
        )
        .await;
        self.register_if_allowed(registry, &toolset_set, "patch", builtin, PatchTool)
            .await;
        self.register_if_allowed(registry, &toolset_set, "datetime", builtin, DateTimeTool)
            .await;
        self.register_if_allowed(registry, &toolset_set, "timestamp", builtin, TimestampTool)
            .await;
        self.register_if_allowed(
            registry,
            &toolset_set,
            "vision_analyze",
            builtin,
            VisionTool,
        )
        .await;

        // Recursive delegation: grant delegate_task ONLY to orchestrator
        // children (hermes: leaf children must never recursively delegate;
        // orchestrators retain the tool). The toolsets vec contains
        // "delegation" iff the effective role is orchestrator (see
        // compute_child_toolsets).
        if toolset_set.contains("delegation") {
            let raw_client = OpenAIClient::from_shared_http_client(
                self.client_config.clone(),
                self.http_client.clone(),
            );
            let child = SubAgentTool::with_parent_tool_policy(
                &raw_client,
                self.model.clone(),
                self.parent_depth + 1,
                self.parent_toolsets.clone(),
                self.parent_disabled_tools.clone(),
                self.parent_disabled_toolsets.clone(),
                self.database.clone(),
                self.event_tx.clone(),
            );
            let _ = registry.register(child).await;
        }
    }

    /// Register one child tool unless the parent disabled it or the child's
    /// computed toolset list does not include its toolset.
    async fn register_if_allowed<T: OperantTool + 'static>(
        &self,
        registry: &ToolRegistry,
        toolset_set: &std::collections::HashSet<&str>,
        name: &'static str,
        toolset: &'static str,
        tool: T,
    ) {
        if self.parent_disabled_tools.contains(name)
            || self.parent_disabled_toolsets.contains(toolset)
            || !toolset_set.contains(toolset)
        {
            return;
        }
        let _ = registry.register(tool).await;
    }

    fn ensure_supported_model(&self) -> std::result::Result<(), BoxedToolError> {
        if is_llama_model(&self.model) {
            return Err(format!(
                "Sub-agent model '{}' is rejected because Llama-family models are unsuitable for this tool-calling context",
                self.model
            )
            .into());
        }

        Ok(())
    }
}

/// Build the system prompt for a child agent based on role
fn build_child_system_prompt(
    goal: &str,
    context: Option<&str>,
    role: SubAgentRole,
    child_depth: u32,
    max_spawn_depth: u32,
) -> String {
    let mut parts = vec![
        "You are a focused subagent working on a specific delegated task.".to_string(),
        format!("\nYOUR TASK:\n{}", goal),
    ];

    if let Some(ctx) = context
        && !ctx.trim().is_empty()
    {
        parts.push(format!("\nCONTEXT:\n{}", ctx));
    }

    parts.push(
        "\nComplete this task using the tools available to you. ".to_string()
            + "When finished, provide a clear, concise summary of:\n"
            + "- What you did\n"
            + "- What you found or accomplished\n"
            + "- Any files you created or modified\n"
            + "- Any issues encountered\n\n"
            + "Be thorough but concise -- your response is returned to the "
            + "parent agent as a summary.",
    );

    // Add orchestrator-specific instructions
    if role == SubAgentRole::Orchestrator {
        let child_note = if child_depth + 1 >= max_spawn_depth {
            "Your own children MUST be leaves (cannot delegate further) because they would be at the depth floor.".to_string()
        } else {
            "Your own children can themselves be orchestrators or leaves, depending on the `role` you pass to delegate_task. Default is 'leaf'; pass role='orchestrator' explicitly when a child needs to further decompose its work.".to_string()
        };

        parts.push(
            "\n## Subagent Spawning (Orchestrator Role)\n".to_string()
                + "You have access to the `delegate_task` tool and CAN spawn "
                + "your own subagents to parallelize independent work.\n\n"
                + "WHEN to delegate:\n"
                + "- The goal decomposes into 2+ independent subtasks that can "
                + "run in parallel (e.g. research A and B simultaneously).\n"
                + "- A subtask is reasoning-heavy and would flood your context "
                + "with intermediate data.\n\n"
                + "WHEN NOT to delegate:\n"
                + "- Single-step mechanical work — do it directly.\n"
                + "- Trivial tasks you can execute in one or two tool calls.\n"
                + "- Re-delegating your entire assigned goal to one worker "
                + "(that's just pass-through with no value added).\n\n"
                + "Coordinate your workers' results and synthesize them before "
                + "reporting back to your parent. You are responsible for the "
                + "final summary, not your workers.\n\n"
                + &format!(
                    "NOTE: You are at depth {}. The delegation tree ",
                    child_depth
                )
                + &format!(
                    "is capped at max_spawn_depth={}. {}",
                    max_spawn_depth, child_note
                ),
        );
    }

    parts.join("\n")
}

/// Format batch results into a summary string
fn format_batch_results(results: &[String], errors: &[String]) -> String {
    let mut output = String::new();

    if !results.is_empty() {
        output.push_str("## Delegation Results\n\n");
        for (i, result) in results.iter().enumerate() {
            output.push_str(&format!("### Task {}\n{}\n\n", i + 1, result));
        }
    }

    if !errors.is_empty() {
        if !output.is_empty() {
            output.push_str("---\n\n");
        }
        output.push_str("## Errors\n\n");
        for (i, error) in errors.iter().enumerate() {
            output.push_str(&format!("{}. {}\n", i + 1, error));
        }
    }

    if output.is_empty() {
        output = "No results".to_string();
    }

    output
}

#[async_trait]
impl OperantTool for SubAgentTool {
    fn name(&self) -> &str {
        TOOL_NAME
    }

    fn description(&self) -> &str {
        "Delegate a focused task to an isolated sub-agent. Use this for deep analysis, \
        specialized coding investigation, architectural review, or other self-contained work. \
        Supports single-task mode (goal + context), batch mode (tasks array for parallel execution), \
        and background mode (background=true returns a handle immediately — poll with query=\"<id>\"). \
        Optional output_schema enforces a structured JSON contract on the child's final answer \
        (exactly one bounded retry on validation failure). \
        The sub-agent has a fresh conversation and does not inherit parent memory. \
        Role 'leaf' (default) cannot delegate further; role 'orchestrator' can spawn its own sub-agents."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<SubAgentArgs>(
            TOOL_NAME,
            "Delegate a focused task to an isolated Operant sub-agent",
        )
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let parsed = match parse_args(args) {
            Ok(parsed) => parsed,
            Err(error) => return ToolResult::error(TOOL_NAME, error),
        };

        // Query mode: poll a previously dispatched background delegation.
        if let Some(query_id) = parsed.query {
            return match async_delegation::get_record(&query_id) {
                Some(record) => {
                    let content = serde_json::to_value(&record).unwrap_or(Value::Null);
                    ToolResult::success_with_name(TOOL_NAME, TOOL_NAME, content)
                }
                None => ToolResult::error(
                    TOOL_NAME,
                    format!("No background delegation found with id '{query_id}'"),
                ),
            };
        }

        // Background mode: dispatch and return a handle immediately.
        if parsed.background.unwrap_or(false) {
            let Some(goal) = parsed.goal else {
                return ToolResult::error(
                    TOOL_NAME,
                    "Background delegation requires 'goal'".to_string(),
                );
            };
            if parsed.tasks.is_some() {
                return ToolResult::error(
                    TOOL_NAME,
                    "Background mode supports a single 'goal', not batch 'tasks'".to_string(),
                );
            }
            let role = parsed.role.unwrap_or(SubAgentRole::Leaf);
            let timeout = parsed
                .timeout_seconds
                .unwrap_or(DEFAULT_CHILD_TIMEOUT_SECONDS);
            return match self
                .dispatch_background(
                    goal,
                    parsed.context,
                    role,
                    parsed.max_iterations,
                    timeout,
                    parsed.output_schema,
                )
                .await
            {
                Ok(handle) => ToolResult::success_with_name(TOOL_NAME, TOOL_NAME, handle),
                Err(error) => ToolResult::error(TOOL_NAME, error.to_string()),
            };
        }

        // Check if batch mode
        if let Some(tasks) = parsed.tasks {
            if !tasks.is_empty() {
                // Batch mode: run tasks in parallel
                let task_params: Vec<_> = tasks
                    .into_iter()
                    .map(|t| {
                        let role = parsed.role.unwrap_or(SubAgentRole::Leaf);
                        let timeout = parsed
                            .timeout_seconds
                            .unwrap_or(DEFAULT_CHILD_TIMEOUT_SECONDS);
                        (
                            t.goal,
                            t.context,
                            role,
                            parsed.max_iterations,
                            timeout,
                            t.output_schema,
                        )
                    })
                    .collect();

                match self.call_batch(task_params).await {
                    Ok(content) => ToolResult {
                        tool_call_id: TOOL_NAME.to_string(),
                        name: TOOL_NAME.to_string(),
                        success: true,
                        content,
                        error: None,
                    },
                    Err(error) => ToolResult::error(TOOL_NAME, error.to_string()),
                }
            } else {
                ToolResult::error(TOOL_NAME, "Tasks array cannot be empty".to_string())
            }
        } else {
            // Single mode: require goal
            let goal = match parsed.goal {
                Some(g) => g,
                None => {
                    return ToolResult::error(
                        TOOL_NAME,
                        "Either 'goal' or 'tasks' must be provided".to_string(),
                    );
                }
            };

            let role = parsed.role.unwrap_or(SubAgentRole::Leaf);
            let timeout = parsed
                .timeout_seconds
                .unwrap_or(DEFAULT_CHILD_TIMEOUT_SECONDS);

            match self
                .call(
                    goal,
                    parsed.context,
                    role,
                    parsed.max_iterations,
                    timeout,
                    parsed.output_schema,
                )
                .await
            {
                Ok(content) => ToolResult {
                    tool_call_id: TOOL_NAME.to_string(),
                    name: TOOL_NAME.to_string(),
                    success: true,
                    content,
                    error: None,
                },
                Err(error) => ToolResult::error(TOOL_NAME, error.to_string()),
            }
        }
    }
}

fn parse_args(args: Value) -> Result<SubAgentArgs, String> {
    let parsed: SubAgentArgs = match args {
        Value::String(s) => serde_json::from_str(&s).map_err(|e| format!("Invalid JSON: {}", e))?,
        value => serde_json::from_value(value).map_err(|e| format!("Invalid arguments: {}", e))?,
    };

    if let Some(ref goal) = parsed.goal
        && goal.trim().is_empty()
    {
        return Err("goal must not be empty".to_string());
    }

    if let Some(ref tasks) = parsed.tasks {
        for (i, task) in tasks.iter().enumerate() {
            if task.goal.trim().is_empty() {
                return Err(format!("tasks[{}].goal must not be empty", i));
            }
        }
    }

    Ok(parsed)
}

fn is_llama_model(model: &str) -> bool {
    model.to_ascii_lowercase().contains("llama")
}

/// One-line preview with a hard character cap (for background handles and
/// completion summaries).
fn preview_of(text: &str, max_chars: usize) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out: String = flat.chars().take(max_chars).collect();
    if flat.chars().count() > max_chars {
        out.push('…');
    }
    out
}

/// Set the maximum spawn depth (for testing/config)
pub fn set_max_spawn_depth(depth: u32) {
    let clamped = depth.clamp(MIN_SPAWN_DEPTH, MAX_SPAWN_DEPTH_CAP);
    MAX_SPAWN_DEPTH.store(clamped, Ordering::Relaxed);
}

/// Set whether orchestrator role is enabled
pub fn set_orchestrator_enabled(enabled: bool) {
    ORCHESTRATOR_ENABLED.store(if enabled { 1 } else { 0 }, Ordering::Relaxed);
}

/// Set maximum concurrent children
pub fn set_max_concurrent_children(count: usize) {
    MAX_CONCURRENT_CHILDREN.store(count.clamp(1, 10) as u32, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn parse_args_accepts_object_argument() {
        let args = parse_args(serde_json::json!({
            "goal": "analyze this module"
        }))
        .unwrap();
        assert_eq!(args.goal, Some("analyze this module".to_string()));
    }

    #[test]
    fn parse_args_accepts_raw_string_as_goal() {
        let args = parse_args(Value::String(r#"{"goal": "analyze this"}"#.to_string())).unwrap();
        assert_eq!(args.goal, Some("analyze this".to_string()));
    }

    #[test]
    fn parse_args_rejects_empty_goal() {
        let error = parse_args(serde_json::json!({ "goal": "  " })).unwrap_err();
        assert!(error.contains("goal") || error.contains("empty"));
    }

    #[test]
    fn parse_args_accepts_batch_tasks() {
        let args = parse_args(serde_json::json!({
            "tasks": [
                { "goal": "task 1" },
                { "goal": "task 2", "context": "some context" }
            ]
        }))
        .unwrap();
        assert_eq!(args.tasks.unwrap().len(), 2);
    }

    #[test]
    fn parse_args_accepts_role() {
        let args = parse_args(serde_json::json!({
            "goal": "test",
            "role": "orchestrator"
        }))
        .unwrap();
        assert_eq!(args.role, Some(SubAgentRole::Orchestrator));
    }

    #[test]
    fn parse_args_accepts_output_schema() {
        let args = parse_args(serde_json::json!({
            "goal": "summarize",
            "output_schema": { "type": "object", "required": ["summary"] }
        }))
        .unwrap();
        let schema = args.output_schema.unwrap();
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["required"][0], "summary");
    }

    #[test]
    fn parse_args_accepts_background_and_query() {
        let bg = parse_args(serde_json::json!({
            "goal": "deep dive",
            "background": true
        }))
        .unwrap();
        assert_eq!(bg.background, Some(true));

        let q = parse_args(serde_json::json!({ "query": "dlg-1-0" })).unwrap();
        assert_eq!(q.query.as_deref(), Some("dlg-1-0"));
    }

    #[test]
    fn parse_args_batch_task_carries_output_schema() {
        let args = parse_args(serde_json::json!({
            "tasks": [
                {
                    "goal": "task 1",
                    "output_schema": { "type": "object", "required": ["answer"] }
                }
            ]
        }))
        .unwrap();
        let task = args.tasks.unwrap().pop().unwrap();
        assert_eq!(task.output_schema.unwrap()["required"][0], "answer");
    }

    #[test]
    fn model_guard_rejects_llama_models() {
        assert!(is_llama_model("meta-llama/Llama-3.1-70B-Instruct"));
        assert!(is_llama_model("llama-3.2"));
    }

    #[test]
    fn model_guard_allows_non_llama_models() {
        assert!(!is_llama_model("gpt-4.1"));
        assert!(!is_llama_model("claude-3-5-sonnet"));
    }

    #[test]
    fn build_leaf_system_prompt_contains_goal() {
        let prompt =
            build_child_system_prompt("Analyze this", Some("context"), SubAgentRole::Leaf, 1, 2);
        assert!(prompt.contains("Analyze this"));
        assert!(prompt.contains("context"));
        assert!(!prompt.contains("Orchestrator Role"));
    }

    #[test]
    fn build_orchestrator_system_prompt_contains_delegation_info() {
        let prompt =
            build_child_system_prompt("Analyze this", None, SubAgentRole::Orchestrator, 1, 2);
        assert!(prompt.contains("Orchestrator Role"));
        assert!(prompt.contains("delegate_task"));
    }

    #[test]
    fn compute_child_toolsets_defaults_to_builtin_when_parent_passes_none() {
        // hermes fallback: a parent that supplies no toolset list (the CLI
        // registers the tool with vec![]) must not produce zero-tool children.
        let tool = SubAgentTool::with_parent_tool_policy(
            &OpenAIClient::new(crate::client::ClientConfig::default()),
            "gpt-4.1",
            0,
            vec![],
            std::collections::HashSet::new(),
            std::collections::HashSet::new(),
            Arc::new(Database::init(PathBuf::from("test_sub_ts.db")).unwrap()),
            None,
        );
        let ts = tool.compute_child_toolsets(SubAgentRole::Leaf);
        assert!(
            ts.contains(&"builtin".to_string()),
            "builtin fallback missing: {ts:?}"
        );
        assert!(!ts.contains(&"delegation".to_string()));
        assert!(!ts.contains(&"memory".to_string()));
        assert!(!ts.contains(&"clarify".to_string()));
        assert!(!ts.contains(&"code_execution".to_string()));
    }

    #[test]
    fn compute_child_toolsets_orchestrator_retains_delegation() {
        let tool = SubAgentTool::with_parent_tool_policy(
            &OpenAIClient::new(crate::client::ClientConfig::default()),
            "gpt-4.1",
            0,
            vec!["builtin".to_string()],
            std::collections::HashSet::new(),
            std::collections::HashSet::new(),
            Arc::new(Database::init(PathBuf::from("test_sub_orch.db")).unwrap()),
            None,
        );
        // Orchestrators retain the delegation toolset (hermes
        // `_blocked_toolsets_for_role` discards delegate_task from the
        // blocklist when role == "orchestrator").
        let ts = tool.compute_child_toolsets(SubAgentRole::Orchestrator);
        assert!(ts.contains(&"delegation".to_string()));
        // ...but the child-blocked toolsets are still stripped.
        assert!(!ts.contains(&"memory".to_string()));
        assert!(!ts.contains(&"clarify".to_string()));
    }

    #[test]
    fn compute_child_toolsets_honors_parent_disabled_toolsets() {
        let disabled_toolsets =
            std::collections::HashSet::from(["builtin".to_string(), "web".to_string()]);
        let tool = SubAgentTool::with_parent_tool_policy(
            &OpenAIClient::new(crate::client::ClientConfig::default()),
            "gpt-4.1",
            0,
            vec!["builtin".to_string(), "web".to_string()],
            std::collections::HashSet::new(),
            disabled_toolsets,
            Arc::new(Database::init(PathBuf::from("test_sub_disabled.db")).unwrap()),
            None,
        );
        let ts = tool.compute_child_toolsets(SubAgentRole::Leaf);
        // "web" is stripped AND the builtin fallback must NOT re-add builtin:
        // the parent supplied an explicit list and disabled builtin itself
        // (hermes: children never gain a toolset the parent disabled).
        assert!(!ts.contains(&"web".to_string()));
        assert!(!ts.contains(&"builtin".to_string()));
    }

    #[test]
    fn compute_child_toolsets_supplied_list_fully_stripped_yields_empty() {
        // Parent supplied an explicit toolset list, but every entry is
        // hermes-blocked for children. hermes semantics: non-empty parent
        // list → intersect/strip only — the child must get ZERO tools, NOT a
        // silent builtin re-addition.
        let tool = SubAgentTool::with_parent_tool_policy(
            &OpenAIClient::new(crate::client::ClientConfig::default()),
            "gpt-4.1",
            0,
            vec!["memory".to_string(), "delegation".to_string()],
            std::collections::HashSet::new(),
            std::collections::HashSet::new(),
            Arc::new(Database::init(PathBuf::from("test_sub_stripped.db")).unwrap()),
            None,
        );
        let ts = tool.compute_child_toolsets(SubAgentRole::Leaf);
        assert!(
            ts.is_empty(),
            "fully-stripped parent list must yield empty, got {ts:?}"
        );

        // Orchestrator role still retains delegation (its own sub-agents),
        // but never the blocked memory toolset.
        let orch_ts = tool.compute_child_toolsets(SubAgentRole::Orchestrator);
        assert_eq!(orch_ts, vec!["delegation".to_string()]);
    }

    #[tokio::test]
    async fn child_registry_grants_delegate_task_only_to_orchestrators() {
        use crate::tools::ToolRegistry;

        let client = OpenAIClient::new(crate::client::ClientConfig::default());
        let database = Arc::new(Database::init(PathBuf::from("test_child_reg.db")).unwrap());
        let tool = SubAgentTool::with_parent_tool_policy(
            &client,
            "gpt-4.1",
            0,
            vec![],
            std::collections::HashSet::new(),
            std::collections::HashSet::new(),
            database,
            None,
        );

        // Leaf child: core tools yes, delegate_task NO (hermes: leaf children
        // must never recursively delegate).
        let leaf_ts = tool.compute_child_toolsets(SubAgentRole::Leaf);
        let leaf_registry = ToolRegistry::new(Duration::from_secs(5));
        tool.register_child_tools(&leaf_registry, &leaf_ts).await;
        assert!(
            leaf_registry.contains("terminal").await,
            "leaf child lost its core tools"
        );
        assert!(
            !leaf_registry.contains(TOOL_NAME).await,
            "leaf child must never receive delegate_task"
        );

        // Orchestrator child: retains delegate_task for recursive delegation
        // (hermes `_blocked_toolsets_for_role` when role == "orchestrator").
        let orch_ts = tool.compute_child_toolsets(SubAgentRole::Orchestrator);
        let orch_registry = ToolRegistry::new(Duration::from_secs(5));
        tool.register_child_tools(&orch_registry, &orch_ts).await;
        assert!(
            orch_registry.contains(TOOL_NAME).await,
            "orchestrator child must retain delegate_task"
        );
    }

    #[tokio::test]
    async fn child_registry_honors_parent_disabled_tools() {
        use crate::tools::ToolRegistry;

        let client = OpenAIClient::new(crate::client::ClientConfig::default());
        let disabled_tools = std::collections::HashSet::from(["terminal".to_string()]);
        let database = Arc::new(Database::init(PathBuf::from("test_child_disabled.db")).unwrap());
        let tool = SubAgentTool::with_parent_tool_policy(
            &client,
            "gpt-4.1",
            0,
            vec![],
            disabled_tools,
            std::collections::HashSet::new(),
            database,
            None,
        );

        let ts = tool.compute_child_toolsets(SubAgentRole::Leaf);
        let registry = ToolRegistry::new(Duration::from_secs(5));
        tool.register_child_tools(&registry, &ts).await;
        // A tool the parent explicitly disabled must not leak into children.
        assert!(!registry.contains("terminal").await);
        assert!(registry.contains("file_read").await);
    }

    #[test]
    fn format_batch_results_handles_empty() {
        let result = format_batch_results(&[], &[]);
        assert_eq!(result, "No results");
    }

    #[test]
    fn format_batch_results_formats_results() {
        let result = format_batch_results(&["result 1".to_string(), "result 2".to_string()], &[]);
        assert!(result.contains("Task 1"));
        assert!(result.contains("result 1"));
        assert!(result.contains("Task 2"));
        assert!(result.contains("result 2"));
    }

    #[test]
    fn format_batch_results_includes_errors() {
        let result = format_batch_results(&["ok".to_string()], &["error 1".to_string()]);
        assert!(result.contains("Errors"));
        assert!(result.contains("error 1"));
    }
}
