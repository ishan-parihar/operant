//! `whatsapp` — extracted verbatim from gateway/mod.rs.

use crate::error::{Error, Result};
use async_trait::async_trait;
use serde_json::json;
use tokio::sync::mpsc;
use tracing::info;

use super::*;

// ---------------------------------------------------------------------------
// WhatsApp adapter (webhook-based inbound, API outbound)
// ---------------------------------------------------------------------------

/// WhatsApp adapter — receives messages via webhook, sends via WhatsApp
/// Cloud API. Requires `whatsapp_token` in config.
pub struct WhatsAppAdapter {
    enabled: bool,
    token: Option<String>,
    verify_token: Option<String>,
    phone_number_id: Option<String>,
}

impl WhatsAppAdapter {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            token: None,
            verify_token: None,
            phone_number_id: None,
        }
    }

    pub fn with_token(mut self, token: Option<String>) -> Self {
        self.token = token;
        self
    }

    /// Set the WhatsApp Cloud API phone_number_id (from Meta Business Manager).
    /// Without this, send_message posts to /v18.0/phone_number_id/messages
    /// which returns 404. (Bug #10 from iter-98 audit.)
    pub fn with_phone_number_id(mut self, phone_number_id: Option<String>) -> Self {
        self.phone_number_id = phone_number_id;
        self
    }

    /// Set the verify token for webhook handshake.
    pub fn with_verify_token(mut self, verify_token: Option<String>) -> Self {
        self.verify_token = verify_token;
        self
    }
}

#[async_trait]
impl PlatformAdapter for WhatsAppAdapter {
    fn name(&self) -> &str {
        "whatsapp"
    }
    fn is_enabled(&self) -> bool {
        self.enabled
    }

    async fn start(&self) -> Result<()> {
        info!("WhatsApp adapter started");
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        info!("WhatsApp adapter stopped");
        Ok(())
    }

    async fn send_message(&self, message: OutgoingMessage) -> Result<()> {
        let token = self
            .token
            .as_ref()
            .ok_or_else(|| Error::Config("WhatsApp token not configured".to_string()))?;
        let phone = message.channel_id.clone();

        let client = shared_http_client().clone();
        let phone_number_id = self.phone_number_id.as_deref().ok_or_else(|| {
            Error::Config(
                "WhatsApp phone_number_id not configured. Set `whatsapp_phone_number_id` in your gateway config (or OPERANT_WHATSAPP_PHONE_NUMBER_ID env) — find it under Meta Business Manager > WhatsApp > API Setup. (R22)".to_string()
            )
        })?;
        let url = format!(
            "https://graph.facebook.com/v18.0/{}/messages",
            phone_number_id
        );

        let body = json!({
            "messaging_product": "whatsapp",
            "to": phone,
            "type": "text",
            "text": {"body": message.content}
        });

        let resp = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Network(e.into()))?;

        if !resp.status().is_success() {
            return Err(Error::Agent(format!(
                "WhatsApp API error: {}",
                resp.status()
            )));
        }

        Ok(())
    }

    async fn handle_update(&self, update: serde_json::Value) -> Result<Option<IncomingMessage>> {
        // Parse WhatsApp webhook payload
        if let Some(entry) = update["entry"].as_array().and_then(|e| e.first())
            && let Some(change) = entry["changes"].as_array().and_then(|c| c.first())
            && let Some(msg) = change["value"]["messages"]
                .as_array()
                .and_then(|m| m.first())
        {
            let from = msg["from"].as_str().unwrap_or("");
            let text = msg["text"]["body"].as_str().unwrap_or("");
            let name = change["value"]["contacts"][0]["profile"]["name"]
                .as_str()
                .unwrap_or("WhatsApp User");

            return Ok(Some(IncomingMessage {
                platform: "whatsapp".to_string(),
                channel_id: from.to_string(),
                user_id: from.to_string(),
                username: name.to_string(),
                content: text.to_string(),
                is_group_chat: false,
                timestamp: chrono::Utc::now().timestamp(),
                thread_id: None,
                media_urls: Vec::new(),

                raw: update,
            }));
        }
        Ok(None)
    }

    fn config_json(&self) -> serde_json::Value {
        json!({
            "platform": "whatsapp",
            "enabled": self.enabled,
            "token_configured": self.token.is_some(),
            "phone_number_id_configured": self.phone_number_id.is_some(),
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
        self.send_message(msg).await?;
        Ok(String::new())
    }
}
