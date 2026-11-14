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
    /// How many characters to return (default 4096).
    ///
    /// The ceiling exists so one recovery cannot spend more context than the
    /// compression saved. Raising it trades that for coverage.
    #[serde(default)]
    pub length: usize,
    /// Which character to start from (default 0).
    ///
    /// Without this the tool returns the head and the rest of the text is
    /// unreachable, which makes the marker's promise dead past the first
    /// window — `session_search` and `ctx_search` index the SHAPED copy, so
    /// no other model-facing tool can fetch it either.
    #[serde(default)]
    pub offset: usize,
}

/// Retrieve a compressed tool result's full text by its key.
pub struct EvidenceRecallTool {
    database: Arc<Database>,
}

/// How many characters of the full output one recall hands back.
///
/// The whole point of the recall is to recover text the model did not see;
/// handing back more than the model can use would make the recovery cost
/// more than the compression saved.
pub const RECALL_WINDOW_CHARS: usize = 4_096;

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
        // that carried only a tool call id. MEASURED 2026-11-14 (audit
        // finding 1): this read the `session_id` FIELD, which production
        // never populated, so the fallback was always empty and a
        // tool-call-only key was refused with "the call carries none" —
        // advice the caller had already followed, since the marker prints
        // exactly that key. `resolved_session_id` reads both channels.
        let session = if session.is_empty() {
            context.resolved_session_id().unwrap_or_default()
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
            Ok(Some((content, tool_name))) => {
                // The recall has a CEILING. A tool result can be 200 000
                // bytes, and handing all of it back costs the model more
                // than the compression saved — the recovery path would be
                // the expensive one. The window is addressable, so the
                // remainder is reachable rather than a dead end.
                let total = content.chars().count();
                let length = if args.length == 0 {
                    RECALL_WINDOW_CHARS
                } else {
                    args.length
                };
                let offset = args.offset.min(total);
                let window: String = content.chars().skip(offset).take(length).collect();
                let withheld = total - offset - window.chars().count();
                let mut payload = json!({
                    "_evidence_tool": "evidence_recall",
                    "key": args.key.trim(),
                    "tool": tool_name,
                    "content": window,
                });
                if withheld > 0 {
                    payload["withheld_chars"] = json!(withheld);
                    payload["offset"] = json!(offset);
                    payload["length"] = json!(window.chars().count());
                    payload["total_chars"] = json!(total);
                    payload["note"] = json!(format!(
                        "characters {offset}..{} of {total} are shown; {withheld} more are \
                         withheld — ask again with a larger offset to read on",
                        offset + window.chars().count()
                    ));
                }
                ToolResult::success("evidence_recall", payload)
            }
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

    /// The text the recall withheld must be REACHABLE. The tool returns one
    /// window so a recovery cannot cost more than the compression saved; a
    /// model that cannot ask for the next window is told the whole text is
    /// "still in the session store" and given no way to read it — and the
    /// search tools index the SHAPED copy, so nothing else can fetch it.
    #[tokio::test]
    async fn the_text_the_recall_withheld_is_reachable_in_the_next_window() {
        let db = test_db("window");
        let big: String = (0..20_000).map(|i| format!("line {i}\n")).collect();
        let msg = crate::database::MessageData {
            id: 0,
            session_id: "sess-window".to_string(),
            role: "tool".to_string(),
            content: Some("the shaped copy".to_string()),
            full_content: Some(big.clone()),
            tool_call_id: Some("call-window".to_string()),
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
        db.save_session(
            "sess-window",
            Some("window"),
            "test",
            "2026-11-12T00:00:00Z",
            "2026-11-12T00:00:00Z",
        )
        .expect("the session row");
        db.save_message_full(&msg).expect("the row");

        let tool = EvidenceRecallTool::new(db);
        let key = "sess-window:call-window";
        let first = tool
            .execute(json!({ "key": key }), ToolContext::default())
            .await;
        let payload: Value = serde_json::from_str(&first.content).expect("parses");
        let head = payload["content"].as_str().expect("a string");
        assert_eq!(
            head.chars().count(),
            RECALL_WINDOW_CHARS,
            "the first window is one window"
        );
        assert!(
            payload["withheld_chars"].as_i64().expect("a count") > 0,
            "and it says how much was withheld: {payload}"
        );

        // The second window must continue where the first stopped.
        let second = tool
            .execute(
                json!({ "key": key, "offset": RECALL_WINDOW_CHARS }),
                ToolContext::default(),
            )
            .await;
        let payload: Value = serde_json::from_str(&second.content).expect("parses");
        let next = payload["content"].as_str().expect("a string");
        assert!(
            next.starts_with(&big.chars().skip(RECALL_WINDOW_CHARS).take(20).collect::<String>()),
            "the second window continues where the first stopped"
        );
        assert_ne!(head, next, "the two windows are different text");
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
