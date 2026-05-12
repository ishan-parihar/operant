use async_trait::async_trait;
use tokio::sync::mpsc;
use tracing::info;

use crate::error::Result;
use crate::gateway::{IncomingMessage, OutgoingMessage, PlatformAdapter};

/// Stub Discord platform adapter.
///
/// Prints messages instead of making real API calls.
/// Full API integration comes later.
pub struct DiscordAdapter {
    name: String,
    token: String,
}

impl DiscordAdapter {
    pub fn new(token: String) -> Self {
        Self {
            name: "discord".to_string(),
            token,
        }
    }
}

#[async_trait]
impl PlatformAdapter for DiscordAdapter {
    fn name(&self) -> &str {
        &self.name
    }

    fn is_enabled(&self) -> bool {
        true
    }

    async fn start(&self) -> Result<()> {
        let preview: String = self.token.chars().take(8).collect();
        info!("Discord adapter starting with token {}...", preview);
        println!("[DiscordAdapter] Connected to Discord gateway (stub)");
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        info!("Discord adapter stopping...");
        println!("[DiscordAdapter] Disconnected from Discord (stub)");
        Ok(())
    }

    async fn send_message(&self, message: OutgoingMessage) -> Result<()> {
        println!(
            "[DiscordAdapter] Would send to channel '{}': {}",
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
            "platform": "discord",
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
