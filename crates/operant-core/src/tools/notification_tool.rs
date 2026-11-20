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
    reason = "serde-argument struct: fields are used to generate the tool schema"
)]
struct NotifyArgs {
    message: String,
    title: Option<String>,
}

#[async_trait]
impl OperantTool for NotificationTool {
    fn name(&self) -> &str {
        "notify"
    }

    fn description(&self) -> &str {
        // iter-2665 (RT-293(1)): this said "Send a notification to the user",
        // and the result said `"delivered": true`. The body logs and returns;
        // nothing is sent anywhere. The tool keeps its NAME (agents call it, and
        // a rename is a model-visible break) but it states what it does.
        "Log a notification intent for the operator. Records the message in the run log; \
         it does not itself deliver anything. Useful for marking that a background task finished."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<NotifyArgs>(
            "notify",
            "Log a notification intent with a message and optional title",
        )
    }

    async fn execute(&self, args: Value, context: ToolContext) -> ToolResult {
        let message = match args.get("message").and_then(|v| v.as_str()) {
            Some(m) => m,
            None => {
                return ToolResult::error("notify", "message is required");
            }
        };
        // (iter-2728) The approval path below reads the asking transport off
        // this context; the notify path itself does not, so touch it here to
        // keep the parameter honest on both arms.
        let _ = context.get("transport");

        let title = args.get("title").and_then(|v| v.as_str());

        info!(message = %message, title = ?title, "Notification intent logged");

        // No `"delivered"` field (iter-2665). It was true only in the sense that
        // the function had returned: the tool never hands the message to a sink.
        // A model reading `delivered: true` reports to the operator that a notice
        // went out, and that is a false statement the transcript preserves.
        // `logged` is what actually happened and is checkable in the run log.
        ToolResult::success(
            "notify",
            json!({
                "logged": true,
                "message": message,
                "title": title,
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

    async fn execute(&self, args: Value, context: ToolContext) -> ToolResult {
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
            // (A3-gateway H-3) The asking session, so an approval prompt
            // reaches the chat that raised it.
            context.resolved_session_id().map(str::to_string),
            // (iter-2728, Hydrogen round-2 F2) The asking transport, so the
            // gateway's multi-transport drain can tell a telegram approval
            // from a discord one without the process-global channel cell.
            context.get("transport").map(str::to_string),
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
    /// RT-352: the `priority` knob was deserialized and never read — the
    /// schema advertised a priority with no consumer behind it.
    #[test]
    fn the_schema_does_not_advertise_a_priority_that_is_never_read() {
        let tool = NotificationTool;
        let rendered = serde_json::to_string(&tool.schema()).expect("schema json");
        // Concatenation-built so this test's own source cannot satisfy it (the
        // doc comment and the assert below must not carry the needle).
        let knob = format!("prior{}", "ity");
        assert!(
            !rendered.contains(&knob),
            "the dead knob must not be in the schema: {rendered}"
        );
        let src = include_str!("notification_tool.rs");
        let any_read = format!("args.{}", "priority");
        assert_eq!(src.matches(&any_read).count(), 0, "no code may read it");
    }

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
