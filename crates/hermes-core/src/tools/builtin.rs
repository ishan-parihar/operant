//! Built-in tools for Hermes-RS
//!
//! This module aggregates all built-in tools and provides a convenient
//! function to register them all with a ToolRegistry.

use std::path::PathBuf;
use std::sync::Arc;
use crate::cronjobs::CronDb;
use crate::database::Database;
use crate::client::OpenAIClient;
use crate::error::Result;
use crate::kanban::KanbanDb;
use crate::tools::{ToolRegistry, SessionSearchTool};

pub use super::browser_tool::BrowserTool;
pub use super::browser_dialog_tool::BrowserDialogTool;
pub use super::browser_cdp_tool::BrowserCdpTool;
pub use super::computer_use_tool::ComputerUseTool;
pub use super::mixture_of_agents_tool::MixtureOfAgentsTool;
pub use super::rl_training_tool::RlTrainingTool;
pub use super::spotify_tool::{
    SpotifyPlaybackTool, SpotifyDevicesTool, SpotifyQueueTool, SpotifySearchTool,
    SpotifyPlaylistsTool, SpotifyAlbumsTool, SpotifyLibraryTool,
};
pub use super::checkpoint_tool::CheckpointTool;
pub use super::clarify_tool::ClarifyTool;
pub use super::code_execution::CodeExecutionTool;
pub use super::cron_tool::CronTool;
pub use super::datetime_tool::{DateTimeTool, TimestampTool};
pub use super::file_tools::{FileListTool, FileReadTool, FileSearchTool, FileWriteTool};
pub use super::http_tool::HttpRequestTool;
pub use super::image_generation_tool::ImageGenerationTool;
pub use super::kanban_tool::KanbanTool;
pub use super::memory_tools::{MemoryRecallTool, MemorySearchTool, MemoryStoreTool};
pub use super::notification_tool::{NotificationTool, ApprovalTool};
pub use super::patch_tool::PatchTool;
pub use super::skills_tool::{SkillsTool, SkillViewTool};
pub use super::sub_agent_tool::SubAgentTool;
pub use super::terminal_tool::TerminalTool;
pub use super::todo_tool::TodoTool;
pub use super::tts_tool::TtsTool;
pub use super::video_analysis_tool::VideoAnalysisTool;
pub use super::vision_tool::VisionTool;
pub use super::web_tools::{WebFetchTool, WebSearchTool};
pub use super::discord_tool::{DiscordAdminTool, DiscordTool};
pub use super::feishu_tool::{FeishuDocTool, FeishuDriveTool};
pub use super::home_assistant_tool::HomeAssistantTool;
pub use super::send_message_tool::SendMessageTool;

/// Register all built-in tools with a registry
pub async fn register_builtin_tools(
    registry: &ToolRegistry,
    database: Arc<Database>,
    cron_db: Arc<CronDb>,
    kanban_db: Arc<KanbanDb>,
) -> Result<()> {
    registry.register(BrowserTool).await?;
    registry.register(CheckpointTool::new()).await?;
    registry.register(FileReadTool).await?;
    registry.register(FileWriteTool).await?;
    registry.register(FileSearchTool).await?;
    registry.register(FileListTool).await?;
    registry.register(TerminalTool).await?;
    registry.register(WebSearchTool).await?;
    registry.register(WebFetchTool).await?;
    registry.register(CodeExecutionTool).await?;
    registry.register(CronTool::new(cron_db)).await?;
    registry.register(KanbanTool::new(kanban_db)).await?;
    registry.register(MemoryStoreTool).await?;
    registry.register(MemorySearchTool).await?;
    registry.register(MemoryRecallTool).await?;
    registry.register(HttpRequestTool).await?;
    registry.register(DateTimeTool).await?;
    registry.register(TimestampTool).await?;
    registry.register(TodoTool).await?;
    registry.register(ClarifyTool).await?;
    registry.register(PatchTool).await?;
    registry.register(VisionTool).await?;
    registry.register(SkillsTool).await?;
    registry.register(SkillViewTool).await?;
    registry.register(NotificationTool).await?;
    registry.register(ApprovalTool).await?;
    registry.register(ImageGenerationTool::new()).await?;
    registry.register(TtsTool::new()).await?;
    registry.register(VideoAnalysisTool::new()).await?;
    registry.register(SessionSearchTool::new(database)).await?;
    registry.register(SendMessageTool).await?;
    registry.register(DiscordTool).await?;
    registry.register(DiscordAdminTool).await?;
    registry.register(FeishuDocTool::new()).await?;
    registry.register(FeishuDriveTool::new()).await?;
    registry.register(HomeAssistantTool::new()).await?;
    registry.register(BrowserDialogTool).await?;
    registry.register(BrowserCdpTool).await?;
    registry.register(ComputerUseTool).await?;
    registry.register(MixtureOfAgentsTool).await?;
    registry.register(RlTrainingTool).await?;
    registry.register(SpotifyPlaybackTool).await?;
    registry.register(SpotifyDevicesTool).await?;
    registry.register(SpotifyQueueTool).await?;
    registry.register(SpotifySearchTool).await?;
    registry.register(SpotifyPlaylistsTool).await?;
    registry.register(SpotifyAlbumsTool).await?;
    registry.register(SpotifyLibraryTool).await?;

    Ok(())
}

/// Register all built-in tools plus the sub-agent delegation tool.
pub async fn register_builtin_tools_with_sub_agent(
    registry: &ToolRegistry,
    parent_client: &OpenAIClient,
    model: impl Into<String>,
    database: Arc<Database>,
    cron_db: Arc<CronDb>,
    kanban_db: Arc<KanbanDb>,
) -> Result<()> {
    register_builtin_tools(registry, database.clone(), cron_db, kanban_db).await?;
    registry
        .register(SubAgentTool::new(parent_client, model.into(), 0, vec![], database))
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
        "cron",
        "datetime",
        "file_list",
        "file_read",
        "file_search",
        "file_write",
        "http_request",
        "image_generate",
        "kanban",
        "memory_recall",
        "memory_search",
        "memory_store",
        "notification",
        "patch",
        "session_search",
        "skills_list",
        "skill_view",
        "terminal",
        "timestamp",
        "todo",
        "tts",
        "video_analyze",
        "vision_analyze",
        "web_fetch",
        "web_search",
        "send_message",
        "discord",
        "discord_admin",
        "feishu_doc_read",
        "feishu_drive",
        "homeassistant",
        "delegate_task",
        "browser_dialog",
        "browser_cdp",
        "computer_use",
        "mixture_of_agents",
        "rl",
        "spotify_playback",
        "spotify_devices",
        "spotify_queue",
        "spotify_search",
        "spotify_playlists",
        "spotify_albums",
        "spotify_library",
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn test_register_all_builtin_tools() {
        let registry = ToolRegistry::new(Duration::from_secs(5));
        let database = Arc::new(Database::init(PathBuf::from("test_all_builtin.db")).unwrap());
        let cron_db = Arc::new(CronDb::init(PathBuf::from("test_all_cron.db")).unwrap());
        let kanban_db = Arc::new(KanbanDb::init(PathBuf::from("test_all_kanban.db")).unwrap());
        register_builtin_tools(&registry, database, cron_db, kanban_db)
            .await
            .unwrap();

        let schemas = registry.get_schemas().await;
        assert_eq!(schemas.len() + 1, builtin_tool_names().len());
        assert!(!registry.contains("delegate_to_sub_agent").await);
    }

    #[tokio::test]
    async fn test_register_builtin_tools_with_sub_agent() {
        let registry = ToolRegistry::new(Duration::from_secs(5));
        let client = OpenAIClient::new(crate::client::ClientConfig::default());
        let database = Arc::new(Database::init(PathBuf::from("test_with_sub.db")).unwrap());
        let cron_db = Arc::new(CronDb::init(PathBuf::from("test_with_sub_cron.db")).unwrap());
        let kanban_db = Arc::new(KanbanDb::init(PathBuf::from("test_with_sub_kanban.db")).unwrap());

        register_builtin_tools_with_sub_agent(&registry, &client, "gpt-4.1", database, cron_db, kanban_db)
            .await
            .unwrap();

        let schemas = registry.get_schemas().await;
        assert_eq!(schemas.len(), builtin_tool_names().len());
        assert!(registry.contains("delegate_task").await);
    }
}
