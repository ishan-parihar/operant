//! React To Message Tool — platform emoji reactions (hermes
//! `react_to_message_tool.py` parity).
//!
//! Lets the agent react to an existing message with an emoji on Telegram,
//! Discord, or Slack. Unlike `send_message`, this attaches a reaction to a
//! specific message id/timestamp instead of delivering new content.

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Duration;
use tracing::info;

use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolResult};

/// Minimal glyph → Slack emoji-name map (Slack `reactions.add` requires a
/// name, not a glyph). Unmapped glyphs on Slack error with a hint instead of
/// silently failing.
const GLYPH_TO_SLACK_NAME: &[(&str, &str)] = &[
    ("👍", "+1"),
    ("👎", "-1"),
    ("❤️", "heart"),
    ("❤", "heart"),
    ("🎉", "tada"),
    ("✅", "white_check_mark"),
    ("🔥", "fire"),
    ("😄", "smile"),
    ("🤔", "thinking_face"),
    ("👀", "eyes"),
    ("🚀", "rocket"),
    ("💡", "bulb"),
    ("🙏", "pray"),
    ("🎯", "dart"),
];

/// Tool that reacts to a platform message with an emoji.
pub struct ReactionTool;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[expect(
    dead_code,
    reason = "serde-argument struct: fields deserialized from tool-call JSON; optional fields kept for schema parity"
)]
struct ReactArgs {
    /// Action: "react" (attach an emoji reaction) or "list" (show configured platforms)
    action: String,

    /// Platform: "telegram", "discord", or "slack"
    via: Option<String>,

    /// Target identifier: chat ID (telegram), channel ID (discord/slack)
    to: Option<String>,

    /// Message to react to: message_id for Telegram/Discord, or the Slack
    /// message timestamp
    message_id: Option<String>,

    /// Emoji to react with. A literal glyph (👍) or Slack-style ":name:"
    emoji: Option<String>,
}

#[async_trait]
impl OperantTool for ReactionTool {
    fn name(&self) -> &str {
        "react_to_message"
    }

    fn description(&self) -> &str {
        "Attach an emoji reaction to an existing message on Telegram, Discord, or Slack \
         (via 'react'), or list configured platforms (via 'list'). Telegram: pass chat_id \
         in 'to' and message_id. Discord: pass channel_id in 'to' and message_id. Slack: \
         pass channel in 'to' and the message timestamp in message_id."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<ReactArgs>(
            "react_to_message",
            "React to an existing message with an emoji on Telegram, Discord, or Slack.",
        )
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let action = match args.get("action").and_then(|v| v.as_str()) {
            Some(a) => a,
            None => return ToolResult::error("react_to_message", "Missing required field: action"),
        };

        match action {
            "react" => self.handle_react(args).await,
            "list" => self.handle_list().await,
            other => ToolResult::error(
                "react_to_message",
                format!("Unknown action: '{}'. Use 'react' or 'list'.", other),
            ),
        }
    }
}

impl ReactionTool {
    async fn handle_react(&self, args: Value) -> ToolResult {
        let via = match args.get("via").and_then(|v| v.as_str()) {
            Some(v) => v,
            None => return ToolResult::error("react_to_message", "Missing required field: via"),
        };
        let to = match args.get("to").and_then(|v| v.as_str()) {
            Some(t) => t,
            None => return ToolResult::error("react_to_message", "Missing required field: to"),
        };
        let message_id = match args.get("message_id").and_then(|v| v.as_str()) {
            Some(m) => m,
            None => {
                return ToolResult::error(
                    "react_to_message",
                    "Missing required field: message_id (message id, or Slack message timestamp)",
                );
            }
        };
        let emoji = match args.get("emoji").and_then(|v| v.as_str()) {
            Some(e) => e,
            None => return ToolResult::error("react_to_message", "Missing required field: emoji"),
        };

        match via {
            "telegram" => self.react_telegram(to, message_id, emoji).await,
            "discord" => self.react_discord(to, message_id, emoji).await,
            "slack" => self.react_slack(to, message_id, emoji).await,
            other => {
                let available = Self::platform_status();
                ToolResult::error(
                    "react_to_message",
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
            "react_to_message",
            json!({
                "success": true,
                "platforms": platforms,
                "count": platforms.len(),
            }),
        )
    }

    fn platform_status() -> Vec<Value> {
        vec![
            json!({
                "name": "telegram",
                "configured": std::env::var("TELEGRAM_BOT_TOKEN").is_ok(),
                "env_var": "TELEGRAM_BOT_TOKEN",
                "endpoint": "setMessageReaction",
            }),
            json!({
                "name": "discord",
                "configured": std::env::var("DISCORD_BOT_TOKEN").is_ok(),
                "env_var": "DISCORD_BOT_TOKEN",
                "endpoint": "PUT /channels/{id}/messages/{id}/reactions",
            }),
            json!({
                "name": "slack",
                "configured": std::env::var("SLACK_BOT_TOKEN").is_ok(),
                "env_var": "SLACK_BOT_TOKEN",
                "endpoint": "reactions.add",
            }),
        ]
    }

    /// Normalize an emoji argument into (glyph, slack_name). Accepts a literal
    /// glyph or Slack-style ":name:". Returns `None` for glyphs with no Slack
    /// name mapping.
    fn normalize_emoji(emoji: &str) -> (String, Option<String>) {
        let trimmed = emoji.trim();
        if trimmed.starts_with(':') && trimmed.ends_with(':') && trimmed.len() > 2 {
            let name = trimmed[1..trimmed.len() - 1].to_string();
            return (String::new(), Some(name));
        }
        for (glyph, name) in GLYPH_TO_SLACK_NAME {
            if trimmed == *glyph {
                return (glyph.to_string(), Some((*name).to_string()));
            }
        }
        if trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            // Plain ASCII token — treat as a Slack name.
            (String::new(), Some(trimmed.to_string()))
        } else {
            // Unknown glyph — keep it; Slack path will reject with a hint.
            (trimmed.to_string(), None)
        }
    }

    async fn react_telegram(&self, chat_id: &str, message_id: &str, emoji: &str) -> ToolResult {
        let token = match std::env::var("TELEGRAM_BOT_TOKEN") {
            Ok(t) => t,
            Err(_) => {
                return ToolResult::error(
                    "react_to_message",
                    "TELEGRAM_BOT_TOKEN environment variable not set.",
                );
            }
        };
        let (glyph, _) = Self::normalize_emoji(emoji);
        if glyph.is_empty() {
            return ToolResult::error(
                "react_to_message",
                "Telegram reactions need a literal emoji glyph (e.g. 👍), not a name.",
            );
        }
        let client = match build_client() {
            Ok(c) => c,
            Err(e) => return ToolResult::error("react_to_message", e),
        };
        let url = format!("https://api.telegram.org/bot{}/setMessageReaction", token);
        let body = json!({
            "chat_id": chat_id,
            "message_id": message_id,
            "reaction": [{"type": "emoji", "emoji": glyph}],
        });
        match client.post(&url).json(&body).send().await {
            Ok(resp) => {
                let status = resp.status();
                if !status.is_success() {
                    let text = resp.text().await.unwrap_or_default();
                    return ToolResult::error(
                        "react_to_message",
                        format!("Telegram API error (HTTP {}): {}", status.as_u16(), text),
                    );
                }
                info!(chat = %chat_id, message = %message_id, emoji = %glyph, "Telegram reaction sent");
                ToolResult::success(
                    "react_to_message",
                    json!({
                        "success": true,
                        "platform": "telegram",
                        "chat_id": chat_id,
                        "message_id": message_id,
                        "emoji": glyph,
                    }),
                )
            }
            Err(e) => {
                ToolResult::error("react_to_message", format!("Telegram network error: {}", e))
            }
        }
    }

    async fn react_discord(&self, channel_id: &str, message_id: &str, emoji: &str) -> ToolResult {
        let token = match std::env::var("DISCORD_BOT_TOKEN") {
            Ok(t) => t,
            Err(_) => {
                return ToolResult::error(
                    "react_to_message",
                    "DISCORD_BOT_TOKEN environment variable not set.",
                );
            }
        };
        let (glyph, _) = Self::normalize_emoji(emoji);
        if glyph.is_empty() {
            return ToolResult::error(
                "react_to_message",
                "Discord reactions need a literal emoji glyph (e.g. 👍), not a name.",
            );
        }
        let encoded = urlencode(&glyph);
        let url = format!(
            "https://discord.com/api/v10/channels/{}/messages/{}/reactions/{}/@me",
            channel_id, message_id, encoded
        );
        let client = match build_client() {
            Ok(c) => c,
            Err(e) => return ToolResult::error("react_to_message", e),
        };
        match client
            .put(&url)
            .header("Authorization", format!("Bot {}", token))
            .send()
            .await
        {
            Ok(resp) => {
                let status = resp.status();
                if !status.is_success() {
                    let text = resp.text().await.unwrap_or_default();
                    return ToolResult::error(
                        "react_to_message",
                        format!("Discord API error (HTTP {}): {}", status.as_u16(), text),
                    );
                }
                info!(channel = %channel_id, message = %message_id, emoji = %glyph, "Discord reaction sent");
                ToolResult::success(
                    "react_to_message",
                    json!({
                        "success": true,
                        "platform": "discord",
                        "channel_id": channel_id,
                        "message_id": message_id,
                        "emoji": glyph,
                    }),
                )
            }
            Err(e) => {
                ToolResult::error("react_to_message", format!("Discord network error: {}", e))
            }
        }
    }

    async fn react_slack(&self, channel: &str, timestamp: &str, emoji: &str) -> ToolResult {
        let token = match std::env::var("SLACK_BOT_TOKEN") {
            Ok(t) => t,
            Err(_) => {
                return ToolResult::error(
                    "react_to_message",
                    "SLACK_BOT_TOKEN environment variable not set.",
                );
            }
        };
        let (_, slack_name) = Self::normalize_emoji(emoji);
        let name = match slack_name {
            Some(n) => n,
            None => {
                return ToolResult::error(
                    "react_to_message",
                    format!(
                        "Slack reactions need an emoji name (e.g. :+1: or '+1'); \
                         the glyph '{}' has no built-in mapping.",
                        emoji
                    ),
                );
            }
        };
        let client = match build_client() {
            Ok(c) => c,
            Err(e) => return ToolResult::error("react_to_message", e),
        };
        let body = json!({
            "channel": channel,
            "name": name,
            "timestamp": timestamp,
        });
        match client
            .post("https://slack.com/api/reactions.add")
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
                        "react_to_message",
                        format!("Slack API error (HTTP {}): {}", status.as_u16(), text),
                    );
                }
                let slack_resp: Value = resp.json().await.unwrap_or_default();
                if slack_resp
                    .get("ok")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                {
                    info!(channel = %channel, ts = %timestamp, name = %name, "Slack reaction sent");
                    ToolResult::success(
                        "react_to_message",
                        json!({
                            "success": true,
                            "platform": "slack",
                            "channel": channel,
                            "timestamp": timestamp,
                            "emoji": format!(":{}:", name),
                        }),
                    )
                } else {
                    let err = slack_resp
                        .get("error")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown_error");
                    ToolResult::error("react_to_message", format!("Slack API error: {}", err))
                }
            }
            Err(e) => ToolResult::error("react_to_message", format!("Slack network error: {}", e)),
        }
    }
}

fn build_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))
}

/// Percent-encode a string for URL path segments (Discord emoji encoding).
fn urlencode(input: &str) -> String {
    let mut out = String::with_capacity(input.len() * 2);
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{:02X}", byte)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_name_and_description() {
        let tool = ReactionTool;
        assert_eq!(tool.name(), "react_to_message");
        assert!(!tool.description().is_empty());
    }

    #[test]
    fn test_schema_has_required_fields() {
        let schema = ReactionTool.schema();
        assert_eq!(schema.name, "react_to_message");
        let schema_json = serde_json::to_value(&schema).unwrap();
        let props = schema_json["parameters"]["properties"]
            .as_object()
            .expect("schema should have properties");
        // camelCase rename: messageId (not message_id).
        for field in ["action", "via", "to", "messageId", "emoji"] {
            assert!(props.contains_key(field), "schema must have '{}'", field);
        }
    }

    #[tokio::test]
    async fn test_execute_missing_action() {
        let tool = ReactionTool;
        let result = tool.execute(json!({}), ToolContext::default()).await;
        assert!(!result.success);
        assert!(result.error.unwrap_or_default().contains("action"));
    }

    #[tokio::test]
    async fn test_execute_unknown_action() {
        let tool = ReactionTool;
        let result = tool
            .execute(json!({"action": "explode"}), ToolContext::default())
            .await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_execute_missing_fields() {
        let tool = ReactionTool;
        let r1 = tool
            .execute(json!({"action": "react"}), ToolContext::default())
            .await;
        assert!(!r1.success);
        let r2 = tool
            .execute(
                json!({"action": "react", "via": "telegram", "to": "123"}),
                ToolContext::default(),
            )
            .await;
        assert!(!r2.success);
        let r3 = tool
            .execute(
                json!({"action": "react", "via": "telegram", "to": "123", "message_id": "456"}),
                ToolContext::default(),
            )
            .await;
        assert!(!r3.success);
    }

    #[tokio::test]
    async fn test_unknown_platform() {
        let tool = ReactionTool;
        let result = tool
            .execute(
                json!({
                    "action": "react",
                    "via": "myspace",
                    "to": "x",
                    "message_id": "1",
                    "emoji": "👍"
                }),
                ToolContext::default(),
            )
            .await;
        assert!(!result.success);
        assert!(result.error.unwrap_or_default().contains("myspace"));
    }

    #[tokio::test]
    async fn test_execute_list() {
        let tool = ReactionTool;
        let result = tool
            .execute(json!({"action": "list"}), ToolContext::default())
            .await;
        assert!(result.success);
        let parsed: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(parsed["success"], true);
        assert_eq!(parsed["platforms"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn test_normalize_emoji_glyph() {
        let (glyph, name) = ReactionTool::normalize_emoji("👍");
        assert_eq!(glyph, "👍");
        assert_eq!(name.as_deref(), Some("+1"));
    }

    #[test]
    fn test_normalize_emoji_slack_style() {
        let (glyph, name) = ReactionTool::normalize_emoji(":white_check_mark:");
        assert!(glyph.is_empty());
        assert_eq!(name.as_deref(), Some("white_check_mark"));
    }

    #[test]
    fn test_normalize_emoji_plain_name() {
        let (glyph, name) = ReactionTool::normalize_emoji("rocket");
        assert!(glyph.is_empty());
        assert_eq!(name.as_deref(), Some("rocket"));
    }

    #[test]
    fn test_normalize_emoji_unmapped_glyph() {
        let (glyph, name) = ReactionTool::normalize_emoji("🦄");
        assert_eq!(glyph, "🦄");
        assert!(name.is_none());
    }

    #[test]
    fn test_urlencode_unicode() {
        assert_eq!(urlencode("👍"), "%F0%9F%91%8D");
        assert_eq!(urlencode("hello"), "hello");
    }
}
