//! `discord` — extracted verbatim from gateway/mod.rs.

use crate::config::runtime_config;
use crate::error::{Error, Result};
use async_trait::async_trait;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use super::*;

/// Discord adapter
pub struct DiscordAdapter {
    token: Option<String>,
    enabled: bool,
}

impl DiscordAdapter {
    /// Create a new Discord adapter
    pub fn new(token: Option<String>) -> Self {
        let enabled = token.is_some();
        Self { token, enabled }
    }

    pub(crate) fn api_url(&self) -> String {
        runtime_config().gateway.discord_api_base
    }
}

#[async_trait]
impl PlatformAdapter for DiscordAdapter {
    fn name(&self) -> &str {
        "discord"
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }

    async fn start(&self) -> Result<()> {
        if !self.is_enabled() {
            return Ok(());
        }

        // Verify the token. Use `?` propagation instead of `.unwrap()` —
        // previously this panicked if `token` was None despite `is_enabled`
        // returning true (race during config reload). (iter-125 — closes
        // the ponytail-audit security bug "token unwrap panics".)
        let token = self
            .token
            .as_ref()
            .ok_or_else(|| Error::Config("Discord token not configured".to_string()))?;
        let client = shared_http_client().clone();
        let response = client
            .get(format!("{}/users/@me", self.api_url()))
            .header("Authorization", format!("Bot {}", token))
            .send()
            .await?;

        if response.status().is_success() {
            info!("Discord bot started successfully");
            Ok(())
        } else {
            Err(crate::error::Error::Agent(
                "Failed to verify Discord bot token".to_string(),
            ))
        }
    }

    async fn stop(&self) -> Result<()> {
        info!("Discord adapter stopped");
        Ok(())
    }

    /// Send a typing indicator. Discord's typing endpoint is
    /// POST /channels/{id}/typing (5s TTL). (Bug #13 from iter-98 audit —
    /// previously Discord used the default no-op, so users saw no indicator.)
    fn send_typing(&self, channel_id: &str, _thread_id: Option<i64>) -> Result<()> {
        let token = self
            .token
            .as_ref()
            .ok_or_else(|| Error::Config("Discord token not configured".to_string()))?;
        let url = format!("{}/channels/{}/typing", self.api_url(), channel_id);
        // Fire and forget — we don't await the response since send_typing
        // is synchronous. The next typing call in 4s will refresh it.
        let client = shared_http_client().clone();
        let token = token.clone();
        let url = url.clone();
        tokio::spawn(async move {
            let _ = client
                .post(&url)
                .header("Authorization", format!("Bot {}", token))
                .send()
                .await;
        });
        Ok(())
    }

    async fn send_message(&self, message: OutgoingMessage) -> Result<()> {
        let client = shared_http_client().clone();
        // Propagate missing token via `?` instead of `.unwrap()` (iter-125 —
        // closes the ponytail-audit "token unwrap panics" security bug).
        let token = self
            .token
            .as_ref()
            .ok_or_else(|| Error::Config("Discord token not configured".to_string()))?;

        // Discord's message limit is 2000 chars. Messages exceeding this
        // are silently rejected by the API (400 Bad Request) — the user
        // gets nothing. Chunk the text to stay under the limit.
        // (Bug #14 from iter-98 audit.)
        let chunks = chunk_text(&message.content, 2000);

        // Deny @everyone/@here and role pings by default (hermes
        // `_build_allowed_mentions` parity) — echoed user content or LLM
        // output containing `@everyone` must never ping the whole server.
        let allowed_mentions = serde_json::json!({
            "parse": ["users"],
            "replied_user": true,
        });

        for chunk in chunks {
            let body = serde_json::json!({
                "content": chunk,
                "allowed_mentions": allowed_mentions,
            });

            let url = format!(
                "{}/channels/{}/messages",
                self.api_url(),
                message.channel_id
            );

            client
                .post(&url)
                .header("Authorization", format!("Bot {}", token))
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await?;
        }

        Ok(())
    }

    async fn handle_update(&self, update: serde_json::Value) -> Result<Option<IncomingMessage>> {
        // Parse Discord message create event
        let d = match update.get("d") {
            Some(d) => d,
            None => return Ok(None),
        };

        let author = match d.get("author") {
            Some(a) => a,
            None => return Ok(None),
        };

        let content = d
            .get("content")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string();

        if content.is_empty() || author.get("bot").and_then(|b| b.as_bool()).unwrap_or(false) {
            return Ok(None);
        }

        let channel_id = d
            .get("channel_id")
            .and_then(|c| c.as_str())
            .unwrap_or_default()
            .to_string();

        Ok(Some(
            IncomingMessage::new(
                "discord",
                author
                    .get("id")
                    .and_then(|id| id.as_str())
                    .unwrap_or("unknown"),
                author
                    .get("username")
                    .and_then(|u| u.as_str())
                    .unwrap_or("unknown"),
                channel_id,
                content,
            )
            .with_raw(update),
        ))
    }

    fn config_json(&self) -> serde_json::Value {
        serde_json::json!({
            "platform": "discord",
            "enabled": self.enabled,
            "has_token": self.token.is_some()
        })
    }

    async fn start_with_channel(
        &self,
        message_tx: mpsc::UnboundedSender<IncomingMessage>,
    ) -> Result<()> {
        // Discord gateway WebSocket — receives MESSAGE_CREATE events.
        // Without this, the Discord adapter could only send outbound
        // messages; users could never talk TO the bot. (iter-129 —
        // closes the ponytail-audit gap "Discord has no gateway WebSocket
        // → no inbound MESSAGE_CREATE events ever".)
        //
        // Implementation notes:
        // - Uses wss://gateway.discord.gg/?v=10&encoding=json
        // - Handshake: OPCODE 10 HELLO → send OPCODE 2 IDENTIFY →
        //   receive OPCODE 0 READY → listen for OPCODE 0 MESSAGE_CREATE
        // - Heartbeat: send OPCODE 1 every `heartbeat_interval` ms
        // - Reconnect: on disconnect, sleep with exponential backoff
        let token = self
            .token
            .as_ref()
            .ok_or_else(|| Error::Config("Discord token not configured".to_string()))?;
        let token = token.clone();
        let api_url = self.api_url();

        // The bare wss://gateway.discord.gg always works for single-shard bots.
        let gateway_url = "wss://gateway.discord.gg/?v=10&encoding=json";

        tokio::spawn(async move {
            let mut reconnect_delay = std::time::Duration::from_secs(1);
            let max_reconnect_delay = std::time::Duration::from_secs(30);

            loop {
                info!(url = %gateway_url, "Discord gateway: connecting");
                match connect_discord_gateway(gateway_url, &token, &api_url, &message_tx).await {
                    Ok(()) => {
                        info!("Discord gateway: connection closed cleanly");
                        reconnect_delay = std::time::Duration::from_secs(1);
                    }
                    Err(e) => {
                        warn!(error = %e, "Discord gateway: connection error");
                        reconnect_delay = (reconnect_delay * 2).min(max_reconnect_delay);
                    }
                }

                if message_tx.is_closed() {
                    info!("Discord gateway: message channel closed, shutting down");
                    break;
                }
                tokio::time::sleep(reconnect_delay).await;
            }
        });

        Ok(())
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

/// Connect to the Discord gateway, complete the handshake, and listen for
/// MESSAGE_CREATE events. Returns when the connection closes (clean or
/// error). The caller is responsible for reconnecting.
///
/// (iter-129 — see DiscordAdapter::start_with_channel.)
pub(crate) async fn connect_discord_gateway(
    url: &str,
    token: &str,
    api_url: &str,
    message_tx: &mpsc::UnboundedSender<IncomingMessage>,
) -> Result<()> {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    // Connect with the Discord-recommended User-Agent.
    let request = tokio_tungstenite::tungstenite::http::Request::builder()
        .uri(url)
        .header(
            "User-Agent",
            "Operant-DiscordBot (https://operant.dev, 0.1.3)",
        )
        .header("Host", "gateway.discord.gg")
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header(
            "Sec-WebSocket-Key",
            tokio_tungstenite::tungstenite::handshake::client::generate_key(),
        );
    let request = request
        .body(())
        .map_err(|e| Error::Agent(format!("Discord WS request build failed: {e}")))?;

    let (ws_stream, _response) = tokio_tungstenite::connect_async(request)
        .await
        .map_err(|e| Error::Agent(format!("Discord WS connect failed: {e}")))?;

    let (mut write, mut read) = ws_stream.split();

    // Step 1: receive HELLO (opcode 10) with heartbeat_interval.
    let hello = read
        .next()
        .await
        .ok_or_else(|| Error::Agent("Discord WS closed before HELLO".to_string()))?
        .map_err(|e| Error::Agent(format!("Discord WS HELLO read failed: {e}")))?;
    let hello_text = match hello {
        Message::Text(t) => t.to_string(),
        Message::Binary(b) => String::from_utf8_lossy(&b).to_string(),
        other => {
            return Err(Error::Agent(format!(
                "Discord WS HELLO unexpected frame: {other:?}"
            )));
        }
    };
    let hello_json: serde_json::Value = serde_json::from_str(&hello_text)
        .map_err(|e| Error::Agent(format!("Discord WS HELLO parse failed: {e}")))?;
    if hello_json.get("op").and_then(|v| v.as_u64()) != Some(10) {
        return Err(Error::Agent(format!(
            "Discord WS expected opcode 10 (HELLO), got: {hello_text}"
        )));
    }
    let heartbeat_interval = hello_json
        .get("d")
        .and_then(|d| d.get("heartbeat_interval"))
        .and_then(|v| v.as_u64())
        .ok_or_else(|| Error::Agent("Discord WS HELLO missing heartbeat_interval".to_string()))?;

    // Step 2: send IDENTIFY (opcode 2).
    let identify = serde_json::json!({
        "op": 2,
        "d": {
            "token": token,
            "intents": (1 << 0) | (1 << 9) | (1 << 15),  // GUILDS + GUILD_MESSAGES + MESSAGE_CONTENT
            "properties": {
                "os": "linux",
                "browser": "operant",
                "device": "operant"
            }
        }
    });
    write
        .send(Message::Text(identify.to_string().into()))
        .await
        .map_err(|e| Error::Agent(format!("Discord WS IDENTIFY send failed: {e}")))?;

    // Step 3: spawn a heartbeat task.
    let (heartbeat_tx, mut heartbeat_rx) = mpsc::unbounded_channel::<()>();
    {
        let heartbeat_tx = heartbeat_tx.clone();
        let interval_ms = heartbeat_interval;
        tokio::spawn(async move {
            // Discord wants the first heartbeat after `interval` ms (not
            // immediately), and then every `interval` ms after.
            let mut ticker = tokio::time::interval(std::time::Duration::from_millis(interval_ms));
            // We need to track the last sequence number for the heartbeat payload.
            // Send via heartbeat_tx to trigger a heartbeat in the main loop.
            loop {
                ticker.tick().await;
                if heartbeat_tx.send(()).is_err() {
                    break;
                }
            }
        });
    }
    // Drop the original heartbeat_tx so the task's clone is the only sender.
    drop(heartbeat_tx);

    // Step 4: listen for events.
    let mut last_seq: Option<u64> = None;
    loop {
        tokio::select! {
            // Heartbeat tick — send OPCODE 1.
            _ = heartbeat_rx.recv() => {
                let heartbeat = serde_json::json!({
                    "op": 1,
                    "d": last_seq
                });
                if let Err(e) = write.send(Message::Text(heartbeat.to_string().into())).await {
                    return Err(Error::Agent(format!("Discord WS heartbeat send failed: {e}")));
                }
            }
            // Incoming message.
            msg = read.next() => {
                let msg = match msg {
                    Some(Ok(Message::Text(t))) => t.to_string(),
                    Some(Ok(Message::Binary(b))) => String::from_utf8_lossy(&b).to_string(),
                    Some(Ok(Message::Ping(p))) => {
                        let _ = write.send(Message::Pong(p)).await;
                        continue;
                    }
                    Some(Ok(Message::Close(_))) => {
                        info!("Discord WS: server sent Close frame");
                        return Ok(());
                    }
                    Some(Ok(_)) => continue,  // Binary/Pong/Frame — ignore
                    Some(Err(e)) => {
                        return Err(Error::Agent(format!("Discord WS read error: {e}")));
                    }
                    None => {
                        info!("Discord WS: stream ended");
                        return Ok(());
                    }
                };
                let json: serde_json::Value = match serde_json::from_str(&msg) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let op = json.get("op").and_then(|v| v.as_u64()).unwrap_or(0);
                // Track sequence number for heartbeats.
                if let Some(s) = json.get("s").and_then(|v| v.as_u64()) {
                    last_seq = Some(s);
                }
                match op {
                    0 => {
                        // Dispatch event.
                        let event_type = json.get("t").and_then(|v| v.as_str()).unwrap_or("");
                        if event_type == "READY" {
                            let bot_user = json
                                .get("d")
                                .and_then(|d| d.get("user"))
                                .and_then(|u| u.get("username"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("(unknown)");
                            info!(bot = %bot_user, "Discord gateway: READY");
                        } else if event_type == "MESSAGE_CREATE" {
                            // Forward to the gateway runner.
                            if let Some(incoming) = parse_discord_message(&json, api_url)
                                && message_tx.send(incoming).is_err() {
                                    info!("Discord WS: message channel closed, exiting");
                                    return Ok(());
                                }
                        }
                    }
                    11 => {
                        // Heartbeat ACK — server confirmed our heartbeat.
                        // (No action needed; just keep going.)
                    }
                    7 => {
                        info!("Discord WS: server requested RECONNECT");
                        return Ok(());  // Caller will reconnect.
                    }
                    9 => {
                        let invalid = json.get("d").and_then(|v| v.as_bool()).unwrap_or(false);
                        if invalid {
                            return Err(Error::Agent(
                                "Discord WS: INVALID_SESSION (invalid token or intents)".to_string()
                            ));
                        }
                        // Resumable invalid session — wait and let caller reconnect.
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                        return Ok(());
                    }
                    _ => {
                        // Unknown opcode — log at debug and ignore.
                        debug!(op = op, "Discord WS: unknown opcode");
                    }
                }
            }
        }
    }
}

/// Parse a Discord MESSAGE_CREATE dispatch into an IncomingMessage.
/// Returns None if the message is from a bot (we don't want bot loops)
/// or is missing required fields.
pub(crate) fn parse_discord_message(
    json: &serde_json::Value,
    _api_url: &str,
) -> Option<IncomingMessage> {
    let d = json.get("d")?;
    let author = d.get("author")?;
    let is_bot = author.get("bot").and_then(|v| v.as_bool()).unwrap_or(false);
    if is_bot {
        return None; // Ignore bot messages (prevents loops).
    }
    let content = d.get("content")?.as_str()?.to_string();
    if content.is_empty() {
        return None; // Embed-only message — skip.
    }
    let channel_id = d.get("channel_id")?.as_str()?.to_string();
    let user_id = author.get("id")?.as_str()?.to_string();
    let username = author.get("username")?.as_str()?.to_string();
    let guild_id = d.get("guild_id").and_then(|v| v.as_str()).is_some();
    let timestamp = d
        .get("timestamp")
        .and_then(|v| v.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.timestamp())
        .unwrap_or(0);

    Some(IncomingMessage {
        platform: "discord".to_string(),
        channel_id,
        user_id,
        username,
        content,
        raw: json.clone(),
        timestamp,
        is_group_chat: guild_id,
        thread_id: None,
        media_urls: Vec::new(),
    })
}
