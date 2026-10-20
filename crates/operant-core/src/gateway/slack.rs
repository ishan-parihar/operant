//! `slack` — extracted verbatim from gateway/mod.rs.

use crate::config::runtime_config;
use crate::error::{Error, Result};
use crate::gateway_markdown::markdown_to_slack_mrkdwn;
use async_trait::async_trait;
use tokio::sync::mpsc;
use tracing::info;

use super::*;

/// Slack adapter.
///
/// **OUTBOUND ONLY.** Operant can post to Slack; it cannot receive from
/// it. The Socket Mode WebSocket the doc used to describe DOES NOT EXIST
/// in this tree: `start()` logs "event-based, no polling" and returns, and
/// nothing opens a `wss://` connection or reads the Events API.
///
/// The `handle_update` parser below is therefore DEAD CODE — no caller
/// ever feeds it an update (the trait's `handle_update` is dispatched by
/// the Telegram and Discord pollers, and by the webhook adapter's HTTP
/// handler; Slack has neither).
///
/// The gateway reports this adapter as "enabled" when a token is
/// configured, which made an operator believe they could receive Slack
/// messages. `config_json` now says `inbound: false`.
///
/// Fixing it means implementing Socket Mode: the `apps.connections.open`
/// handshake, the WebSocket, the reconnect, and the event routing. That
/// is a feature, not a patch, and it is tracked as RT-86.
pub struct SlackAdapter {
    token: Option<String>,
    enabled: bool,
}

impl SlackAdapter {
    /// Create a new Slack adapter.
    ///
    /// The token is used for the OUTBOUND `chat.postMessage` call. It
    /// does not authenticate an inbound connection, because there is no
    /// inbound connection.
    pub fn new(token: Option<String>) -> Self {
        let enabled = token.is_some();
        Self { token, enabled }
    }
}

#[async_trait]
impl PlatformAdapter for SlackAdapter {
    fn name(&self) -> &str {
        "slack"
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }

    async fn start(&self) -> Result<()> {
        if !self.is_enabled() {
            return Ok(());
        }

        info!("Slack adapter started (event-based, no polling)");
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        info!("Slack adapter stopped");
        Ok(())
    }

    async fn send_message(&self, message: OutgoingMessage) -> Result<()> {
        let client = shared_http_client().clone();
        // Propagate missing token via `?` instead of `.unwrap()` (iter-125 —
        // closes the ponytail-audit "token unwrap panics" security bug).
        let token = self
            .token
            .as_ref()
            .ok_or_else(|| Error::Config("Slack token not configured".to_string()))?;

        // Slack uses mrkdwn format, NOT standard Markdown. Raw **bold**
        // renders as literal asterisks. Convert before sending.
        // (Bug #15 from iter-98 audit.)
        let text = markdown_to_slack_mrkdwn(&message.content);

        let body = serde_json::json!({
            "channel": message.channel_id,
            "text": text,
            // Prevent link-preview noise.
            "unfurl_links": false,
            "unfurl_media": false,
        });

        client
            .post(format!(
                "{}/chat.postMessage",
                runtime_config()
                    .gateway
                    .slack_api_base
                    .trim_end_matches('/')
            ))
            .header("Authorization", format!("Bearer {}", token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        Ok(())
    }

    async fn handle_update(&self, update: serde_json::Value) -> Result<Option<IncomingMessage>> {
        // Parse Slack event
        let event = match update.get("event") {
            Some(e) => e,
            None => return Ok(None),
        };

        let msg_type = event
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or_default();

        if msg_type != "message" {
            return Ok(None);
        }

        let user = event
            .get("user")
            .and_then(|u| u.as_str())
            .unwrap_or_default()
            .to_string();

        let content = event
            .get("text")
            .and_then(|t| t.as_str())
            .unwrap_or_default()
            .to_string();

        let channel = event
            .get("channel")
            .and_then(|c| c.as_str())
            .unwrap_or_default()
            .to_string();

        if content.is_empty() {
            return Ok(None);
        }

        Ok(Some(
            IncomingMessage::new("slack", user.clone(), user, channel, content).with_raw(update),
        ))
    }

    fn config_json(&self) -> serde_json::Value {
        serde_json::json!({
            "platform": "slack",
            "enabled": self.enabled,
            "has_token": self.token.is_some(),
            // The adapter can SEND but not RECEIVE. The gateway's status
            // report used to say "enabled" and nothing more, so an
            // operator believed they could receive Slack messages.
            "inbound": false,
            "outbound": true,
        })
    }

    async fn start_with_channel(
        &self,
        _message_tx: mpsc::UnboundedSender<IncomingMessage>,
    ) -> Result<()> {
        self.start().await
    }

    async fn send_message_to_channel(
        &self,
        channel_id: &str,
        message: &OutgoingMessage,
    ) -> Result<String> {
        let mut msg = OutgoingMessage::new(channel_id.to_string(), message.content.clone());
        msg.parse_markdown = message.parse_markdown;
        msg.reply_to = message.reply_to.clone();
        self.send_message(msg).await?;
        Ok(String::new())
    }
}
