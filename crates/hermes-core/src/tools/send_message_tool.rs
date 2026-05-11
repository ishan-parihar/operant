//! Send Message Tool - Multi-platform message delivery
//!
//! Sends messages to Telegram, Discord, Slack, or generic webhooks.
//! Supports smart chunking for platform-specific message length limits
//! and lists configured platforms on demand.

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;
use tracing::info;

use crate::schema::ToolSchema;
use crate::tools::{HermesTool, ToolContext, ToolResult};

// ── Platform message size limits ────────────────────────────────────────────
const TELEGRAM_MAX_LEN: usize = 4096;
const DISCORD_MAX_LEN: usize = 2000;
const SLACK_MAX_LEN: usize = 40000;
// Webhook: no limit enforced

/// Tool that sends messages to external platforms.
pub struct SendMessageTool;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct SendMessageArgs {
    /// Action: "send" (deliver a message) or "list" (show configured platforms)
    action: String,

    /// Platform to send through: "telegram", "discord", "slack", or "webhook"
    via: Option<String>,

    /// Target identifier: channel ID for Telegram/Discord/Slack, or full webhook URL
    to: Option<String>,

    /// Text content of the message
    message: Option<String>,

    /// Optional media reference: "MEDIA:<path>" format or a URL
    media: Option<String>,
}

// ── HermesTool trait impl ───────────────────────────────────────────────────

#[async_trait]
impl HermesTool for SendMessageTool {
    fn name(&self) -> &str {
        "send_message"
    }

    fn description(&self) -> &str {
        "Send a message to Telegram, Discord, Slack, or a generic webhook (via 'send'), \
         or list available platforms and their configuration status (via 'list'). \
         Messages that exceed platform limits are automatically split into chunks."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<SendMessageArgs>(
            "send_message",
            "Send a message to an external platform — Telegram, Discord, Slack, or a generic webhook.",
        )
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let action = match args.get("action").and_then(|v| v.as_str()) {
            Some(a) => a,
            None => return ToolResult::error("send_message", "Missing required field: action"),
        };

        match action {
            "send" => self.handle_send(args).await,
            "list" => self.handle_list().await,
            other => ToolResult::error(
                "send_message",
                format!("Unknown action: '{}'. Use 'send' or 'list'.", other),
            ),
        }
    }
}

// ── Internal helpers ────────────────────────────────────────────────────────

impl SendMessageTool {
    // ── Action handlers ─────────────────────────────────────────────────────

    async fn handle_send(&self, args: Value) -> ToolResult {
        let via = match args.get("via").and_then(|v| v.as_str()) {
            Some(v) => v,
            None => return ToolResult::error("send_message", "Missing required field: via"),
        };

        let to = match args.get("to").and_then(|v| v.as_str()) {
            Some(t) => t,
            None => return ToolResult::error("send_message", "Missing required field: to"),
        };

        let message = match args.get("message").and_then(|v| v.as_str()) {
            Some(m) => m,
            None => return ToolResult::error("send_message", "Missing required field: message"),
        };

        let media = args.get("media").and_then(|v| v.as_str());

        match via {
            "telegram" => self.send_telegram(to, message, media).await,
            "discord" => self.send_discord(to, message, media).await,
            "slack" => self.send_slack(to, message, media).await,
            "webhook" => self.send_webhook(to, message, media).await,
            other => {
                let available = Self::platform_status();
                ToolResult::error(
                    "send_message",
                    json!({
                        "error": format!("Unknown platform: '{}'", other),
                        "hint": "Use the 'list' action to see available platforms",
                        "available_platforms": available,
                    })
                    .to_string(),
                )
            }
        }
    }

    async fn handle_list(&self) -> ToolResult {
        let platforms = Self::platform_status();
        ToolResult::success(
            "send_message",
            json!({
                "success": true,
                "platforms": platforms,
                "count": platforms.len(),
            }),
        )
    }

    // ── Platform status ─────────────────────────────────────────────────────

    fn platform_status() -> Vec<Value> {
        vec![
            json!({
                "name": "telegram",
                "configured": std::env::var("TELEGRAM_BOT_TOKEN").is_ok(),
                "env_var": "TELEGRAM_BOT_TOKEN",
                "description": "Send messages to Telegram chats via a bot",
            }),
            json!({
                "name": "discord",
                "configured": std::env::var("DISCORD_BOT_TOKEN").is_ok(),
                "env_var": "DISCORD_BOT_TOKEN",
                "description": "Send messages to Discord channels via a bot",
            }),
            json!({
                "name": "slack",
                "configured": std::env::var("SLACK_BOT_TOKEN").is_ok(),
                "env_var": "SLACK_BOT_TOKEN",
                "description": "Send messages to Slack channels via a bot",
            }),
            json!({
                "name": "webhook",
                "configured": true,
                "description": "Send messages to any webhook URL (no env var needed)",
            }),
        ]
    }

    // ── Smart chunking ──────────────────────────────────────────────────────

    /// Split `message` into chunks each at most `max_len` bytes, prefixing every
    /// chunk with `(i/N) ` so the receiver can reassemble them in order.
    fn chunk_message(message: &str, max_len: usize) -> Vec<String> {
        if message.len() <= max_len {
            return vec![message.to_string()];
        }

        // Reserve 14 bytes for "(NNN/NNN) " which covers up to 999 chunks
        // (more than enough given realistic message sizes).
        let reserve = 14;
        let usable = max_len.saturating_sub(reserve);
        if usable < 1 {
            return vec![message.to_string()];
        }

        let estimated = (message.len() + usable - 1) / usable;
        let mut chunks: Vec<String> = Vec::new();
        let mut start = 0;
        let mut i = 1;

        while start < message.len() {
            let prefix = format!("({}/{}) ", i, estimated);
            let available = max_len - prefix.len();
            let end = (start + available).min(message.len());
            chunks.push(format!("{}{}", prefix, &message[start..end]));
            start = end;
            i += 1;
        }

        chunks
    }

    // ── Platform senders ────────────────────────────────────────────────────

    async fn send_telegram(&self, to: &str, message: &str, media: Option<&str>) -> ToolResult {
        let token = match std::env::var("TELEGRAM_BOT_TOKEN") {
            Ok(t) => t,
            Err(_) => {
                return ToolResult::error(
                    "send_message",
                    "TELEGRAM_BOT_TOKEN environment variable not set. \
                     Create a bot via @BotFather and set this to the token you receive.",
                );
            }
        };

        let client = match build_client() {
            Ok(c) => c,
            Err(e) => return ToolResult::error("send_message", e),
        };

        let chunks = Self::chunk_message(message, TELEGRAM_MAX_LEN);
        let mut results = Vec::new();
        let is_document = media.is_some();

        for chunk in &chunks {
            let url = if is_document {
                format!("https://api.telegram.org/bot{}/sendDocument", token)
            } else {
                format!("https://api.telegram.org/bot{}/sendMessage", token)
            };

            let mut body = json!({
                "chat_id": to,
            });

            if is_document {
                body["caption"] = json!(chunk);
            } else {
                body["text"] = json!(chunk);
                body["parse_mode"] = json!("Markdown");
            }

            match client.post(&url).json(&body).send().await {
                Ok(resp) => {
                    let status = resp.status();
                    if !status.is_success() {
                        let text = resp.text().await.unwrap_or_default();
                        return ToolResult::error(
                            "send_message",
                            format!("Telegram API error (HTTP {}): {}", status.as_u16(), text),
                        );
                    }
                    results.push(json!({"chunk": chunk, "status": "sent"}));
                }
                Err(e) => {
                    return ToolResult::error(
                        "send_message",
                        format!("Telegram network error: {}", e),
                    );
                }
            }
        }

        info!(to = %to, chunks = %chunks.len(), "Sent message via Telegram");
        ToolResult::success(
            "send_message",
            json!({
                "success": true,
                "platform": "telegram",
                "to": to,
                "chunks_sent": chunks.len(),
                "results": results,
            }),
        )
    }

    async fn send_discord(&self, to: &str, message: &str, _media: Option<&str>) -> ToolResult {
        let token = match std::env::var("DISCORD_BOT_TOKEN") {
            Ok(t) => t,
            Err(_) => {
                return ToolResult::error(
                    "send_message",
                    "DISCORD_BOT_TOKEN environment variable not set. \
                     Create a bot at https://discord.com/developers/applications \
                     and set this to the bot token.",
                );
            }
        };

        let client = match build_client() {
            Ok(c) => c,
            Err(e) => return ToolResult::error("send_message", e),
        };

        let url = format!("https://discord.com/api/v10/channels/{}/messages", to);
        let chunks = Self::chunk_message(message, DISCORD_MAX_LEN);
        let mut results = Vec::new();

        for chunk in &chunks {
            let body = json!({ "content": chunk });

            match client
                .post(&url)
                .header("Authorization", format!("Bot {}", token))
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await
            {
                Ok(resp) => {
                    let status = resp.status();
                    if !status.is_success() {
                        let text = resp.text().await.unwrap_or_default();
                        return ToolResult::error(
                            "send_message",
                            format!("Discord API error (HTTP {}): {}", status.as_u16(), text),
                        );
                    }
                    results.push(json!({"chunk": chunk, "status": "sent"}));
                }
                Err(e) => {
                    return ToolResult::error(
                        "send_message",
                        format!("Discord network error: {}", e),
                    );
                }
            }
        }

        info!(to = %to, chunks = %chunks.len(), "Sent message via Discord");
        ToolResult::success(
            "send_message",
            json!({
                "success": true,
                "platform": "discord",
                "to": to,
                "chunks_sent": chunks.len(),
                "results": results,
            }),
        )
    }

    async fn send_slack(&self, to: &str, message: &str, _media: Option<&str>) -> ToolResult {
        let token = match std::env::var("SLACK_BOT_TOKEN") {
            Ok(t) => t,
            Err(_) => {
                return ToolResult::error(
                    "send_message",
                    "SLACK_BOT_TOKEN environment variable not set. \
                     Create a Slack app, add the chat:write scope, install it to your workspace, \
                     and set this to the bot token (starts with xoxb-).",
                );
            }
        };

        let client = match build_client() {
            Ok(c) => c,
            Err(e) => return ToolResult::error("send_message", e),
        };

        let url = "https://slack.com/api/chat.postMessage";
        let chunks = Self::chunk_message(message, SLACK_MAX_LEN);
        let mut results = Vec::new();

        for chunk in &chunks {
            let body = json!({
                "channel": to,
                "text": chunk,
            });

            match client
                .post(url)
                .header("Authorization", format!("Bearer {}", token))
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await
            {
                Ok(resp) => {
                    let status = resp.status();
                    if !status.is_success() {
                        let text = resp.text().await.unwrap_or_default();
                        return ToolResult::error(
                            "send_message",
                            format!("Slack API error (HTTP {}): {}", status.as_u16(), text),
                        );
                    }

                    // Slack returns 200 even for application-level errors.
                    let slack_resp: Value = resp.json().await.unwrap_or_default();
                    if slack_resp.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
                        results.push(json!({"chunk": chunk, "status": "sent"}));
                    } else {
                        let err = slack_resp
                            .get("error")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown_error");
                        return ToolResult::error(
                            "send_message",
                            format!("Slack API error: {}", err),
                        );
                    }
                }
                Err(e) => {
                    return ToolResult::error(
                        "send_message",
                        format!("Slack network error: {}", e),
                    );
                }
            }
        }

        info!(to = %to, chunks = %chunks.len(), "Sent message via Slack");
        ToolResult::success(
            "send_message",
            json!({
                "success": true,
                "platform": "slack",
                "to": to,
                "chunks_sent": chunks.len(),
                "results": results,
            }),
        )
    }

    async fn send_webhook(&self, url: &str, message: &str, _media: Option<&str>) -> ToolResult {
        if reqwest::Url::parse(url).is_err() {
            return ToolResult::error(
                "send_message",
                format!("Invalid webhook URL: '{}'. Must be a valid HTTP or HTTPS URL.", url),
            );
        }

        let client = match build_client() {
            Ok(c) => c,
            Err(e) => return ToolResult::error("send_message", e),
        };

        let body = json!({ "text": message });

        match client
            .post(url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
        {
            Ok(resp) => {
                let status = resp.status();
                if !status.is_success() {
                    let text = resp.text().await.unwrap_or_default();
                    return ToolResult::error(
                        "send_message",
                        format!("Webhook error (HTTP {}): {}", status.as_u16(), text),
                    );
                }
                info!(url = %url, "Sent message via webhook");
                ToolResult::success(
                    "send_message",
                    json!({
                        "success": true,
                        "platform": "webhook",
                        "to": url,
                        "status_code": status.as_u16(),
                    }),
                )
            }
            Err(e) => ToolResult::error("send_message", format!("Webhook network error: {}", e)),
        }
    }
}

// ── Shared HTTP client builder ──────────────────────────────────────────────

fn build_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_name_and_description() {
        let tool = SendMessageTool;
        assert_eq!(tool.name(), "send_message");
        assert!(!tool.description().is_empty());
    }

    #[test]
    fn test_schema_has_action_field() {
        let schema = SendMessageTool.schema();
        assert_eq!(schema.name, "send_message");
        let schema_json = serde_json::to_value(&schema).unwrap();
        let props = schema_json["parameters"]["properties"]
            .as_object()
            .expect("schema should have properties");
        assert!(props.contains_key("action"), "schema must have 'action'");
        assert!(props.contains_key("via"), "schema must have 'via'");
    }

    #[tokio::test]
    async fn test_execute_missing_action() {
        let tool = SendMessageTool;
        let result = tool.execute(json!({}), ToolContext::default()).await;
        assert!(!result.success);
        assert!(result.error.unwrap_or_default().contains("action"));
    }

    #[tokio::test]
    async fn test_execute_unknown_action() {
        let tool = SendMessageTool;
        let result = tool
            .execute(json!({"action": "explode"}), ToolContext::default())
            .await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_execute_list() {
        let tool = SendMessageTool;
        let result = tool
            .execute(json!({"action": "list"}), ToolContext::default())
            .await;
        assert!(result.success);
        let parsed: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(parsed["success"], true);
        assert!(parsed["platforms"].is_array());
    }

    #[tokio::test]
    async fn test_execute_send_missing_fields() {
        let tool = SendMessageTool;
        // Missing via
        let r1 = tool
            .execute(json!({"action": "send"}), ToolContext::default())
            .await;
        assert!(!r1.success);

        // Missing to
        let r2 = tool
            .execute(
                json!({"action": "send", "via": "telegram"}),
                ToolContext::default(),
            )
            .await;
        assert!(!r2.success);

        // Missing message
        let r3 = tool
            .execute(
                json!({"action": "send", "via": "telegram", "to": "123"}),
                ToolContext::default(),
            )
            .await;
        assert!(!r3.success);
    }

    #[tokio::test]
    async fn test_unknown_platform_lists_available() {
        let tool = SendMessageTool;
        let result = tool
            .execute(
                json!({
                    "action": "send",
                    "via": "myspace",
                    "to": "x",
                    "message": "hello"
                }),
                ToolContext::default(),
            )
            .await;
        assert!(!result.success);
        let err = result.error.unwrap_or_default();
        assert!(err.contains("myspace"));
        assert!(err.contains("available_platforms"));
    }

    #[test]
    fn test_chunk_message_small() {
        let msg = "short";
        let chunks = SendMessageTool::chunk_message(msg, 100);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "short");
    }

    #[test]
    fn test_chunk_message_exact_fit() {
        let msg = "A".repeat(200);
        let chunks = SendMessageTool::chunk_message(&msg, 200);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].len(), 200);
    }

    #[test]
    fn test_chunk_message_splits() {
        let msg = "A".repeat(5000);
        let chunks = SendMessageTool::chunk_message(&msg, 4096);
        assert!(chunks.len() > 1, "should split into multiple chunks");
        // Verify prefixes
        for (i, chunk) in chunks.iter().enumerate() {
            assert!(
                chunk.starts_with(&format!("({}/", i + 1)),
                "chunk {} should start with prefix",
                i
            );
            assert!(
                chunk.len() <= 4096,
                "chunk {} exceeds max length: {} > 4096",
                i,
                chunk.len()
            );
        }
    }

    #[test]
    fn test_chunk_message_roundtrip() {
        let original = "Hello, this is a test message that should be split into chunks! ".repeat(50);
        let chunks = SendMessageTool::chunk_message(&original, 500);
        let mut reconstructed = String::new();
        for chunk in &chunks {
            // Strip prefix "(i/N) " — find first space after numbers
            if let Some(content_start) = chunk.find(' ') {
                reconstructed.push_str(&chunk[content_start + 1..]);
            }
        }
        assert_eq!(reconstructed.len(), original.len());
        assert_eq!(reconstructed, original);
    }
}
