//! `sms` — extracted verbatim from gateway/mod.rs.

use crate::error::{Error, Result};
use async_trait::async_trait;
use serde_json::json;
use tokio::sync::mpsc;
use tracing::info;

use super::*;

// ---------------------------------------------------------------------------
// SMS adapter (Twilio API)
// ---------------------------------------------------------------------------

/// SMS adapter — sends/receives via Twilio REST API.
/// Requires `sms_twilio_account_sid` and `sms_twilio_auth_token` env vars.
pub struct SmsAdapter {
    enabled: bool,
    account_sid: Option<String>,
    auth_token: Option<String>,
    from_number: Option<String>,
}

impl SmsAdapter {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            account_sid: std::env::var("TWILIO_ACCOUNT_SID").ok(),
            auth_token: std::env::var("TWILIO_AUTH_TOKEN").ok(),
            from_number: std::env::var("TWILIO_FROM_NUMBER").ok(),
        }
    }
}

#[async_trait]
impl PlatformAdapter for SmsAdapter {
    fn name(&self) -> &str {
        "sms"
    }
    fn is_enabled(&self) -> bool {
        self.enabled
    }

    async fn start(&self) -> Result<()> {
        info!("SMS adapter started");
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        info!("SMS adapter stopped");
        Ok(())
    }

    async fn send_message(&self, message: OutgoingMessage) -> Result<()> {
        let sid = self
            .account_sid
            .as_ref()
            .ok_or_else(|| Error::Config("TWILIO_ACCOUNT_SID not set".to_string()))?;
        let token = self
            .auth_token
            .as_ref()
            .ok_or_else(|| Error::Config("TWILIO_AUTH_TOKEN not set".to_string()))?;
        let from = self
            .from_number
            .as_ref()
            .ok_or_else(|| Error::Config("TWILIO_FROM_NUMBER not set".to_string()))?;

        let url = format!(
            "https://api.twilio.com/2010-04-01/Accounts/{}/Messages.json",
            sid
        );
        let client = shared_http_client().clone();

        let resp = client
            .post(&url)
            .basic_auth(sid, Some(token))
            .form(&[
                ("From", from.as_str()),
                ("To", &message.channel_id),
                ("Body", &message.content),
            ])
            .send()
            .await
            .map_err(|e| Error::Network(e.into()))?;

        if !resp.status().is_success() {
            return Err(Error::Agent(format!("Twilio API error: {}", resp.status())));
        }

        Ok(())
    }

    async fn handle_update(&self, update: serde_json::Value) -> Result<Option<IncomingMessage>> {
        // Parse Twilio webhook payload
        let from = update["From"]
            .as_str()
            .or_else(|| update["from"].as_str())
            .unwrap_or("");
        let body = update["Body"]
            .as_str()
            .or_else(|| update["body"].as_str())
            .unwrap_or("");

        if from.is_empty() && body.is_empty() {
            return Ok(None);
        }

        Ok(Some(IncomingMessage {
            platform: "sms".to_string(),
            channel_id: from.to_string(),
            user_id: from.to_string(),
            username: from.to_string(),
            content: body.to_string(),
            is_group_chat: false,
            timestamp: chrono::Utc::now().timestamp(),
            thread_id: None,
            media_urls: Vec::new(),

            raw: update,
        }))
    }

    fn config_json(&self) -> serde_json::Value {
        json!({
            "platform": "sms",
            "enabled": self.enabled,
            "account_sid_configured": self.account_sid.is_some(),
            "from_number": self.from_number,
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
