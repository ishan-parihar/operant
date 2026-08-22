//! `types` — extracted verbatim from gateway/mod.rs.

use crate::config::runtime_config;
use crate::error::Result;
use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::sync::mpsc;
use uuid::Uuid;

/// Configuration for the gateway
#[derive(Debug, Clone)]
pub struct GatewayConfig {
    /// Enable Telegram bot
    pub telegram_enabled: bool,
    /// Telegram bot token
    pub telegram_token: Option<String>,
    /// Enable Discord bot
    pub discord_enabled: bool,
    /// Discord bot token
    pub discord_token: Option<String>,
    /// Enable Slack bot
    pub slack_enabled: bool,
    /// Slack bot token
    pub slack_token: Option<String>,
    /// Enable WhatsApp
    pub whatsapp_enabled: bool,
    /// WhatsApp Cloud API token
    pub whatsapp_token: Option<String>,
    /// WhatsApp Cloud API phone number ID (Meta Business Manager).
    /// Required for outbound sends. (R22)
    pub whatsapp_phone_number_id: Option<String>,
    /// Enable Email (SMTP)
    pub email_enabled: bool,
    /// SMTP host
    pub email_smtp_host: Option<String>,
    /// SMTP user
    pub email_smtp_user: Option<String>,
    /// SMTP password
    pub email_smtp_pass: Option<String>,
    /// Enable SMS (Twilio)
    pub sms_twilio_enabled: bool,
    /// Enable webhooks
    pub webhooks_enabled: bool,
    /// Webhook listen address
    pub webhooks_addr: Option<String>,
    /// Shared secret for HMAC-SHA256 verification of inbound webhook
    /// signatures (GitHub/Stripe/Slack/custom `x-webhook-signature`).
    pub webhooks_secret: Option<String>,
    /// Default admin users (user IDs that can access admin commands)
    pub admins: Vec<String>,
    /// Streaming transport mode: "auto", "edit", "draft", "off"
    pub streaming_transport: String,
    /// HTTP/SOCKS5 proxy URL for Telegram API requests
    pub telegram_proxy: Option<String>,
    /// Bot username for @mention detection in groups
    pub telegram_bot_username: Option<String>,
    /// Enable DM topic creation for private chats (Bot API 9.4+)
    pub telegram_dm_topics_enabled: bool,
    /// Cap on concurrent gateway sessions (hermes `max_concurrent_sessions`
    /// parity). When reached, new sessions get a refusal reply while existing
    /// holders keep their slots. `None` = unlimited.
    pub max_concurrent_sessions: Option<usize>,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        let settings = runtime_config().gateway;
        Self {
            telegram_enabled: settings.telegram_enabled,
            telegram_token: settings.telegram_token,
            discord_enabled: settings.discord_enabled,
            discord_token: settings.discord_token,
            slack_enabled: settings.slack_enabled,
            slack_token: settings.slack_token,
            whatsapp_enabled: settings.whatsapp_enabled,
            whatsapp_token: settings.whatsapp_token,
            whatsapp_phone_number_id: settings.whatsapp_phone_number_id,
            email_enabled: settings.email_enabled,
            email_smtp_host: settings.email_smtp_host,
            email_smtp_user: settings.email_smtp_user,
            email_smtp_pass: settings.email_smtp_pass,
            sms_twilio_enabled: settings.sms_twilio_enabled,
            webhooks_enabled: settings.webhooks_enabled,
            webhooks_addr: settings.webhooks_addr,
            webhooks_secret: settings.webhooks_secret,
            admins: settings.admins,
            streaming_transport: settings.streaming_transport,
            telegram_proxy: settings.telegram_proxy,
            telegram_bot_username: settings.telegram_bot_username,
            telegram_dm_topics_enabled: settings.telegram_dm_topics_enabled,
            max_concurrent_sessions: settings.max_concurrent_sessions,
        }
    }
}

/// Incoming message from a platform
#[derive(Debug, Clone)]
pub struct IncomingMessage {
    /// Platform source (e.g., "telegram", "discord", "slack")
    pub platform: String,
    /// User ID on the platform
    pub user_id: String,
    /// Username or display name
    pub username: String,
    /// Channel/chat ID
    pub channel_id: String,
    /// Message content
    pub content: String,
    /// Original raw message (platform-specific)
    pub raw: serde_json::Value,
    /// Timestamp
    pub timestamp: i64,
    /// Whether this message is from a group chat
    pub is_group_chat: bool,
    /// Forum thread/topic ID (Telegram-specific)
    pub thread_id: Option<i64>,
    /// Locally-cached paths for attachments on this message (photos,
    /// documents, voice/audio, video). The gateway downloads platform
    /// attachments to disk so the agent can inspect them with native tools
    /// (vision_analyze for images, file_read/STT for others) — hermes
    /// `event.media_urls` parity. Empty when the message is plain text.
    pub media_urls: Vec<String>,
}

impl IncomingMessage {
    /// Create a new incoming message
    pub fn new(
        platform: impl Into<String>,
        user_id: impl Into<String>,
        username: impl Into<String>,
        channel_id: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            platform: platform.into(),
            user_id: user_id.into(),
            username: username.into(),
            channel_id: channel_id.into(),
            content: content.into(),
            raw: serde_json::json!({}),
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
            is_group_chat: false,
            thread_id: None,
            media_urls: Vec::new(),
        }
    }

    /// Set the raw message
    pub fn with_raw(mut self, raw: serde_json::Value) -> Self {
        self.raw = raw;
        self
    }

    /// Mark as group chat message
    pub fn with_group_chat(mut self, is_group: bool) -> Self {
        self.is_group_chat = is_group;
        self
    }

    /// Set the thread/topic ID (Telegram forum topics)
    pub fn with_thread_id(mut self, thread_id: Option<i64>) -> Self {
        self.thread_id = thread_id;
        self
    }

    /// Attach locally-cached media file paths (hermes `event.media_urls`
    /// parity). The agent can then inspect these with vision/file tools.
    pub fn with_media_urls(mut self, media_urls: Vec<String>) -> Self {
        self.media_urls = media_urls;
        self
    }
}

/// Outgoing message to a platform
#[derive(Debug, Clone)]
pub struct OutgoingMessage {
    /// Target channel/chat ID
    pub channel_id: String,
    /// Message content (markdown or plain text)
    pub content: String,
    /// Whether to parse markdown
    pub parse_markdown: bool,
    /// Reply to message ID (if any)
    pub reply_to: Option<String>,
    /// Forum thread/topic ID (Telegram-specific; forwarded from the
    /// incoming message so replies land in the same topic the user sent
    /// from instead of the general chat).
    pub thread_id: Option<i64>,
}

impl OutgoingMessage {
    /// Create a new outgoing message
    pub fn new(channel_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            channel_id: channel_id.into(),
            content: content.into(),
            parse_markdown: true,
            reply_to: None,
            thread_id: None,
        }
    }

    /// Disable markdown parsing
    pub fn no_markdown(mut self) -> Self {
        self.parse_markdown = false;
        self
    }

    /// Set reply-to message ID
    pub fn with_reply_to(mut self, message_id: impl Into<String>) -> Self {
        self.reply_to = Some(message_id.into());
        self
    }

    /// Set the forum thread/topic ID (Telegram message_thread_id).
    pub fn with_thread_id(mut self, thread_id: Option<i64>) -> Self {
        self.thread_id = thread_id;
        self
    }
}

/// Information about a platform user
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInfo {
    pub user_id: String,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub platform: String,
}

/// Session state for a platform conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformSession {
    pub session_id: String,
    pub platform: String,
    pub platform_user_id: String,
    pub platform_channel_id: String,
    pub operant_session_id: String,
    pub created_at: String,
    pub last_active: String,
    pub metadata: HashMap<String, String>,
}

/// Manages platform session state
pub struct SessionStore {
    sessions: std::sync::RwLock<HashMap<String, PlatformSession>>,
}

impl Default for SessionStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionStore {
    pub fn new() -> Self {
        Self {
            sessions: std::sync::RwLock::new(HashMap::new()),
        }
    }

    #[expect(
        clippy::expect_used,
        reason = "poisoned lock: panic is the intended recovery"
    )]
    /// Create a new session and return it
    pub fn create_session(
        &self,
        platform: &str,
        user_id: &str,
        channel_id: &str,
    ) -> Result<PlatformSession> {
        let now = Utc::now().to_rfc3339();
        let session = PlatformSession {
            session_id: Uuid::new_v4().to_string(),
            platform: platform.to_string(),
            platform_user_id: user_id.to_string(),
            platform_channel_id: channel_id.to_string(),
            operant_session_id: String::new(),
            created_at: now.clone(),
            last_active: now,
            metadata: HashMap::new(),
        };
        let mut sessions = self.sessions.write().expect("Session store lock poisoned");
        let session_id = session.session_id.clone();
        sessions.insert(session_id, session.clone());
        Ok(session)
    }

    #[expect(
        clippy::expect_used,
        reason = "poisoned lock: panic is the intended recovery"
    )]
    /// Get a session by its ID
    pub fn get_session(&self, session_id: &str) -> Option<PlatformSession> {
        let sessions = self.sessions.read().expect("Session store lock poisoned");
        sessions.get(session_id).cloned()
    }

    #[expect(
        clippy::expect_used,
        reason = "poisoned lock: panic is the intended recovery"
    )]
    /// Find a session matching platform + user + channel
    pub fn find_session(
        &self,
        platform: &str,
        user_id: &str,
        channel_id: &str,
    ) -> Option<PlatformSession> {
        let sessions = self.sessions.read().expect("Session store lock poisoned");
        sessions
            .values()
            .find(|s| {
                s.platform == platform
                    && s.platform_user_id == user_id
                    && s.platform_channel_id == channel_id
            })
            .cloned()
    }

    #[expect(
        clippy::expect_used,
        reason = "poisoned lock: panic is the intended recovery"
    )]
    /// Update the last_active timestamp for a session
    pub fn update_activity(&self, session_id: &str) -> Result<()> {
        let mut sessions = self.sessions.write().expect("Session store lock poisoned");
        if let Some(session) = sessions.get_mut(session_id) {
            session.last_active = Utc::now().to_rfc3339();
            Ok(())
        } else {
            Err(crate::error::Error::Agent(format!(
                "Session not found: {}",
                session_id
            )))
        }
    }

    #[expect(
        clippy::expect_used,
        reason = "poisoned lock: panic is the intended recovery"
    )]
    /// Remove a session
    pub fn close_session(&self, session_id: &str) -> Result<()> {
        let mut sessions = self.sessions.write().expect("Session store lock poisoned");
        sessions.remove(session_id);
        Ok(())
    }

    #[expect(
        clippy::expect_used,
        reason = "poisoned lock: panic is the intended recovery"
    )]
    /// List all active sessions, optionally filtered by platform
    pub fn list_active_sessions(&self, platform: Option<&str>) -> Vec<PlatformSession> {
        let sessions = self.sessions.read().expect("Session store lock poisoned");
        match platform {
            Some(p) => sessions
                .values()
                .filter(|s| s.platform == p)
                .cloned()
                .collect(),
            None => sessions.values().cloned().collect(),
        }
    }

    #[expect(
        clippy::expect_used,
        reason = "poisoned lock: panic is the intended recovery"
    )]
    /// Total number of sessions
    pub fn get_session_count(&self) -> usize {
        let sessions = self.sessions.read().expect("Session store lock poisoned");
        sessions.len()
    }

    #[expect(
        clippy::expect_used,
        reason = "poisoned lock: panic is the intended recovery"
    )]
    /// Find a session by its Operant session ID
    pub fn get_operant_session(&self, operant_session_id: &str) -> Option<PlatformSession> {
        let sessions = self.sessions.read().expect("Session store lock poisoned");
        sessions
            .values()
            .find(|s| s.operant_session_id == operant_session_id)
            .cloned()
    }

    #[expect(
        clippy::expect_used,
        reason = "poisoned lock: panic is the intended recovery"
    )]
    /// Update metadata fields on a session identified by platform + user + channel.
    /// Returns Ok(true) if session was found and updated, Ok(false) otherwise.
    pub fn update_session_metadata(
        &self,
        platform: &str,
        user_id: &str,
        channel_id: &str,
        updates: &[(String, String)],
    ) -> bool {
        let mut sessions = self.sessions.write().expect("Session store lock poisoned");
        let session = sessions.values_mut().find(|s| {
            s.platform == platform
                && s.platform_user_id == user_id
                && s.platform_channel_id == channel_id
        });
        if let Some(s) = session {
            for (key, value) in updates {
                s.metadata.insert(key.clone(), value.clone());
            }
            s.last_active = Utc::now().to_rfc3339();
            true
        } else {
            false
        }
    }

    /// Find or create a shared session for a group chat. When `thread_id`
    /// (forum topic) is present, the session is scoped to that thread so
    /// each topic gets its own record — hermes build_session_key parity.
    pub fn find_or_create_shared_session(
        &self,
        platform: &str,
        channel_id: &str,
        thread_id: Option<i64>,
    ) -> Result<PlatformSession> {
        if let Some(tid) = thread_id {
            let scoped = format!("{}__thread_{}", channel_id, tid);
            if let Some(s) = self.find_session(platform, "__shared__", &scoped) {
                let _ = self.update_activity(&s.session_id);
                return Ok(s);
            }
            return self.create_session(platform, "__shared__", &scoped);
        }
        if let Some(s) = self.find_session(platform, "__shared__", channel_id) {
            let _ = self.update_activity(&s.session_id);
            return Ok(s);
        }
        self.create_session(platform, "__shared__", channel_id)
    }
}

/// Type of channel
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ChannelType {
    Direct,
    Group,
    Channel,
    Unknown,
}

/// Information about a registered channel
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelInfo {
    pub channel_id: String,
    pub platform: String,
    pub name: Option<String>,
    pub channel_type: ChannelType,
    pub admins: Vec<String>,
}

/// Directory mapping channels to their metadata
pub struct ChannelDirectory {
    channels: std::sync::RwLock<HashMap<String, ChannelInfo>>,
}

impl Default for ChannelDirectory {
    fn default() -> Self {
        Self::new()
    }
}

impl ChannelDirectory {
    pub fn new() -> Self {
        Self {
            channels: std::sync::RwLock::new(HashMap::new()),
        }
    }

    #[expect(
        clippy::expect_used,
        reason = "poisoned lock: panic is the intended recovery"
    )]
    /// Register a new channel
    pub fn register_channel(
        &self,
        channel_id: &str,
        platform: &str,
        name: Option<&str>,
        channel_type: ChannelType,
        admins: Vec<String>,
    ) -> Result<()> {
        let mut channels = self.channels.write().expect("Channel store lock poisoned");
        if channels.contains_key(channel_id) {
            return Err(crate::error::Error::Agent(format!(
                "Channel already registered: {}",
                channel_id
            )));
        }
        channels.insert(
            channel_id.to_string(),
            ChannelInfo {
                channel_id: channel_id.to_string(),
                platform: platform.to_string(),
                name: name.map(|n| n.to_string()),
                channel_type,
                admins,
            },
        );
        Ok(())
    }

    #[expect(
        clippy::expect_used,
        reason = "poisoned lock: panic is the intended recovery"
    )]
    /// Get channel info by ID
    pub fn get_channel(&self, channel_id: &str) -> Option<ChannelInfo> {
        let channels = self.channels.read().expect("Channel store lock poisoned");
        channels.get(channel_id).cloned()
    }

    #[expect(
        clippy::expect_used,
        reason = "poisoned lock: panic is the intended recovery"
    )]
    /// Remove a channel
    pub fn remove_channel(&self, channel_id: &str) -> Result<()> {
        let mut channels = self.channels.write().expect("Channel store lock poisoned");
        channels.remove(channel_id);
        Ok(())
    }

    #[expect(
        clippy::expect_used,
        reason = "poisoned lock: panic is the intended recovery"
    )]
    /// List channels, optionally filtered by platform
    pub fn list_channels(&self, platform: Option<&str>) -> Vec<ChannelInfo> {
        let channels = self.channels.read().expect("Channel store lock poisoned");
        match platform {
            Some(p) => channels
                .values()
                .filter(|c| c.platform == p)
                .cloned()
                .collect(),
            None => channels.values().cloned().collect(),
        }
    }

    #[expect(
        clippy::expect_used,
        reason = "poisoned lock: panic is the intended recovery"
    )]
    /// Check if a user is admin of a channel
    pub fn is_admin(&self, channel_id: &str, user_id: &str) -> bool {
        let channels = self.channels.read().expect("Channel store lock poisoned");
        channels
            .get(channel_id)
            .map(|c| c.admins.iter().any(|a| a == user_id))
            .unwrap_or(false)
    }
}

/// Statistics about the gateway
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayStats {
    pub uptime_seconds: u64,
    pub active_sessions: usize,
    pub registered_channels: usize,
    pub active_adapters: usize,
    pub messages_processed: u64,
    pub start_time: String,
}

/// Shared HTTP client pool for all gateway adapters. Previously each
/// `send_message` / `send_typing` call did `reqwest::Client::new()` which
/// allocated a fresh connection pool per request — a connection-pool leak
/// in the hot path. (iter-125 — closes the ponytail-audit security/perform-
/// ance bug "4 adapters call reqwest::Client::new() per send_message".)
pub(crate) fn shared_http_client() -> &'static reqwest::Client {
    use std::sync::OnceLock;
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

/// Trait for platform adapters
#[async_trait]
pub trait PlatformAdapter: Send + Sync {
    /// Get the platform name (e.g., "telegram", "discord")
    fn name(&self) -> &str;

    /// Check if the adapter is enabled and configured
    fn is_enabled(&self) -> bool;

    /// Start the adapter (e.g., start polling or webhooks)
    async fn start(&self) -> Result<()>;

    /// Stop the adapter
    async fn stop(&self) -> Result<()>;

    /// Send a message through the platform
    async fn send_message(&self, message: OutgoingMessage) -> Result<()>;

    /// Handle an incoming update (webhook or poll result)
    async fn handle_update(&self, update: serde_json::Value) -> Result<Option<IncomingMessage>>;

    /// Get the adapter's specific configuration as JSON
    fn config_json(&self) -> serde_json::Value;

    /// Platform name, defaults to `name()`.
    fn platform_name(&self) -> &str {
        self.name()
    }

    /// Check if the adapter is healthy.
    fn health_check(&self) -> Result<bool> {
        Ok(true)
    }

    /// Send a typing indicator to a channel. `thread_id` is the platform
    /// forum topic (Telegram message_thread_id) so the indicator shows in
    /// the same topic the user sent from, not the general chat.
    fn send_typing(&self, _channel_id: &str, _thread_id: Option<i64>) -> Result<()> {
        Ok(())
    }

    /// Send a message and return the platform message ID.
    async fn send_message_return_id(&self, message: OutgoingMessage) -> Result<String> {
        self.send_message(message).await?;
        Ok(String::new())
    }

    /// Edit an existing message and return the platform message ID.
    async fn edit_message(
        &self,
        _channel_id: &str,
        _message_id: &str,
        _message: &OutgoingMessage,
    ) -> Result<String> {
        Ok(String::new())
    }

    /// Delete a message.
    fn delete_message(&self, _channel_id: &str, _message_id: &str) -> Result<()> {
        Ok(())
    }

    /// Get information about a user.
    fn get_user_info(&self, user_id: &str) -> Result<UserInfo> {
        Ok(UserInfo {
            user_id: user_id.to_string(),
            username: None,
            display_name: None,
            avatar_url: None,
            platform: self.name().to_string(),
        })
    }

    /// Start the adapter with a message channel for incoming messages.
    async fn start_with_channel(
        &self,
        message_tx: mpsc::UnboundedSender<IncomingMessage>,
    ) -> Result<()>;

    /// Send a message to a specific channel, returning the message ID.
    async fn send_message_to_channel(
        &self,
        channel_id: &str,
        message: &OutgoingMessage,
    ) -> Result<String>;

    /// Send a voice/audio message to a channel.
    async fn send_voice(&self, _channel_id: &str, _audio_data: &[u8], _format: &str) -> Result<()> {
        Ok(()) // default no-op for platforms that don't support voice
    }

    /// Ask the operator to approve/deny a tool permission prompt.
    ///
    /// `thread_id` is the forum topic (Telegram message_thread_id) so the
    /// prompt lands in the same topic the discussion is in, not the general
    /// chat. Default implementation sends a plain-text prompt instructing
    /// the operator to reply `/approve` or `/deny`. Platforms that support
    /// interactive components (e.g. Telegram inline keyboards) override this
    /// to render tappable buttons; the resulting tap must resolve the same
    /// way a text reply would (hermes `send_exec_approval` parity).
    async fn send_approval_prompt(
        &self,
        channel_id: &str,
        thread_id: Option<i64>,
        tool_name: &str,
        description: &str,
    ) -> Result<Option<String>> {
        let prompt = format!(
            "🔧 Permission required: {tool_name} — {description}\nReply /approve to allow, /deny to cancel (60s timeout)"
        );
        self.send_message(
            OutgoingMessage::new(channel_id, &prompt)
                .no_markdown()
                .with_thread_id(thread_id),
        )
        .await?;
        Ok(None)
    }

    /// Ask the operator to pick from a list of choices (the `clarify` tool).
    ///
    /// `thread_id` is the forum topic so the question lands in the ongoing
    /// thread. Default implementation sends a plain-text numbered list;
    /// platforms with interactive components override this to render
    /// tappable buttons, and a tap must resolve the pending question the
    /// same way a typed reply would (hermes `send_clarify` parity).
    async fn send_choice_prompt(
        &self,
        channel_id: &str,
        thread_id: Option<i64>,
        question: &str,
        choices: &[String],
    ) -> Result<()> {
        let body = format!(
            "❓ {question}\n\n{}",
            choices
                .iter()
                .enumerate()
                .map(|(i, c)| format!("{}. {c}", i + 1))
                .collect::<Vec<_>>()
                .join("\n")
        );
        self.send_message(
            OutgoingMessage::new(channel_id, &body)
                .no_markdown()
                .with_thread_id(thread_id),
        )
        .await
    }
}
