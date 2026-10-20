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

/// WhatsApp's per-message character limit.
///
/// The Graph API rejects a text message over this limit.
const WHATSAPP_MESSAGE_LIMIT: usize = 4096;

/// The message from a Graph API error body.
///
/// The body is `{"error": {"message": "...", "code": N, ...}}`. A body
/// that does not parse, or has no message, yields a short marker rather
/// than the raw body — the raw body can be large and can echo the
/// request.
fn graph_error_message(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| {
            v.get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "<no error message in the response body>".to_string())
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
        // The Graph API base comes from the configuration, like the
        // Telegram, Discord and Slack adapters. It was hard-coded, so a
        // deployment behind a proxy (or on a regional endpoint) could not
        // reach the WhatsApp API at all.
        let url = format!(
            "{}/v18.0/{}/messages",
            crate::tools::platform_bases::whatsapp_api_base(),
            phone_number_id
        );

        // zeroclaw WhatsApp port: WhatsApp renders neither `**` nor `~~` —
        // convert the mapped subset when markdown parsing is enabled for this
        // message (OutgoingMessage::new defaults parse_markdown=true).
        let body_text = if message.parse_markdown {
            crate::gateway_markdown::markdown_to_whatsapp(&message.content)
        } else {
            message.content.clone()
        };

        // WhatsApp's text-message limit is 4096 characters. Over it, the
        // Graph API rejects the request and the message is NOT delivered —
        // so a long answer was silently lost. The text is CHUNKED, the way
        // the Telegram, Discord and Slack sends are.
        for chunk in crate::gateway::telegram::chunk_text(&body_text, WHATSAPP_MESSAGE_LIMIT) {
            let body = json!({
                "messaging_product": "whatsapp",
                "to": phone,
                "type": "text",
                "text": {"body": chunk}
            });

            let resp = client
                .post(&url)
                .header("Authorization", format!("Bearer {}", token))
                .json(&body)
                .send()
                .await
                .map_err(|e| Error::Network(e.into()))?;

            let status = resp.status();
            if !status.is_success() {
                // The Graph API's body carries the REASON ("Unsupported post
                // request.", "Invalid OAuth access token", the code), and a
                // bare status leaves the operator guessing which of the four
                // common causes it was.
                let body = resp.text().await.unwrap_or_default();
                return Err(Error::Agent(format!(
                    "WhatsApp API error: {}: {}",
                    status,
                    graph_error_message(&body)
                )));
            }
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
                is_channel_post: false,
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
            // The adapter can SEND but not RECEIVE: nothing in the
            // runtime invokes this adapter's `handle_update`, so an
            // inbound message never reaches the agent. The status report
            // used to say "enabled" and nothing more, so an operator
            // believed they could receive WhatsApp messages. (RT-86.)
            // The inbound truth, from the gateway's single table — so
            // this and `operant gateway status` cannot disagree.
            "inbound": crate::gateway::platform_can_receive("whatsapp"),
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

    /// The Graph API's error message must reach the operator.
    ///
    /// A bare "WhatsApp API error: 400" leaves four common causes
    /// indistinguishable: an expired token, an unverified phone number, a
    /// rejected template, and a message outside the 24-hour window.
    #[test]
    fn the_graph_api_error_message_reaches_the_operator() {
        let body = r#"{"error":{"message":"Unsupported post request.","code":100,"type":"GraphMethodException"}}"#;
        assert_eq!(graph_error_message(body), "Unsupported post request.");
    }

    /// A body that does not parse, or has no message, must not be echoed
    /// raw — the raw body can be large and can echo the request.
    #[test]
    fn an_unparseable_graph_api_body_is_not_echoed() {
        let msg = graph_error_message("<html>502 Bad Gateway</html>");
        assert!(
            !msg.contains("<html>"),
            "the raw body must not be echoed, got: {msg}"
        );
        assert!(msg.contains("no error message"), "got: {msg}");
    }
}
