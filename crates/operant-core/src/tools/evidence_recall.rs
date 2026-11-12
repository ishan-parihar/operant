//! Retrieve the full text of a result the evidence layer compressed.
//!
//! When the evidence layer compresses a tool result with a LOSSY method, it
//! keeps the full text and prints a provenance marker naming the key. This
//! tool is the other end of that pointer: without it the marker would name a
//! retrieval path that does not exist, and the model would be told it could
//! recover the full output when it could not.
//!
//! The full text lives in the SESSION STORE — the row the tool result was
//! written to. That is the single owner of every full tool output, so there
//! is one copy, one direction, and one pointer vocabulary: this tool, the
//! evidence layer's marker, and the aggregate trim's own marker all name the
//! same place.
//!
//! This is deliberately NOT `lcm_recall`. That tool searches the lossless
//! context DAG for earlier CONVERSATION content; this one resolves a key in
//! one session's tool results. They answer different questions.

use std::sync::Arc;

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::database::Database;
use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolResult};

/// The arguments for `evidence_recall`.
#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceRecallArgs {
    /// The key from the compression marker, e.g. `session-id:tool-call-id`.
    pub key: String,
}

/// Retrieve a compressed tool result's full text by its key.
pub struct EvidenceRecallTool {
    database: Arc<Database>,
}

impl EvidenceRecallTool {
    /// The tool needs the session store: that is where the full text is.
    pub fn new(database: Arc<Database>) -> Self {
        Self { database }
    }
}

#[async_trait::async_trait]
impl OperantTool for EvidenceRecallTool {
    fn name(&self) -> &str {
        "evidence_recall"
    }

    fn description(&self) -> &str {
        "Retrieve the FULL, uncompressed text of a tool result that was shortened by the evidence layer. Use this when a tool result carries a '[compressed: …]' marker and you need the parts that were elided. The key is in that marker."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<EvidenceRecallArgs>(
            "evidence_recall",
            "Retrieve the full text of a compressed tool result",
        )
    }

    async fn execute(&self, args: Value, context: ToolContext) -> ToolResult {
        let args: EvidenceRecallArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::error("evidence_recall", format!("Invalid arguments: {e}"));
            }
        };
        if args.key.trim().is_empty() {
            return ToolResult::error("evidence_recall", "key is required");
        }
        let Some((session, tool_call_id)) =
            crate::agent::evidence::recall::split_key(args.key.trim())
        else {
            return ToolResult::error("evidence_recall", "key is required");
        };
        // The key names the session; the context is the fallback for a key
        // that carried only a tool call id.
        let session = if session.is_empty() {
            context.session_id.as_deref().unwrap_or_default()
        } else {
            session
        };
        if session.is_empty() {
            return ToolResult::error(
                "evidence_recall",
                "the key names no session and the call carries none — pass the key exactly as the marker printed it",
            );
        }
        match self.database.get_tool_result(session, tool_call_id) {
            Ok(Some((content, tool_name))) => ToolResult::success(
                "evidence_recall",
                json!({
                    "_evidence_tool": "evidence_recall",
                    "key": args.key.trim(),
                    "tool": tool_name,
                    "content": content,
                }),
            ),
            // The row is gone (the session was deleted) or the key was never
            // one of ours. Say so plainly — an empty result would read as
            // "the tool returned nothing", which is a different and wrong claim.
            Ok(None) => ToolResult::success(
                "evidence_recall",
                json!({
                    "_evidence_tool": "evidence_recall",
                    "key": args.key.trim(),
                    "content": Value::Null,
                    "note": "no retained text for this key — the session no longer holds that tool result",
                }),
            ),
            Err(e) => ToolResult::error("evidence_recall", format!("recall failed: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A database for one test.
    ///
    /// The file name carries the process id: two concurrent test runs must
    /// not share a database, or one run's rows answer the other's query.
    fn test_db(suffix: &str) -> Arc<Database> {
        let path = std::env::temp_dir().join(format!(
            "test_evidence_recall_{}_{suffix}.db",
            std::process::id()
        ));
        Arc::new(Database::init(path).expect("database"))
    }

    #[tokio::test]
    async fn an_unknown_key_reports_that_nothing_is_retained() {
        let result = EvidenceRecallTool::new(test_db("unknown"))
            .execute(
                json!({ "key": "no-such-session:call_1" }),
                ToolContext::default(),
            )
            .await;
        assert!(result.success, "an unknown key is a miss, not a failure");
        let payload: Value = serde_json::from_str(&result.content).expect("parses");
        assert!(payload["content"].is_null());
        assert!(
            payload["note"]
                .as_str()
                .expect("note")
                .contains("no retained text"),
            "got {payload}"
        );
    }

    #[tokio::test]
    async fn an_empty_key_is_refused() {
        let result = EvidenceRecallTool::new(test_db("empty"))
            .execute(json!({ "key": "   " }), ToolContext::default())
            .await;
        assert!(!result.success);
    }

    /// The round trip the marker promises: store a tool result in the session
    /// store, and the marker's key resolves back to it.
    #[tokio::test]
    async fn a_stored_tool_result_resolves_from_its_markers_key() {
        let db = test_db("roundtrip");
        let msg = crate::database::MessageData {
            id: 0,
            session_id: "sess-1".to_string(),
            role: "tool".to_string(),
            content: Some("the full output nobody compressed".to_string()),
            full_content: None,
            tool_call_id: Some("call_7".to_string()),
            tool_calls: None,
            tool_name: Some("terminal".to_string()),
            timestamp: chrono::Utc::now().to_rfc3339(),
            token_count: None,
            finish_reason: None,
            reasoning: None,
            reasoning_content: None,
            reasoning_details: None,
            codex_reasoning_items: None,
            codex_message_items: None,
            platform_message_id: None,
            observed: None,
            active: 1,
        };
        // The session row must exist: `messages` carries a FOREIGN KEY on
        // session_id, so a result row for a session that does not exist is
        // refused. The chokepoint has the same dependency.
        db.save_session(
            "sess-1",
            Some("recall"),
            "test",
            "2026-11-12T00:00:00Z",
            "2026-11-12T00:00:00Z",
        )
        .expect("the session row");
        db.save_message_full(&msg).expect("saved");
        let result = EvidenceRecallTool::new(db)
            .execute(
                json!({ "key": crate::agent::evidence::recall::key_for(Some("sess-1"), "call_7") }),
                ToolContext::default(),
            )
            .await;
        assert!(result.success);
        let payload: Value = serde_json::from_str(&result.content).expect("parses");
        assert_eq!(payload["content"], "the full output nobody compressed");
        assert_eq!(payload["tool"], "terminal");
    }
}
