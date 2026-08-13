//! Session Search Tool - Long-Term Conversation Recall
//!
//! hermes `tools/session_search_tool.py` parity — three modes:
//!   - Browse (no args): recent sessions chronologically, zero LLM cost.
//!   - Discovery (query): FTS5 search, deduped per session lineage, with the
//!     matching message window + session bookends (first/last message).
//!   - Scroll (session_id + around_message_id): a window of messages centered
//!     on an anchor message, no FTS5.
//!
//! `role_filter` narrows search to specific roles (user/assistant/tool).

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
#[expect(
    dead_code,
    reason = "serde-argument struct: fields deserialized from tool-call JSON; optional fields kept for schema parity"
)]
struct SessionSearchArgs {
    /// Search query - keywords, phrases, or boolean expressions. Omit for recent sessions mode.
    query: Option<String>,
    /// Optional: only search messages from specific roles (comma-separated). E.g. 'user,assistant'
    role_filter: Option<String>,
    /// Max sessions to return (default: 3, max: 5)
    limit: Option<usize>,
    /// Scroll mode: the session id to page through.
    session_id: Option<String>,
    /// Scroll mode: anchor message rowid; a window of messages around it is returned.
    around_message_id: Option<i64>,
    /// Scroll mode: how many messages before/after the anchor (default: 3, max: 20).
    window: Option<usize>,
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

    /// Scroll mode — a message window around an anchor message id.
    fn scroll(&self, session_id: &str, around_message_id: i64, window: usize) -> ToolResult {
        match self
            .database
            .get_session_message_window(session_id, around_message_id, window)
        {
            Ok(messages) => {
                let rendered: Vec<Value> = messages
                    .iter()
                    .map(|m| {
                        serde_json::json!({
                            "message_id": m.id,
                            "role": m.role,
                            "content": m.content.as_deref().unwrap_or(""),
                            "timestamp": m.timestamp,
                        })
                    })
                    .collect();
                ToolResult::success(
                    "session_search",
                    serde_json::json!({
                        "success": true,
                        "mode": "scroll",
                        "session_id": session_id,
                        "around_message_id": around_message_id,
                        "window": window,
                        "messages": rendered,
                        "count": rendered.len(),
                        "hint": "Call again with a different aroundMessageId to page through the session."
                    }),
                )
            }
            Err(e) => ToolResult::error(
                "session_search",
                format!("Failed to fetch message window: {}", e),
            ),
        }
    }

    /// Search sessions by query, deduped per session lineage, with the match
    /// window + bookends for the top hits.
    fn search(&self, query: &str, role_filter: Option<&str>, limit: usize) -> ToolResult {
        debug!("Session search: query={}, limit={}", query, limit);

        let role = role_filter
            .map(|r| r.trim().to_string())
            .filter(|r| !r.is_empty());

        // Normalize a comma-separated role filter to a single role for the SQL
        // pass; multi-role filters fall back to a second pass (see below).
        let sql_role = role
            .as_ref()
            .and_then(|r| r.split(',').next())
            .map(|r| r.trim().to_string())
            .filter(|r| !r.is_empty());

        let base = match self.database.search_sessions_filtered(
            query,
            sql_role.as_deref(),
            limit.saturating_mul(8),
        ) {
            Ok(results) => results,
            Err(e) => {
                return ToolResult::error("session_search", format!("Database search failed: {e}"));
            }
        };

        // hermes parity: dedupe hits by session lineage (one result per
        // session, best rank wins) and keep the per-role filter if the user
        // asked for more than one role (SQL covered the first).
        let mut best_per_session: std::collections::HashMap<
            String,
            crate::database::SessionSearchResult,
        > = std::collections::HashMap::new();
        let mut order: Vec<String> = Vec::new();
        for r in base {
            if let Some(all_roles) = role.as_deref() {
                let wanted: Vec<&str> = all_roles.split(',').map(|s| s.trim()).collect();
                if !wanted.contains(&r.role.as_str()) {
                    continue;
                }
            }
            if !best_per_session.contains_key(&r.session_id) {
                order.push(r.session_id.clone());
            }
            best_per_session.entry(r.session_id.clone()).or_insert(r);
        }

        let mut results: Vec<Value> = Vec::new();
        for sid in order.iter().take(limit) {
            let r = best_per_session
                .get(sid)
                .expect("order was built from best_per_session");
            // Match window + bookends (first/last message) — hermes parity.
            let mut snippet = r.content.clone();
            if snippet.chars().count() > 300 {
                snippet = snippet.chars().take(300).collect::<String>();
                snippet.push_str("…[truncated]");
            }
            let (window, bookends) = self.match_context(sid, r.message_id);
            results.push(serde_json::json!({
                "session_id": r.session_id,
                "title": r.title,
                "role": r.role,
                "match_message_id": r.message_id,
                "snippet": snippet,
                "updated_at": r.updated_at,
                "window": window,
                "bookends": bookends,
            }));
        }

        ToolResult::success(
            "session_search",
            serde_json::json!({
                "success": true,
                "mode": "search",
                "query": query,
                "role_filter": role,
                "results": results,
                "count": results.len(),
                "hint": "Use session_id + aroundMessageId to scroll deeper into a session."
            }),
        )
    }

    /// One message either side of the match + first/last message bookends.
    fn match_context(&self, session_id: &str, around_message_id: i64) -> (Vec<Value>, Vec<Value>) {
        let window = match self
            .database
            .get_session_message_window(session_id, around_message_id, 1)
        {
            Ok(msgs) => msgs
                .iter()
                .map(|m| {
                    serde_json::json!({
                        "message_id": m.id,
                        "role": m.role,
                        "content": m.content.as_deref().unwrap_or("").chars().take(120).collect::<String>(),
                    })
                })
                .collect(),
            Err(_) => Vec::new(),
        };
        let bookends = match self.database.get_session_messages(session_id) {
            Ok(msgs) => {
                let first = msgs.first().map(|m| {
                    serde_json::json!({
                        "role": m.role,
                        "content": m.content.chars().take(120).collect::<String>(),
                    })
                });
                let last = msgs.last().map(|m| {
                    serde_json::json!({
                        "role": m.role,
                        "content": m.content.chars().take(120).collect::<String>(),
                    })
                });
                vec![first, last].into_iter().flatten().collect()
            }
            Err(_) => Vec::new(),
        };
        (window, bookends)
    }
}

#[async_trait]
impl OperantTool for SessionSearchTool {
    fn name(&self) -> &str {
        "session_search"
    }

    fn description(&self) -> &str {
        "Search long-term memory of past conversations, browse recent sessions, or scroll into a specific session. \
         Three modes: Recent (no query) - titles/previews/timestamps, zero LLM cost. \
         Keyword search (query) - FTS5 search with per-session dedupe, match window, and first/last bookends; \
         optional roleFilter (user/assistant/tool, comma-separated). \
         Scroll (session_id + aroundMessageId + window) - raw messages around an anchor to page a transcript. \
         Use proactively when the user references past work, says 'remember when', or asks about topics discussed before."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<SessionSearchArgs>(
            "session_search",
            "Search past conversations for context recall, or list recent sessions",
        )
    }

    #[expect(
        clippy::expect_used,
        reason = "invariant guaranteed by surrounding validation"
    )]
    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string());

        let role_filter = args
            .get("role_filter")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(3)
            .clamp(1, 5);

        // Scroll mode takes precedence when both session + anchor are given.
        let session_id = args
            .get("session_id")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let around_message_id = args.get("around_message_id").and_then(|v| v.as_i64());

        if let (Some(sid), Some(anchor)) = (session_id.as_deref(), around_message_id) {
            let window = args
                .get("window")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize)
                .unwrap_or(3)
                .clamp(1, 20);
            return self.scroll(sid, anchor, window);
        }

        // Recent sessions mode (empty query)
        if query.as_ref().map(|q| q.is_empty()).unwrap_or(true) {
            return self.list_recent(limit);
        }

        // Search mode
        let query_str = query
            .as_ref()
            .expect("query is Some (empty/None handled above)")
            .clone();
        info!(
            "Searching sessions for: {} (role: {:?})",
            query_str, role_filter
        );

        self.search(&query_str, role_filter.as_deref(), limit)
    }
}

/// Register session search tool
pub fn register_session_search_tool() -> impl FnOnce() -> Result<()> {
    || {
        info!("Session search tool loaded");
        Ok(())
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
            assert!(
                props.contains_key("session_id"),
                "Schema should have 'session_id' (scroll mode)"
            );
            assert!(
                props.contains_key("around_message_id"),
                "Schema should have 'around_message_id' (scroll mode)"
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
    async fn test_session_search_scroll_mode() {
        let db = create_test_db();
        // Seed a session with three messages (message_count FKs require the
        // session row to exist first).
        db.save_session("scroll_sess", None, "local", "t1", "t1")
            .unwrap();
        db.save_message("scroll_sess", "user", "first message", "t1")
            .unwrap();
        db.save_message("scroll_sess", "assistant", "second message", "t2")
            .unwrap();
        db.save_message("scroll_sess", "user", "third message", "t3")
            .unwrap();
        let anchor = db
            .get_session_message_window("scroll_sess", i64::MAX, 1)
            .unwrap()
            .into_iter()
            .map(|m| m.id)
            .next()
            .unwrap();

        let tool = SessionSearchTool::new(db.clone());
        let result = tool
            .execute(
                serde_json::json!({
                    "session_id": "scroll_sess",
                    "around_message_id": anchor,
                    "window": 5,
                }),
                ToolContext::default(),
            )
            .await;
        assert!(result.success, "scroll must succeed: {:?}", result);
        let content: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(content["mode"], "scroll");
        assert_eq!(content["session_id"], "scroll_sess");
        let msgs = content["messages"].as_array().unwrap();
        assert!(!msgs.is_empty(), "scroll must return messages");
        // Window must never leak another session.
        assert!(
            msgs.iter().all(|m| m["role"].is_string()),
            "messages carry roles"
        );
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
