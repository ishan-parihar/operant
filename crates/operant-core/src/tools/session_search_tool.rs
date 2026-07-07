//! Session Search Tool - Long-Term Conversation Recall
//!
//! Searches past session transcripts and returns focused summaries.
//! Currently a placeholder - full implementation requires SQLite + LLM integration.

use crate::database::Database;
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use tracing::{debug, info};

use crate::error::Result;
use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolResult};

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

/// Session Search Tool
pub struct SessionSearchTool {
    database: Arc<Database>,
}

impl SessionSearchTool {
    pub fn new(database: Arc<Database>) -> Self {
        Self { database }
    }

    /// List recent sessions (no query - returns metadata only)
    fn list_recent(&self, limit: usize) -> ToolResult {
        match self.database.list_sessions(limit) {
            Ok(sessions_db) => {
                let sessions: Vec<SessionMeta> = sessions_db
                    .into_iter()
                    .map(|s| SessionMeta {
                        session_id: s.id,
                        title: s.title,
                        source: s.source,
                        started_at: s.created_at,
                        last_active: s.updated_at,
                        message_count: s.message_count,
                    })
                    .collect();

                ToolResult::success(
                    "session_search",
                    serde_json::json!({
                        "success": true,
                        "mode": "recent",
                        "results": sessions,
                        "count": sessions.len(),
                        "message": "Recent sessions retrieved successfully."
                    }),
                )
            }
            Err(e) => ToolResult::error(
                "session_search",
                format!("Failed to list recent sessions: {}", e),
            ),
        }
    }

    /// Search sessions by query
    fn search(&self, query: &str, limit: usize) -> ToolResult {
        debug!("Session search: query={}, limit={}", query, limit);

        match self.database.search_sessions(query, limit) {
            Ok(results) => ToolResult::success(
                "session_search",
                serde_json::json!({
                    "success": true,
                    "query": query,
                    "results": results,
                    "count": results.len(),
                    "sessions_searched": results.len(),
                }),
            ),
            Err(e) => ToolResult::error("session_search", format!("Database search failed: {}", e)),
        }
    }
}

#[async_trait]
impl OperantTool for SessionSearchTool {
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

        let _role_filter = args
            .get("role_filter")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string());
        // TODO: Implement role filtering in search query

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_DB_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn create_test_db() -> Arc<Database> {
        let counter = TEST_DB_COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = PathBuf::from(format!(
            "/tmp/operant_test_session_{}_{}.db",
            std::process::id(),
            counter
        ));
        let _ = std::fs::remove_file(&path);
        Arc::new(Database::init(path).unwrap())
    }

    #[test]
    fn test_session_search_name_and_description() {
        let db = create_test_db();
        let tool = SessionSearchTool::new(db);
        assert_eq!(tool.name(), "session_search");
        assert!(!tool.description().is_empty());
    }

    #[test]
    fn test_session_search_schema() {
        let db = create_test_db();
        let schema = SessionSearchTool::new(db).schema();
        assert_eq!(schema.name, "session_search");
        let schema_json = serde_json::to_value(&schema).unwrap();
        if let Some(props) = schema_json["inputSchema"]["properties"].as_object() {
            assert!(
                props.contains_key("query"),
                "Schema should have 'query' property"
            );
            assert!(
                props.contains_key("limit"),
                "Schema should have 'limit' property"
            );
        }
    }

    #[tokio::test]
    async fn test_session_search_recent_mode() {
        let db = create_test_db();
        let tool = SessionSearchTool::new(db);
        let result = tool
            .execute(serde_json::json!({}), ToolContext::default())
            .await;
        assert!(result.success);
        let content: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(content["mode"], "recent");
    }

    #[tokio::test]
    async fn test_session_search_recent_with_empty_query() {
        let db = create_test_db();
        let tool = SessionSearchTool::new(db);
        let result = tool
            .execute(serde_json::json!({ "query": "" }), ToolContext::default())
            .await;
        assert!(result.success);
    }

    #[tokio::test]
    async fn test_session_search_with_query() {
        let db = create_test_db();
        let tool = SessionSearchTool::new(db);
        let result = tool
            .execute(
                serde_json::json!({ "query": "test query", "limit": 5 }),
                ToolContext::default(),
            )
            .await;
        assert!(result.success);
        let content: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(content["query"], "test query");
    }

    #[tokio::test]
    async fn test_session_search_limit_clamping() {
        let db = create_test_db();
        let tool = SessionSearchTool::new(db);
        let result = tool
            .execute(
                serde_json::json!({ "query": "test", "limit": 100 }),
                ToolContext::default(),
            )
            .await;
        assert!(result.success);
        let result = tool
            .execute(
                serde_json::json!({ "query": "test", "limit": 0 }),
                ToolContext::default(),
            )
            .await;
        assert!(result.success);
    }

    #[tokio::test]
    async fn test_session_search_role_filter_accepted() {
        let db = create_test_db();
        let tool = SessionSearchTool::new(db);
        let result = tool
            .execute(
                serde_json::json!({ "query": "test", "role_filter": "user,assistant" }),
                ToolContext::default(),
            )
            .await;
        assert!(result.success);
    }
}

/// Register session search tool
pub fn register_session_search_tool() -> impl FnOnce() -> Result<()> {
    || {
        info!("Session search tool loaded");
        Ok(())
    }
}
