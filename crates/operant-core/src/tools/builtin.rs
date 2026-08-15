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

pub use super::browser_camofox_state::CamofoxStateTool;
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
pub use super::feishu_tool::{FeishuDocTool, FeishuDriveTool};
pub use super::file_state::FileStateTool;
pub use super::file_tools::{FileListTool, FileReadTool, FileSearchTool, FileWriteTool};
pub use super::home_assistant_tool::HomeAssistantTool;
pub use super::http_tool::HttpRequestTool;
pub use super::igs::{WebCrawlTool, WebExtractTool, WebScrapeTool};
pub use super::image_generation_tool::ImageGenerationTool;
pub use super::insights_tool::InsightsTool;
pub use super::kanban_tool::KanbanTool;
pub use super::learning_mutation_tool::LearningMutationTool;
pub use super::mcp_tool::McpManagementTool;
pub use super::memory_tools::{MemoryRecallTool, MemorySearchTool, MemoryStoreTool};
pub use super::neutts_synth::NeuttsSynthTool;
pub use super::notification_tool::{ApprovalTool, NotificationTool};
pub use super::openrouter_client::OpenRouterTool;
pub use super::osv_check::OsvCheckTool;
pub use super::patch_tool::PatchTool;
pub use super::process_tool::ProcessTool;
pub use super::reaction_tool::ReactionTool;
pub use super::send_message_tool::SendMessageTool;
pub use super::skills_tool::{SkillManageTool, SkillViewTool, SkillsTool};
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
pub use super::video_analysis_tool::VideoAnalysisTool;
pub use super::vision_tool::VisionTool;
pub use super::web_tools::{WebFetchTool, WebSearchTool};
pub use super::xai_http::XaiHttpTool;

/// Register all built-in tools with a registry
pub async fn register_builtin_tools(
    registry: &ToolRegistry,
    skills_dir: &Path,
    memory_dir: &Path,
    database: Arc<Database>,
    cron_db: Arc<CronDb>,
    kanban_db: Arc<KanbanDb>,
    mcp_manager: Option<McpManager>,
) -> Result<()> {
    registry.register(CamofoxStateTool).await?;
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
    // IGS-backed web tools (web_scrape / web_extract / web_crawl). They
    // self-disable via is_available() when the `igs` binary is not installed.
    registry.register(WebScrapeTool).await?;
    registry.register(WebExtractTool).await?;
    registry.register(WebCrawlTool).await?;
    registry.register(XaiHttpTool).await?;
    registry.register(CodeExecutionTool).await?;
    registry.register(ConfigManageTool).await?;
    registry.register(CronTool::new(cron_db)).await?;
    registry.register(KanbanTool::new(kanban_db)).await?;
    registry.register(MemoryStoreTool).await?;
    registry.register(MemorySearchTool).await?;
    registry.register(MemoryRecallTool).await?;
    // Memory provider tools (agentmemory_*, tdg_*) are NOT registered here.
    // The agent-visible memory tools (memory_store / memory_search / memory_recall)
    // route through the ACTIVE_MEMORY_MANAGER global hook set by the CLI, so tool
    // writes reach the injected MemoryManager store.
    registry.register(HttpRequestTool).await?;
    registry.register(DateTimeTool).await?;
    registry.register(TimestampTool).await?;
    registry.register(EchoTool).await?;
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
    registry
        .register(ProcessTool::new(ProcessRegistry::new()))
        .await?;
    registry.register(NotificationTool).await?;
    registry.register(ApprovalTool).await?;
    registry.register(OpenRouterTool).await?;
    registry.register(OsvCheckTool).await?;
    registry.register(ImageGenerationTool::new()).await?;
    registry.register(TtsTool::new()).await?;
    registry.register(NeuttsSynthTool).await?;
    registry.register(VideoAnalysisTool::new()).await?;
    // Register insights tool for session analytics (before SessionSearchTool takes ownership)
    registry
        .register(InsightsTool::new(database.clone()))
        .await?;

    registry.register(SessionSearchTool::new(database)).await?;
    registry.register(SendMessageTool).await?;
    registry.register(ReactionTool).await?;
    registry.register(DiscordTool).await?;
    registry.register(DiscordAdminTool).await?;
    registry.register(FeishuDocTool::new()).await?;
    registry.register(FeishuDriveTool::new()).await?;
    registry.register(HomeAssistantTool::new()).await?;
    registry.register(BrowserDialogTool).await?;
    registry.register(BrowserCdpTool).await?;
    registry.register(TranscriptionTool::new()).await?;
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
        "browser_camofox_state",
        "checkpoint",
        "clarify",
        "code_execution",
        "config_manage",
        "cron",
        "datetime",
        "echo",
        "debug_env",
        "debug_system",
        "file_list",
        "file_read",
        "file_search",
        "file_state",
        "file_write",
        "http_request",
        "image_generate",
        "kanban",
        "mcp_management",
        "memory_recall",
        "memory_search",
        "memory_store",
        "neutts_synthesize",
        "notification",
        "openrouter_query",
        "osv_check",
        "patch",
        "process",
        "session_insights",
        "session_search",
        "skills_list",
        "skill_manage",
        "skill_view",
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
        "web_crawl",
        "xai_http_request",
        "send_message",
        "react_to_message",
        "discord",
        "discord_admin",
        "feishu_doc_read",
        "feishu_drive",
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
        let database = Arc::new(Database::init(PathBuf::from("test_all_builtin.db")).unwrap());
        let cron_db = Arc::new(CronDb::init(PathBuf::from("test_all_cron.db")).unwrap());
        let kanban_db = Arc::new(KanbanDb::init(PathBuf::from("test_all_kanban.db")).unwrap());
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
        let database = Arc::new(Database::init(PathBuf::from("test_with_sub.db")).unwrap());
        let cron_db = Arc::new(CronDb::init(PathBuf::from("test_with_sub_cron.db")).unwrap());
        let kanban_db = Arc::new(KanbanDb::init(PathBuf::from("test_with_sub_kanban.db")).unwrap());

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
    }
}
