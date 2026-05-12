use async_trait::async_trait;
use tokio::sync::mpsc;
use tracing::info;

use crate::error::Result;
use crate::gateway::{IncomingMessage, OutgoingMessage, PlatformAdapter};

/// Stub Slack platform adapter.
///
/// Prints messages instead of making real API calls.
/// Full API integration comes later.
pub struct SlackAdapter {
    name: String,
    token: String,
}

impl SlackAdapter {
    pub fn new(token: String) -> Self {
        Self {
            name: "slack".to_string(),
            token,
        }
    }
}

#[async_trait]
impl PlatformAdapter for SlackAdapter {
    fn name(&self) -> &str {
        &self.name
    }

    fn is_enabled(&self) -> bool {
        true
    }

    async fn start(&self) -> Result<()> {
        let preview: String = self.token.chars().take(8).collect();
        info!("Slack adapter starting with token {}...", preview);
        println!("[SlackAdapter] Connected to Slack gateway (stub)");
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        info!("Slack adapter stopping...");
        println!("[SlackAdapter] Disconnected from Slack (stub)");
        Ok(())
    }

    async fn send_message(&self, message: OutgoingMessage) -> Result<()> {
        println!(
            "[SlackAdapter] Would send to channel '{}': {}",
            message.channel_id, message.content
        );
        Ok(())
    }

    async fn handle_update(
        &self,
        _update: serde_json::Value,
    ) -> Result<Option<IncomingMessage>> {
        Ok(None)
    }

    fn config_json(&self) -> serde_json::Value {
        serde_json::json!({
            "platform": "slack",
            "stub": true,
            "has_token": !self.token.is_empty(),
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
        self.send_message(OutgoingMessage {
            channel_id: channel_id.to_string(),
            content: message.content.clone(),
            parse_markdown: message.parse_markdown,
            reply_to: message.reply_to.clone(),
        })
        .await?;
        Ok(String::new())
    }
}
