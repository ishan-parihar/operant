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
    /// The Twilio API base.
    ///
    /// From `TWILIO_API_BASE`, like the credentials above. It was
    /// hard-coded to `api.twilio.com`, so a deployment behind a proxy (or
    /// on a regional endpoint) could not reach the Twilio API at all.
    api_base: String,
}

impl SmsAdapter {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            account_sid: std::env::var("TWILIO_ACCOUNT_SID").ok(),
            auth_token: std::env::var("TWILIO_AUTH_TOKEN").ok(),
            from_number: std::env::var("TWILIO_FROM_NUMBER").ok(),
            // The API base, like the credentials above. It was hard-coded
            // to `api.twilio.com`, so a deployment behind a proxy (or on a
            // regional endpoint) could not reach the Twilio API at all.
            api_base: std::env::var("TWILIO_API_BASE")
                .unwrap_or_else(|_| "https://api.twilio.com".to_string()),
        }
    }
}

/// Twilio's message-body character limit.
///
/// Over it, Twilio answers with a 400 and the message is not delivered.
const SMS_MESSAGE_LIMIT: usize = 1600;

/// The message from a Twilio error body.
///
/// The body is `{"code": 21211, "message": "The 'To' number ...", ...}`.
fn twilio_error_message(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| {
            v.get("message")
                .and_then(|m| m.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "<no error message in the response body>".to_string())
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
            "{}/2010-04-01/Accounts/{}/Messages.json",
            self.api_base.trim_end_matches('/'),
            sid
        );
        let client = shared_http_client().clone();

        // Twilio's message-body limit is 1,600 characters. Over it,
        // Twilio answers with a 400 and the message is NOT delivered — so
        // a long answer was lost. The body is CHUNKED, the way the
        // Telegram, Discord, Slack and WhatsApp sends are.
        for chunk in crate::gateway::telegram::chunk_text(&message.content, SMS_MESSAGE_LIMIT) {
            let resp = client
                .post(&url)
                .basic_auth(sid, Some(token))
                .form(&[
                    ("From", from.as_str()),
                    ("To", &message.channel_id),
                    ("Body", &chunk),
                ])
                .send()
                .await
                .map_err(|e| Error::Network(e.into()))?;

            let status = resp.status();
            if !status.is_success() {
                // The body carries the REASON ("The 'To' number ... is not a
                // valid phone number", "Unauthenticated"), and a bare status
                // leaves the operator guessing.
                let body = resp.text().await.unwrap_or_default();
                return Err(Error::Agent(format!(
                    "Twilio API error: {}: {}",
                    status,
                    twilio_error_message(&body)
                )));
            }
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
            is_channel_post: false,
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
            // The adapter can SEND but not RECEIVE: nothing in the
            // runtime invokes this adapter's `handle_update`, so an
            // inbound message never reaches the agent. The status report
            // used to say "enabled" and nothing more, so an operator
            // believed they could receive SMS messages. (RT-86.)
            // The inbound truth, from the gateway's single table — so
            // this and `operant gateway status` cannot disagree.
            "inbound": crate::gateway::platform_can_receive("sms"),
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
        self.send_message(msg).await?;
        Ok(String::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Twilio's error message must reach the operator.
    #[test]
    fn the_twilio_error_message_reaches_the_operator() {
        let body = r#"{"code":21211,"message":"The 'To' number is not a valid phone number"}"#;
        assert_eq!(
            twilio_error_message(body),
            "The 'To' number is not a valid phone number"
        );
        let msg = twilio_error_message("not json");
        assert!(msg.contains("no error message"), "got: {msg}");
    }
}
