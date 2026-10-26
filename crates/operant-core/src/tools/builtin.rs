//! Built-in tools for Operant-RS
//!
//! This module aggregates all built-in tools and provides a convenient
//! function to register them all with a ToolRegistry.

use crate::client::OpenAIClient;
use crate::cronjobs::CronDb;
use crate::database::Database;
use crate::error::Result;
use crate::kanban::KanbanDb;
use crate::mcp::McpManager;
use crate::process_registry::ProcessRegistry;
use crate::tools::{SessionSearchTool, ToolRegistry};
use std::path::Path;
use std::sync::Arc;

pub use super::browser_cdp_tool::BrowserCdpTool;
pub use super::browser_dialog_tool::BrowserDialogTool;
pub use super::browser_tool::BrowserTool;
pub use super::checkpoint_tool::CheckpointTool;
pub use super::clarify_tool::ClarifyTool;
pub use super::code_execution::CodeExecutionTool;
pub use super::config_tool::ConfigManageTool;
pub use super::cron_tool::CronTool;
pub use super::datetime_tool::{DateTimeTool, TimestampTool};
pub use super::debug_helpers::{EchoTool, EnvVarTool, SystemInfoTool};
pub use super::discord_tool::{DiscordAdminTool, DiscordTool};
pub use super::env_probe_tool::EnvProbeTool;
pub use super::file_state::FileStateTool;
pub use super::file_tools::{FileListTool, FileReadTool, FileSearchTool, FileWriteTool};
pub use super::home_assistant_tool::HomeAssistantTool;
pub use super::http_tool::HttpRequestTool;
pub use super::image_generation_tool::ImageGenerationTool;
pub use super::insights_tool::InsightsTool;
pub use super::kanban_tool::KanbanTool;
pub use super::learning_mutation_tool::LearningMutationTool;
pub use super::mcp_tool::McpManagementTool;
pub use super::memory_tools::{MemoryRecallTool, MemorySearchTool, MemoryStoreTool};
pub use super::notification_tool::{ApprovalTool, NotificationTool};
pub use super::osv_check::OsvCheckTool;
pub use super::patch_tool::PatchTool;
pub use super::process_tool::ProcessTool;
pub use super::reaction_tool::ReactionTool;
pub use super::send_message_tool::SendMessageTool;
pub use super::skills_tool::{SkillManageTool, SkillViewTool, SkillsTool};
pub use super::sop_tool::SopTool;
pub use super::sourcehound::{WebCrawlTool, WebExtractTool, WebScrapeTool};
pub use super::spotify_tool::{
    SpotifyAlbumsTool, SpotifyDevicesTool, SpotifyLibraryTool, SpotifyPlaybackTool,
    SpotifyPlaylistsTool, SpotifyQueueTool, SpotifySearchTool,
};
pub use super::sub_agent_tool::SubAgentTool;
pub use super::terminal_tool::TerminalTool;
pub use super::todo_tool::TodoTool;
pub use super::tool_backend_helpers::ToolBackendTool;
pub use super::transcription_tool::TranscriptionTool;
pub use super::tts_tool::TtsTool;
pub use super::verification_tool::VerifyTaskTool;
pub use super::video_analysis_tool::VideoAnalysisTool;
pub use super::vision_tool::VisionTool;
pub use super::web_tools::{WebFetchTool, WebSearchTool};
pub use super::working_diff_tool::WorkingDiffTool;

/// Register all built-in tools with a registry
/// (plan 2026-10-26 phase 2, RT-192) The consolidated memory toolset:
/// exactly `memory_store` + `memory_search` on the wire. The retired
/// names (`memory_recall`, `memory_save`, `memory_smart_search`) resolve
/// through the registry's ALIAS layer — callable, zero schema cost — for
/// one release, after which the aliases drop.
pub async fn register_memory_tools(registry: &ToolRegistry) -> Result<()> {
    registry.register(MemoryStoreTool).await?;
    registry.register(MemorySearchTool).await?;
    Ok(())
}

pub async fn register_builtin_tools(
    registry: &ToolRegistry,
    skills_dir: &Path,
    memory_dir: &Path,
    database: Arc<Database>,
    cron_db: Arc<CronDb>,
    kanban_db: Arc<KanbanDb>,
    mcp_manager: Option<McpManager>,
) -> Result<()> {
    registry.register(BrowserTool).await?;
    registry.register(CheckpointTool::new()).await?;
    registry.register(FileReadTool).await?;
    registry.register(FileWriteTool).await?;
    registry.register(FileSearchTool).await?;
    registry.register(FileListTool).await?;
    registry.register(TerminalTool).await?;
    registry.register(ToolBackendTool).await?;
    registry.register(WebSearchTool).await?;
    registry.register(WebFetchTool).await?;
    // sourcehound-backed web tools (web_scrape / web_extract / web_crawl). They
    // self-disable via is_available() when the `sourcehound` binary is not installed.
    registry.register(WebScrapeTool::global()).await?;
    registry.register(WebExtractTool::global()).await?;
    registry.register(WebCrawlTool::global()).await?;
    registry.register(CodeExecutionTool).await?;
    // Prime Kernel (plan 015): persistent stateful Python kernel + continual
    // harness. Registered only when [tools.kernel] enabled; kernel::register
    // also installs the phase-2.5 bridged executor against this registry clone
    // (shared internals) and spawns the bridge drainer.
    if let Err(e) =
        super::kernel::register(registry, &crate::config::runtime_config().tools.kernel).await
    {
        tracing::warn!(target: "kernel", error = %e, "kernel tool registration failed");
    }
    registry.register(ConfigManageTool).await?;
    registry.register(CronTool::new(cron_db)).await?;
    registry.register(KanbanTool::new(kanban_db)).await?;
    register_memory_tools(registry).await?;
    // Memory provider tools are NOT registered here. The agent-visible
    // memory tools (memory_store / memory_search) route through the
    // ACTIVE_MEMORY_PROVIDER global hook set by the CLI, so tool writes
    // reach the injected memory-wire store. The provider twins
    // (memory_save / memory_smart_search) and the folded memory_recall
    // are RETIRED from the wire (plan 2026-10-26 phase 2, RT-192) — the
    // registry's alias layer resolves the old names for one release so
    // pinned prompts elsewhere keep working at zero schema cost.
    registry.register(HttpRequestTool).await?;
    registry.register(DateTimeTool).await?;
    registry.register(TimestampTool).await?;
    registry.register(EchoTool).await?;
    registry.register(EnvProbeTool).await?;
    registry.register(EnvVarTool).await?;
    registry.register(SystemInfoTool).await?;
    registry.register(FileStateTool).await?;
    registry.register(TodoTool).await?;
    registry.register(ClarifyTool).await?;
    registry.register(PatchTool).await?;
    registry.register(VisionTool).await?;
    registry
        .register(SkillsTool::new(skills_dir.to_path_buf()))
        .await?;
    registry
        .register(SkillViewTool::new(skills_dir.to_path_buf()))
        .await?;
    registry
        .register(SkillManageTool::new(skills_dir.to_path_buf()))
        .await?;
    // The operator's SOP playbooks (iter-1126) — the agent reads them.
    registry.register(SopTool).await?;
    registry
        .register(ProcessTool::new(ProcessRegistry::new()))
        .await?;
    registry.register(NotificationTool).await?;
    registry.register(ApprovalTool).await?;
    registry.register(OsvCheckTool).await?;
    registry.register(ImageGenerationTool::new()).await?;
    registry.register(TtsTool::new()).await?;
    registry.register(VideoAnalysisTool::new()).await?;
    // Register insights tool for session analytics (before SessionSearchTool takes ownership)
    registry
        .register(InsightsTool::new(database.clone()))
        .await?;

    registry
        .register(SessionSearchTool::new(database.clone()))
        .await?;
    registry
        .register(VerifyTaskTool::new(database.clone()))
        .await?;
    registry.register(SendMessageTool).await?;
    registry.register(ReactionTool).await?;
    registry.register(DiscordTool).await?;
    registry.register(DiscordAdminTool).await?;
    registry.register(HomeAssistantTool::new()).await?;
    registry.register(BrowserDialogTool).await?;
    registry.register(BrowserCdpTool).await?;
    registry.register(TranscriptionTool::new()).await?;
    registry.register(WorkingDiffTool).await?;
    registry.register(SpotifyPlaybackTool).await?;
    registry.register(SpotifyDevicesTool).await?;
    registry.register(SpotifyQueueTool).await?;
    registry.register(SpotifySearchTool).await?;
    registry.register(SpotifyPlaylistsTool).await?;
    registry.register(SpotifyAlbumsTool).await?;
    registry.register(SpotifyLibraryTool).await?;

    // Register learning mutation tool
    registry
        .register(LearningMutationTool::new(
            skills_dir.to_path_buf(),
            memory_dir.to_path_buf(),
        ))
        .await?;

    // Register MCP management tool if a manager reference is provided
    if let Some(manager) = mcp_manager {
        registry.register(McpManagementTool::new(manager)).await?;
    }

    // Progressive tool disclosure bridge (hermes `tools/tool_search.py`
    // parity). Always registered so `tool_call` can dispatch to deferred
    // MCP tools; their schemas are hidden from the model-visible array
    // unless the bridge is active (see `tool_search::assemble_tools`).
    registry
        .register(super::tool_search::ToolSearchTool::new(registry.clone()))
        .await?;
    registry
        .register(super::tool_search::ToolDescribeTool::new(registry.clone()))
        .await?;
    registry
        .register(super::tool_search::ToolCallTool::new(registry.clone()))
        .await?;

    Ok(())
}

/// Register all built-in tools plus the sub-agent delegation tool.
///
/// `parent_disabled_tools` / `parent_disabled_toolsets` are the parent
/// registry's explicit tool bans. They are inherited by every spawned child
/// so a delegated sub-agent can never regain a tool the parent disabled
/// (hermes delegate_tool.py: "subagent must not gain tools the parent
/// lacks").
#[allow(clippy::too_many_arguments)]
pub async fn register_builtin_tools_with_sub_agent(
    registry: &ToolRegistry,
    skills_dir: &Path,
    memory_dir: &Path,
    parent_client: &OpenAIClient,
    model: impl Into<String>,
    database: Arc<Database>,
    cron_db: Arc<CronDb>,
    kanban_db: Arc<KanbanDb>,
    mcp_manager: Option<McpManager>,
    event_tx: Option<tokio::sync::mpsc::Sender<crate::agent::AgentEvent>>,
    parent_disabled_tools: std::collections::HashSet<String>,
    parent_disabled_toolsets: std::collections::HashSet<String>,
) -> Result<()> {
    register_builtin_tools(
        registry,
        skills_dir,
        memory_dir,
        database.clone(),
        cron_db,
        kanban_db,
        mcp_manager,
    )
    .await?;
    registry
        .register(SubAgentTool::with_parent_tool_policy(
            parent_client,
            model.into(),
            0,
            vec![],
            parent_disabled_tools,
            parent_disabled_toolsets,
            database,
            event_tx,
        ))
        .await?;
    Ok(())
}

/// Get a list of all built-in tool names
pub fn builtin_tool_names() -> Vec<&'static str> {
    vec![
        "approval_request",
        "browser",
        "checkpoint",
        "clarify",
        "code_execution",
        "config_manage",
        "cron",
        "datetime",
        "echo",
        "debug_env",
        "debug_system",
        "env_probe",
        "file_list",
        "file_read",
        "file_search",
        "file_state",
        "file_write",
        "http_request",
        "image_generate",
        "kanban",
        "mcp_management",
        "memory_search",
        "memory_store",
        "notification",
        "osv_check",
        "patch",
        "process",
        "session_insights",
        "session_search",
        "skills_list",
        "skill_manage",
        "skill_view",
        "sop",
        "terminal",
        "tool_backend",
        "timestamp",
        "todo",
        "transcribe_audio",
        "tts",
        "video_analyze",
        "vision_analyze",
        "web_fetch",
        "web_search",
        "web_scrape",
        "web_extract",
        "working_diff",
        "web_crawl",
        "send_message",
        "react_to_message",
        "verify_task",
        "discord",
        "discord_admin",
        "homeassistant",
        "delegate_task",
        "browser_dialog",
        "browser_cdp",
        "learning_manage",
        "spotify_playback",
        "spotify_devices",
        "spotify_queue",
        "spotify_search",
        "spotify_playlists",
        "spotify_albums",
        "spotify_library",
        // Memory provider tools (agentmemory_*, tdg_*) are NOT in
        // builtin_tool_names — they're registered conditionally when the
        // matching memory provider is active. Including them here would
        // make the test_register_all_builtin_tools count wrong.
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::Duration;
    use tempfile::TempDir;

    fn setup_skills_dir() -> (TempDir, PathBuf) {
        let dir = TempDir::new().unwrap();
        let skills_dir = dir.path().join("skills");
        std::fs::create_dir(&skills_dir).unwrap();
        (dir, skills_dir)
    }

    #[tokio::test]
    async fn test_register_all_builtin_tools() {
        let registry = ToolRegistry::new(Duration::from_secs(5));
        let (_tmp, skills_dir) = setup_skills_dir();
        let database =
            Arc::new(Database::init(std::env::temp_dir().join("test_all_builtin.db")).unwrap());
        let cron_db =
            Arc::new(CronDb::init(std::env::temp_dir().join("test_all_cron.db")).unwrap());
        let kanban_db =
            Arc::new(KanbanDb::init(std::env::temp_dir().join("test_all_kanban.db")).unwrap());
        let memory_dir = skills_dir.parent().unwrap_or(&skills_dir).join("memory");
        register_builtin_tools(
            &registry,
            &skills_dir,
            &memory_dir,
            database,
            cron_db,
            kanban_db,
            None,
        )
        .await
        .unwrap();

        let count = registry.len().await;
        // register_builtin_tools registers everything in builtin_tool_names
        // EXCEPT delegate_task (only in _with_sub_agent variant) AND
        // mcp_management (conditionally registered only when mcp_manager is Some),
        // PLUS the three tool_search bridge tools (always registered).
        // So: registry.len() + 2 == builtin_tool_names().len() + 3.
        assert_eq!(
            count + 2,
            builtin_tool_names().len() + 3,
            "registry has {} tools, names list has {}; the difference should be 1 (bridge tools minus delegate_task + mcp_management)",
            count,
            builtin_tool_names().len()
        );
        assert!(!registry.contains("delegate_to_sub_agent").await);
    }

    #[tokio::test]
    async fn test_register_builtin_tools_with_sub_agent() {
        let registry = ToolRegistry::new(Duration::from_secs(5));
        let (_tmp, skills_dir) = setup_skills_dir();
        let client = OpenAIClient::new(crate::client::ClientConfig::default());
        let database =
            Arc::new(Database::init(std::env::temp_dir().join("test_with_sub.db")).unwrap());
        let cron_db =
            Arc::new(CronDb::init(std::env::temp_dir().join("test_with_sub_cron.db")).unwrap());
        let kanban_db =
            Arc::new(KanbanDb::init(std::env::temp_dir().join("test_with_sub_kanban.db")).unwrap());

        let memory_dir = skills_dir.parent().unwrap_or(&skills_dir).join("memory");
        register_builtin_tools_with_sub_agent(
            &registry,
            &skills_dir,
            &memory_dir,
            &client,
            "gpt-4.1",
            database,
            cron_db,
            kanban_db,
            None,
            None,
            std::collections::HashSet::new(),
            std::collections::HashSet::new(),
        )
        .await
        .unwrap();

        let count = registry.len().await;
        // _with_sub_agent registers delegate_task but mcp_management is
        // still conditional on mcp_manager being Some (passed as None here),
        // and the three tool_search bridge tools are always registered.
        assert_eq!(
            count + 1,
            builtin_tool_names().len() + 3,
            "registry has {} tools, names list has {}; difference should be 2 (bridge tools minus mcp_management)",
            count,
            builtin_tool_names().len()
        );
        assert!(registry.contains("delegate_task").await);
        // iter-1126: the operator's SOP playbooks must be reachable BY THE
        // AGENT — the `sop` tool is registered with the builtins, so the
        // playbook directory is not an operator-only dead end.
        assert!(
            registry.contains("sop").await,
            "the sop tool must be registered with the builtins"
        );
    }

    /// (iter-1231) The five purged tools stay purged.
    ///
    /// `feishu_doc_read`, `feishu_drive`, `openrouter_query`, `xai_http_request`
    /// and `neutts_synthesize` were removed by operator order: each duplicated a
    /// capability a live surface already serves better (`http_request` for xai,
    /// the model-provider routes for openrouter, the `tts` tool's neutts/xai
    /// provider paths for neutts, and no Feishu credentials anywhere in this
    /// tree's config or .env pinning). A tool whose registration sneaks back
    /// re-advertises a capability whose env vars nobody sets, which is worse
    /// than the tool existing: the model calls it and gets a key error. This
    /// pins BOTH the registration (the registry is what the agent sees) and
    /// the advertised-name list (what the doctor/`operant tools` surface
    /// reports) — a tool restored to only one of the two desynchronizes them.
    #[test]
    fn purged_tools_are_neither_registered_nor_advertised() {
        const PURGED: &[&str] = &[
            "feishu_doc_read",
            "feishu_drive",
            "openrouter_query",
            "xai_http_request",
            "neutts_synthesize",
        ];
        let names = builtin_tool_names();
        for tool in PURGED {
            assert!(
                !names.contains(tool),
                "`{tool}` was purged by operator order; it must not be advertised"
            );
        }
        // The live registry check needs async setup, so this test covers the
        // advertised list; the async pair is below.
    }

    /// (iter-1231) The async half: after a full registration pass, the
    /// registry must not contain any purged tool.
    #[tokio::test]
    async fn purged_tools_are_not_in_a_registered_registry() {
        let registry = ToolRegistry::new(Duration::from_secs(5));
        let (_tmp, skills_dir) = setup_skills_dir();
        let database =
            Arc::new(Database::init(std::env::temp_dir().join("test_purged_builtin.db")).unwrap());
        let cron_db =
            Arc::new(CronDb::init(std::env::temp_dir().join("test_purged_cron.db")).unwrap());
        let kanban_db =
            Arc::new(KanbanDb::init(std::env::temp_dir().join("test_purged_kanban.db")).unwrap());
        let memory_dir = skills_dir.parent().unwrap_or(&skills_dir).join("memory");
        register_builtin_tools(
            &registry,
            &skills_dir,
            &memory_dir,
            database,
            cron_db,
            kanban_db,
            None,
        )
        .await
        .unwrap();

        for tool in [
            "feishu_doc_read",
            "feishu_drive",
            "openrouter_query",
            "xai_http_request",
            "neutts_synthesize",
        ] {
            assert!(
                !registry.contains(tool).await,
                "`{tool}` was purged by operator order; the registry must not serve it"
            );
        }
    }
}
