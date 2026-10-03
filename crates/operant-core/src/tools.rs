//! Tool system for Operant-RS
//!
//! This module provides the core tool infrastructure including:
//! - `OperantTool` trait for defining tools
//! - `ToolRegistry` for managing and executing tools
//! - Built-in tools for common operations

pub mod aft_tools;
pub mod async_delegation;
pub mod browser_camofox_state;
pub mod browser_cdp_tool;
pub mod browser_dialog_tool;
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
pub mod delegation_output_schema;
pub mod env_probe_tool;
pub mod file_state;
pub mod file_tools;
pub mod harness_tools;
pub mod http_tool;
pub mod image_generation_tool;
pub mod insights_tool;
pub mod kanban_tool;
pub mod kernel;
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
pub mod reaction_tool;
pub mod send_message_tool;
pub mod session_search_tool;
pub mod skills_tool;
pub mod sourcehound;
pub mod sourcehound_update;
pub mod spotify_tool;
pub mod sub_agent_tool;
pub mod terminal_backend;
pub mod terminal_tool;
pub mod todo_tool;
pub mod tool_backend_helpers;
pub mod tool_search;
pub mod transcription_tool;
pub mod tts_command_provider;
pub mod tts_provider;
pub mod tts_registry;
pub mod tts_tool;
pub mod verification_tool;
pub mod video_analysis_tool;
pub mod vision_tool;
pub mod web_providers;
pub mod web_tools;
pub mod working_diff_tool;
pub mod xai_http;

pub use web_providers::{
    DDGProvider, ExaProvider, SearXNGProvider, TavilyProvider, WebSearchProvider, WebSearchResult,
};
pub mod discord_tool;
pub mod feishu_tool;
pub mod home_assistant_tool;

// Re-export commonly used types
pub use aft_tools::register_aft_tools;
pub use async_delegation::{
    AsyncDelegationRecord, AsyncDelegationStatus, DEFAULT_MAX_ASYNC_CHILDREN, create_record,
    get_record, list_records, pending_count, try_create_record,
};
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
pub use delegation_output_schema::{
    MAX_SCHEMA_RETRIES, append_output_contract, build_retry_message, coerce_output_schema,
    extract_json_candidate, validate_output,
};
pub use discord_tool::{DiscordAdminTool, DiscordTool};
pub use feishu_tool::{FeishuDocTool, FeishuDriveTool};
pub use home_assistant_tool::HomeAssistantTool;
pub use kanban_tool::KanbanTool;
pub use lcm_tools::register_lcm_tools;
pub use mcp_tool::McpManagementTool;
pub use osv_check::OsvCheckTool;
pub use process_tool::ProcessTool;
pub use reaction_tool::ReactionTool;
pub use send_message_tool::SendMessageTool;
pub use session_search_tool::{SessionMeta, SessionResult, SessionSearchTool};
pub use skills_tool::{
    SkillManageTool, SkillMeta, SkillTreeValidation, SkillViewTool, SkillsTool,
    collect_skill_children, validate_skill_tree,
};
pub use sourcehound::{WebExtractTool, WebScrapeTool};
pub use spotify_tool::{
    SpotifyAlbumsTool, SpotifyDevicesTool, SpotifyLibraryTool, SpotifyPlaybackTool,
    SpotifyPlaylistsTool, SpotifyQueueTool, SpotifySearchTool,
};
pub use transcription_tool::TranscriptionTool;
pub use tts_command_provider::CommandProvider;
pub use tts_provider::{AudioFormat, TtsError, TtsProvider};
pub use tts_registry::TtsPluginRegistry;
pub use verification_tool::VerifyTaskTool;

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
use crate::org::authority::{AuthorityScope, Grant, GrantDb, ScopeCheck};
use crate::org::employee::Employee;
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
    /// Optional session id (plan 015). Currently dormant for all tools
    /// that don't read it — background review is the only consumer
    /// today, and the pk-bridge in plan 015 Phase 2.5 is the
    /// downstream user. Default `None` keeps every existing tool
    /// build green.
    pub session_id: Option<String>,
}

impl ToolContext {
    /// Builder helper: copy the context with a session id attached.
    /// Used by the gateway dispatch path to wire the active session
    /// id into tool calls without sprinkling `.clone()` through the
    /// call sites.
    pub fn with_session_id(mut self, id: impl Into<String>) -> Self {
        self.session_id = Some(id.into());
        self
    }
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
    /// §2.4 authority bindings, keyed by tool name. Absent = global.
    ///
    /// Additive state: a registry that declares nothing has an empty map and
    /// every method below that predates this field behaves exactly as it did.
    tool_authority: Arc<RwLock<HashMap<String, ToolAuthority>>>,
    executor: ToolExecutor,
}

impl Clone for ToolRegistry {
    fn clone(&self) -> Self {
        Self {
            tools: Arc::clone(&self.tools),
            disabled_names: Arc::clone(&self.disabled_names),
            disabled_toolsets: Arc::clone(&self.disabled_toolsets),
            tool_authority: Arc::clone(&self.tool_authority),
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
            tool_authority: Arc::new(RwLock::new(HashMap::new())),
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

    /// Register a pre-boxed tool object (harness-kernel seam path). Additive:
    /// identical bookkeeping to [`Self::register`], but accepts an already-
    /// erased `Arc<dyn OperantTool>`.
    #[instrument(skip(self, tool), fields(tool = %tool.name()))]
    pub async fn register_dyn(&self, tool: Arc<dyn OperantTool>) -> Result<()> {
        let name = tool.name().to_string();
        let mut tools = self.tools.write().await;

        if tools.contains_key(&name) {
            warn!(tool = %name, "Tool already registered, replacing");
        }

        tools.insert(name.clone(), tool);
        info!(tool = %name, "Tool registered successfully");
        Ok(())
    }

    /// Remove a tool by name (harness-kernel effect-undo path). Returns true
    /// when the tool existed and was removed.
    pub async fn unregister_tool(&self, name: &str) -> bool {
        let mut tools = self.tools.write().await;
        let removed = tools.remove(name).is_some();
        if removed {
            tracing::info!(tool = %name, "Tool unregistered");
        }
        removed
    }

    /// Remove a tool only when the live value is the same `Arc` as
    /// `expected`. G2 — fixes the same-name replace race: a staging
    /// provider with id `echo` mounting a new `Arc<EchoTool>` while the
    /// old `echo` is still registered would otherwise have its
    /// effect-undo delete the new value. By comparing `Arc` identity
    /// the old undo is a no-op when the live value has been swapped.
    /// Returns true when the live value matched and was removed.
    pub async fn unregister_tool_if(
        &self,
        name: &str,
        expected: &std::sync::Arc<dyn OperantTool>,
    ) -> bool {
        let mut tools = self.tools.write().await;
        let live = tools.get(name);
        let same_identity = match live {
            Some(live_arc) => std::sync::Arc::ptr_eq(live_arc, expected),
            None => false,
        };
        if same_identity {
            tools.remove(name);
            tracing::info!(tool = %name, "Tool unregistered (identity match)");
            true
        } else {
            tracing::debug!(
                tool = %name,
                "Tool unregister skipped — live value differs (G2 same-name replace race guard)"
            );
            false
        }
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

    /// Progressive tool disclosure: assemble the model-visible tools array
    /// by applying the `tool_search` bridge (hermes parity). When the
    /// bridge is active and MCP tools are present, `mcp_*` schemas are
    /// hidden behind `tool_search`/`tool_describe`/`tool_call`; otherwise
    /// this is a pure passthrough of [`ToolRegistry::get_schemas`].
    pub async fn get_schemas_for_request(
        &self,
        settings: &crate::config::ToolSearchSettings,
        context_window: usize,
    ) -> Vec<ToolSchema> {
        let all = self.get_schemas().await;
        tool_search::assemble_tools(all, settings, context_window).visible
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

    /// Whether a tool is registered AND currently available (not disabled
    /// by name or toolset, and `is_available()` reports true). The bridge
    /// `tool_call` uses this so a deferred tool that the user disabled can
    /// never be invoked around the ban (guardrail parity with direct
    /// calls).
    pub async fn is_available(&self, name: &str) -> bool {
        let tools = self.tools.read().await;
        let disabled_names = self.disabled_names.read().await;
        let disabled_toolsets = self.disabled_toolsets.read().await;
        match tools.get(name) {
            Some(t) => {
                t.is_available()
                    && !disabled_names.contains(name)
                    && !disabled_toolsets.contains(t.toolset())
            }
            None => false,
        }
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

// =====================================================================
// Authority-filtered tool availability — §2.4
// =====================================================================

/// The authority binding declared for one tool.
///
/// §2.4: *"the tool registry is filtered by authority at agent construction. A
/// `Department`-scoped agent is offered the department's tool surface. Reaching
/// a sibling department's tooling requires the capability grant, which is
/// individually recorded and individually revocable."*
///
/// ## `department: None` means a global tool
///
/// A tool with no department binding acts on no department's system —
/// `datetime`, `web_search`, `file_read` on the agent's own workspace — so
/// there is nothing for authority to gate and it is offered to every actor.
/// A tool with a department binding belongs to that department's *surface*, and
/// reaching it from outside requires a grant.
///
/// This is what keeps the change **additive**: the ~100 tools that predate the
/// org layer carry no binding and keep exactly the availability they have
/// today. Narrowing a tool is a declaration the mounting code makes at
/// construction time, never something the model can influence.
///
/// ## Failing closed on a half-declared binding
///
/// A binding that names a department but **no** capability cannot be admitted
/// across a boundary by any grant, because there is no capability string for a
/// grantor to name. That is deliberate. A tool that declared a department and
/// then silently became globally reachable when a grant's spelling drifted
/// would be worse than one that refuses every cross-department request.
///
/// §2.3's four enforcement sites are board-side; this is the fifth, and the
/// only one that changes what the model is *shown*.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolAuthority {
    /// The department whose surface this tool belongs to. `None` = global.
    pub department: Option<String>,
    /// The capability a cross-department grant must name to admit this tool.
    ///
    /// Must be present whenever `department` is `Some` for the tool to be
    /// reachable from another department at all.
    pub capability: Option<String>,
    /// The lattice rung the actor's own scope must contain, in every case.
    pub required_scope: AuthorityScope,
}

impl Default for ToolAuthority {
    /// Hand-written rather than derived because [`AuthorityScope`] has no
    /// `Default` — deliberately, since a lattice rung that defaulted to the
    /// *widest* value would silently widen every undeclared binding. `Own` is
    /// the only safe default: the narrowest rung.
    fn default() -> Self {
        Self::global()
    }
}

impl ToolAuthority {
    /// A global tool every actor may see. The default binding.
    pub const fn global() -> Self {
        Self {
            department: None,
            capability: None,
            required_scope: AuthorityScope::Own,
        }
    }

    /// A global tool that still demands a lattice rung (e.g. an org-wide
    /// admin action that no `Peers`-scoped employee may run).
    pub const fn global_requiring(required_scope: AuthorityScope) -> Self {
        Self {
            department: None,
            capability: None,
            required_scope,
        }
    }

    /// A tool on `department`'s own surface, admitting
    /// `required_scope` holders from that department and grant-holders from
    /// elsewhere.
    ///
    /// `capability` is the string a grant's `capability` column must equal for
    /// this tool to be reachable across the boundary — the `platform_infra.
    /// tooling` / `content.read` shape §2.5's table names.
    pub fn in_department(
        department: impl Into<String>,
        capability: impl Into<String>,
        required_scope: AuthorityScope,
    ) -> Self {
        Self {
            department: Some(department.into()),
            capability: Some(capability.into()),
            required_scope,
        }
    }
}

/// The actor a registry is being asked about: who they are, which department
/// they sit in, the hierarchy scope that resolves from their seat, and the
/// grants held.
///
/// `grants` is passed in rather than read from a store here so the expiry
/// comparison happens in exactly one place and is visible in the code — see
/// [`tool_authority_check`]. A caller that reads through
/// [`GrantDb::list_for_grantee`] is already filtering lapsed rows; a caller
/// that hands the full history is filtered here instead. Both are safe; only
/// one of them makes the rule legible.
#[derive(Debug, Clone)]
pub struct AuthorityActor {
    pub employee_id: String,
    /// `None` = no department. An actor with no department is treated as
    /// outside every department's surface, so a departmental tool is a
    /// crossing request and needs a grant. Fail-closed on absence, because the
    /// alternative — treating "unknown department" as "in every department" —
    /// would make an unbackfilled row the widest seat in the org.
    pub department: Option<String>,
    /// The hierarchy-resolved scope (`resolve_scope`). Hierarchy is the PRIMARY
    /// grant; this is it.
    pub scope: AuthorityScope,
    /// Every grant naming this actor, live and lapsed alike.
    pub grants: Vec<Grant>,
}

impl AuthorityActor {
    /// Build an actor directly.
    pub fn new(
        employee_id: impl Into<String>,
        department: Option<String>,
        scope: AuthorityScope,
        grants: Vec<Grant>,
    ) -> Self {
        Self {
            employee_id: employee_id.into(),
            department,
            scope,
            grants,
        }
    }

    /// Build an actor from a registry row plus its resolved scope.
    pub fn from_employee(employee: &Employee, scope: AuthorityScope, grants: Vec<Grant>) -> Self {
        Self {
            employee_id: employee.employee_id.clone(),
            department: employee.department.clone(),
            scope,
            grants,
        }
    }

    /// Resolve an actor against the real stores.
    ///
    /// **Fails closed on an unknown seat.** `None` from the directory becomes
    /// an `Err`, not a scopeless actor: a caller that reached this function
    /// with an id nobody holds has not established *who is asking*, and the
    /// honest answer to "what may they use" is no answer.
    pub fn resolve(
        employee_id: &str,
        scope: AuthorityScope,
        seats: &dyn SeatDirectory,
        grants: &GrantDb,
    ) -> Result<Self> {
        let employee = seats.employee(employee_id).ok_or_else(|| {
            Error::Agent(format!(
                "authority: no employee record for seat '{employee_id}'; \
                 refusing to resolve an actor nobody holds"
            ))
        })?;
        // Fails closed on an INVALID seat, not merely an absent one. This is the
        // same predicate `issue_grant` applies before recording authority, and
        // it has to be applied here too: an empty `department` or a missing
        // required field is a row that cannot act, and an actor built over one
        // would inherit a department (or the lack of a department) that no
        // legitimate work is entitled to. Refusing at construction means no
        // caller can hold an actor for a seat the identity gate would reject.
        if !employee.is_valid() {
            return Err(Error::Agent(format!(
                "authority: seat '{employee_id}' is invalid (missing {}); refusing to \
                 resolve an actor for an identity that cannot act",
                employee.missing_required_fields().join(", ")
            )));
        }
        Ok(Self::from_employee(
            &employee,
            scope,
            grants.list_for_grantee(employee_id).map_err(|e| {
                Error::Agent(format!("authority: read grants for {employee_id}: {e}"))
            })?,
        ))
    }
}

/// Looks up a seat. `None` means the seat is **vacant** — nobody holds it.
///
/// A trait rather than a direct `EmployeeDb` call for the same reason
/// [`crate::org::identity_gate::EmployeeLookup`] is one: this module owns the
/// *decision*, and the store is somebody else's. Implement it over `EmployeeDb`
/// at the wiring site.
pub trait SeatDirectory {
    fn employee(&self, employee_id: &str) -> Option<Employee>;
}

/// The verdict on one tool for one actor, as a [`ScopeCheck`] so every
/// enforcement site in §2.3 speaks the same type — and so a denial always
/// carries the sentence an operator reads to learn which grant to issue.
///
/// # The rules, in the order they fire
///
/// 1. **Undeclared tool** → allowed. A tool with no [`ToolAuthority`] is
///    global; this is what keeps the feature additive.
/// 2. **The actor's hierarchy scope must contain the tool's rung.** This is
///    the primary grant (owner rule 1) and it applies to global tools too.
/// 3. **Global tool** (no department binding) → allowed.
/// 4. **Own department** → allowed. **No grant is consulted, and none is
///    written.** Hierarchy already answered the question inside a department;
///    recording a row here would attribute the authority to whoever last ran
///    `issue_grant` rather than to the org chart (owner rules 1 and 5).
/// 5. **Crossing** → requires a grant that is *simultaneously* capability-matched,
///    department-covering, scope-sufficient, unrevoked, **and unexpired as of
///    `now`**. Anything less is a denial. Authority is never inferred from a
///    grant that does not exist (owner rule 2).
///
/// `now` must come from [`crate::org::notice::rfc3339`] — fixed millisecond
/// width is what makes [`Grant::is_lapsed_at`]'s lexicographic `<` a
/// chronological one. §2.5: "grants expire by default"; this comparison is
/// where that becomes true for tool access.
pub fn tool_authority_check(
    binding: Option<&ToolAuthority>,
    actor: &AuthorityActor,
    tool_name: &str,
    now: &str,
) -> ScopeCheck {
    // RULE 1 — no binding: a global tool, visible to everyone.
    let Some(binding) = binding else {
        return ScopeCheck::allow(
            format!(
                "tool {tool_name} carries no department binding, so it is a global tool \
                 available to every actor"
            ),
            AuthorityScope::Own,
        );
    };

    // RULE 2 — the PRIMARY grant: hierarchy. Every actor must hold the rung
    // the tool demands, including for global tools.
    if !actor.scope.contains(binding.required_scope) {
        return ScopeCheck::deny(
            format!(
                "actor {} holds scope {} which does not contain the {} scope tool {tool_name} \
                 requires; hierarchy is the primary grant and it does not reach this far",
                actor.employee_id, actor.scope, binding.required_scope
            ),
            binding.required_scope,
        );
    }

    // RULE 3 — a global tool clears on hierarchy alone.
    let Some(tool_dept) = binding.department.as_deref() else {
        return ScopeCheck::allow(
            format!(
                "tool {tool_name} is a global tool and actor {} holds scope {}",
                actor.employee_id, actor.scope
            ),
            binding.required_scope,
        );
    };

    let actor_dept = actor.department.as_deref().unwrap_or("(undept)");

    // RULE 4 — own department. Hierarchy is sufficient and no grant row is
    // recorded. The absence of any `grants` read on this path is the proof.
    if actor.department.as_deref() == Some(tool_dept) {
        return ScopeCheck::allow(
            format!(
                "department tier: tool {tool_name} is on dept:{tool_dept}'s own surface and \
                 actor {} sits in it; §2.1 makes hierarchy the primary grant here, so no \
                 capability grant is required or recorded",
                actor.employee_id
            ),
            binding.required_scope,
        );
    }

    // RULE 5 — crossing a department boundary. Everything below is the
    // "grant required" path, and it stays visually distinct from the
    // "no grant needed" path above.
    let Some(capability) = binding.capability.as_deref() else {
        return ScopeCheck::deny(
            format!(
                "tool {tool_name} sits on dept:{tool_dept}'s surface, actor {} is in {actor_dept}, \
                 and the tool declares no capability — no grant can name it, so the crossing \
                 request is refused rather than inferred",
                actor.employee_id
            ),
            AuthorityScope::Org,
        );
    };

    let matched = actor.grants.iter().find(|g| {
        g.capability == capability
            && g.covers_department(tool_dept)
            // The grant extends a scope; it must extend far enough to cover the
            // rung this tool demands.
            && g.scope.contains(binding.required_scope)
            && !g.is_revoked()
            // §2.5 expiry, compared HERE and explicitly rather than assumed to
            // have been filtered upstream.
            && !g.is_lapsed_at(now)
    });

    match matched {
        Some(g) => ScopeCheck::allow(
            format!(
                "tool {tool_name} is on dept:{tool_dept}'s surface and actor {} sits in \
                 {actor_dept}; crossing is covered by grant {} (capability {capability}, \
                 scope {}, granted by {}, reason: {})",
                actor.employee_id, g.grant_id, g.scope, g.grantor, g.reason
            ),
            binding.required_scope,
        ),
        None => {
            // Name the lapsed grant when that is the reason: "you had one" and
            // "you never had one" are different operator actions.
            let candidate = actor.grants.iter().find(|g| {
                g.capability == capability && g.covers_department(tool_dept) && !g.is_revoked()
            });
            let reason = match candidate {
                Some(g) if g.is_lapsed_at(now) => format!(
                    "the only grant covering capability {capability} in dept:{tool_dept} \
                     ({}) lapsed at {}; §2.5 expires grants, so the crossing request is \
                     refused until a new grant is recorded",
                    g.grant_id,
                    g.expires_at.as_deref().unwrap_or("(unset)")
                ),
                Some(g) => format!(
                    "grant {} names capability {capability} for dept:{tool_dept} but extends \
                     only scope {}, which does not contain the {} the tool requires",
                    g.grant_id, g.scope, binding.required_scope
                ),
                None => format!(
                    "no grant covers capability {capability} in dept:{tool_dept}; §2.1 \
                     requires an explicit, attributable grant to cross a department boundary \
                     and authority is never inferred from a grant that does not exist"
                ),
            };
            ScopeCheck::deny(reason, AuthorityScope::Org)
        }
    }
}

/// Issue one cross-department grant: validate first, persist second.
///
/// # The ordering is the feature
///
/// Every refusal path returns **before** `insert`, so a rejected grant leaves
/// the table byte-identical. There is no code path that writes a row and then
/// reports failure, and no sweeper that has to reconcile "grants we stored but
/// then rejected" — the failure mode owner rule 4 names (inert fabricated
/// authority sitting in the table for a seat nobody fills) cannot be
/// represented in this store.
///
/// # The checks
///
/// 1. the grantee's seat is **filled** — §1's vacant-seat rule. A grant naming
///    an unstaffed seat is a grant to nobody;
/// 2. the grantee's row is **valid** ([`Employee::is_valid`]) — the same
///    fail-closed predicate the identity gate blocks on;
/// 3. the grantor's seat is filled, and
/// 4. the grantor **holds at least** the scope being granted. This is strictly
///    stronger than §2.5's "must hold `Org` to grant an `Org`-touching scope",
///    and it is the rule that makes delegation non-amplifying: a department
///    head cannot mint an org-wide grant.
pub fn issue_grant(
    grant: &Grant,
    grantor_scope: AuthorityScope,
    seats: &dyn SeatDirectory,
    grants: &GrantDb,
) -> Result<String> {
    let grantee = seats.employee(&grant.grantee).ok_or_else(|| {
        Error::Agent(format!(
            "authority: refusing grant {} — seat '{}' is vacant; a grant to nobody is inert \
             authority, not authority",
            grant.grant_id, grant.grantee
        ))
    })?;
    if !grantee.is_valid() {
        return Err(Error::Agent(format!(
            "authority: refusing grant {} — seat '{}' is invalid (missing {}); refusing to \
             record authority against an identity that cannot act",
            grant.grant_id,
            grant.grantee,
            grantee.missing_required_fields().join(", ")
        )));
    }

    let _grantor = seats.employee(&grant.grantor).ok_or_else(|| {
        Error::Agent(format!(
            "authority: refusing grant {} — grantor seat '{}' is vacant; §2.5 requires the \
             grant to be attributable to a real seat",
            grant.grant_id, grant.grantor
        ))
    })?;

    if !grantor_scope.contains(grant.scope) {
        return Err(Error::Agent(format!(
            "authority: refusing grant {} — grantor {} holds {} which does not contain the \
             {} scope being granted; delegation may not amplify authority",
            grant.grant_id, grant.grantor, grantor_scope, grant.scope
        )));
    }

    grants
        .insert(grant)
        .map_err(|e| Error::Agent(format!("authority: grant {}: {e}", grant.grant_id)))?;
    Ok(grant.grant_id.clone())
}

impl ToolRegistry {
    /// Declare a tool's authority binding.
    ///
    /// Returns `false` when no tool of that name is registered — the binding is
    /// still recorded, but a silent typo here would read later as "that tool
    /// is global" rather than as "that tool does not exist", so it is worth
    /// surfacing at the call site.
    pub async fn set_tool_authority(&self, name: &str, authority: ToolAuthority) -> bool {
        let registered = self.contains(name).await;
        self.tool_authority
            .write()
            .await
            .insert(name.to_string(), authority);
        registered
    }

    /// The binding declared for a tool, or `None` for a global tool.
    pub async fn tool_authority(&self, name: &str) -> Option<ToolAuthority> {
        self.tool_authority.read().await.get(name).cloned()
    }

    /// §2.4 — the tool list this actor is **offered**.
    ///
    /// This is the advertised list: what the caller puts in the request's
    /// `tools` array. A tool the actor may not use is *absent from it*, not
    /// present and rejected on call. Owner rule 4 is explicit that the second
    /// is insufficient — a tool the model can see but cannot call burns a turn
    /// of confusion and a retry every single time it is reached for.
    ///
    /// Additive: [`Self::get_schemas`] is untouched and still returns the
    /// unfiltered list for every caller that has no actor.
    pub async fn advertise_for(&self, actor: &AuthorityActor, now: &str) -> Vec<ToolSchema> {
        let visible = self.visible_now().await;
        let bindings = self.tool_authority.read().await;
        visible
            .into_iter()
            .filter(|t| {
                tool_authority_check(bindings.get(t.name()), actor, t.name(), now).is_allowed()
            })
            .map(|t| t.schema())
            .collect()
    }

    /// The names of the tools this actor may call. Same verdict as
    /// [`Self::advertise_for`]; useful for a log line or a prompt footer.
    pub async fn tools_for(&self, actor: &AuthorityActor, now: &str) -> Vec<String> {
        let visible = self.visible_now().await;
        let bindings = self.tool_authority.read().await;
        visible
            .into_iter()
            .filter(|t| {
                tool_authority_check(bindings.get(t.name()), actor, t.name(), now).is_allowed()
            })
            .map(|t| t.name().to_string())
            .collect()
    }

    /// Whether this actor may call this tool right now.
    pub async fn is_available_for(
        &self,
        actor: &AuthorityActor,
        tool_name: &str,
        now: &str,
    ) -> bool {
        if !self.is_available(tool_name).await {
            return false;
        }
        let bindings = self.tool_authority.read().await;
        tool_authority_check(bindings.get(tool_name), actor, tool_name, now).is_allowed()
    }

    /// Execute under the same authority filter that shaped the advertised
    /// list — the second half of owner rule 3.
    ///
    /// Defence in depth: `advertise_for` already removed the tool, so a model
    /// cannot normally name it. It still can — a hallucinated name, a stale
    /// request replayed against a lapsed grant, a caller that skipped the
    /// advertise step — and this is where that is caught. An advertisement
    /// filter with no execution check is one refactor away from decoration.
    pub async fn execute_for(
        &self,
        actor: &AuthorityActor,
        tool_name: &str,
        tool_call_id: &str,
        args: Value,
        context: ToolContext,
        now: &str,
    ) -> Result<ToolResult> {
        if !self.is_available_for(actor, tool_name, now).await {
            let detail = {
                let bindings = self.tool_authority.read().await;
                match bindings.get(tool_name) {
                    Some(_) => {
                        tool_authority_check(bindings.get(tool_name), actor, tool_name, now).reason
                    }
                    None => "it is not registered, or it is disabled".to_string(),
                }
            };
            return Err(Error::Agent(format!(
                "authority: tool {tool_name} is not available to {}: {detail}",
                actor.employee_id
            )));
        }
        self.execute(tool_name, tool_call_id, args, context).await
    }

    /// The tools that are registered, available, and not disabled by name or
    /// toolset.
    ///
    /// The availability predicate [`Self::get_schemas`] applies, factored out
    /// so the advertised list and the execution-time check cannot drift apart.
    async fn visible_now(&self) -> Vec<Arc<dyn OperantTool>> {
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
            .cloned()
            .collect()
    }
}

// =====================================================================
// End of the authority-filtered tool availability section (§2.4)
// =====================================================================

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

    /// Plan 015 Phase 0: `session_id` is dormant on every existing tool
    /// (the field is `Option<String>` and defaults to `None`), but the
    /// builder must work and the value must survive a clone. This is
    /// the contract `pk-bridge` will rely on in Phase 2.5.
    #[test]
    fn tool_context_session_id_is_optional_and_settable() {
        let ctx = crate::tools::ToolContext::default();
        assert!(ctx.session_id.is_none(), "default session_id is None");

        let with_id = ctx.clone().with_session_id("sess-abc123");
        assert_eq!(with_id.session_id.as_deref(), Some("sess-abc123"));

        // clone preserves the session id (callers need to pass the
        // same context into the dispatch + the result-builder without
        // the id evaporating).
        let cloned = with_id.clone();
        assert_eq!(cloned.session_id.as_deref(), Some("sess-abc123"));
    }
}
