//! Clarify tool
//!
//! A tool for the agent to ask the user clarifying questions.
//! Matches Python's clarify_tool.py. Returns a structured question
//! for the frontend/CLI layer to present to the user.

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolResult};

/// Maximum number of choices allowed
const MAX_CHOICES: usize = 4;

/// Arguments for the clarify tool
#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClarifyArgs {
    /// The clarifying question to ask the user
    question: String,
    /// Optional list of choices for the user to pick from. More than 4 are
    /// truncated to the first 4 (hermes parity — a 5th "Other" option is
    /// always appended by the UI).
    choices: Option<Vec<String>>,
}

/// Tool for asking the user clarifying questions
pub struct ClarifyTool;

#[async_trait]
impl OperantTool for ClarifyTool {
    fn name(&self) -> &str {
        "clarify"
    }

    fn description(&self) -> &str {
        "Ask the user a clarifying question when you need more information to proceed. \
        Optionally provide up to 4 choices for the user to select from."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<ClarifyArgs>("clarify", "Ask the user a clarifying question")
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: ClarifyArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("clarify", format!("Invalid arguments: {}", e)),
        };

        if args.question.trim().is_empty() {
            return ToolResult::error("clarify", "Question must not be empty");
        }

        // hermes parity: choices beyond MAX_CHOICES are TRUNCATED, not
        // rejected — an LLM that offers 8 options must not fail the whole
        // clarify dialog (the gateway showed "Too many choices: 8 provided"
        // and the interactive prompt never appeared). An empty list becomes
        // an open-ended question, exactly like hermes' clarify_tool.py.
        let choices = match args.choices {
            Some(ref c) if c.len() > MAX_CHOICES => Some(c[..MAX_CHOICES].to_vec()),
            Some(c) if c.is_empty() => None,
            Some(c) => Some(c),
            None => None,
        };

        // If a TUI is running (user_question sender is set), push the
        // question to the TUI and await the user's reply. The TUI opens
        // the ask_user_dialog, the user picks/types an answer, and the
        // reply flows back via the oneshot. The agent loop blocks here
        // until the user responds. (iter-97 — closes Bug #2.)
        if let Some(reply_rx) =
            crate::user_question::try_send_user_question(args.question.clone(), choices.clone())
        {
            match reply_rx.await {
                Ok(answer) => {
                    // Return the user's answer as the tool result. The
                    // agent sees this as the tool's output and continues.
                    return ToolResult::success("clarify", answer);
                }
                Err(_) => {
                    // The TUI dropped the reply_tx without sending —
                    // typically means the user pressed Esc (dismissed).
                    // This is not a successful answer; return an error so the
                    // agent knows the clarification was not provided.
                    return ToolResult::error("clarify", "user dismissed the question");
                }
            }
        }

        // CLI mode (no TUI): return the question as a structured JSON
        // result. The user sees it in the transcript but can't respond
        // interactively. This is the original behavior.
        let mut response = serde_json::json!({
            "type": "clarification",
            "question": args.question,
        });

        if let Some(ref choices) = choices {
            response["choices"] = serde_json::json!(choices);
            response["choiceCount"] = serde_json::json!(choices.len());
        }

        ToolResult::success("clarify", response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::ToolContext;

    fn default_context() -> ToolContext {
        ToolContext::default()
    }

    #[tokio::test]
    async fn test_clarify_simple_question() {
        let tool = ClarifyTool;
        let result = tool
            .execute(
                serde_json::json!({
                    "question": "Which database do you want to use?"
                }),
                default_context(),
            )
            .await;

        assert!(result.success);
        let parsed: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(parsed["type"], "clarification");
        assert_eq!(parsed["question"], "Which database do you want to use?");
        assert!(parsed.get("choices").is_none());
    }

    #[tokio::test]
    async fn test_clarify_with_choices() {
        let tool = ClarifyTool;
        let result = tool
            .execute(
                serde_json::json!({
                    "question": "Pick a framework:",
                    "choices": ["React", "Vue", "Svelte"]
                }),
                default_context(),
            )
            .await;

        assert!(result.success);
        let parsed: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(parsed["choiceCount"], 3);
        assert_eq!(parsed["choices"][0], "React");
    }

    #[tokio::test]
    async fn test_clarify_too_many_choices_truncated() {
        // hermes parity: more than MAX_CHOICES are TRUNCATED, not rejected —
        // an LLM offering 8 options must still get a working interactive
        // dialog (the gateway previously showed "Too many choices: 8
        // provided" and the clarify prompt never appeared).
        let tool = ClarifyTool;
        let result = tool
            .execute(
                serde_json::json!({
                    "question": "Pick one:",
                    "choices": ["A", "B", "C", "D", "E", "F", "G", "H"]
                }),
                default_context(),
            )
            .await;

        assert!(result.success, "8 choices must not fail the dialog");
        let parsed: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(parsed["choiceCount"], 4);
        assert_eq!(parsed["choices"][0], "A");
        assert_eq!(parsed["choices"][3], "D");
    }

    #[tokio::test]
    async fn test_clarify_empty_choices_becomes_open_ended() {
        // hermes parity: an empty choices list becomes an open-ended
        // question (choices = None) instead of a 0-choice dialog.
        let tool = ClarifyTool;
        let result = tool
            .execute(
                serde_json::json!({
                    "question": "Tell me more:",
                    "choices": []
                }),
                default_context(),
            )
            .await;

        assert!(result.success);
        let parsed: Value = serde_json::from_str(&result.content).unwrap();
        assert!(parsed.get("choices").is_none());
    }

    #[tokio::test]
    async fn test_clarify_empty_question() {
        let tool = ClarifyTool;
        let result = tool
            .execute(
                serde_json::json!({
                    "question": "   "
                }),
                default_context(),
            )
            .await;

        assert!(!result.success);
        assert!(result.error.unwrap().contains("empty"));
    }

    #[tokio::test]
    async fn test_clarify_max_choices_allowed() {
        let tool = ClarifyTool;
        let result = tool
            .execute(
                serde_json::json!({
                    "question": "Pick one:",
                    "choices": ["A", "B", "C", "D"]
                }),
                default_context(),
            )
            .await;

        assert!(result.success);
        let parsed: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(parsed["choiceCount"], 4);
    }
}
