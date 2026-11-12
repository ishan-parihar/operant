//! Retrieve the full text of a result the evidence layer compressed.
//!
//! When the evidence layer compresses a tool result with a LOSSY method, it
//! keeps the original text in the recall store and prints a provenance marker
//! naming the key. This tool is the other end of that pointer: without it the
//! marker would name a retrieval path that does not exist, and the model
//! would be told it could recover the full output when it could not.
//!
//! This is deliberately NOT `lcm_recall`. That tool searches the lossless
//! context DAG for earlier CONVERSATION content; this one resolves a key in
//! the evidence layer's own bounded store, which holds the pre-compression
//! text of a single tool result. They answer different questions.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

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
pub struct EvidenceRecallTool;

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

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: EvidenceRecallArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::error("evidence_recall", format!("Invalid arguments: {e}"));
            }
        };
        if args.key.trim().is_empty() {
            return ToolResult::error("evidence_recall", "key is required");
        }
        match crate::agent::evidence::recall::shared().get(args.key.trim()) {
            Some(text) => ToolResult::success(
                "evidence_recall",
                json!({
                    "_evidence_tool": "evidence_recall",
                    "key": args.key.trim(),
                    "content": text,
                }),
            ),
            // The store is BOUNDED, so an older record may have been evicted.
            // Say so plainly — an empty result would read as "the tool
            // returned nothing", which is a different and wrong claim.
            None => ToolResult::success(
                "evidence_recall",
                json!({
                    "_evidence_tool": "evidence_recall",
                    "key": args.key.trim(),
                    "content": Value::Null,
                    "note": "no retained text for this key — it was never stored, or the bounded store has since evicted it",
                }),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn an_unknown_key_reports_that_nothing_is_retained() {
        let tool = EvidenceRecallTool;
        let result = tool
            .execute(json!({ "key": "no-such-key" }), ToolContext::default())
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
        let tool = EvidenceRecallTool;
        let result = tool
            .execute(json!({ "key": "   " }), ToolContext::default())
            .await;
        assert!(!result.success);
    }
}
