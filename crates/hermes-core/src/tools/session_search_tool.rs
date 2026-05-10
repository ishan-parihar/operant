//! Session Search Tool - Long-Term Conversation Recall
//!
//! Searches past session transcripts and returns focused summaries.
//! Currently a placeholder - full implementation requires SQLite + LLM integration.

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::OnceLock;
use tracing::{debug, info};

use crate::error::Result;
use crate::schema::ToolSchema;
use crate::tools::{HermesTool, ToolContext, ToolResult};

/// Session metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub session_id: String,
    pub title: Option<String>,
    pub source: String,
    pub started_at: String,
    pub last_active: String,
    pub message_count: usize,
}

/// Session search result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionResult {
    pub session_id: String,
    pub when: String,
    pub source: String,
    pub model: Option<String>,
    pub summary: Option<String>,
}

/// Arguments for session search
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct SessionSearchArgs {
    /// Search query - keywords, phrases, or boolean expressions. Omit for recent sessions mode.
    query: Option<String>,
    /// Optional: only search messages from specific roles (comma-separated). E.g. 'user,assistant'
    role_filter: Option<String>,
    /// Max sessions to return (default: 3, max: 5)
    limit: Option<usize>,
}

/// Global session search state
static SESSION_SEARCH: OnceLock<SessionSearchState> = OnceLock::new();

struct SessionSearchState {
    db_path: Option<std::path::PathBuf>,
}

impl SessionSearchState {
    fn new() -> Self {
        Self { db_path: None }
    }

    fn set_db_path(&mut self, path: std::path::PathBuf) {
        self.db_path = Some(path);
    }
}

fn get_session_search_state() -> &'static SessionSearchState {
    SESSION_SEARCH.get_or_init(|| SessionSearchState::new())
}

/// Session Search Tool
pub struct SessionSearchTool;

impl SessionSearchTool {
    pub fn new() -> Self {
        Self
    }

    /// List recent sessions (no query - returns metadata only)
    fn list_recent(&self, limit: usize) -> ToolResult {
        let results: Vec<SessionMeta> = Vec::new();

        ToolResult::success(
            "session_search",
            serde_json::json!({
                "success": true,
                "mode": "recent",
                "results": results,
                "count": 0,
                "message": "Session database not available. SQLite persistence (Phase 3) required for full functionality."
            }),
        )
    }

    /// Search sessions by query
    fn search(&self, query: &str, limit: usize) -> ToolResult {
        debug!("Session search: query={}, limit={}", query, limit);

        // Placeholder - full implementation would:
        // 1. Use SQLite FTS5 to search messages
        // 2. Group by session
        // 3. Use LLM to summarize matching sessions

        ToolResult::success(
            "session_search",
            serde_json::json!({
                "success": true,
                "query": query,
                "results": Vec::<SessionResult>::new(),
                "count": 0,
                "sessions_searched": 0,
                "message": "Session search requires SQLite persistence (Phase 3). Currently showing recent sessions placeholder."
            }),
        )
    }
}

#[async_trait]
impl HermesTool for SessionSearchTool {
    fn name(&self) -> &str {
        "session_search"
    }

    fn description(&self) -> &str {
        "Search long-term memory of past conversations or browse recent sessions. \
         Two modes: Recent (no query) - returns titles/previews/timestamps, zero LLM cost. \
         Keyword search (with query) - searches past sessions and returns summaries. \
         Use proactively when user references past work, says 'remember when', or asks about topics you've discussed before."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<SessionSearchArgs>(
            "session_search",
            "Search past conversations for context recall, or list recent sessions",
        )
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string());

        let role_filter = args
            .get("role_filter")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string());

        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(3)
            .max(1)
            .min(5);

        // Recent sessions mode (empty query)
        if query.as_ref().map(|q| q.is_empty()).unwrap_or(true) {
            return self.list_recent(limit);
        }

        // Search mode
        let query_str = query.unwrap();
        info!("Searching sessions for: {}", query_str);

        self.search(&query_str, limit)
    }
}

/// Register session search tool
pub fn register_session_search_tool() -> impl FnOnce() -> Result<()> {
    || {
        info!("Session search tool loaded");
        Ok(())
    }
}