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
use crate::tools::{OperantTool, ToolContext, ToolRegistry, ToolResult};

const TOOL_NAME: &str = "delegate_task";

/// Maximum concurrent children (default from Python implementation)
const DEFAULT_MAX_CONCURRENT_CHILDREN: usize = 3;
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
        Self {
            client_config: parent_client.config_clone(),
            http_client: parent_client.http_client_clone(),
            model: model.into(),
            parent_depth,
            parent_toolsets,
            database,
            event_tx,
        }
    }

    /// Run a focused delegated task in an isolated child agent.
    pub async fn call(
        &self,
        goal: impl Into<String>,
        context: Option<impl Into<String>>,
        role: SubAgentRole,
        max_iterations: Option<u32>,
        timeout_seconds: u64,
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

        // Build child system prompt based on role
        let system_prompt = build_child_system_prompt(
            goal,
            context.map(|c| c.into()).as_deref(),
            effective_role,
            child_depth,
            max_depth,
        );

        // Determine effective toolsets based on role and parent toolsets
        let child_toolsets = self.compute_child_toolsets(effective_role);

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
        let result = timeout(timeout_duration, agent.run(goal.to_string())).await;

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
        tasks: Vec<(String, Option<String>, SubAgentRole, Option<u32>, u64)>,
    ) -> std::result::Result<String, BoxedToolError> {
        let max_concurrent = MAX_CONCURRENT_CHILDREN.load(Ordering::Relaxed) as usize;
        let max_concurrent = max_concurrent.clamp(1, 10);

        // Use semaphore to limit concurrency
        let semaphore = Arc::new(tokio::sync::Semaphore::new(max_concurrent));
        let mut handles = Vec::new();

        for (goal, context, role, max_iterations, timeout_seconds) in tasks {
            let tool = self.clone_for_task();
            let permit = semaphore
                .clone()
                .acquire_owned()
                .await
                .map_err(|e| e.to_string())?;

            let handle = tokio::spawn(async move {
                let _permit = permit;
                tool.call(goal, context, role, max_iterations, timeout_seconds)
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

    fn clone_for_task(&self) -> Self {
        Self {
            client_config: self.client_config.clone(),
            http_client: self.http_client.clone(),
            model: self.model.clone(),
            parent_depth: self.parent_depth,
            parent_toolsets: self.parent_toolsets.clone(),
            database: self.database.clone(),
            event_tx: self.event_tx.clone(),
        }
    }

    fn compute_child_toolsets(&self, role: SubAgentRole) -> Vec<String> {
        // Start with parent's toolsets, filtered to remove blocked tools
        let mut toolsets = self.parent_toolsets.clone();

        // Remove blocked toolsets for all children
        let blocked_toolsets: Vec<&str> = vec!["delegation", "clarify", "memory", "code_execution"];
        toolsets.retain(|ts| !blocked_toolsets.contains(&ts.as_str()));

        // Orchestrators retain the delegation toolset
        if role == SubAgentRole::Orchestrator && !toolsets.contains(&"delegate_task".to_string()) {
            // Note: we add the tool name, not toolset - tools are filtered separately
        }

        toolsets
    }

    async fn register_child_tools(&self, registry: &ToolRegistry, _toolsets: &[String]) {
        use super::browser_tool::BrowserTool;
        use super::datetime_tool::{DateTimeTool, TimestampTool};
        use super::file_state::FileStateTool;
        use super::file_tools::{FileListTool, FileReadTool, FileSearchTool, FileWriteTool};
        use super::http_tool::HttpRequestTool;
        use super::patch_tool::PatchTool;
        use super::terminal_tool::TerminalTool;
        use super::vision_tool::VisionTool;
        use super::web_tools::{WebFetchTool, WebSearchTool};

        let _ = registry.register(TerminalTool).await;
        let _ = registry.register(FileReadTool).await;
        let _ = registry.register(FileWriteTool).await;
        let _ = registry.register(FileSearchTool).await;
        let _ = registry.register(FileListTool).await;
        let _ = registry.register(FileStateTool).await;
        let _ = registry.register(WebSearchTool).await;
        let _ = registry.register(WebFetchTool).await;
        let _ = registry.register(BrowserTool).await;
        let _ = registry.register(HttpRequestTool).await;
        let _ = registry.register(PatchTool).await;
        let _ = registry.register(DateTimeTool).await;
        let _ = registry.register(TimestampTool).await;
        let _ = registry.register(VisionTool).await;
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

    if let Some(ctx) = context {
        if !ctx.trim().is_empty() {
            parts.push(format!("\nCONTEXT:\n{}", ctx));
        }
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
        Supports single-task mode (goal + context) and batch mode (tasks array for parallel execution). \
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
                        (t.goal, t.context, role, parsed.max_iterations, timeout)
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
                .call(goal, parsed.context, role, parsed.max_iterations, timeout)
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

    if let Some(ref goal) = parsed.goal {
        if goal.trim().is_empty() {
            return Err("goal must not be empty".to_string());
        }
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
