use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::info;

use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolResult};

pub struct NotificationTool;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[expect(
    dead_code,
    reason = "serde-argument struct: fields deserialized from tool-call JSON; optional fields kept for schema parity"
)]
struct NotifyArgs {
    message: String,
    title: Option<String>,
    priority: Option<String>,
}

#[async_trait]
impl OperantTool for NotificationTool {
    fn name(&self) -> &str {
        "notify"
    }

    fn description(&self) -> &str {
        "Send a notification to the user. Useful for alerting when background tasks complete."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<NotifyArgs>(
            "notify",
            "Send a notification to the user with a message and optional title",
        )
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let message = match args.get("message").and_then(|v| v.as_str()) {
            Some(m) => m,
            None => {
                return ToolResult::error("notify", "message is required");
            }
        };

        let title = args.get("title").and_then(|v| v.as_str());

        info!(message = %message, title = ?title, "Sending notification");

        ToolResult::success(
            "notify",
            json!({
                "success": true,
                "message": message,
                "title": title,
                "delivered": true
            }),
        )
    }
}

pub struct ApprovalTool;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[expect(
    dead_code,
    reason = "serde-argument struct: fields deserialized from tool-call JSON; optional fields kept for schema parity"
)]
struct ApprovalRequestArgs {
    request: String,
    reason: Option<String>,
}

#[async_trait]
impl OperantTool for ApprovalTool {
    fn name(&self) -> &str {
        "approval_request"
    }

    fn description(&self) -> &str {
        "Request human approval before proceeding with a potentially dangerous operation."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<ApprovalRequestArgs>(
            "approval_request",
            "Request approval from a human before executing a sensitive operation",
        )
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let request = match args.get("request").and_then(|v| v.as_str()) {
            Some(r) => r,
            None => {
                return ToolResult::error("approval_request", "request is required");
            }
        };

        let reason = args.get("reason").and_then(|v| v.as_str());

        // Interactive approval (hermes parity): when an interactive surface is
        // attached (TUI ask-user dialog, or a gateway channel that handles
        // user questions), block and surface the request as a yes/no prompt.
        // The operator's answer flows back through the same user_question
        // channel the `clarify` tool uses. In non-interactive (CLI/headless)
        // mode, fall back to a `pending` result — the runtime's own
        // pre-execution ApprovalManager gates dangerous tools regardless.
        let full_request = match reason {
            Some(r) => format!("{request}\n\nReason: {r}"),
            None => request.to_string(),
        };
        if let Some(reply_rx) = crate::user_question::try_send_user_question(
            full_request,
            Some(vec!["approve".to_string(), "deny".to_string()]),
        ) {
            match reply_rx.await {
                Ok(answer) => {
                    let approved = answer.trim().eq_ignore_ascii_case("approve")
                        || answer.trim().eq_ignore_ascii_case("yes")
                        || answer.trim().eq_ignore_ascii_case("y");
                    if approved {
                        return ToolResult::success(
                            "approval_request",
                            json!({
                                "success": true,
                                "request": request,
                                "reason": reason,
                                "status": "approved",
                                "approved": true
                            }),
                        );
                    }
                    return ToolResult::success(
                        "approval_request",
                        json!({
                            "success": false,
                            "request": request,
                            "reason": reason,
                            "status": "denied",
                            "approved": false
                        }),
                    );
                }
                Err(_) => {
                    // Operator dismissed the prompt (Esc) — treat as denied.
                    return ToolResult::error(
                        "approval_request",
                        "approval request dismissed by the user",
                    );
                }
            }
        }

        // Non-interactive fallback: surface the pending request; the runtime
        // approval gate still applies to dangerous tools.
        ToolResult::success(
            "approval_request",
            json!({
                "success": true,
                "request": request,
                "reason": reason,
                "status": "pending",
                "hint": "Approval is handled interactively when a TUI or gateway channel is attached; otherwise the runtime approval gate applies."
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_notification_schema() {
        let schema = NotificationTool.schema();
        let json = serde_json::to_string(&schema).unwrap();
        assert!(!json.is_empty());
        assert_eq!(schema.name, "notify");
    }

    #[tokio::test]
    async fn test_notification_success() {
        let tool = NotificationTool;
        let result = tool
            .execute(json!({"message": "test message"}), ToolContext::default())
            .await;
        assert!(result.success);
    }

    #[tokio::test]
    async fn test_notification_missing_message() {
        let tool = NotificationTool;
        let result = tool.execute(json!({}), ToolContext::default()).await;
        assert!(!result.success);
    }

    #[test]
    fn test_approval_schema() {
        let schema = ApprovalTool.schema();
        let json = serde_json::to_string(&schema).unwrap();
        assert!(!json.is_empty());
        assert_eq!(schema.name, "approval_request");
    }

    #[tokio::test]
    async fn test_approval_success() {
        let tool = ApprovalTool;
        let result = tool
            .execute(json!({"request": "approve this"}), ToolContext::default())
            .await;
        assert!(result.success);
    }

    #[tokio::test]
    async fn test_approval_missing_request() {
        let tool = ApprovalTool;
        let result = tool.execute(json!({}), ToolContext::default()).await;
        assert!(!result.success);
    }
}
