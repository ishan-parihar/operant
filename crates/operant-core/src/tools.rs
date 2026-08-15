//! Tool system for Operant-RS
//!
//! This module provides the core tool infrastructure including:
//! - `OperantTool` trait for defining tools
//! - `ToolRegistry` for managing and executing tools
//! - Built-in tools for common operations

pub mod aft_tools;
pub mod browser_camofox_state;
pub mod browser_cdp_tool;
pub mod browser_dialog_tool;
pub mod browser_downloader;
pub mod browser_tool;
pub mod builtin;
pub mod cdp_utils;
pub mod checkpoint_tool;
pub mod clarify_tool;
pub mod code_execution;
pub mod config_tool;
pub mod cron_tool;
pub mod datetime_tool;
pub mod debug_helpers;
pub mod file_state;
pub mod file_tools;
pub mod http_tool;
pub mod igs;
pub mod image_generation_tool;
pub mod insights_tool;
pub mod kanban_tool;
pub mod lcm_tools;
pub mod learning_mutation_tool;
pub mod mcp_tool;
pub mod memory_tools;
pub mod neutts_synth;
pub mod notification_tool;
pub mod openrouter_client;
pub mod osv_check;
pub mod patch_tool;
pub mod process_tool;
pub mod send_message_tool;
pub mod session_search_tool;
pub mod skills_tool;
pub mod spotify_tool;
pub mod sub_agent_tool;
pub mod terminal_backend;
pub mod terminal_tool;
pub mod todo_tool;
pub mod tool_backend_helpers;
pub mod transcription_tool;
pub mod tts_command_provider;
pub mod tts_provider;
pub mod tts_registry;
pub mod tts_tool;
pub mod video_analysis_tool;
pub mod vision_tool;
pub mod web_providers;
pub mod web_tools;
pub mod xai_http;

pub use web_providers::{
    DDGProvider, ExaProvider, SearXNGProvider, TavilyProvider, WebSearchProvider, WebSearchResult,
};
pub mod discord_tool;
pub mod feishu_tool;
pub mod home_assistant_tool;

// Re-export commonly used types
pub use aft_tools::register_aft_tools;
pub use browser_cdp_tool::BrowserCdpTool;
pub use browser_dialog_tool::BrowserDialogTool;
pub use builtin::{
    ApprovalTool, ClarifyTool, CodeExecutionTool, DateTimeTool, FileListTool, FileReadTool,
    FileSearchTool, FileWriteTool, HttpRequestTool, ImageGenerationTool, MemoryRecallTool,
    MemorySearchTool, MemoryStoreTool, PatchTool, SubAgentTool, TerminalTool, TimestampTool,
    TodoTool, TtsTool, VideoAnalysisTool, VisionTool, WebFetchTool, WebSearchTool,
    builtin_tool_names, register_builtin_tools, register_builtin_tools_with_sub_agent,
};
pub use checkpoint_tool::{
    Checkpoint, CheckpointConfig, CheckpointManager, CheckpointTool, get_checkpoint_manager,
};
pub use config_tool::{ConfigManageArgs, ConfigManageTool};
pub use cron_tool::CronTool;
pub use discord_tool::{DiscordAdminTool, DiscordTool};
pub use feishu_tool::{FeishuDocTool, FeishuDriveTool};
pub use home_assistant_tool::HomeAssistantTool;
pub use igs::{WebExtractTool, WebScrapeTool};
pub use kanban_tool::KanbanTool;
pub use lcm_tools::register_lcm_tools;
pub use mcp_tool::McpManagementTool;
pub use osv_check::OsvCheckTool;
pub use process_tool::ProcessTool;
pub use send_message_tool::SendMessageTool;
pub use session_search_tool::{SessionMeta, SessionResult, SessionSearchTool};
pub use skills_tool::{
    SkillManageTool, SkillMeta, SkillTreeValidation, SkillViewTool, SkillsTool,
    collect_skill_children, validate_skill_tree,
};
pub use spotify_tool::{
    SpotifyAlbumsTool, SpotifyDevicesTool, SpotifyLibraryTool, SpotifyPlaybackTool,
    SpotifyPlaylistsTool, SpotifyQueueTool, SpotifySearchTool,
};
pub use transcription_tool::TranscriptionTool;
pub use tts_command_provider::CommandProvider;
pub use tts_provider::{AudioFormat, TtsError, TtsProvider};
pub use tts_registry::TtsPluginRegistry;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::RwLock;
use tokio::time::timeout;
use tracing::{debug, error, info, instrument, warn};

use crate::error::{Error, Result};
use crate::schema::ToolSchema;

/// Result of tool execution
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolResult {
    /// Tool call ID this result is for
    pub tool_call_id: String,
    /// Tool name (for API compatibility)
    pub name: String,
    /// Whether the execution succeeded
    pub success: bool,
    /// Result content (serialized JSON or error message)
    pub content: String,
    /// Optional error details
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ToolResult {
    #[expect(
        clippy::expect_used,
        reason = "invariant guaranteed by surrounding validation"
    )]
    /// Create a successful result
    pub fn success<T: Serialize>(tool_call_id: impl Into<String>, content: T) -> Self {
        let content =
            serde_json::to_string(&content).expect("serializable tool result always serializes");
        Self {
            tool_call_id: tool_call_id.into(),
            name: String::new(),
            success: true,
            content,
            error: None,
        }
    }

    #[expect(
        clippy::expect_used,
        reason = "invariant guaranteed by surrounding validation"
    )]
    /// Create a successful result with tool name
    pub fn success_with_name<T: Serialize>(
        name: impl Into<String>,
        tool_call_id: impl Into<String>,
        content: T,
    ) -> Self {
        let content =
            serde_json::to_string(&content).expect("serializable tool result always serializes");
        Self {
            tool_call_id: tool_call_id.into(),
            name: name.into(),
            success: true,
            content,
            error: None,
        }
    }

    /// Create an error result
    pub fn error(tool_call_id: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            tool_call_id: tool_call_id.into(),
            name: String::new(),
            success: false,
            content: String::new(),
            error: Some(error.into()),
        }
    }

    /// Create an error result with tool name
    pub fn error_with_name(
        name: impl Into<String>,
        tool_call_id: impl Into<String>,
        error: impl Into<String>,
    ) -> Self {
        Self {
            tool_call_id: tool_call_id.into(),
            name: name.into(),
            success: false,
            content: String::new(),
            error: Some(error.into()),
        }
    }

    /// Get the content as a parsed JSON value
    pub fn parse_content<T: for<'de> Deserialize<'de>>(&self) -> Result<T> {
        serde_json::from_str(&self.content)
            .map_err(|e| Error::ParseResponse(format!("Failed to parse tool result: {}", e)))
    }
}

#[async_trait]
pub trait OperantTool: Send + Sync {
    fn name(&self) -> &str;

    fn description(&self) -> &str;

    fn schema(&self) -> ToolSchema;

    fn toolset(&self) -> &str {
        "builtin"
    }

    fn is_available(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value, context: ToolContext) -> ToolResult;
}

/// Context passed to tool execution
#[derive(Debug, Clone, Default)]
pub struct ToolContext {
    /// Additional metadata about the execution
    pub metadata: HashMap<String, String>,
}

impl ToolContext {
    /// Create a new context with metadata
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Get a metadata value
    pub fn get(&self, key: &str) -> Option<&str> {
        self.metadata.get(key).map(|s| s.as_str())
    }
}

/// Sandboxed tool executor with timeout support. The default `timeout`
/// applies to every tool unless an override is present (see
/// [`ToolRegistry::set_tool_timeout`]).
struct ToolExecutor {
    pub(crate) timeout: Duration,
    /// Per-tool timeout overrides keyed by tool name. LLM-backed tools
    /// (e.g. `lcm_assert action="extract"`, which runs a reasoning-model
    /// completion) legitimately exceed the generic 30s cap. An
    /// `Arc<std::sync::Mutex>` is fine: written once at boot from async
    /// context (tokio's `blocking_write` would panic there) and read for a
    /// copy on every execution (no await while held); clones share the same
    /// map so a boot-time override propagates to every registry copy.
    overrides: Arc<std::sync::Mutex<HashMap<String, Duration>>>,
}

impl ToolExecutor {
    fn new(timeout: Duration) -> Self {
        Self {
            timeout,
            overrides: Arc::new(std::sync::Mutex::new(HashMap::new())),
        }
    }

    fn set_override(&self, name: &str, timeout: Duration) {
        if let Ok(mut overrides) = self.overrides.lock() {
            overrides.insert(name.to_string(), timeout);
        }
    }

    /// Effective timeout for a tool: the per-tool override when present,
    /// otherwise the registry default.
    fn timeout_for(&self, tool_name: &str) -> Duration {
        self.overrides
            .lock()
            .map(|overrides| overrides.get(tool_name).copied().unwrap_or(self.timeout))
            .unwrap_or(self.timeout)
    }

    async fn execute_with_timeout(
        &self,
        tool: Arc<dyn OperantTool>,
        tool_name: String,
        tool_call_id: String,
        args: Value,
        context: ToolContext,
    ) -> ToolResult {
        let effective = self.timeout_for(&tool_name);
        let result = timeout(effective, tool.execute(args, context)).await;

        match result {
            Ok(mut result) => {
                // Ensure the result has the correct tool_call_id and name
                result.tool_call_id = tool_call_id;
                result.name = tool_name;
                result
            }
            Err(_) => {
                warn!(tool = %tool_name, timeout = ?effective, "Tool execution timed out");
                ToolResult::error_with_name(
                    &tool_name,
                    &tool_call_id,
                    format!("Tool timed out after {:?}", effective),
                )
            }
        }
    }
}

pub struct ToolRegistry {
    tools: Arc<RwLock<HashMap<String, Arc<dyn OperantTool>>>>,
    disabled_names: Arc<RwLock<HashSet<String>>>,
    disabled_toolsets: Arc<RwLock<HashSet<String>>>,
    executor: ToolExecutor,
}

impl Clone for ToolRegistry {
    fn clone(&self) -> Self {
        Self {
            tools: Arc::clone(&self.tools),
            disabled_names: Arc::clone(&self.disabled_names),
            disabled_toolsets: Arc::clone(&self.disabled_toolsets),
            executor: ToolExecutor {
                timeout: self.executor.timeout,
                overrides: Arc::clone(&self.executor.overrides),
            },
        }
    }
}

impl ToolRegistry {
    pub fn new(timeout: Duration) -> Self {
        Self {
            tools: Arc::new(RwLock::new(HashMap::new())),
            disabled_names: Arc::new(RwLock::new(HashSet::new())),
            disabled_toolsets: Arc::new(RwLock::new(HashSet::new())),
            executor: ToolExecutor::new(timeout),
        }
    }

    /// Override the execution timeout for a single tool. LLM-backed tools
    /// (e.g. `lcm_assert action="extract"`) may need a longer window than
    /// the generic default; everything else keeps the registry timeout.
    pub fn set_tool_timeout(&self, name: &str, timeout: Duration) {
        self.executor.set_override(name, timeout);
    }

    #[instrument(skip(self, tool), fields(tool = % tool.name()))]
    pub async fn register<T: OperantTool + 'static>(&self, tool: T) -> Result<()> {
        let name = tool.name().to_string();
        let mut tools = self.tools.write().await;

        if tools.contains_key(&name) {
            warn!(tool = %name, "Tool already registered, replacing");
        }

        tools.insert(name.clone(), Arc::new(tool));
        info!(tool = %name, "Tool registered successfully");
        Ok(())
    }

    pub async fn disable_tool(&self, name: &str) {
        let mut disabled = self.disabled_names.write().await;
        disabled.insert(name.to_string());
    }

    pub async fn enable_tool(&self, name: &str) {
        let mut disabled = self.disabled_names.write().await;
        disabled.remove(name);
    }

    pub async fn disable_toolset(&self, toolset: &str) {
        let mut disabled = self.disabled_toolsets.write().await;
        disabled.insert(toolset.to_string());
    }

    pub async fn enable_toolset(&self, toolset: &str) {
        let mut disabled = self.disabled_toolsets.write().await;
        disabled.remove(toolset);
    }

    pub async fn set_disabled_tools(&self, names: HashSet<String>) {
        let mut disabled = self.disabled_names.write().await;
        *disabled = names;
    }

    pub async fn set_disabled_toolsets(&self, toolsets: HashSet<String>) {
        let mut disabled = self.disabled_toolsets.write().await;
        *disabled = toolsets;
    }

    pub async fn get_schemas(&self) -> Vec<ToolSchema> {
        let tools = self.tools.read().await;
        let disabled_names = self.disabled_names.read().await;
        let disabled_toolsets = self.disabled_toolsets.read().await;
        tools
            .values()
            .filter(|t| {
                if t.is_available() {
                    !disabled_names.contains(t.name()) && !disabled_toolsets.contains(t.toolset())
                } else {
                    false
                }
            })
            .map(|t| t.schema())
            .collect()
    }

    pub async fn get_available_schemas_filtered(&self, filter: &[String]) -> Vec<ToolSchema> {
        let tools = self.tools.read().await;
        let disabled_names = self.disabled_names.read().await;
        let disabled_toolsets = self.disabled_toolsets.read().await;
        tools
            .values()
            .filter(|t| {
                if !t.is_available() {
                    return false;
                }
                if disabled_names.contains(t.name()) {
                    return false;
                }
                if disabled_toolsets.contains(t.toolset()) {
                    return false;
                }
                if !filter.is_empty() && !filter.contains(&t.name().to_string()) {
                    return false;
                }
                true
            })
            .map(|t| t.schema())
            .collect()
    }

    pub async fn get(&self, name: &str) -> Option<Arc<dyn OperantTool>> {
        let tools = self.tools.read().await;
        tools.get(name).cloned()
    }

    pub async fn unregister(&self, name: &str) -> bool {
        let mut tools = self.tools.write().await;
        tools.remove(name).is_some()
    }

    pub async fn contains(&self, name: &str) -> bool {
        let tools = self.tools.read().await;
        tools.contains_key(name)
    }

    pub async fn len(&self) -> usize {
        let tools = self.tools.read().await;
        tools.len()
    }

    pub async fn is_empty(&self) -> bool {
        let tools = self.tools.read().await;
        tools.is_empty()
    }

    #[instrument(skip(self, args, context), fields(tool = % tool_name))]
    pub async fn execute(
        &self,
        tool_name: &str,
        tool_call_id: &str,
        args: Value,
        context: ToolContext,
    ) -> Result<ToolResult> {
        let tool = {
            let tools = self.tools.read().await;
            tools.get(tool_name).cloned()
        };

        match tool {
            Some(tool) => {
                let name = tool_name.to_string();
                let id = tool_call_id.to_string();
                debug!(tool = %name, args = ?args, "Executing tool");
                let result = self
                    .executor
                    .execute_with_timeout(tool, name, id, args, context)
                    .await;
                Ok(result)
            }
            None => {
                error!(tool = %tool_name, "Tool not found in registry");
                Err(Error::ToolNotFound {
                    name: tool_name.to_string(),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use schemars::JsonSchema;

    #[derive(JsonSchema, Deserialize)]
    #[serde(rename_all = "camelCase")]
    #[expect(dead_code, reason = "test-only argument struct")]
    struct TestArgs {
        query: String,
        limit: Option<i32>,
    }

    struct TestTool;

    #[async_trait]
    impl OperantTool for TestTool {
        fn name(&self) -> &str {
            "test_tool"
        }

        fn description(&self) -> &str {
            "A test tool for unit testing"
        }

        fn schema(&self) -> ToolSchema {
            ToolSchema::from_type::<TestArgs>("test_tool", "A test tool")
        }

        async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
            if let Some(query) = args.get("query").and_then(|v| v.as_str()) {
                ToolResult::success(
                    "call_1",
                    serde_json::json!({ "result": format!("Processed: {}", query) }),
                )
            } else {
                ToolResult::error_with_name("test_tool", "call_1", "Missing 'query' argument")
            }
        }
    }

    #[tokio::test]
    async fn test_registry_operations() {
        let registry = ToolRegistry::new(Duration::from_secs(5));

        // Register a tool
        registry.register(TestTool).await.unwrap();

        // Check tool exists
        assert!(registry.contains("test_tool").await);
        assert_eq!(registry.len().await, 1);

        // Get schemas
        let schemas = registry.get_schemas().await;
        assert_eq!(schemas.len(), 1);
        assert_eq!(schemas[0].name, "test_tool");
    }

    #[tokio::test]
    async fn test_tool_execution() {
        let registry = ToolRegistry::new(Duration::from_secs(5));
        registry.register(TestTool).await.unwrap();

        let args = serde_json::json!({
            "query": "test query",
            "limit": 10
        });

        let result = registry
            .execute("test_tool", "call_1", args, ToolContext::default())
            .await
            .unwrap();

        assert!(result.success);
        assert!(result.content.contains("Processed:"));
    }

    #[tokio::test]
    async fn test_tool_not_found() {
        let registry = ToolRegistry::new(Duration::from_secs(5));

        let result = registry
            .execute(
                "nonexistent",
                "call_1",
                serde_json::json!({}),
                ToolContext::default(),
            )
            .await;

        assert!(result.is_err());
        match result.unwrap_err() {
            Error::ToolNotFound { name } => assert_eq!(name, "nonexistent"),
            _ => panic!("Expected ToolNotFound error"),
        }
    }

    /// Test that ToolResult::success serializes correctly for normal types
    #[test]
    fn test_toolresult_success_serialization() {
        let result = ToolResult::success("call_1", serde_json::json!({"key": "value"}));
        assert!(result.success);
        assert_eq!(result.tool_call_id, "call_1");
        assert_eq!(result.content, r#"{"key":"value"}"#);
        assert!(result.error.is_none());

        let result2 = ToolResult::success_with_name("my_tool", "call_2", 42);
        assert!(result2.success);
        assert_eq!(result2.name, "my_tool");
        assert_eq!(result2.content, "42");
    }

    /// Per-tool timeout overrides: a tool with an override gets the longer
    /// window; every other tool keeps the registry default. Regression for
    /// the `lcm_assert action="extract"` live failure where the reasoning-
    /// model LLM call was killed by the generic 30s tool timeout.
    #[tokio::test]
    async fn test_per_tool_timeout_override() {
        let registry = ToolRegistry::new(Duration::from_secs(5));
        registry.set_tool_timeout("slow_tool", Duration::from_secs(120));
        assert_eq!(
            registry.executor.timeout_for("slow_tool"),
            Duration::from_secs(120)
        );
        assert_eq!(
            registry.executor.timeout_for("other_tool"),
            Duration::from_secs(5)
        );

        // Overrides survive registry clones (shared executor state).
        let cloned = registry.clone();
        assert_eq!(
            cloned.executor.timeout_for("slow_tool"),
            Duration::from_secs(120)
        );
    }

    /// The override actually extends the execution window: a tool that would
    /// time out under the default succeeds under its override.
    #[tokio::test]
    async fn test_override_extends_execution_window() {
        struct SlowTool;
        #[async_trait]
        impl OperantTool for SlowTool {
            fn name(&self) -> &str {
                "slow_tool"
            }
            fn description(&self) -> &str {
                "sleeps past the default timeout"
            }
            fn schema(&self) -> ToolSchema {
                ToolSchema::from_type::<TestArgs>("slow_tool", "slow")
            }
            async fn execute(&self, _args: Value, _context: ToolContext) -> ToolResult {
                tokio::time::sleep(Duration::from_millis(200)).await;
                ToolResult::success("call_1", serde_json::json!({ "done": true }))
            }
        }

        // Default timeout (50ms) → would time out.
        let registry = ToolRegistry::new(Duration::from_millis(50));
        registry.register(SlowTool).await.unwrap();
        let result = registry
            .execute(
                "slow_tool",
                "call_1",
                serde_json::json!({}),
                ToolContext::default(),
            )
            .await
            .unwrap();
        assert!(!result.success, "default short timeout must fail the tool");
        assert!(result.error.unwrap_or_default().contains("timed out"));

        // Override (500ms) → succeeds.
        let registry = ToolRegistry::new(Duration::from_millis(50));
        registry.set_tool_timeout("slow_tool", Duration::from_millis(500));
        registry.register(SlowTool).await.unwrap();
        let result = registry
            .execute(
                "slow_tool",
                "call_1",
                serde_json::json!({}),
                ToolContext::default(),
            )
            .await
            .unwrap();
        assert!(result.success, "override must extend the execution window");
    }
}
