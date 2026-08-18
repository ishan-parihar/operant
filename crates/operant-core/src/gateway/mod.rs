//! Multi-platform gateway for Operant-RS
//!
//! Provides unified messaging interface across multiple platforms including
//! Telegram, Discord, Slack, WhatsApp, and more.

pub mod lifecycle;

use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::config::runtime_config;
use crate::error::{Error, Result};
use crate::gateway::lifecycle::{DeliveryLedger, MirrorRule, SessionStallTracker, TurnLease};
use crate::gateway_markdown::{markdown_to_slack_mrkdwn, markdown_to_telegram_html};
use crate::gateway_session::{PersistentSessionStore, SessionSource};

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

    /// Find or create a shared session keyed by channel_id only (for group chats).
    pub fn find_or_create_shared_session(
        &self,
        platform: &str,
        channel_id: &str,
    ) -> Result<PlatformSession> {
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
fn shared_http_client() -> &'static reqwest::Client {
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
    ) -> Result<()> {
        let prompt = format!(
            "🔧 Permission required: {tool_name} — {description}\nReply /approve to allow, /deny to cancel (60s timeout)"
        );
        self.send_message(
            OutgoingMessage::new(channel_id, &prompt)
                .no_markdown()
                .with_thread_id(thread_id),
        )
        .await
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

/// Gateway for routing messages between platforms and the agent
pub struct Gateway {
    config: GatewayConfig,
    adapters: HashMap<String, Arc<dyn PlatformAdapter>>,
    message_handler: Option<Arc<dyn MessageHandler>>,
    running: Arc<RwLock<bool>>,
    session_store: SessionStore,
    persistent_sessions: Option<Arc<PersistentSessionStore>>,
    channel_directory: ChannelDirectory,
    start_time: Instant,
    start_time_formatted: String,
    messages_processed: Arc<AtomicU64>,
    /// Per-session turn lease (hermes `gateway/turn_lease.py` parity):
    /// only one in-flight agent turn per session key.
    turn_lease: TurnLease,
    /// Last-activity tracker for stall detection (hermes
    /// `gateway/session_stall.py` parity).
    stall_tracker: SessionStallTracker,
    /// Bounded record of outbound deliveries (hermes
    /// `gateway/delivery_ledger.py` parity).
    delivery_ledger: DeliveryLedger,
    /// Response mirror rules (hermes `gateway/mirror.py` parity).
    mirror_rules: Vec<MirrorRule>,
}

/// Handler for incoming messages from any platform
#[async_trait::async_trait]
pub trait MessageHandler: Send + Sync {
    /// Handle an incoming message
    async fn handle(&self, message: IncomingMessage) -> Result<OutgoingMessage>;
}

impl Gateway {
    /// Create a new gateway with the given configuration
    pub fn new(config: GatewayConfig) -> Self {
        Self {
            config,
            adapters: HashMap::new(),
            message_handler: None,
            running: Arc::new(RwLock::new(false)),
            session_store: SessionStore::new(),
            persistent_sessions: None,
            channel_directory: ChannelDirectory::new(),
            start_time: Instant::now(),
            start_time_formatted: Utc::now().to_rfc3339(),
            messages_processed: Arc::new(AtomicU64::new(0)),
            turn_lease: TurnLease::new(),
            stall_tracker: SessionStallTracker::new(),
            delivery_ledger: DeliveryLedger::default(),
            mirror_rules: Vec::new(),
        }
    }

    /// Register a platform adapter
    pub fn with_adapter(mut self, adapter: Arc<dyn PlatformAdapter>) -> Self {
        let name = adapter.name().to_string();
        info!(platform = %name, "Registering platform adapter");
        self.adapters.insert(name, adapter);
        self
    }

    /// Set the message handler
    pub fn with_handler(mut self, handler: Arc<dyn MessageHandler>) -> Self {
        self.message_handler = Some(handler);
        self
    }

    /// Attach a persistent session store for cross-restart session tracking.
    pub fn with_persistent_sessions(mut self, store: Arc<PersistentSessionStore>) -> Self {
        self.persistent_sessions = Some(store);
        self
    }

    /// Add a response mirror rule (hermes `gateway/mirror.py` parity).
    pub fn with_mirror_rule(mut self, rule: MirrorRule) -> Self {
        self.mirror_rules.push(rule);
        self
    }

    /// Register a mirror rule in place.
    pub fn add_mirror_rule(&mut self, rule: MirrorRule) {
        self.mirror_rules.push(rule);
    }

    /// Configure the delivery ledger capacity (default 500).
    pub fn with_delivery_ledger_capacity(mut self, max: usize) -> Self {
        self.delivery_ledger = DeliveryLedger::new(max);
        self
    }

    /// Access the per-session turn lease.
    pub fn turn_lease(&self) -> &TurnLease {
        &self.turn_lease
    }

    /// Access the session stall tracker.
    pub fn stall_tracker(&self) -> &SessionStallTracker {
        &self.stall_tracker
    }

    /// Access the delivery ledger.
    pub fn delivery_ledger(&self) -> &DeliveryLedger {
        &self.delivery_ledger
    }

    /// Registered mirror rules.
    pub fn mirror_rules(&self) -> &[MirrorRule] {
        &self.mirror_rules
    }

    /// Target channels an outbound response on `platform`/`channel` should
    /// be mirrored to (hermes `gateway/mirror.py` parity). The caller sends
    /// the mirrored copies via `send_to_platform`.
    pub fn mirror_targets(&self, platform: &str, channel: &str) -> Vec<String> {
        self.mirror_rules
            .iter()
            .filter(|r| r.matches(platform, channel))
            .map(|r| r.target_channel.clone())
            .collect()
    }

    /// Run one handler turn under the gateway lifecycle: acquire the
    /// per-session turn lease (busy sessions get a polite refusal instead
    /// of a concurrent agent run), track the turn for stall detection, and
    /// record the delivery outcome in the ledger.
    async fn handle_with_lifecycle(
        &self,
        handler: &Arc<dyn MessageHandler>,
        message: IncomingMessage,
        session_key: &str,
    ) -> Result<OutgoingMessage> {
        // Turn lease — one in-flight turn per session key.
        let Some(_guard) = self.turn_lease.try_acquire(session_key).await else {
            debug!(
                session = %session_key,
                "Turn lease busy; polite refusal instead of concurrent agent run"
            );
            return Ok(OutgoingMessage::new(
                &message.channel_id,
                "⏳ Still working on your previous message — I'll reply as soon as it's done.",
            )
            .with_thread_id(message.thread_id));
        };

        self.stall_tracker.touch(session_key).await;
        let result = handler.handle(message.clone()).await;
        self.stall_tracker.complete(session_key).await;

        let response = match result {
            Ok(r) => r,
            Err(e) => {
                self.delivery_ledger
                    .record(&message.platform, &message.channel_id, "", "failed")
                    .await;
                return Err(e);
            }
        };

        self.delivery_ledger
            .record(
                &message.platform,
                &message.channel_id,
                &response.content,
                "delivered",
            )
            .await;
        Ok(response)
    }

    /// Start the gateway and all enabled adapters
    pub async fn start(&self) -> Result<()> {
        *self.running.write().await = true;

        for (name, adapter) in &self.adapters {
            if adapter.is_enabled() {
                info!(platform = %name, "Starting platform adapter");
                if let Err(e) = adapter.start().await {
                    error!(platform = %name, error = %e, "Failed to start adapter");
                }
            }
        }

        Ok(())
    }

    /// Stop the gateway and all adapters
    pub async fn stop(&self) -> Result<()> {
        *self.running.write().await = false;

        for (name, adapter) in &self.adapters {
            info!(platform = %name, "Stopping platform adapter");
            if let Err(e) = adapter.stop().await {
                error!(platform = %name, error = %e, "Failed to stop adapter");
            }
        }

        Ok(())
    }

    /// Start all enabled adapters with a shared message channel.
    /// Each adapter that supports it will spawn a polling/listening
    /// task and forward incoming messages through the sender.
    pub async fn start_with_channel(
        &self,
        message_tx: mpsc::UnboundedSender<IncomingMessage>,
    ) -> Result<()> {
        for (name, adapter) in &self.adapters {
            if adapter.is_enabled() {
                info!(platform = %name, "Starting platform adapter with channel");
                if let Err(e) = adapter.start_with_channel(message_tx.clone()).await {
                    error!(platform = %name, error = %e, "Failed to start adapter with channel");
                }
            }
        }
        Ok(())
    }

    /// Check if the gateway is running
    pub async fn is_running(&self) -> bool {
        *self.running.read().await
    }

    /// Get the number of registered adapters.
    pub fn adapter_count(&self) -> usize {
        self.adapters.len()
    }

    /// Get a registered adapter by platform name (adapters are keyed by
    /// `PlatformAdapter::name()`).
    pub fn adapter_for(&self, platform: &str) -> Option<Arc<dyn PlatformAdapter>> {
        self.adapters.get(platform).cloned()
    }

    // (iter-151: Gateway::status() deleted — duplicate of get_platform_status,
    // both had zero external callers. Use adapter_count() for adapter count.)

    /// Route an incoming message to the handler and send response
    pub async fn route_message(&self, message: IncomingMessage) -> Result<Option<OutgoingMessage>> {
        self.messages_processed.fetch_add(1, Ordering::SeqCst);

        debug!(
            platform = %message.platform,
            user = %message.user_id,
            content = %message.content,
            "Routing message"
        );

        // Track session in persistent store if available
        if let Some(ref store) = self.persistent_sessions {
            let source = SessionSource {
                platform: message.platform.clone(),
                chat_id: message.channel_id.clone(),
                chat_name: None,
                chat_type: if message.is_group_chat { "group" } else { "dm" }.to_string(),
                user_id: Some(message.user_id.clone()),
                user_name: Some(message.username.clone()),
                thread_id: None,
                chat_topic: None,
                user_id_alt: None,
                chat_id_alt: None,
                is_bot: false,
                guild_id: None,
                parent_chat_id: None,
                message_id: None,
                role_authorized: false,
            };
            let _ = store.get_or_create_session(&source, false);
        }

        // Check if user is admin
        if !self.config.admins.is_empty() && !self.config.admins.contains(&message.user_id) {
            debug!(user = %message.user_id, "User not authorized");
            return Ok(Some(
                OutgoingMessage::new(
                    &message.channel_id,
                    "You are not authorized to use this bot.",
                )
                .with_thread_id(message.thread_id),
            ));
        }

        let handler = match &self.message_handler {
            Some(h) => h,
            None => {
                warn!("No message handler configured");
                return Ok(None);
            }
        };

        // Session key for the turn lease / stall tracker / delivery ledger.
        let session_key = format!(
            "{}:{}:{}",
            message.platform, message.user_id, message.channel_id
        );

        // Global emergency stop (hermes estop parity): while engaged, new
        // turns get a brief paused reply instead of an agent run. In-flight
        // work is never killed — this is pause-new-work.
        if crate::estop::is_engaged() {
            let state = crate::estop::state();
            let reason = state
                .reason
                .as_deref()
                .map(|r| format!(" ({r})"))
                .unwrap_or_default();
            info!("Gateway turn rejected: ESTOP engaged{reason}");
            return Ok(Some(
                OutgoingMessage::new(
                    &message.channel_id,
                    format!("⏸️ Operant is paused{reason}. Resume with `operant resume`."),
                )
                .with_thread_id(message.thread_id),
            ));
        }

        // Active-session cap (hermes `max_concurrent_sessions` parity): a
        // new session is refused when the cap is reached; existing holders
        // keep their slots (their locks refresh on every message).
        if let Some(max) = self.config.max_concurrent_sessions {
            let tracker = crate::active_sessions::ActiveSessionTracker::new(
                crate::active_sessions::ActiveSessionTracker::default_dir(),
                Some(max),
            );
            if !tracker.acquire(&session_key)? {
                warn!(
                    session = %session_key,
                    max,
                    "Gateway session refused: concurrency cap reached"
                );
                return Ok(Some(
                    OutgoingMessage::new(
                        &message.channel_id,
                        format!(
                            "Too many concurrent sessions (limit {max}). Please try again later."
                        ),
                    )
                    .with_thread_id(message.thread_id),
                ));
            }
            let result = self
                .handle_with_lifecycle(handler, message, &session_key)
                .await;
            tracker.release(&session_key);
            let response = result?;
            return Ok(Some(response));
        }

        let response = self
            .handle_with_lifecycle(handler, message, &session_key)
            .await?;

        Ok(Some(response))
    }

    /// Send a message to a specific platform
    pub async fn send_to_platform(&self, platform: &str, message: OutgoingMessage) -> Result<()> {
        let adapter = match self.adapters.get(platform) {
            Some(a) => a,
            None => {
                return Err(crate::error::Error::Agent(format!(
                    "Unknown platform: {}",
                    platform
                )));
            }
        };

        adapter.send_message(message).await
    }

    /// Send a message and return the platform message ID.
    pub async fn send_message_return_id(
        &self,
        platform: &str,
        message: OutgoingMessage,
    ) -> Result<String> {
        let adapter = match self.adapters.get(platform) {
            Some(a) => a,
            None => {
                return Err(crate::error::Error::Agent(format!(
                    "Unknown platform: {}",
                    platform
                )));
            }
        };
        adapter.send_message_return_id(message).await
    }

    /// Edit an existing message on a platform.
    pub async fn edit_message(
        &self,
        platform: &str,
        channel_id: &str,
        message_id: &str,
        message: OutgoingMessage,
    ) -> Result<String> {
        let adapter = match self.adapters.get(platform) {
            Some(a) => a,
            None => {
                return Err(crate::error::Error::Agent(format!(
                    "Unknown platform: {}",
                    platform
                )));
            }
        };
        adapter.edit_message(channel_id, message_id, &message).await
    }

    /// Send a voice/audio message to a platform channel.
    pub async fn send_voice(
        &self,
        platform: &str,
        channel_id: &str,
        audio_data: &[u8],
        format: &str,
    ) -> Result<()> {
        let adapter = match self.adapters.get(platform) {
            Some(a) => a,
            None => return Ok(()), // silently skip unsupported platforms
        };
        adapter.send_voice(channel_id, audio_data, format).await
    }

    /// Send a typing indicator to a platform channel
    pub fn send_typing(
        &self,
        platform: &str,
        channel_id: &str,
        thread_id: Option<i64>,
    ) -> Result<()> {
        if let Some(adapter) = self.adapters.get(platform) {
            adapter.send_typing(channel_id, thread_id)?;
        }
        Ok(())
    }

    // ── Extended Gateway API ──

    /// Register a platform adapter using `&mut self` (in-place)
    pub fn register_adapter(&mut self, adapter: Arc<dyn PlatformAdapter>) {
        let name = adapter.name().to_string();
        info!(platform = %name, "Registering platform adapter");
        self.adapters.insert(name, adapter);
    }

    /// Get gateway statistics
    pub async fn get_stats(&self) -> GatewayStats {
        let uptime = self.start_time.elapsed().as_secs();

        GatewayStats {
            uptime_seconds: uptime,
            active_sessions: self
                .persistent_sessions
                .as_ref()
                .map(|s| s.session_count())
                .unwrap_or_else(|| self.session_store.get_session_count()),
            registered_channels: self.channel_directory.list_channels(None).len(),
            active_adapters: self.adapters.len(),
            messages_processed: self.messages_processed.load(Ordering::SeqCst),
            start_time: self.start_time_formatted.clone(),
        }
    }

    // (iter-151: get_platform_status deleted — zero external callers)

    /// Get a reference to the session store
    pub fn get_session_store(&self) -> &SessionStore {
        &self.session_store
    }

    /// Get a reference to the channel directory
    pub fn get_channel_directory(&self) -> &ChannelDirectory {
        &self.channel_directory
    }
}

/// Telegram adapter
pub struct TelegramAdapter {
    token: Option<String>,
    enabled: bool,
    running: Arc<AtomicBool>,
    client: reqwest::Client,
}

impl TelegramAdapter {
    /// Create a new Telegram adapter
    pub fn new(token: Option<String>) -> Self {
        let enabled = token.is_some();
        Self {
            token,
            enabled,
            running: Arc::new(AtomicBool::new(false)),
            client: reqwest::Client::new(),
        }
    }

    /// Create a Telegram adapter with full configuration
    pub fn with_config(
        token: Option<String>,
        _bot_username: Option<String>,
        _dm_topics_enabled: bool,
        proxy_url: Option<&str>,
    ) -> Self {
        let enabled = token.is_some();
        let mut client_builder = reqwest::Client::builder();
        if let Some(proxy) = proxy_url {
            if let Ok(proxy_obj) = reqwest::Proxy::all(proxy) {
                client_builder = client_builder.proxy(proxy_obj);
                tracing::info!("Telegram adapter using proxy: {}", proxy);
            } else {
                tracing::warn!("Invalid proxy URL, ignoring: {}", proxy);
            }
        }
        let client = client_builder
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self {
            token,
            enabled,
            running: Arc::new(AtomicBool::new(false)),
            client,
        }
    }

    fn api_url(&self) -> String {
        let base = runtime_config().gateway.telegram_api_base;
        format!(
            "{}/bot{}",
            base.trim_end_matches('/'),
            self.token.as_ref().unwrap_or(&String::new())
        )
    }

    /// Send a message to a Telegram chat and return the message_id.
    /// Uses HTML parse_mode; falls back to plain text on 400 Bad Request.
    async fn send_telegram_inner(
        &self,
        channel_id: &str,
        text: &str,
        reply_to: Option<&str>,
        thread_id: Option<i64>,
    ) -> Result<String> {
        let html = markdown_to_telegram_html(text);
        let mut body = serde_json::json!({
            "chat_id": channel_id,
            "text": html,
            "parse_mode": "HTML",
        });

        if let Some(reply) = reply_to {
            body["reply_to_message_id"] = serde_json::json!(reply);
        }
        if let Some(tid) = thread_id {
            body["message_thread_id"] = serde_json::json!(tid);
        }

        tracing::debug!(
            "Sending message to chat {} ({} chars)",
            channel_id,
            text.len()
        );
        let response = self
            .client
            .post(format!("{}/sendMessage", self.api_url()))
            .json(&body)
            .send()
            .await?;

        let status = response.status();

        // If HTML parsing fails (400 Bad Request), retry as plain text
        if status.as_u16() == 400 {
            warn!("Telegram HTML parse failed, retrying as plain text");
            tracing::warn!(
                "HTML send failed (400), falling back to plain text for chat {}",
                channel_id
            );
            let mut plain_body = serde_json::json!({
                "chat_id": channel_id,
                "text": text,
            });
            if let Some(reply) = reply_to {
                plain_body["reply_to_message_id"] = serde_json::json!(reply);
            }
            if let Some(tid) = thread_id {
                plain_body["message_thread_id"] = serde_json::json!(tid);
            }
            let resp = self
                .client
                .post(format!("{}/sendMessage", self.api_url()))
                .json(&plain_body)
                .send()
                .await?;
            if !resp.status().is_success() {
                tracing::error!(
                    "Send failed for chat {}: HTTP {}",
                    channel_id,
                    resp.status()
                );
            }
            let data: serde_json::Value = resp.json().await?;
            tracing::info!(
                "Message sent to chat {} via plain text, message_id: {:?}",
                channel_id,
                data["result"]["message_id"].as_i64()
            );
            return Ok(data["result"]["message_id"]
                .as_i64()
                .map(|id| id.to_string())
                .unwrap_or_default());
        }

        tracing::info!("Message sent to chat {}", channel_id);
        let data: serde_json::Value = response.json().await?;
        tracing::info!(
            "Message sent to chat {}, message_id: {:?}",
            channel_id,
            data["result"]["message_id"].as_i64()
        );
        Ok(data["result"]["message_id"]
            .as_i64()
            .map(|id| id.to_string())
            .unwrap_or_default())
    }
}

/// Count UTF-16 code units in a string (Telegram's length metric).
fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// Split text into chunks that respect Telegram's 4096 UTF-16 code unit limit.
///
/// - Measures length using UTF-16 code units, not bytes or chars.
/// - Adds `(X/Y)` suffix indicators when multiple chunks are produced.
/// - Reserves 14 UTF-16 code units for the suffix ` (NNN/NNN)`.
/// - Splits at natural boundaries: prefers `\n\n`, then `\n`, then spaces.
/// - Code-block aware: avoids splitting inside ``` fences; if a split would
///   fall inside a code block, closes the fence and reopens it in the next chunk.
fn chunk_text(text: &str, max_chunk_size: usize) -> Vec<String> {
    const SUFFIX_RESERVE: usize = 14; // room for " (NNN/NNN)"
    const FENCE_CLOSE: &str = "\n```";

    if utf16_len(text) <= max_chunk_size {
        return vec![text.to_string()];
    }

    // First pass: estimate total chunks to know Y in (X/Y).
    // This is a rough estimate — we refine during actual splitting.
    let body_budget = max_chunk_size - SUFFIX_RESERVE;
    let estimated_chunks = utf16_len(text).div_ceil(body_budget);
    let estimated_chunks = estimated_chunks.max(1);

    // Second pass: actual splitting with code-block awareness.
    let mut chunks: Vec<String> = Vec::with_capacity(estimated_chunks);
    let mut remaining = text;
    // When continuing from a code block opened in the previous chunk,
    // holds the language tag so we can reopen the fence.
    let mut carry_lang: Option<String> = None;

    while !remaining.is_empty() {
        let prefix = if let Some(ref lang) = carry_lang {
            format!("```{}\n", lang)
        } else {
            String::new()
        };
        let prefix_utf16 = utf16_len(&prefix);
        let fence_close_utf16 = utf16_len(FENCE_CLOSE);

        // If everything remaining fits in one final chunk
        if prefix_utf16 + utf16_len(remaining) + SUFFIX_RESERVE <= max_chunk_size {
            chunks.push(format!("{}{}", prefix, remaining));
            break;
        }

        // How much body text we can fit after accounting for prefix,
        // a potential closing fence, and the suffix indicator.
        let headroom = max_chunk_size
            .saturating_sub(SUFFIX_RESERVE)
            .saturating_sub(prefix_utf16)
            .saturating_sub(fence_close_utf16);
        let headroom = if headroom < 1 {
            max_chunk_size / 2
        } else {
            headroom
        };

        // Find the largest codepoint prefix of `remaining` whose UTF-16
        // length is ≤ headroom.
        let cp_limit = utf16_char_limit(remaining, headroom);
        let region = &remaining[..cp_limit];

        // Find a natural split point: prefer \n\n, then \n, then space.
        let split_at = find_split_point(region, cp_limit);

        let chunk_body = &remaining[..split_at];
        // Skip leading whitespace on remaining for next iteration
        remaining = remaining[split_at..].trim_start();

        let mut full_chunk = prefix.clone();
        full_chunk.push_str(chunk_body);

        // Determine if we end inside an open code block.
        let (in_code, lang) = scan_code_blocks(chunk_body, carry_lang.as_deref());

        if in_code {
            full_chunk.push_str(FENCE_CLOSE);
            carry_lang = Some(lang);
        } else {
            carry_lang = None;
        }

        chunks.push(full_chunk);
    }

    // Append (X/Y) indicators when multiple chunks.
    if chunks.len() > 1 {
        let total = chunks.len();
        chunks = chunks
            .into_iter()
            .enumerate()
            .map(|(i, chunk)| format!("{} ({}/{})", chunk, i + 1, total))
            .collect();
    }

    chunks
}

/// Find the largest codepoint index such that `s[..index]` has UTF-16 length ≤ limit.
fn utf16_char_limit(s: &str, limit: usize) -> usize {
    let mut count = 0;
    let mut byte_pos = 0;
    for ch in s.chars() {
        let ch_utf16 = ch.len_utf16();
        if count + ch_utf16 > limit {
            break;
        }
        count += ch_utf16;
        byte_pos += ch.len_utf8();
    }
    byte_pos
}

/// Find a natural split point in `region` (a string slice of `remaining`).
/// Prefers double newlines, then single newlines, then spaces.
/// Falls back to `cp_limit` if no natural boundary found.
fn find_split_point(region: &str, cp_limit: usize) -> usize {
    // Prefer \n\n
    if let Some(pos) = region.rfind("\n\n") {
        let split = pos + 2; // include both newlines
        if split > cp_limit / 4 {
            return split;
        }
    }
    // Then \n
    if let Some(pos) = region.rfind('\n') {
        let split = pos + 1; // include the newline
        if split > cp_limit / 4 {
            return split;
        }
    }
    // Then space
    if let Some(pos) = region.rfind(' ')
        && pos > cp_limit / 4
    {
        return pos;
    }
    // Fallback: hard split at the limit
    cp_limit
}

/// Scan `chunk_body` for code block fences, starting from `carry_lang` state.
/// Returns (in_code_block, language_tag) at the end of the body.
fn scan_code_blocks(chunk_body: &str, carry_lang: Option<&str>) -> (bool, String) {
    let mut in_code = carry_lang.is_some();
    let mut lang = carry_lang.unwrap_or("").to_string();

    for line in chunk_body.lines() {
        let stripped = line.trim();
        if let Some(rest) = stripped.strip_prefix("```") {
            if in_code {
                in_code = false;
                lang = String::new();
            } else {
                in_code = true;
                let tag = rest.trim();
                lang = tag.split_whitespace().next().unwrap_or("").to_string();
            }
        }
    }

    (in_code, lang)
}

/// Path used to persist the Telegram polling offset across restarts.
fn get_offset_path() -> PathBuf {
    std::env::current_dir()
        .ok()
        .map(|p| p.join("telegram_offset.txt"))
        .unwrap_or_else(|| PathBuf::from("telegram_offset.txt"))
}

#[async_trait]
impl PlatformAdapter for TelegramAdapter {
    fn name(&self) -> &str {
        "telegram"
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }

    async fn start(&self) -> Result<()> {
        if !self.is_enabled() {
            return Ok(());
        }

        // Verify the token by getting bot info
        let response = self
            .client
            .get(format!("{}/getMe", self.api_url()))
            .send()
            .await?;

        if response.status().is_success() {
            info!("Telegram bot started successfully");
            Ok(())
        } else {
            Err(crate::error::Error::Agent(
                "Failed to verify Telegram bot token".to_string(),
            ))
        }
    }

    async fn stop(&self) -> Result<()> {
        self.running.store(false, Ordering::SeqCst);
        info!("Telegram adapter stopped");
        Ok(())
    }

    async fn send_message(&self, message: OutgoingMessage) -> Result<()> {
        let chunks = chunk_text(&message.content, 4000);
        for (i, chunk) in chunks.iter().enumerate() {
            let reply_to = if i == 0 {
                message.reply_to.as_deref()
            } else {
                None
            };
            self.send_telegram_inner(&message.channel_id, chunk, reply_to, message.thread_id)
                .await?;
        }
        Ok(())
    }

    fn send_typing(&self, channel_id: &str, thread_id: Option<i64>) -> Result<()> {
        let url = format!("{}/sendChatAction", self.api_url());
        let mut body = serde_json::json!({
            "chat_id": channel_id,
            "action": "typing",
        });
        // Route the typing indicator into the same forum topic the user
        // sent from — otherwise it shows in the general chat.
        if let Some(tid) = thread_id {
            body["message_thread_id"] = serde_json::json!(tid);
        }
        let client = self.client.clone();
        tokio::spawn(async move {
            let _ = client.post(&url).json(&body).send().await;
        });
        tracing::debug!("Sent typing indicator to chat {}", channel_id);
        Ok(())
    }

    async fn send_message_return_id(&self, message: OutgoingMessage) -> Result<String> {
        let chunks = chunk_text(&message.content, 4000);
        let id = self
            .send_telegram_inner(
                &message.channel_id,
                &chunks[0],
                message.reply_to.as_deref(),
                message.thread_id,
            )
            .await?;
        for chunk in &chunks[1..] {
            self.send_telegram_inner(&message.channel_id, chunk, None, message.thread_id)
                .await?;
        }
        Ok(id)
    }

    async fn edit_message(
        &self,
        channel_id: &str,
        message_id: &str,
        message: &OutgoingMessage,
    ) -> Result<String> {
        let url = format!("{}/editMessageText", self.api_url());
        let html = markdown_to_telegram_html(&message.content);
        // An explicit empty inline keyboard clears any buttons on the edited
        // message — this is how approval/clarify prompts lose their buttons
        // once resolved (hermes `query.edit_message_text(reply_markup=None)`
        // parity). Harmless for plain stream edits (no keyboard to clear).
        let body = serde_json::json!({
            "chat_id": channel_id,
            "message_id": message_id,
            "text": html,
            "parse_mode": "HTML",
            "reply_markup": { "inline_keyboard": [] },
        });
        let resp = self.client.post(&url).json(&body).send().await?;
        if resp.status().as_u16() == 400 {
            // "message is not modified" — return existing ID.
            return Ok(message_id.to_string());
        }
        if !resp.status().is_success() {
            return Err(crate::error::Error::Agent(format!(
                "Telegram edit_message failed: HTTP {}",
                resp.status()
            )));
        }
        Ok(message_id.to_string())
    }

    fn delete_message(&self, channel_id: &str, message_id: &str) -> Result<()> {
        let url = format!("{}/deleteMessage", self.api_url());
        let body = serde_json::json!({
            "chat_id": channel_id,
            "message_id": message_id,
        });
        let client = self.client.clone();
        tokio::spawn(async move {
            if let Err(e) = client.post(&url).json(&body).send().await {
                tracing::error!("Telegram delete_message error: {}", e);
            }
        });
        Ok(())
    }

    async fn handle_update(&self, update: serde_json::Value) -> Result<Option<IncomingMessage>> {
        // Shared callback-query routing (inline keyboard taps) first, then
        // the regular message parser. The polling loop uses the same static
        // helper so button taps work on every Telegram transport.
        if let Some(msg) = Self::handle_callback_update(
            &self.client,
            self.token.as_deref().unwrap_or_default(),
            &runtime_config().gateway.telegram_api_base,
            update.clone(),
        )
        .await
        {
            return Ok(Some(msg));
        }

        // Parse Telegram update — delegates to the static parse_update
        TelegramAdapter::parse_update(update)
    }

    async fn send_approval_prompt(
        &self,
        channel_id: &str,
        thread_id: Option<i64>,
        tool_name: &str,
        description: &str,
    ) -> Result<()> {
        let prompt = format!(
            "🔧 Permission required: {tool_name} — {description}\nTap a button to allow or cancel (60s timeout), or reply /approve / /deny."
        );
        // Inline keyboard with tappable approve/deny buttons (hermes
        // `send_exec_approval` parity). Callback data uses the `approval:`
        // prefix handled in `handle_callback_update` below.
        let mut body = serde_json::json!({
            "chat_id": channel_id,
            "text": prompt,
            "reply_markup": {
                "inline_keyboard": [[
                    { "text": "✅ Approve", "callback_data": "approval:approve" },
                    { "text": "❌ Deny", "callback_data": "approval:deny" }
                ]]
            }
        });
        // Route the prompt into the same forum topic the discussion is in —
        // without message_thread_id it lands in the general chat instead.
        if let Some(tid) = thread_id {
            body["message_thread_id"] = serde_json::json!(tid);
        }
        self.client
            .post(format!("{}/sendMessage", self.api_url()))
            .json(&body)
            .send()
            .await?;
        Ok(())
    }

    async fn send_choice_prompt(
        &self,
        channel_id: &str,
        thread_id: Option<i64>,
        question: &str,
        choices: &[String],
    ) -> Result<()> {
        // One tappable button per choice (hermes `send_clarify` parity).
        // Index-based callback_data (`choice:<idx>`) stays under Telegram's
        // 64-byte payload limit; the dispatch layer maps the index back to
        // the option text when resolving the pending question.
        let rows: Vec<Vec<serde_json::Value>> = choices
            .iter()
            .enumerate()
            .map(|(i, c)| {
                vec![serde_json::json!({
                    "text": c,
                    "callback_data": format!("choice:{i}"),
                })]
            })
            .collect();
        let mut body = serde_json::json!({
            "chat_id": channel_id,
            "text": format!("❓ {question}"),
            "reply_markup": { "inline_keyboard": rows },
        });
        if let Some(tid) = thread_id {
            body["message_thread_id"] = serde_json::json!(tid);
        }
        self.client
            .post(format!("{}/sendMessage", self.api_url()))
            .json(&body)
            .send()
            .await?;
        Ok(())
    }

    fn config_json(&self) -> serde_json::Value {
        serde_json::json!({
            "platform": "telegram",
            "enabled": self.enabled,
            "has_token": self.token.is_some()
        })
    }

    async fn start_with_channel(
        &self,
        message_tx: mpsc::UnboundedSender<IncomingMessage>,
    ) -> Result<()> {
        if !self.is_enabled() {
            return Ok(());
        }

        self.start().await?;
        tracing::info!("Telegram token verified via getMe");

        let token = self.token.clone().unwrap_or_default();
        let base = runtime_config().gateway.telegram_api_base;
        let url = format!("{}/bot{}/getUpdates", base.trim_end_matches('/'), token);
        let running = self.running.clone();
        running.store(true, Ordering::SeqCst);

        let client = self.client.clone();
        let media_base = base.clone();
        let media_token = token.clone();

        tracing::info!("Telegram polling task spawned");
        tokio::spawn(async move {
            // ── OUTER SUPERVISED RESTART LOOP ──
            // On 409 Conflict the inner loop breaks here, triggering a fresh
            // startup probe (timeout=0) before re-entering the polling loop.
            'restart: while running.load(Ordering::SeqCst) {
                let mut offset: i64 = 0;

                // === STARTUP PROBE: claim any pending updates before long-poll starts ===
                if let Ok(resp) = client
                    .post(&url)
                    .json(&serde_json::json!({
                        "offset": 0,
                        "timeout": 0,
                    }))
                    .send()
                    .await
                    && let Ok(data) = resp.json::<serde_json::Value>().await
                    && let Some(updates) = data["result"].as_array()
                {
                    for update in updates {
                        if let Some(update_id) = update["update_id"].as_i64() {
                            offset = update_id + 1;
                        }
                    }
                }
                tracing::info!("Startup probe completed, initial offset: {}", offset);

                // === LOAD SAVED OFFSET (persist across restarts) ===
                let offset_path = get_offset_path();
                if offset_path.exists()
                    && let Ok(saved) = tokio::fs::read_to_string(&offset_path).await
                    && let Ok(n) = saved.trim().parse::<i64>()
                    && n > offset
                {
                    offset = n;
                }
                tracing::info!("Loaded saved offset: {}", offset);

                // ── INNER POLLING LOOP ──
                let mut retry_delay: u64 = 1;
                let mut last_heartbeat = Instant::now();

                // Defensive dedup window: remember recently processed
                // update_ids so a duplicate delivery (offset-file race,
                // restart edge case) can never double-process the same
                // update and double-reply. Telegram guarantees no dupes
                // when offset handling is correct — this is belt-and-braces.
                const RECENT_UPDATE_WINDOW: usize = 256;
                let mut recent_updates: std::collections::VecDeque<i64> =
                    std::collections::VecDeque::new();

                tracing::info!("Entering main polling loop");
                while running.load(Ordering::SeqCst) {
                    // Early exit if the gateway receiver has been dropped (clean shutdown).
                    if message_tx.is_closed() {
                        info!("Telegram: message channel closed, stopping poll loop");
                        running.store(false, Ordering::SeqCst);
                        break;
                    }

                    // Heartbeat: log every 60s without receiving updates
                    if last_heartbeat.elapsed() >= Duration::from_secs(60) {
                        info!("Polling active, last update offset: {}", offset);
                        last_heartbeat = Instant::now();
                    }

                    let response = client
                        .post(&url)
                        .json(&serde_json::json!({
                            "offset": offset,
                            "timeout": 30,
                        }))
                        .send()
                        .await;

                    let mut had_updates = false;

                    match response {
                        Ok(resp) => {
                            let status = resp.status();

                            // Handle 409 Conflict — break to outer loop for a clean re-probe
                            if status.as_u16() == 409 {
                                tracing::warn!(
                                    "Telegram 409 Conflict (another instance?), restarting from probe in 35s"
                                );
                                tokio::time::sleep(Duration::from_secs(35)).await;
                                break;
                            }

                            // Any other successful HTTP response resets the retry delay.
                            retry_delay = 1;

                            if let Ok(data) = resp.json::<serde_json::Value>().await
                                && let Some(updates) = data["result"].as_array()
                            {
                                had_updates = !updates.is_empty();
                                if had_updates {
                                    tracing::info!(
                                        "Received {} update(s) from Telegram",
                                        updates.len()
                                    );
                                    last_heartbeat = Instant::now();
                                }
                                for update in updates {
                                    let Some(update_id) = update["update_id"].as_i64() else {
                                        continue;
                                    };
                                    // Skip anything already processed (defense
                                    // against duplicate delivery).
                                    if recent_updates.contains(&update_id) {
                                        tracing::warn!(
                                            update_id,
                                            "Skipping duplicate Telegram update (already processed)"
                                        );
                                        continue;
                                    }
                                    offset = update_id + 1;
                                    // Route inline-keyboard taps (approval /
                                    // clarify buttons) before the regular
                                    // message parser — parse_update drops
                                    // callback_query updates entirely.
                                    let parsed = if let Some(m) =
                                        TelegramAdapter::handle_callback_update(
                                            &client,
                                            &media_token,
                                            &media_base,
                                            update.clone(),
                                        )
                                        .await
                                    {
                                        Some(m)
                                    } else {
                                        TelegramAdapter::parse_update(update.clone()).ok().flatten()
                                    };
                                    if let Some(mut msg) = parsed {
                                        // Download attachments (photos, documents,
                                        // voice/audio/video) into the local media
                                        // cache so the agent can inspect them with
                                        // native tools (hermes parity). Best-effort:
                                        // a failed download keeps the placeholder
                                        // text and still delivers the message.
                                        let media = download_telegram_attachments(
                                            &client,
                                            &media_token,
                                            &media_base,
                                            update,
                                        )
                                        .await;
                                        if !media.is_empty() {
                                            msg = msg.with_media_urls(media);
                                        }
                                        tracing::info!(
                                            "Sent message to gateway handler (chat: {}, content: {:.50}, media: {})",
                                            msg.channel_id,
                                            msg.content,
                                            msg.media_urls.len()
                                        );
                                        if let Err(e) = message_tx.send(msg) {
                                            tracing::error!(
                                                "Failed to send message to gateway handler: {}",
                                                e
                                            );
                                            // Receiver dropped — likely shutting down.
                                            running.store(false, Ordering::SeqCst);
                                            break;
                                        }
                                    }
                                    recent_updates.push_back(update_id);
                                    if recent_updates.len() > RECENT_UPDATE_WINDOW {
                                        recent_updates.pop_front();
                                    }
                                }
                            }

                            // Persist offset to disk when updates were received
                            if had_updates {
                                let _ = std::fs::write(&offset_path, offset.to_string());
                            }
                        }
                        Err(e) => {
                            tracing::error!(
                                "Telegram polling error (retrying in {}s): {}",
                                retry_delay,
                                e
                            );
                            tokio::time::sleep(Duration::from_secs(retry_delay)).await;
                            retry_delay = (retry_delay * 2).min(30);
                            continue; // Stay in inner loop, skip the 2s pause below
                        }
                    }

                    // Only sleep when no updates arrived (long-poll timed out)
                    // so we don't add latency between receiving updates and polling again.
                    if !had_updates {
                        tokio::time::sleep(Duration::from_secs(2)).await;
                    }
                }

                // Before re-probing, check if we should shut down cleanly.
                if !running.load(Ordering::SeqCst) || message_tx.is_closed() {
                    break 'restart;
                }
            }
        });

        info!("Telegram bot started with polling");
        Ok(())
    }

    async fn send_message_to_channel(
        &self,
        channel_id: &str,
        message: &OutgoingMessage,
    ) -> Result<String> {
        let chunks = chunk_text(&message.content, 4000);
        let first_id = self
            .send_telegram_inner(
                channel_id,
                &chunks[0],
                message.reply_to.as_deref(),
                message.thread_id,
            )
            .await?;
        for chunk in &chunks[1..] {
            self.send_telegram_inner(channel_id, chunk, None, message.thread_id)
                .await?;
        }
        Ok(first_id)
    }

    async fn send_voice(&self, channel_id: &str, audio_data: &[u8], format: &str) -> Result<()> {
        let url = format!(
            "https://api.telegram.org/bot{}/sendVoice",
            self.token.as_deref().unwrap_or("")
        );
        let filename = format!("voice.{}", format);
        let mime = match format {
            "ogg" | "opus" => "audio/ogg",
            "mp3" => "audio/mpeg",
            _ => "audio/wav",
        };
        let part = reqwest::multipart::Part::bytes(audio_data.to_vec())
            .file_name(filename)
            .mime_str(mime)
            .map_err(|e| crate::error::Error::Agent(e.to_string()))?;
        let form = reqwest::multipart::Form::new()
            .text("chat_id", channel_id.to_string())
            .part("voice", part);
        let resp = self.client.post(&url).multipart(form).send().await?;
        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(crate::error::Error::Agent(format!(
                "sendVoice failed: {}",
                body
            )));
        }
        Ok(())
    }
}

impl TelegramAdapter {
    /// Process a Telegram `callback_query` update (inline keyboard tap) and
    /// synthesize the equivalent inbound message, if any.
    ///
    /// Two tap families are supported, mirroring hermes' button contract:
    ///
    /// 1. `approval:<action>` — tool-permission Approve/Deny buttons. The tap
    ///    is answered (spinner dismissed) and synthesized into a `/approve`
    ///    or `/deny` text command that flows through the shared command
    ///    resolver — identical to a typed reply. The prompt message's
    ///    chat/message ids ride in `IncomingMessage.raw["approval_callback"]`
    ///    so the dispatch layer can edit the prompt to show the outcome
    ///    (hermes `resolve_gateway_approval` + `query.edit_message_text`
    ///    parity).
    /// 2. `choice:<idx>` — clarify() multiple-choice buttons. Synthesized
    ///    with a `choice_callback` raw marker; the dispatch layer resolves
    ///    the pending user question with the selected option text and edits
    ///    the prompt message.
    ///
    /// Returns `None` for non-callback updates (delegate to `parse_update`).
    /// Shared by `handle_update` and the Telegram polling loop so button
    /// taps work on every transport.
    async fn handle_callback_update(
        client: &reqwest::Client,
        token: &str,
        base: &str,
        update: serde_json::Value,
    ) -> Option<IncomingMessage> {
        let cb = update.get("callback_query")?;
        let data = cb.get("data").and_then(serde_json::Value::as_str)?;
        let cb_id = cb
            .get("id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let api = format!("{}/bot{}", base.trim_end_matches('/'), token);

        // ── Tool-permission approve/deny taps ────────────────────────────
        if let Some(action) = data.strip_prefix("approval:") {
            let label = match action {
                "approve" => "✅ Approved",
                "deny" => "❌ Denied",
                _ => "⚠️ Unknown action",
            };
            // Answer the callback so the inline keyboard stops its spinner
            // (best-effort; a failure must not drop the tap).
            let _ = client
                .post(format!("{api}/answerCallbackQuery"))
                .json(&serde_json::json!({ "callback_query_id": cb_id, "text": label }))
                .send()
                .await;
            // Synthesize the equivalent text command so the shared
            // gateway_commands resolver handles the tap identically to a
            // typed reply.
            return Self::approval_message_from_callback(cb);
        }

        // ── clarify() multiple-choice taps ───────────────────────────────
        if data.starts_with("choice:") {
            let _ = client
                .post(format!("{api}/answerCallbackQuery"))
                .json(&serde_json::json!({
                    "callback_query_id": cb_id,
                    "text": "✅ Selected",
                }))
                .send()
                .await;
            return Self::choice_message_from_callback(cb);
        }

        None
    }

    /// Synthesize the `/approve` / `/deny` text command that an inline-keyboard
    /// tap resolves to, so taps and text replies share the gateway_commands
    /// resolution path (hermes `send_exec_approval` parity). Pure — no I/O —
    /// so it is unit-testable without a live client. Carries the thread_id
    /// and an `approval_callback` raw marker (prompt chat/message ids, tap
    /// user and action) so the dispatch layer can edit the prompt message to
    /// reflect the outcome.
    fn approval_message_from_callback(cb: &serde_json::Value) -> Option<IncomingMessage> {
        let action = cb
            .get("data")
            .and_then(serde_json::Value::as_str)
            .and_then(|d| d.strip_prefix("approval:"))?;
        let message = cb.get("message")?;
        let chat = message.get("chat")?;
        let channel_id = chat
            .get("id")
            .and_then(serde_json::Value::as_i64)
            .map(|i| i.to_string())?;
        let thread_id = message.get("message_thread_id").and_then(|t| t.as_i64());
        let is_group = matches!(
            chat.get("type").and_then(|t| t.as_str()),
            Some("group" | "supergroup")
        );
        let user_name = cb
            .get("from")
            .and_then(|f| f.get("first_name"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("User")
            .to_string();
        Some(
            IncomingMessage::new(
                "telegram",
                cb.get("from")
                    .and_then(|f| f.get("id"))
                    .and_then(serde_json::Value::as_i64)
                    .map(|i| i.to_string())
                    .unwrap_or_default(),
                cb.get("from")
                    .and_then(|f| f.get("username"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("unknown")
                    .to_string(),
                channel_id.clone(),
                match action {
                    "approve" => "/approve".to_string(),
                    "deny" => "/deny".to_string(),
                    other => format!("/approve-unknown-{other}"),
                },
            )
            .with_thread_id(thread_id)
            .with_group_chat(is_group)
            .with_raw(serde_json::json!({
                "approval_callback": {
                    "chat_id": channel_id,
                    "message_id": message.get("message_id").and_then(serde_json::Value::as_i64),
                    "thread_id": thread_id,
                    "user": user_name,
                    "action": action,
                }
            })),
        )
    }

    /// Synthesize the inbound message a clarify() button tap produces. Pure —
    /// no I/O — so it is unit-testable. The message carries a `choice_callback`
    /// raw marker (prompt chat/message ids, tap user and selected index) so
    /// the dispatch layer can resolve the pending question with the option
    /// text and edit the prompt message.
    fn choice_message_from_callback(cb: &serde_json::Value) -> Option<IncomingMessage> {
        let idx_str = cb
            .get("data")
            .and_then(serde_json::Value::as_str)
            .and_then(|d| d.strip_prefix("choice:"))?;
        let message = cb.get("message")?;
        let chat = message.get("chat")?;
        let channel_id = chat
            .get("id")
            .and_then(serde_json::Value::as_i64)
            .map(|i| i.to_string())?;
        let thread_id = message.get("message_thread_id").and_then(|t| t.as_i64());
        let is_group = matches!(
            chat.get("type").and_then(|t| t.as_str()),
            Some("group" | "supergroup")
        );
        let user_name = cb
            .get("from")
            .and_then(|f| f.get("first_name"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("User")
            .to_string();
        let idx = idx_str.parse::<usize>().unwrap_or(0);
        Some(
            IncomingMessage::new(
                "telegram",
                cb.get("from")
                    .and_then(|f| f.get("id"))
                    .and_then(serde_json::Value::as_i64)
                    .map(|i| i.to_string())
                    .unwrap_or_default(),
                user_name.clone(),
                channel_id.clone(),
                format!("__choice__{idx}"),
            )
            .with_thread_id(thread_id)
            .with_group_chat(is_group)
            .with_raw(serde_json::json!({
                "choice_callback": {
                    "chat_id": channel_id,
                    "message_id": message.get("message_id").and_then(serde_json::Value::as_i64),
                    "thread_id": thread_id,
                    "user": user_name,
                    "idx": idx,
                }
            })),
        )
    }
}

/// Telegram bot API file path for a message attachment, if any.
///
/// Returns the `file_id` for the largest photo size, or the document/voice/
/// video/audio/sticker file — mirroring hermes' `_media_message_type`
/// extraction. `None` for plain text messages.
fn telegram_attachment_file_id(message: &serde_json::Value) -> Option<String> {
    if let Some(photos) = message.get("photo").and_then(|p| p.as_array()) {
        // PhotoSize list is sorted by size ascending — take the largest.
        return photos
            .last()
            .and_then(|p| p.get("file_id"))
            .and_then(|f| f.as_str())
            .map(|s| s.to_string());
    }
    for key in [
        "document",
        "voice",
        "video",
        "audio",
        "video_note",
        "sticker",
    ] {
        if let Some(fid) = message
            .get(key)
            .and_then(|v| v.get("file_id"))
            .and_then(|f| f.as_str())
        {
            return Some(fid.to_string());
        }
    }
    None
}

/// Guess a file extension for a Telegram file path / mime-ish hint.
fn telegram_media_extension(file_path: &str, fallback: &str) -> String {
    let lower = file_path.to_lowercase();
    let known = [
        ".jpg", ".jpeg", ".png", ".webp", ".gif", ".mp4", ".ogg", ".oga", ".mp3", ".m4a", ".wav",
        ".opus", ".pdf", ".txt", ".md", ".docx", ".xlsx", ".csv", ".json", ".zip", ".tar", ".gz",
        ".py", ".rs", ".toml", ".yaml", ".yml", ".html",
    ];
    for ext in known {
        if lower.ends_with(ext) {
            return ext.to_string();
        }
    }
    fallback.to_string()
}

/// Download a Telegram attachment (photo/document/voice/video) to the local
/// media cache (`~/.operant/media/`) so the agent can inspect it with native
/// tools. Hermes `cache_image_from_bytes`/`cache_audio_from_bytes` parity —
/// Telegram file URLs are ephemeral (~1h), so the gateway must persist them.
///
/// Returns the local file path on success. Failures log a warning and return
/// None — an attachment that can't be cached degrades to the placeholder text
/// rather than dropping the message.
async fn download_telegram_attachment(
    client: &reqwest::Client,
    token: &str,
    base: &str,
    file_id: &str,
    file_name_hint: Option<&str>,
) -> Option<String> {
    // 1. Resolve file path via getFile
    let api_base = base.trim_end_matches('/');
    let get_file_url = format!("{api_base}/bot{token}/getFile");
    let resp = client
        .post(&get_file_url)
        .json(&serde_json::json!({ "file_id": file_id }))
        .send()
        .await
        .ok()?;
    let data: serde_json::Value = resp.json().await.ok()?;
    let file_path = data["result"]["file_path"].as_str()?;

    // 2. Download the bytes
    let download_url = format!("{api_base}/file/bot{token}/{file_path}");
    let bytes = client
        .get(&download_url)
        .send()
        .await
        .ok()?
        .bytes()
        .await
        .ok()?;
    if bytes.is_empty() {
        return None;
    }

    // 3. Persist to ~/.operant/media/ with a stable, descriptive name
    let media_dir = crate::platform::operant_home().join("media");
    if std::fs::create_dir_all(&media_dir).is_err() {
        return None;
    }
    let ext = telegram_media_extension(
        file_path,
        file_name_hint
            .and_then(|f| std::path::Path::new(f).extension().and_then(|e| e.to_str()))
            .map(|e| format!(".{}", e.to_lowercase()))
            .unwrap_or_else(|| ".bin".to_string())
            .as_str(),
    );
    let file_name = file_name_hint
        .and_then(|f| {
            let stem = std::path::Path::new(f)
                .file_stem()
                .and_then(|s| s.to_str())?
                .to_string();
            Some(stem)
        })
        .unwrap_or_else(|| "attachment".to_string());
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    // Sanitize the stem so it can't escape the media dir.
    let safe_stem: String = file_name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let dest = media_dir.join(format!("{safe_stem}_{ts}{ext}"));
    if std::fs::write(&dest, &bytes).is_err() {
        return None;
    }
    tracing::info!(path = %dest.display(), file_id = %file_id, "Cached Telegram attachment");
    Some(dest.to_string_lossy().to_string())
}

/// Download all attachments on a Telegram update message into the local media
/// cache. Returns the list of cached local paths (empty for text-only
/// messages).
async fn download_telegram_attachments(
    client: &reqwest::Client,
    token: &str,
    base: &str,
    update: &serde_json::Value,
) -> Vec<String> {
    let message = match update.get("message") {
        Some(m) => m,
        None => return Vec::new(),
    };
    let file_id = match telegram_attachment_file_id(message) {
        Some(fid) => fid,
        None => return Vec::new(),
    };
    let file_name_hint = message
        .get("document")
        .and_then(|d| d.get("file_name"))
        .and_then(|f| f.as_str());
    match download_telegram_attachment(client, token, base, &file_id, file_name_hint).await {
        Some(path) => vec![path],
        None => {
            tracing::warn!(
                file_id = %file_id,
                "Failed to cache Telegram attachment — degraded to placeholder text"
            );
            Vec::new()
        }
    }
}

/// Telegram update message keys that indicate a *service* message — no user
/// content: topic lifecycle, membership changes, pins, migrations, payments,
/// etc. These must be filtered in `parse_update` before the attachment
/// fallbacks, otherwise they surface to the agent as "[sent an attachment]"
/// and spawn a bogus agent turn. That was the double-reply bug when a user
/// creates a forum topic and types a message: the `forum_topic_created`
/// service message got its own turn (→ "I received another attachment")
/// *in addition to* the real text turn.
const TELEGRAM_SERVICE_MESSAGE_KEYS: &[&str] = &[
    "forum_topic_created",
    "forum_topic_closed",
    "forum_topic_reopened",
    "forum_topic_edited",
    "general_forum_topic_hidden",
    "general_forum_topic_unhidden",
    "new_chat_members",
    "left_chat_member",
    "new_chat_title",
    "new_chat_photo",
    "delete_chat_photo",
    "group_chat_created",
    "supergroup_chat_created",
    "channel_chat_created",
    "message_auto_delete_timer_changed",
    "migrate_to_chat_id",
    "migrate_from_chat_id",
    "pinned_message",
    "chat_background_set",
    "video_chat_started",
    "video_chat_ended",
    "video_chat_scheduled",
    "video_chat_participants_invited",
    "proximity_alert_triggered",
    "boost_added",
    "user_shared",
    "chat_shared",
    "write_access_allowed",
    "connected_website",
    "passport_data",
    "successful_payment",
    "refunded_payment",
    "invoice",
    "giveaway_created",
    "giveaway_completed",
    "giveaway_winners",
];

/// True when a Telegram update message is a service message (topic created,
/// member joined/left, pinned, migrated, …) carrying no user content.
fn telegram_message_is_service(message: &serde_json::Value) -> bool {
    TELEGRAM_SERVICE_MESSAGE_KEYS
        .iter()
        .any(|k| message.get(*k).is_some())
}

impl TelegramAdapter {
    /// Parse a Telegram update into an IncomingMessage.
    /// This is the same logic used by handle_update but callable without a trait object.
    fn parse_update(update: serde_json::Value) -> Result<Option<IncomingMessage>> {
        let message = match update.get("message") {
            Some(m) => m,
            None => return Ok(None),
        };

        // Filter out messages from bots
        if let Some(from) = message.get("from")
            && from
                .get("is_bot")
                .and_then(|b| b.as_bool())
                .unwrap_or(false)
        {
            return Ok(None);
        }

        // Service messages (forum topic created, member joined/left, pinned,
        // migrated, …) carry no user content — skip them entirely instead of
        // fabricating an "[sent an attachment]" placeholder that spawns a
        // bogus agent turn.
        if telegram_message_is_service(message) {
            return Ok(None);
        }

        let chat = match message.get("chat") {
            Some(c) => c,
            None => return Ok(None),
        };

        let from = message.get("from");

        let content = if let Some(text) = message.get("text").and_then(|t| t.as_str()) {
            text.to_string()
        } else if message.get("photo").is_some() {
            message
                .get("caption")
                .and_then(|c| c.as_str())
                .unwrap_or("[sent a photo]")
                .to_string()
        } else if let Some(doc) = message.get("document") {
            let filename = doc
                .get("file_name")
                .and_then(|f| f.as_str())
                .unwrap_or("unknown");
            message
                .get("caption")
                .and_then(|c| c.as_str())
                .map(|c| c.to_string())
                .unwrap_or_else(|| format!("[sent a document: {}]", filename))
        } else if message.get("voice").is_some() {
            message
                .get("caption")
                .and_then(|c| c.as_str())
                .unwrap_or("[sent a voice message]")
                .to_string()
        } else if message.get("video").is_some() {
            message
                .get("caption")
                .and_then(|c| c.as_str())
                .unwrap_or("[sent a video]")
                .to_string()
        } else if message.get("video_note").is_some() {
            "[sent a video note]".to_string()
        } else if message.get("animation").is_some() {
            message
                .get("caption")
                .and_then(|c| c.as_str())
                .unwrap_or("[sent an animation]")
                .to_string()
        } else if message.get("audio").is_some() {
            message
                .get("caption")
                .and_then(|c| c.as_str())
                .unwrap_or("[sent an audio message]")
                .to_string()
        } else if message.get("sticker").is_some() {
            "[sent a sticker]".to_string()
        } else {
            // No text, no caption, and no recognized media — not a user
            // message (unknown service updates fall here). Skip rather than
            // fabricate an attachment the agent can't act on.
            return Ok(None);
        };

        let thread_id = message.get("message_thread_id").and_then(|t| t.as_i64());

        if content.is_empty() {
            return Ok(None);
        }

        Ok(Some(
            IncomingMessage::new(
                "telegram",
                from.and_then(|f| f.get("id"))
                    .and_then(|id| id.as_i64())
                    .map(|i| i.to_string())
                    .unwrap_or_default(),
                from.and_then(|f| f.get("username"))
                    .and_then(|u| u.as_str())
                    .unwrap_or("unknown"),
                chat.get("id")
                    .and_then(|id| id.as_i64())
                    .map(|i| i.to_string())
                    .unwrap_or_default(),
                content,
            )
            .with_group_chat(matches!(
                chat.get("type").and_then(|t| t.as_str()),
                Some("group" | "supergroup")
            ))
            .with_raw(update)
            .with_thread_id(thread_id),
        ))
    }
}

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

    fn api_url(&self) -> String {
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
async fn connect_discord_gateway(
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
fn parse_discord_message(json: &serde_json::Value, _api_url: &str) -> Option<IncomingMessage> {
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

/// Slack adapter
pub struct SlackAdapter {
    token: Option<String>,
    enabled: bool,
    /// Signing secret for verifying Slack request signatures (used in webhook mode)
    _signing_secret: Option<String>,
}

impl SlackAdapter {
    /// Create a new Slack adapter
    pub fn new(token: Option<String>, signing_secret: Option<String>) -> Self {
        let enabled = token.is_some();
        Self {
            token,
            enabled,
            _signing_secret: signing_secret,
        }
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
            "has_token": self.token.is_some()
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

/// Webhook adapter — HTTP server that receives webhook POSTs and forwards
/// them as IncomingMessages. Supports HMAC signature validation and
/// route-based webhook handling.
///
/// Routes:
///   POST /webhook/{route}  — receives a JSON payload, validates HMAC
///   signature (if configured), and forwards as an IncomingMessage.
///   GET  /health           — health check endpoint.
pub struct WebhookAdapter {
    enabled: bool,
    listen_addr: String,
    secret: Option<String>,
}

impl WebhookAdapter {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            listen_addr: "0.0.0.0:8080".to_string(),
            secret: None,
        }
    }

    pub fn with_addr(mut self, addr: String) -> Self {
        self.listen_addr = addr;
        self
    }

    pub fn with_secret(mut self, secret: Option<String>) -> Self {
        self.secret = secret;
        self
    }
}

#[async_trait]
impl PlatformAdapter for WebhookAdapter {
    fn name(&self) -> &str {
        "webhook"
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }

    async fn start(&self) -> Result<()> {
        info!(addr = %self.listen_addr, "Webhook adapter starting");
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        info!("Webhook adapter stopped");
        Ok(())
    }

    async fn send_message(&self, _message: OutgoingMessage) -> Result<()> {
        // Webhook adapter is inbound-only; sending messages back is done
        // via the deliver mechanism in the gateway runner, not here.
        Ok(())
    }

    async fn handle_update(&self, _update: serde_json::Value) -> Result<Option<IncomingMessage>> {
        Ok(None)
    }

    fn config_json(&self) -> serde_json::Value {
        serde_json::json!({
            "platform": "webhook",
            "enabled": self.enabled,
            "listen_addr": self.listen_addr,
            "hmac_secret_configured": self.secret.is_some(),
        })
    }

    #[expect(
        clippy::expect_used,
        reason = "invariant guaranteed by surrounding validation"
    )]
    async fn start_with_channel(
        &self,
        message_tx: mpsc::UnboundedSender<IncomingMessage>,
    ) -> Result<()> {
        use axum::Router;
        use axum::extract::{Path, State};
        use axum::http::HeaderMap;
        use axum::response::IntoResponse;
        use axum::routing::get;

        let addr: std::net::SocketAddr = self
            .listen_addr
            .parse()
            .map_err(|e| Error::Config(format!("Invalid webhook listen addr: {e}")))?;

        let secret = self.secret.clone();
        let tx = message_tx.clone();

        // Build the axum router
        let app = Router::new()
            .route("/health", get(|| async { "ok" }))
            .route(
                "/webhook/{route}",
                // GET handler: WhatsApp/Meta webhook verification handshake.
                // Meta sends `GET /webhook/{route}?hub.mode=subscribe&hub.verify_token=<token>&hub.challenge=<int>`
                // when you first register the webhook URL in the Meta app dashboard.
                // We respond with the challenge value iff the verify_token matches
                // our secret. (iter-131 — closes the ponytail-audit gap "WhatsApp
                // webhook handshake (hub.mode=subscribe) not implemented → no
                // inbound from Meta".)
                get(
                    move |Path(_route): Path<String>,
                          State((_, secret)): State<(
                        mpsc::UnboundedSender<IncomingMessage>,
                        Option<String>,
                    )>,
                          axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>| async move {
                        let mode = params.get("hub.mode").map(|s| s.as_str()).unwrap_or("");
                        let verify_token = params.get("hub.verify_token").map(|s| s.as_str()).unwrap_or("");
                        let challenge = params.get("hub.challenge").cloned().unwrap_or_default();

                        if mode == "subscribe" {
                            // Verify the token matches our secret (if one is configured).
                            let token_ok = match &secret {
                                Some(expected) => verify_token == expected.as_str(),
                                None => true,
                            };
                            if token_ok {
                                debug!("WhatsApp/Meta webhook verification: challenge accepted");
                                return (axum::http::StatusCode::OK, challenge).into_response();
                            } else {
                                warn!("WhatsApp/Meta webhook verification: verify_token mismatch");
                                return (axum::http::StatusCode::FORBIDDEN, "verify_token mismatch").into_response();
                            }
                        }
                        (axum::http::StatusCode::BAD_REQUEST, "Expected hub.mode=subscribe").into_response()
                    },
                )
                .post(
                    move |Path(route): Path<String>,
                          headers: HeaderMap,
                          State((tx, secret)): State<(
                        mpsc::UnboundedSender<IncomingMessage>,
                        Option<String>,
                    )>,
                          body: axum::body::Bytes| async move {
                        // ────────────────────────────────────────────────────────
                        // URL Verification Handshakes (iter-131)
                        // ────────────────────────────────────────────────────────
                        // Several platforms require a one-time handshake when you
                        // first register the webhook URL with them:
                        //
                        //   • Slack Events API — POSTs `{"type":"url_verification",
                        //     "challenge":"<token>"}` and expects the same token
                        //     back in the response body.
                        //
                        //   • WhatsApp Cloud API (Meta) — GETs the webhook with
                        //     `hub.mode=subscribe` + `hub.verify_token=<token>` +
                        //     `hub.challenge=<int>`. Expects the challenge value
                        //     back in the response body.
                        //
                        //   • Meta Webhooks (Instagram/Messenger) — same as
                        //     WhatsApp (the Meta Webhooks product is shared).
                        //
                        // These handshakes have NO HMAC signature (they happen
                        // before the platform starts signing events), so they
                        // must be handled BEFORE the signature check below.
                        // ────────────────────────────────────────────────────────

                        // Try parsing the body as JSON first (Slack url_verification
                        // is JSON; WhatsApp handshake is GET with query params).
                        if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&body) {
                            // Slack url_verification
                            if v.get("type").and_then(|t| t.as_str()) == Some("url_verification")
                                && let Some(challenge) = v.get("challenge").and_then(|c| c.as_str()) {
                                    debug!("Slack url_verification challenge — responding with challenge token");
                                    return (
                                        axum::http::StatusCode::OK,
                                        [(axum::http::header::CONTENT_TYPE, "text/plain; charset=utf-8")],
                                        challenge.to_string(),
                                    ).into_response();
                                }
                        }

                        // Validate HMAC signature if secret is configured.
                        // Uses standard HMAC-SHA256(secret, body) — not the
                        // old non-standard SHA256(route+secret). Supports
                        // multiple signature header names for interop:
                        // x-webhook-signature (custom), x-hub-signature-256
                        // (GitHub), Stripe-Signature (Stripe).
                        // (iter-101 — closes Bug #9 from iter-98 audit.)
                        if let Some(ref sec) = secret {
                            // Slack-specific signature verification: Slack uses
                            // `X-Slack-Signature` (HMAC-SHA256 hex of
                            // "v0:<X-Slack-Request-Timestamp>:<body>") +
                            // `X-Slack-Request-Timestamp`. We special-case
                            // this because Slack's format is non-standard.
                            // (iter-125 — closes the ponytail-audit security
                            // bug "Slack signing_secret collected but HMAC
                            // verification never performed".)
                            let slack_sig = headers
                                .get("x-slack-signature")
                                .and_then(|v| v.to_str().ok());
                            let slack_ts = headers
                                .get("x-slack-request-timestamp")
                                .and_then(|v| v.to_str().ok());

                            if let (Some(sig), Some(ts)) = (slack_sig, slack_ts) {
                                // Replay protection: reject requests older
                                // than 5 minutes (Slack's own recommendation).
                                if let Ok(ts_secs) = ts.parse::<i64>() {
                                    let now_secs = std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .map(|d| d.as_secs() as i64)
                                        .unwrap_or(0);
                                    if (now_secs - ts_secs).abs() > 300 {
                                        return (
                                            axum::http::StatusCode::UNAUTHORIZED,
                                            "Stale Slack request (replay-protected)",
                                        )
                                            .into_response();
                                    }
                                }
                                // Compute HMAC-SHA256(signing_secret, "v0:<ts>:<body>").
                                use hmac::{Hmac, Mac};
                                use sha2::Sha256;
                                type HmacSha256 = Hmac<Sha256>;
                                let mut mac = match HmacSha256::new_from_slice(sec.as_bytes()) {
                                    Ok(m) => m,
                                    Err(_) => {
                                        return (
                                            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                                            "HMAC key error",
                                        )
                                            .into_response();
                                    }
                                };
                                let basestring = format!("v0:{}:", ts);
                                mac.update(basestring.as_bytes());
                                mac.update(&body);
                                let expected = mac.finalize().into_bytes();
                                let expected_hex = format!("v0={}", hex::encode(expected));
                                if !constant_time_eq::constant_time_eq(
                                    sig.as_bytes(),
                                    expected_hex.as_bytes(),
                                ) {
                                    return (
                                        axum::http::StatusCode::UNAUTHORIZED,
                                        "Invalid Slack signature",
                                    )
                                        .into_response();
                                }
                            } else {
                                // Fall back to the standard HMAC verification
                                // used by GitHub / Stripe / generic webhooks.
                                let sig = headers
                                    .get("x-webhook-signature")
                                    .or_else(|| headers.get("x-hub-signature-256"))
                                    .or_else(|| headers.get("stripe-signature"))
                                    .and_then(|v| v.to_str().ok());

                                if let Some(sig) = sig {
                                    // Strip "sha256=" prefix if present (GitHub/Stripe format).
                                    let sig_hex = sig.strip_prefix("sha256=").unwrap_or(sig);
                                    // Compute HMAC-SHA256(secret, body).
                                    use hmac::{Hmac, Mac};
                                    use sha2::Sha256;
                                    type HmacSha256 = Hmac<Sha256>;
                                    let mut mac = match HmacSha256::new_from_slice(sec.as_bytes()) {
                                        Ok(m) => m,
                                        Err(_) => {
                                            return (
                                                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                                                "HMAC key error",
                                            )
                                                .into_response();
                                        }
                                    };
                                    mac.update(&body);
                                    let expected = mac.finalize().into_bytes();
                                    let expected_hex = hex::encode(expected);
                                    // Constant-time comparison via the
                                    // constant_time_eq crate.
                                    if !constant_time_eq::constant_time_eq(sig_hex.as_bytes(), expected_hex.as_bytes()) {
                                        return (
                                            axum::http::StatusCode::UNAUTHORIZED,
                                            "Invalid signature",
                                        )
                                            .into_response();
                                    }
                                } else {
                                    return (
                                        axum::http::StatusCode::UNAUTHORIZED,
                                        "Missing signature header (x-slack-signature / x-webhook-signature / x-hub-signature-256 / stripe-signature)",
                                    )
                                        .into_response();
                                }
                            }
                        }

                        // Parse the body as the message content. Try JSON
                        // first (most webhooks send JSON); fall back to UTF-8
                        // text. The route name becomes the channel_id.
                        // (iter-101 — previously the body was thrown away and
                        // the agent received "Webhook received on /{route}".)
                        //
                        // iter-131: Slack Events API forwarding. Slack sends
                        // event callbacks as `{"type":"event_callback","event":
                        // {"type":"message","text":"...","user":"...","channel":"..."}}`.
                        // We detect this shape and forward it as a real
                        // IncomingMessage with platform="slack" instead of the
                        // generic "webhook" platform.
                        if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&body) {
                            // Slack event_callback forwarding.
                            if v.get("type").and_then(|t| t.as_str()) == Some("event_callback")
                                && let Some(event) = v.get("event")
                                    && event.get("type").and_then(|t| t.as_str()) == Some("message") {
                                        // Skip bot messages (prevents echo loops).
                                        let is_bot = event
                                            .get("bot_id")
                                            .or_else(|| event.get("bot_profile"))
                                            .is_some();
                                        if !is_bot {
                                            let content = event.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string();
                                            if !content.is_empty() {
                                                let channel = event.get("channel").and_then(|c| c.as_str()).unwrap_or("").to_string();
                                                let user = event.get("user").and_then(|u| u.as_str()).unwrap_or("slack").to_string();
                                                let ts = event.get("ts").and_then(|t| t.as_str())
                                                    .and_then(|s| s.split('.').next().and_then(|n| n.parse::<i64>().ok()))
                                                    .unwrap_or_else(|| chrono::Utc::now().timestamp());
                                                let slack_msg = IncomingMessage {
                                                    platform: "slack".to_string(),
                                                    channel_id: channel,
                                                    user_id: user.clone(),
                                                    username: user,
                                                    content,
                                                    is_group_chat: true,  // Slack events come from channels by default
                                                    timestamp: ts,
                                                    thread_id: event.get("thread_ts")
                                                        .and_then(|t| t.as_str())
                                                        .and_then(|s| s.split('.').next().and_then(|n| n.parse::<i64>().ok())),
                                                    raw: v.clone(),
                                                    media_urls: Vec::new(),
                                                };
                                                let _ = tx.send(slack_msg);
                                            }
                                        }
                                        // Always 200 OK to Slack — otherwise it retries.
                                        return (axum::http::StatusCode::OK, "ok").into_response();
                                    }

                            // WhatsApp Cloud API event forwarding. Meta sends
                            // `{"entry":[{"changes":[{"value":{"messages":[{"from":"...","text":{"body":"..."}}]}}]}]}`.
                            if let Some(entry) = v.get("entry").and_then(|e| e.as_array()).and_then(|a| a.first())
                                && let Some(change) = entry.get("changes").and_then(|c| c.as_array()).and_then(|a| a.first())
                                    && let Some(messages) = change.get("value").and_then(|val| val.get("messages")).and_then(|m| m.as_array()) {
                                        for msg in messages {
                                            let from = msg.get("from").and_then(|f| f.as_str()).unwrap_or("").to_string();
                                            let text = msg.get("text").and_then(|t| t.get("body")).and_then(|b| b.as_str()).unwrap_or("").to_string();
                                            if !text.is_empty() {
                                                let wa_msg = IncomingMessage {
                                                    platform: "whatsapp".to_string(),
                                                    channel_id: from.clone(),
                                                    user_id: from.clone(),
                                                    username: change.get("value").and_then(|val| val.get("contacts")).and_then(|c| c.as_array()).and_then(|a| a.first()).and_then(|c| c.get("profile")).and_then(|p| p.get("name")).and_then(|n| n.as_str()).unwrap_or(&from).to_string(),
                                                    content: text,
                                                    is_group_chat: false,
                                                    timestamp: msg.get("timestamp").and_then(|t| t.as_str()).and_then(|s| s.parse::<i64>().ok()).unwrap_or_else(|| chrono::Utc::now().timestamp()),
                                                    thread_id: None,
                                                    media_urls: Vec::new(),
                                                    raw: msg.clone(),
                                                };
                                                let _ = tx.send(wa_msg);
                                            }
                                        }
                                        return (axum::http::StatusCode::OK, "ok").into_response();
                                    }
                        }

                        let content = if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&body) {
                            // Try common fields: text, message, content, body, data.
                            // If none match, pretty-print the whole JSON.
                            v.get("text")
                                .or_else(|| v.get("message"))
                                .or_else(|| v.get("content"))
                                .or_else(|| v.get("body"))
                                .or_else(|| v.get("data"))
                                .and_then(|t| t.as_str())
                                .map(|s| s.to_string())
                                .unwrap_or_else(|| serde_json::to_string_pretty(&v).unwrap_or_default())
                        } else {
                            // Not JSON — use raw UTF-8 text.
                            String::from_utf8_lossy(&body).to_string()
                        };

                        let msg = IncomingMessage {
                            platform: "webhook".to_string(),
                            channel_id: format!("webhook:{route}"),
                            user_id: "webhook".to_string(),
                            username: "Webhook".to_string(),
                            content,
                            is_group_chat: false,
                            timestamp: chrono::Utc::now().timestamp(),
                            thread_id: None,
                            media_urls: Vec::new(),
                            raw: serde_json::from_slice(&body).unwrap_or(serde_json::json!({"route": route})),
                        };

                        if tx.send(msg).is_err() {
                            return (
                                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                                "Channel closed",
                            )
                                .into_response();
                        }

                        (axum::http::StatusCode::OK, "accepted").into_response()
                    },
                ),
            )
            .with_state((tx, secret));

        // Spawn the HTTP server
        tokio::spawn(async move {
            info!("Webhook HTTP server starting");
            let listener = tokio::net::TcpListener::bind(addr)
                .await
                .expect("Failed to bind webhook listener");
            if let Err(e) = axum::serve(listener, app).await {
                error!(error = %e, "Webhook HTTP server error");
            }
        });

        info!("Webhook adapter started with HTTP server");
        Ok(())
    }

    async fn send_message_to_channel(
        &self,
        _channel_id: &str,
        _message: &OutgoingMessage,
    ) -> Result<String> {
        // Webhook adapter is inbound-only
        Ok(String::new())
    }
}

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

// ---------------------------------------------------------------------------
// Email adapter (SMTP outbound, webhook inbound)
// ---------------------------------------------------------------------------

/// Email adapter — sends replies via SMTP, receives via webhook.
/// Requires `email_smtp_host`, `email_smtp_user`, `email_smtp_pass` in config.
pub struct EmailAdapter {
    enabled: bool,
    smtp_host: Option<String>,
    smtp_user: Option<String>,
    smtp_pass: Option<String>,
}

impl EmailAdapter {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            smtp_host: None,
            smtp_user: None,
            smtp_pass: None,
        }
    }

    pub fn with_smtp(
        mut self,
        host: Option<String>,
        user: Option<String>,
        pass: Option<String>,
    ) -> Self {
        self.smtp_host = host;
        self.smtp_user = user;
        self.smtp_pass = pass;
        self
    }
}

#[async_trait]
impl PlatformAdapter for EmailAdapter {
    fn name(&self) -> &str {
        "email"
    }
    fn is_enabled(&self) -> bool {
        self.enabled
    }

    async fn start(&self) -> Result<()> {
        info!("Email adapter started");
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        info!("Email adapter stopped");
        Ok(())
    }

    async fn send_message(&self, message: OutgoingMessage) -> Result<()> {
        // Email sending uses SMTP which is blocking — spawn_blocking.
        let host = self
            .smtp_host
            .clone()
            .ok_or_else(|| Error::Config("SMTP host not configured".to_string()))?;
        let user = self.smtp_user.clone().unwrap_or_default();
        let pass = self.smtp_pass.clone().unwrap_or_default();
        let to = message.channel_id.clone();
        let body = message.content.clone();

        tokio::task::spawn_blocking(move || -> std::result::Result<(), String> {
            // SMTP send with mandatory STARTTLS upgrade. Previously this
            // ran AUTH LOGIN over plaintext TCP, leaking the SMTP password
            // to anyone on the network path. (iter-125 — closes the
            // ponytail-audit security bug "EmailAdapter SMTP AUTH over
            // plaintext TCP".)
            //
            // NOTE: This is a minimal STARTTLS implementation. Production
            // use should switch to the `lettre` crate for proper MIME,
            // attachments, certificate validation, and connection pooling.
            use std::io::{Read, Write};
            use std::net::TcpStream;

            let port = if host.contains(":") { "" } else { ":587" };
            let addr = format!("{}{}", host, port);

            let mut stream = TcpStream::connect(&addr)
                .map_err(|e| format!("SMTP connect failed: {e}"))?;
            // Set a 30s timeout so a hung SMTP server doesn't block the
            // gateway forever.
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(30)));
            let _ = stream.set_write_timeout(Some(std::time::Duration::from_secs(30)));

            // Helper: read a multi-line SMTP response.
            fn read_response(stream: &mut TcpStream, buf: &mut [u8]) -> std::result::Result<String, String> {
                let n = stream.read(buf).map_err(|e| format!("SMTP read: {e}"))?;
                let text = String::from_utf8_lossy(&buf[..n]).to_string();
                // SMTP multi-line responses end with "<code> <text>" (space
                // after the code, not dash). We need to keep reading until
                // we see that.
                let mut full = text.clone();
                while let Some(line) = full.lines().last() {
                    if line.len() >= 4 && line.as_bytes()[3] == b' ' {
                        break;
                    }
                    let n2 = stream.read(buf).map_err(|e| format!("SMTP read: {e}"))?;
                    full.push_str(&String::from_utf8_lossy(&buf[..n2]));
                }
                Ok(full)
            }

            // Read greeting
            let mut buf = [0u8; 4096];
            let _ = read_response(&mut stream, &mut buf)?;

            // EHLO
            write!(stream, "EHLO operant\r\n").map_err(|e| format!("SMTP write: {e}"))?;
            let ehlo_resp = read_response(&mut stream, &mut buf)?;

            // STARTTLS — refuse to proceed without TLS upgrade.
            write!(stream, "STARTTLS\r\n").map_err(|e| format!("SMTP write: {e}"))?;
            let starttls_resp = read_response(&mut stream, &mut buf)?;
            if !starttls_resp.starts_with("220") {
                return Err(format!(
                    "SMTP server does not support STARTTLS (got: {}). Refusing to send credentials over plaintext. Configure an SMTP host that supports STARTTLS, or set up a TLS tunnel.",
                    starttls_resp.lines().next().unwrap_or("").trim()
                ));
            }

            // Upgrade to TLS. Use a minimal TLS connector with the system
            // root store + hostname verification (rustls).
            use std::sync::OnceLock;
            static TLS_CONFIG: OnceLock<std::sync::Arc<rustls::ClientConfig>> = OnceLock::new();
            let config = TLS_CONFIG.get_or_init(|| {
                let mut roots = rustls::RootCertStore::empty();
                // rustls-native-certs v0.7 returns its own cert iterator;
                // we add each to the rustls RootCertStore manually.
                for cert in rustls_native_certs::load_native_certs()
                    .unwrap_or_default()
                    .into_iter()
                {
                    let _ = roots.add(cert);
                }
                std::sync::Arc::new(
                    rustls::ClientConfig::builder()
                        .with_root_certificates(roots)
                        .with_no_client_auth(),
                )
            });
            let server_name = host
                .split(':')
                .next()
                .unwrap_or(&host)
                .to_string();
            let server_name = rustls::pki_types::ServerName::try_from(server_name)
                .map_err(|e| format!("invalid SMTP hostname for TLS: {e}"))?
                .to_owned();
            let mut connector = rustls::client::ClientConnection::new(
                config.clone(),
                server_name,
            ).map_err(|e| format!("TLS init: {e}"))?;
            let mut tls_stream = rustls::Stream::new(&mut connector, &mut stream);

            // EHLO again over the encrypted channel.
            write!(tls_stream, "EHLO operant\r\n").map_err(|e| format!("SMTP write (TLS): {e}"))?;
            tls_stream.read(&mut buf).map_err(|e| format!("SMTP read (TLS): {e}"))?;

            // AUTH LOGIN (base64-encoded user/pass — now encrypted in transit)
            use base64::Engine;
            write!(tls_stream, "AUTH LOGIN\r\n").map_err(|e| format!("SMTP write (TLS): {e}"))?;
            tls_stream.read(&mut buf).map_err(|e| format!("SMTP read (TLS): {e}"))?;
            write!(tls_stream, "{}\r\n", base64::engine::general_purpose::STANDARD.encode(&user))
                .map_err(|e| format!("SMTP write (TLS): {e}"))?;
            tls_stream.read(&mut buf).map_err(|e| format!("SMTP read (TLS): {e}"))?;
            write!(tls_stream, "{}\r\n", base64::engine::general_purpose::STANDARD.encode(&pass))
                .map_err(|e| format!("SMTP write (TLS): {e}"))?;
            tls_stream.read(&mut buf).map_err(|e| format!("SMTP read (TLS): {e}"))?;

            // MAIL FROM
            write!(tls_stream, "MAIL FROM:<{}>\r\n", user).map_err(|e| format!("SMTP write (TLS): {e}"))?;
            tls_stream.read(&mut buf).map_err(|e| format!("SMTP read (TLS): {e}"))?;

            // RCPT TO
            write!(tls_stream, "RCPT TO:<{}>\r\n", to).map_err(|e| format!("SMTP write (TLS): {e}"))?;
            tls_stream.read(&mut buf).map_err(|e| format!("SMTP read (TLS): {e}"))?;

            // DATA
            write!(tls_stream, "DATA\r\n").map_err(|e| format!("SMTP write (TLS): {e}"))?;
            tls_stream.read(&mut buf).map_err(|e| format!("SMTP read (TLS): {e}"))?;

            // Email body
            write!(
                tls_stream,
                "From: <{}>\r\nTo: <{}>\r\nSubject: Operant Reply\r\n\r\n{}\r\n.\r\n",
                user, to, body
            )
            .map_err(|e| format!("SMTP write (TLS): {e}"))?;
            tls_stream.read(&mut buf).map_err(|e| format!("SMTP read (TLS): {e}"))?;

            // QUIT
            write!(tls_stream, "QUIT\r\n").map_err(|e| format!("SMTP write (TLS): {e}"))?;

            let _ = ehlo_resp; // suppress unused warning
            Ok(())
        })
        .await
        .map_err(|e| Error::Agent(format!("SMTP task failed: {e}")))?
        .map_err(|e| Error::Agent(format!("SMTP send failed: {e}")))?;

        Ok(())
    }

    async fn handle_update(&self, update: serde_json::Value) -> Result<Option<IncomingMessage>> {
        // Parse email webhook payload (format depends on the email forwarding service)
        let from = update["from"]
            .as_str()
            .or_else(|| update["sender"].as_str())
            .unwrap_or("");
        let subject = update["subject"].as_str().unwrap_or("(no subject)");
        let body = update["body"]
            .as_str()
            .or_else(|| update["text"].as_str())
            .unwrap_or("");

        if from.is_empty() && body.is_empty() {
            return Ok(None);
        }

        Ok(Some(IncomingMessage {
            platform: "email".to_string(),
            channel_id: from.to_string(),
            user_id: from.to_string(),
            username: from.to_string(),
            content: format!("Subject: {}\n\n{}", subject, body),
            is_group_chat: false,
            timestamp: chrono::Utc::now().timestamp(),
            thread_id: None,
            media_urls: Vec::new(),

            raw: update,
        }))
    }

    fn config_json(&self) -> serde_json::Value {
        json!({
            "platform": "email",
            "enabled": self.enabled,
            "smtp_host": self.smtp_host,
            "smtp_user_configured": self.smtp_user.is_some(),
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

/// Handle admin commands (sessions, channels, broadcast, shutdown, help)
pub async fn handle_admin_command(
    command: &str,
    _args: &[&str],
    channel_id: &str,
    user_id: &str,
    store: &SessionStore,
    directory: &ChannelDirectory,
    global_admins: &[String],
) -> Result<String> {
    let is_admin =
        global_admins.iter().any(|a| a == user_id) || directory.is_admin(channel_id, user_id);

    if !is_admin {
        return Ok("You are not authorized to use admin commands.".to_string());
    }

    match command {
        "sessions" => {
            let sessions = store.list_active_sessions(None);
            if sessions.is_empty() {
                Ok("No active sessions.".to_string())
            } else {
                let mut response = format!("Active sessions ({}):\n", sessions.len());
                for s in sessions {
                    response.push_str(&format!(
                        "  - {} | {} | {} | last active: {}\n",
                        s.session_id, s.platform, s.platform_user_id, s.last_active
                    ));
                }
                Ok(response)
            }
        }
        "channels" => {
            let channels = directory.list_channels(None);
            if channels.is_empty() {
                Ok("No registered channels.".to_string())
            } else {
                let mut response = format!("Registered channels ({}):\n", channels.len());
                for c in channels {
                    let ct = match c.channel_type {
                        ChannelType::Direct => "direct",
                        ChannelType::Group => "group",
                        ChannelType::Channel => "channel",
                        ChannelType::Unknown => "unknown",
                    };
                    response.push_str(&format!(
                        "  - {} | {} | {}\n",
                        c.channel_id, c.platform, ct
                    ));
                }
                Ok(response)
            }
        }
        "broadcast" => {
            Ok("Broadcast command received. Not yet implemented.".to_string())
        }
        "shutdown" => {
            Ok("Shutdown initiated. Goodbye!".to_string())
        }
        "help" => {
            Ok(
                "Available admin commands:\n  sessions  - List active sessions\n  channels  - List registered channels\n  broadcast - Send a broadcast message\n  shutdown  - Shutdown the gateway\n  help      - Show this help"
                    .to_string(),
            )
        }
        _ => Ok(format!(
            "Unknown command: {}. Type 'help' for available commands.",
            command
        )),
    }
}

/// Format startup announcement message
pub fn format_startup_message(config: &GatewayConfig) -> String {
    let mut msg = String::from("=== Operant Gateway Started ===\n");
    msg.push_str("Enabled Platforms:\n");
    msg.push_str(&format!(
        "  - Telegram  : {}\n",
        if config.telegram_enabled {
            "ENABLED"
        } else {
            "DISABLED"
        }
    ));
    msg.push_str(&format!(
        "  - Discord   : {}\n",
        if config.discord_enabled {
            "ENABLED"
        } else {
            "DISABLED"
        }
    ));
    msg.push_str(&format!(
        "  - Slack     : {}\n",
        if config.slack_enabled {
            "ENABLED"
        } else {
            "DISABLED"
        }
    ));
    msg.push_str(&format!(
        "  - Webhooks  : {}\n",
        if config.webhooks_enabled {
            "ENABLED"
        } else {
            "DISABLED"
        }
    ));
    msg.push_str(&format!(
        "\nAdmin Users: {} configured",
        config.admins.len()
    ));
    msg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_incoming_message() {
        let msg = IncomingMessage::new("telegram", "12345", "testuser", "67890", "Hello, world!");

        assert_eq!(msg.platform, "telegram");
        assert_eq!(msg.user_id, "12345");
        assert_eq!(msg.content, "Hello, world!");
    }

    #[test]
    fn test_outgoing_message() {
        let msg = OutgoingMessage::new("67890", "Response to you")
            .no_markdown()
            .with_reply_to("111");

        assert_eq!(msg.channel_id, "67890");
        assert_eq!(msg.content, "Response to you");
        assert!(!msg.parse_markdown);
        assert_eq!(msg.reply_to, Some("111".to_string()));
        assert_eq!(msg.thread_id, None);
    }

    /// A reply to a forum-topic message must carry the incoming
    /// message_thread_id so it lands in the same topic (hermes parity).
    #[test]
    fn test_outgoing_message_thread_id_roundtrip() {
        // Incoming forum message with a topic thread id.
        let incoming = IncomingMessage::new("telegram", "111", "ishan", "-100123", "hello")
            .with_thread_id(Some(65901));
        assert_eq!(incoming.thread_id, Some(65901));

        // Reply built from it forwards the thread id.
        let reply = OutgoingMessage::new(&incoming.channel_id, "hi back")
            .with_thread_id(incoming.thread_id);
        assert_eq!(reply.thread_id, Some(65901));

        // Messages without a thread context stay None (general chat).
        let plain = OutgoingMessage::new("-100123", "plain");
        assert_eq!(plain.thread_id, None);
    }

    /// A forum message update must surface message_thread_id on the parsed
    /// IncomingMessage so replies are routed back to the topic.
    #[test]
    fn parse_update_captures_message_thread_id() {
        let update = serde_json::json!({
            "update_id": 1,
            "message": {
                "message_id": 10,
                "message_thread_id": 91609,
                "from": {"id": 111, "is_bot": false, "username": "ishan"},
                "chat": {"id": -100123, "type": "supergroup"},
                "text": "Testing the operant functioning."
            }
        });
        let msg = TelegramAdapter::parse_update(update)
            .expect("parse ok")
            .expect("message present");
        assert_eq!(msg.channel_id, "-100123");
        assert_eq!(msg.thread_id, Some(91609));
        assert!(msg.is_group_chat);
    }

    /// Service messages (forum topic created, member joined, …) carry no
    /// user content and must be dropped by parse_update — they previously
    /// fell through to "[sent an attachment]" and spawned a bogus agent turn
    /// (the double-reply bug: topic-creation service message + real text).
    #[test]
    fn parse_update_filters_service_messages() {
        // forum_topic_created — the exact shape Telegram sends when a user
        // creates a new topic (no text, no from, no media).
        let topic_created = serde_json::json!({
            "update_id": 1,
            "message": {
                "message_id": 10,
                "message_thread_id": 91663,
                "chat": {"id": -100123, "type": "supergroup"},
                "date": 1752872700,
                "forum_topic_created": {"name": "New Chat", "icon_color": 0}
            }
        });
        assert!(
            TelegramAdapter::parse_update(topic_created)
                .expect("parse ok")
                .is_none()
        );

        // new_chat_members
        let joined = serde_json::json!({
            "update_id": 2,
            "message": {
                "message_id": 11,
                "chat": {"id": -100123, "type": "supergroup"},
                "new_chat_members": [{"id": 999, "first_name": "Bot"}]
            }
        });
        assert!(
            TelegramAdapter::parse_update(joined)
                .expect("parse ok")
                .is_none()
        );

        // pinned_message
        let pinned = serde_json::json!({
            "update_id": 3,
            "message": {
                "message_id": 12,
                "chat": {"id": -100123, "type": "supergroup"},
                "pinned_message": {"message_id": 9, "text": "rules"}
            }
        });
        assert!(
            TelegramAdapter::parse_update(pinned)
                .expect("parse ok")
                .is_none()
        );

        // A media-less message with no recognized content must NOT fabricate
        // "[sent an attachment]" — it is not a user message.
        let unknown = serde_json::json!({
            "update_id": 4,
            "message": {
                "message_id": 13,
                "from": {"id": 111, "is_bot": false, "username": "ishan"},
                "chat": {"id": -100123, "type": "supergroup"},
                "date": 1752872700
            }
        });
        assert!(
            TelegramAdapter::parse_update(unknown)
                .expect("parse ok")
                .is_none()
        );

        // Sanity: a real text message in the same topic still parses.
        let text = serde_json::json!({
            "update_id": 5,
            "message": {
                "message_id": 14,
                "message_thread_id": 91663,
                "from": {"id": 111, "is_bot": false, "username": "ishan"},
                "chat": {"id": -100123, "type": "supergroup"},
                "text": "Testing operant executive functions."
            }
        });
        let msg = TelegramAdapter::parse_update(text)
            .expect("parse ok")
            .expect("message present");
        assert_eq!(msg.content, "Testing operant executive functions.");
        assert_eq!(msg.thread_id, Some(91663));
    }

    /// Media types beyond photo/document/voice must get accurate placeholders
    /// instead of the generic "[sent an attachment]" fallback.
    #[test]
    fn parse_update_media_placeholders() {
        let video = serde_json::json!({
            "update_id": 1,
            "message": {
                "message_id": 1,
                "from": {"id": 1, "is_bot": false, "username": "u"},
                "chat": {"id": 2, "type": "private"},
                "video": {"file_id": "v1"}
            }
        });
        let msg = TelegramAdapter::parse_update(video)
            .expect("parse ok")
            .expect("present");
        assert_eq!(msg.content, "[sent a video]");

        let audio = serde_json::json!({
            "update_id": 2,
            "message": {
                "message_id": 2,
                "from": {"id": 1, "is_bot": false, "username": "u"},
                "chat": {"id": 2, "type": "private"},
                "audio": {"file_id": "a1"}
            }
        });
        let msg = TelegramAdapter::parse_update(audio)
            .expect("parse ok")
            .expect("present");
        assert_eq!(msg.content, "[sent an audio message]");

        // caption-bearing video uses the caption.
        let video_cap = serde_json::json!({
            "update_id": 3,
            "message": {
                "message_id": 3,
                "from": {"id": 1, "is_bot": false, "username": "u"},
                "chat": {"id": 2, "type": "private"},
                "video": {"file_id": "v2"},
                "caption": "watch this"
            }
        });
        let msg = TelegramAdapter::parse_update(video_cap)
            .expect("parse ok")
            .expect("present");
        assert_eq!(msg.content, "watch this");
    }

    /// A photo update must extract the largest PhotoSize file_id (hermes
    /// takes `photo[-1]` — sizes are sorted ascending).
    #[test]
    fn telegram_attachment_file_id_takes_largest_photo() {
        let message = serde_json::json!({
            "photo": [
                {"file_id": "small", "width": 100, "height": 100},
                {"file_id": "large", "width": 1000, "height": 1000},
            ]
        });
        assert_eq!(
            telegram_attachment_file_id(&message).as_deref(),
            Some("large")
        );
    }

    /// A document update must extract the document file_id.
    #[test]
    fn telegram_attachment_file_id_document() {
        let message = serde_json::json!({
            "document": {"file_id": "doc_1", "file_name": "report.pdf"}
        });
        assert_eq!(
            telegram_attachment_file_id(&message).as_deref(),
            Some("doc_1")
        );
    }

    /// A voice update must extract the voice file_id; plain text yields None.
    #[test]
    fn telegram_attachment_file_id_voice_and_text() {
        let voice = serde_json::json!({"voice": {"file_id": "voice_1"}});
        assert_eq!(
            telegram_attachment_file_id(&voice).as_deref(),
            Some("voice_1")
        );
        assert_eq!(telegram_attachment_file_id(&serde_json::json!({})), None);
    }

    /// Extension guessing must map common Telegram file paths and fall back
    /// to the hint for unknown ones.
    #[test]
    fn telegram_media_extension_guessing() {
        assert_eq!(
            telegram_media_extension("docs/photo_123.jpg", ".bin"),
            ".jpg"
        );
        assert_eq!(telegram_media_extension("voice/msg.oga", ".bin"), ".oga");
        assert_eq!(
            telegram_media_extension("documents/notes.md", ".bin"),
            ".md"
        );
        assert_eq!(
            telegram_media_extension("files/unknown-123", ".pdf"),
            ".pdf"
        );
    }

    /// with_media_urls must round-trip the cached attachment paths onto the
    /// IncomingMessage so the runner can inject them into the agent prompt.
    #[test]
    fn incoming_message_media_urls_roundtrip() {
        let msg = IncomingMessage::new("telegram", "1", "u", "c", "[sent a photo]")
            .with_media_urls(vec!["/home/u/.operant/media/photo_1.jpg".to_string()]);
        assert_eq!(msg.media_urls.len(), 1);
        assert!(msg.media_urls[0].ends_with("photo_1.jpg"));
        assert!(
            IncomingMessage::new("telegram", "1", "u", "c", "hi")
                .media_urls
                .is_empty()
        );
    }

    #[tokio::test]
    async fn test_gateway_config() {
        let config = GatewayConfig::default();
        assert!(!config.telegram_enabled);
        assert!(!config.discord_enabled);
    }

    /// An inline-keyboard approve tap must synthesize a `/approve` text
    /// message on the same channel, so the shared gateway_commands resolver
    /// handles it identically to a typed reply.
    #[test]
    fn approval_callback_approve_synthesizes_slash_approve() {
        let cb = serde_json::json!({
            "id": "cb_1",
            "data": "approval:approve",
            "from": {"id": 111, "username": "ishan"},
            "message": {
                "chat": {"id": -100123},
                "message_id": 42,
                "text": "🔧 Permission required: terminal"
            }
        });
        let msg = TelegramAdapter::approval_message_from_callback(&cb).expect("callback parsed");
        assert_eq!(msg.platform, "telegram");
        assert_eq!(msg.user_id, "111");
        assert_eq!(msg.channel_id, "-100123");
        assert_eq!(msg.content, "/approve");
    }

    #[test]
    fn approval_callback_deny_synthesizes_slash_deny() {
        let cb = serde_json::json!({
            "id": "cb_2",
            "data": "approval:deny",
            "from": {"id": 222, "username": "other"},
            "message": {"chat": {"id": 999}}
        });
        let msg = TelegramAdapter::approval_message_from_callback(&cb).expect("callback parsed");
        assert_eq!(msg.content, "/deny");
        assert_eq!(msg.channel_id, "999");
    }

    #[test]
    fn non_approval_callback_is_ignored() {
        let cb = serde_json::json!({
            "id": "cb_3",
            "data": "some-other-action",
            "from": {"id": 333},
            "message": {"chat": {"id": 1}}
        });
        assert!(TelegramAdapter::approval_message_from_callback(&cb).is_none());
    }

    #[test]
    fn callback_without_chat_is_ignored() {
        let cb = serde_json::json!({
            "id": "cb_4",
            "data": "approval:approve",
            "from": {"id": 444}
        });
        assert!(TelegramAdapter::approval_message_from_callback(&cb).is_none());
    }

    /// An approval tap in a forum topic must carry the thread_id (so the
    /// `/approve` resolution reply lands in the same topic, not the general
    /// chat) and the `approval_callback` raw marker (so the dispatch layer
    /// can edit the prompt message to show the outcome — hermes
    /// `query.edit_message_text` parity).
    #[test]
    fn approval_callback_carries_thread_and_edit_marker() {
        let cb = serde_json::json!({
            "id": "cb_5",
            "data": "approval:approve",
            "from": {"id": 555, "first_name": "Ishan", "username": "ishan"},
            "message": {
                "chat": {"id": -100123, "type": "supergroup"},
                "message_id": 42,
                "message_thread_id": 91609,
                "text": "🔧 Permission required: terminal"
            }
        });
        let msg = TelegramAdapter::approval_message_from_callback(&cb).expect("callback parsed");
        assert_eq!(msg.thread_id, Some(91609));
        assert!(msg.is_group_chat);
        let marker = msg
            .raw
            .get("approval_callback")
            .expect("edit marker present");
        assert_eq!(marker["chat_id"], "-100123");
        assert_eq!(marker["message_id"], 42);
        assert_eq!(marker["thread_id"], 91609);
        assert_eq!(marker["user"], "Ishan");
        assert_eq!(marker["action"], "approve");
    }

    /// A clarify() button tap must synthesize a message carrying the
    /// `choice_callback` raw marker (prompt chat/message ids, tap user and
    /// the selected index) so the dispatch layer resolves the pending
    /// question with the option text and edits the prompt message.
    #[test]
    fn choice_callback_synthesizes_marked_message() {
        let cb = serde_json::json!({
            "id": "cb_6",
            "data": "choice:1",
            "from": {"id": 666, "first_name": "Ishan"},
            "message": {
                "chat": {"id": -100123, "type": "supergroup"},
                "message_id": 77,
                "message_thread_id": 91609,
                "text": "❓ Which approach?"
            }
        });
        let msg = TelegramAdapter::choice_message_from_callback(&cb).expect("callback parsed");
        assert_eq!(msg.thread_id, Some(91609));
        assert!(msg.is_group_chat);
        assert_eq!(msg.content, "__choice__1");
        let marker = msg.raw.get("choice_callback").expect("edit marker present");
        assert_eq!(marker["chat_id"], "-100123");
        assert_eq!(marker["message_id"], 77);
        assert_eq!(marker["user"], "Ishan");
        assert_eq!(marker["idx"], 1);
    }

    #[test]
    fn non_choice_callback_is_ignored() {
        let cb = serde_json::json!({
            "id": "cb_7",
            "data": "approval:deny",
            "from": {"id": 777},
            "message": {"chat": {"id": 1}}
        });
        assert!(TelegramAdapter::choice_message_from_callback(&cb).is_none());
    }

    /// Echo handler that optionally blocks inside `handle` until released
    /// (simulates a long-running agent turn).
    struct BlockingEchoHandler {
        gate: Option<Arc<tokio::sync::Notify>>,
    }

    #[async_trait::async_trait]
    impl MessageHandler for BlockingEchoHandler {
        async fn handle(&self, message: IncomingMessage) -> Result<OutgoingMessage> {
            if let Some(gate) = &self.gate {
                gate.notified().await;
            }
            Ok(OutgoingMessage::new(
                &message.channel_id,
                format!("echo:{}", message.content),
            ))
        }
    }

    /// The per-session turn lease must refuse a second concurrent turn for
    /// the same session while the first is in flight, and the stall tracker
    /// must observe the in-flight turn.
    #[tokio::test]
    async fn route_message_serializes_concurrent_turns_via_lease() {
        let gate = Arc::new(tokio::sync::Notify::new());
        let handler = Arc::new(BlockingEchoHandler {
            gate: Some(gate.clone()),
        });
        let gw = Arc::new(Gateway::new(GatewayConfig::default()).with_handler(handler));

        let msg1 = IncomingMessage::new("telegram", "u1", "alice", "c1", "first");
        let msg2 = IncomingMessage::new("telegram", "u1", "alice", "c1", "second");

        let t1 = tokio::spawn({
            let gw = gw.clone();
            async move { gw.route_message(msg1).await.unwrap() }
        });

        // Wait for turn 1 to acquire the lease and enter handle().
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if gw.turn_lease().is_busy("telegram:u1:c1").await {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(
            gw.turn_lease().is_busy("telegram:u1:c1").await,
            "turn 1 must hold the lease"
        );
        assert_eq!(gw.stall_tracker().active_count().await, 1);

        // Second message for the same session: busy reply, no agent run.
        let busy = gw.route_message(msg2).await.unwrap().expect("busy reply");
        assert!(
            busy.content
                .contains("Still working on your previous message"),
            "got: {}",
            busy.content
        );
        assert_eq!(gw.stall_tracker().active_count().await, 1, "no second turn");

        // Release turn 1 — it completes and the lease is freed.
        gate.notify_one();
        let first = t1.await.expect("task panicked").expect("response");
        assert_eq!(first.content, "echo:first");
        assert!(!gw.turn_lease().is_busy("telegram:u1:c1").await);
        assert_eq!(gw.stall_tracker().active_count().await, 0);
    }

    /// Every successful route records a delivery in the ledger; mirror rules
    /// resolve matching target channels.
    #[tokio::test]
    async fn route_message_records_delivery_and_mirror_targets() {
        let handler = Arc::new(BlockingEchoHandler { gate: None });
        let gw = Arc::new(
            Gateway::new(GatewayConfig::default())
                .with_handler(handler)
                .with_mirror_rule(MirrorRule {
                    platform: "telegram".to_string(),
                    source_channel: "c1".to_string(),
                    target_channel: "group:ops".to_string(),
                }),
        );

        let msg = IncomingMessage::new("telegram", "u1", "alice", "c1", "hi");
        let resp = gw.route_message(msg).await.unwrap().expect("response");
        assert_eq!(resp.content, "echo:hi");

        assert_eq!(gw.delivery_ledger().delivered_count().await, 1);
        assert_eq!(gw.delivery_ledger().failed_count().await, 0);
        let recent = gw.delivery_ledger().recent(5).await;
        assert_eq!(recent[0].platform, "telegram");
        assert_eq!(recent[0].channel_id, "c1");
        assert!(recent[0].content_len > 0);

        // Mirror rule matches the source channel and resolves the target.
        assert_eq!(
            gw.mirror_targets("telegram", "c1"),
            vec!["group:ops".to_string()]
        );
        assert!(gw.mirror_targets("telegram", "c9").is_empty());
        assert!(gw.mirror_targets("discord", "c1").is_empty());
    }

    #[tokio::test]
    async fn test_telegram_adapter_disabled() {
        let adapter = TelegramAdapter::new(None);
        assert!(!adapter.is_enabled());
    }

    #[tokio::test]
    async fn test_discord_adapter_disabled() {
        let adapter = DiscordAdapter::new(None);
        assert!(!adapter.is_enabled());
    }

    #[tokio::test]
    async fn test_slack_adapter_disabled() {
        let adapter = SlackAdapter::new(None, None);
        assert!(!adapter.is_enabled());
    }

    // ── SessionStore tests ──

    #[test]
    fn test_session_store_create_and_get() {
        let store = SessionStore::new();
        let session = store
            .create_session("telegram", "user123", "channel456")
            .unwrap();
        assert_eq!(session.platform, "telegram");
        assert_eq!(session.platform_user_id, "user123");
        assert_eq!(session.platform_channel_id, "channel456");
        assert!(!session.session_id.is_empty());

        let fetched = store.get_session(&session.session_id);
        assert!(fetched.is_some());
        assert_eq!(fetched.unwrap().platform_user_id, "user123");
    }

    #[test]
    fn test_session_store_find() {
        let store = SessionStore::new();
        store.create_session("discord", "user1", "chan1").unwrap();
        let s2 = store.create_session("telegram", "user2", "chan2").unwrap();

        let found = store.find_session("telegram", "user2", "chan2");
        assert!(found.is_some());
        assert_eq!(found.unwrap().session_id, s2.session_id);

        let not_found = store.find_session("telegram", "user1", "chan1");
        assert!(not_found.is_none());
    }

    #[test]
    fn test_session_store_list() {
        let store = SessionStore::new();
        store.create_session("telegram", "a", "c1").unwrap();
        store.create_session("telegram", "b", "c2").unwrap();
        store.create_session("discord", "c", "c3").unwrap();

        assert_eq!(store.get_session_count(), 3);

        let all = store.list_active_sessions(None);
        assert_eq!(all.len(), 3);

        let telegram_only = store.list_active_sessions(Some("telegram"));
        assert_eq!(telegram_only.len(), 2);

        let discord_only = store.list_active_sessions(Some("discord"));
        assert_eq!(discord_only.len(), 1);
    }

    #[test]
    fn test_session_store_close_and_update() {
        let store = SessionStore::new();
        let session = store.create_session("test", "user", "chan").unwrap();
        let sid = session.session_id.clone();

        assert!(store.update_activity(&sid).is_ok());
        assert!(store.close_session(&sid).is_ok());
        assert!(store.get_session(&sid).is_none());
        assert_eq!(store.get_session_count(), 0);
    }

    #[test]
    fn test_session_operant_lookup() {
        let store = SessionStore::new();
        let session = store.create_session("test", "user", "chan").unwrap();
        // manually assign operant_session_id via metadata
        let _sid = session.session_id.clone();

        // get_operant_session uses the field, which is empty by default
        let found = store.get_operant_session("");
        // the operant_session_id is empty string for newly created sessions
        assert!(found.is_some());
    }

    #[tokio::test]
    async fn test_concurrent_session_access() {
        let store = std::sync::Arc::new(SessionStore::new());
        let mut handles = Vec::new();
        for i in 0..10 {
            let s = store.clone();
            handles.push(tokio::spawn(async move {
                let session = s
                    .create_session("test", &format!("user{}", i), "chan1")
                    .unwrap();
                assert_eq!(session.platform, "test");
                let fetched = s.get_session(&session.session_id);
                assert!(fetched.is_some());
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        assert_eq!(store.get_session_count(), 10);
    }

    // ── ChannelDirectory tests ──

    #[test]
    fn test_channel_directory_register_and_get() {
        let dir = ChannelDirectory::new();
        dir.register_channel(
            "ch1",
            "telegram",
            Some("General"),
            ChannelType::Group,
            vec!["admin1".to_string()],
        )
        .unwrap();

        let ch = dir.get_channel("ch1");
        assert!(ch.is_some());
        let ch = ch.unwrap();
        assert_eq!(ch.platform, "telegram");
        assert_eq!(ch.name, Some("General".to_string()));
        assert_eq!(ch.channel_type, ChannelType::Group);
        assert!(ch.admins.contains(&"admin1".to_string()));
    }

    #[test]
    fn test_channel_directory_duplicate_fails() {
        let dir = ChannelDirectory::new();
        dir.register_channel("ch1", "t", None, ChannelType::Direct, vec![])
            .unwrap();
        let result = dir.register_channel("ch1", "t", None, ChannelType::Direct, vec![]);
        assert!(result.is_err());
    }

    #[test]
    fn test_channel_directory_list_and_remove() {
        let dir = ChannelDirectory::new();
        dir.register_channel("a", "t", None, ChannelType::Direct, vec![])
            .unwrap();
        dir.register_channel("b", "d", None, ChannelType::Group, vec![])
            .unwrap();
        dir.register_channel("c", "t", None, ChannelType::Channel, vec![])
            .unwrap();

        assert_eq!(dir.list_channels(None).len(), 3);
        assert_eq!(dir.list_channels(Some("t")).len(), 2);
        assert_eq!(dir.list_channels(Some("d")).len(), 1);

        dir.remove_channel("a").unwrap();
        assert_eq!(dir.list_channels(None).len(), 2);
    }

    #[test]
    fn test_channel_directory_is_admin() {
        let dir = ChannelDirectory::new();
        dir.register_channel(
            "ch1",
            "t",
            None,
            ChannelType::Group,
            vec!["admin1".to_string(), "admin2".to_string()],
        )
        .unwrap();

        assert!(dir.is_admin("ch1", "admin1"));
        assert!(dir.is_admin("ch1", "admin2"));
        assert!(!dir.is_admin("ch1", "user3"));
        assert!(!dir.is_admin("nonexistent", "admin1"));
    }

    // ── GatewayStats tests ──

    #[tokio::test]
    async fn test_gateway_stats() {
        let config = GatewayConfig {
            telegram_enabled: false,
            telegram_token: None,
            discord_enabled: false,
            discord_token: None,
            slack_enabled: false,
            slack_token: None,
            webhooks_enabled: false,
            webhooks_addr: None,
            webhooks_secret: None,
            admins: vec![],
            streaming_transport: "auto".to_string(),
            telegram_proxy: None,
            telegram_bot_username: None,
            telegram_dm_topics_enabled: false,
            whatsapp_enabled: false,
            whatsapp_token: None,
            whatsapp_phone_number_id: None,
            email_enabled: false,
            email_smtp_host: None,
            email_smtp_user: None,
            email_smtp_pass: None,
            sms_twilio_enabled: false,
            max_concurrent_sessions: None,
        };
        let gw = Gateway::new(config);
        let stats = gw.get_stats().await;

        assert_eq!(stats.active_sessions, 0);
        assert_eq!(stats.registered_channels, 0);
        assert_eq!(stats.active_adapters, 0);
        assert_eq!(stats.messages_processed, 0);
        assert!(stats.uptime_seconds < 5); // just started
    }

    // ── Admin command tests ──

    #[tokio::test]
    async fn test_admin_command_unauthorized() {
        let store = SessionStore::new();
        let dir = ChannelDirectory::new();
        let result = handle_admin_command(
            "help",
            &[],
            "ch1",
            "unknown_user",
            &store,
            &dir,
            &["admin1".to_string()],
        )
        .await
        .unwrap();
        assert!(result.contains("not authorized"));
    }

    #[tokio::test]
    async fn test_admin_command_help() {
        let store = SessionStore::new();
        let dir = ChannelDirectory::new();
        let result = handle_admin_command(
            "help",
            &[],
            "ch1",
            "admin1",
            &store,
            &dir,
            &["admin1".to_string()],
        )
        .await
        .unwrap();
        assert!(result.contains("sessions"));
        assert!(result.contains("channels"));
        assert!(result.contains("broadcast"));
        assert!(result.contains("shutdown"));
    }

    #[tokio::test]
    async fn test_admin_command_sessions_empty() {
        let store = SessionStore::new();
        let dir = ChannelDirectory::new();
        let result = handle_admin_command(
            "sessions",
            &[],
            "ch1",
            "admin1",
            &store,
            &dir,
            &["admin1".to_string()],
        )
        .await
        .unwrap();
        assert!(result.contains("No active sessions"));
    }

    #[tokio::test]
    async fn test_admin_command_channels_empty() {
        let store = SessionStore::new();
        let dir = ChannelDirectory::new();
        let result = handle_admin_command(
            "channels",
            &[],
            "ch1",
            "admin1",
            &store,
            &dir,
            &["admin1".to_string()],
        )
        .await
        .unwrap();
        assert!(result.contains("No registered channels"));
    }

    #[tokio::test]
    async fn test_admin_command_unknown() {
        let store = SessionStore::new();
        let dir = ChannelDirectory::new();
        let result = handle_admin_command(
            "foobar",
            &[],
            "ch1",
            "admin1",
            &store,
            &dir,
            &["admin1".to_string()],
        )
        .await
        .unwrap();
        assert!(result.contains("Unknown command"));
    }

    #[tokio::test]
    async fn test_admin_command_broadcast() {
        let store = SessionStore::new();
        let dir = ChannelDirectory::new();
        let result = handle_admin_command(
            "broadcast",
            &[],
            "ch1",
            "admin1",
            &store,
            &dir,
            &["admin1".to_string()],
        )
        .await
        .unwrap();
        assert!(result.contains("Broadcast"));
    }

    // ── Adapter health check tests ──

    #[test]
    fn test_webhook_adapter_health() {
        let adapter = WebhookAdapter::new(true);
        assert!(adapter.health_check().unwrap());
    }

    #[test]
    fn test_webhook_secret_flows_from_settings_to_adapter() {
        // Regression: webhooks_secret existed in the schema and the webhook
        // handler supports HMAC verification, but the CLI never wired it —
        // a configured secret silently did nothing (unsigned webhooks accepted).
        let adapter = WebhookAdapter::new(true).with_secret(Some("s3cret".to_string()));
        assert!(adapter.config_json()["hmac_secret_configured"] == serde_json::json!(true));

        let no_secret = WebhookAdapter::new(true);
        assert!(no_secret.config_json()["hmac_secret_configured"] == serde_json::json!(false));
    }

    #[tokio::test]
    async fn test_webhook_hmac_live_server() {
        // End-to-end: boot the real axum server with a configured secret and
        // prove the verification gate actually rejects/accepts on the wire.
        // (Regression for R14-1: the secret used to be dead config — every
        // webhook was accepted unsigned.)
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        type HmacSha256 = Hmac<Sha256>;
        use tokio::sync::mpsc;

        // Grab a free ephemeral port.
        let probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = probe.local_addr().unwrap();
        drop(probe);

        let (tx, mut rx) = mpsc::unbounded_channel::<IncomingMessage>();
        let adapter = WebhookAdapter::new(true)
            .with_addr(addr.to_string())
            .with_secret(Some("topsecret".to_string()));
        adapter
            .start_with_channel(tx.clone())
            .await
            .expect("webhook adapter should start");

        let url = format!("http://{}/webhook/r14", addr);
        let client = reqwest::Client::new();
        let body = r#"{"text":"hmac live test"}"#;

        // The server binds inside a spawned task — wait until it accepts.
        for _ in 0..50 {
            if client
                .get(url.replace("/webhook/r14", "/health"))
                .send()
                .await
                .is_ok()
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }

        // 1) No signature -> 401
        let resp = client
            .post(&url)
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::UNAUTHORIZED);

        // 2) Wrong signature -> 401
        let resp = client
            .post(&url)
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .header("x-webhook-signature", hex::encode(vec![0u8; 32]))
            .body(body)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::UNAUTHORIZED);

        // 3) Correct HMAC-SHA256(secret, body) -> 200 + message forwarded
        let mut mac = HmacSha256::new_from_slice(b"topsecret").unwrap();
        mac.update(body.as_bytes());
        let expected = hex::encode(mac.finalize().into_bytes());
        let resp = client
            .post(&url)
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .header("x-webhook-signature", &expected)
            .body(body)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);

        // The forwarded message must carry the body as content.
        let msg = tokio::time::timeout(std::time::Duration::from_secs(3), rx.recv())
            .await
            .expect("timed out waiting for forwarded webhook")
            .expect("channel closed");
        assert_eq!(msg.platform, "webhook");
        assert_eq!(msg.channel_id, "webhook:r14");
        assert_eq!(msg.content, "hmac live test");
    }

    #[test]
    fn test_telegram_adapter_health_check_default() {
        let adapter = TelegramAdapter::new(None);
        let healthy = adapter.health_check().unwrap();
        assert!(healthy);
    }

    // ── Startup message tests ──

    #[test]
    fn test_format_startup_message() {
        let config = GatewayConfig {
            telegram_enabled: true,
            telegram_token: Some("tok".to_string()),
            discord_enabled: false,
            discord_token: None,
            slack_enabled: true,
            slack_token: Some("tok".to_string()),
            webhooks_enabled: false,
            webhooks_addr: None,
            webhooks_secret: None,
            admins: vec!["admin1".to_string()],
            streaming_transport: "auto".to_string(),
            telegram_proxy: None,
            telegram_bot_username: None,
            telegram_dm_topics_enabled: false,
            whatsapp_enabled: false,
            whatsapp_token: None,
            whatsapp_phone_number_id: None,
            email_enabled: false,
            email_smtp_host: None,
            email_smtp_user: None,
            email_smtp_pass: None,
            sms_twilio_enabled: false,
            max_concurrent_sessions: None,
        };
        let msg = format_startup_message(&config);
        assert!(msg.contains("Operant Gateway"));
        assert!(msg.contains("Telegram"));
        assert!(msg.contains("ENABLED"));
        assert!(msg.contains("DISABLED"));
        assert!(msg.contains("Admin Users: 1"));
    }

    // ── ChannelType tests ──

    #[test]
    fn test_channel_type_equality() {
        assert_eq!(ChannelType::Direct, ChannelType::Direct);
        assert_eq!(ChannelType::Group, ChannelType::Group);
        assert_ne!(ChannelType::Direct, ChannelType::Group);
    }

    // ── chunk_text tests ──

    #[test]
    fn test_chunk_text_small_message_no_split() {
        let text = "Hello, world!";
        let chunks = chunk_text(text, 4000);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "Hello, world!");
    }

    #[test]
    fn test_chunk_text_exact_boundary() {
        // Text that fits exactly in one chunk (UTF-16 length == max_chunk_size)
        let text = "A".repeat(4000);
        let chunks = chunk_text(&text, 4000);
        assert_eq!(chunks.len(), 1);
        assert_eq!(utf16_len(&chunks[0]), 4000);
    }

    #[test]
    fn test_chunk_text_multi_chunk_with_suffix_indicators() {
        let text = "A".repeat(9000);
        let chunks = chunk_text(&text, 4000);
        assert!(chunks.len() >= 2);
        // Each chunk should have (X/Y) suffix
        for (i, chunk) in chunks.iter().enumerate() {
            assert!(
                chunk.contains(&format!("({}/{})", i + 1, chunks.len())),
                "chunk {} missing suffix: {}",
                i,
                chunk
            );
        }
    }

    #[test]
    fn test_chunk_text_code_block_preservation() {
        let text = format!(
            "intro text\n\n```rust\n{}\n```\n\noutro",
            "let x = 1;\n".repeat(500)
        );
        let chunks = chunk_text(&text, 4000);
        assert!(chunks.len() >= 2);
        for chunk in &chunks {
            let opens = chunk.matches("```").count();
            assert_eq!(opens % 2, 0, "chunk has unbalanced code fences: {}", chunk);
        }
    }

    #[test]
    fn test_chunk_text_word_boundary_splitting() {
        let text = "word ".repeat(1000);
        let chunks = chunk_text(&text, 4000);
        assert!(chunks.len() >= 2);
        for chunk in &chunks {
            let content = chunk.split(" (").next().unwrap_or(chunk);
            if !content.is_empty() {
                let trimmed = content.trim_end();
                assert!(
                    trimmed.ends_with("word"),
                    "chunk doesn't end at word boundary: ...{}",
                    &trimmed[trimmed.len().saturating_sub(20)..]
                );
            }
        }
    }

    #[test]
    fn test_chunk_text_utf16_emoji_handling() {
        // Emojis are 2 UTF-16 code units each
        let emoji_text = "🎉".repeat(2500); // 5000 UTF-16 code units
        let chunks = chunk_text(&emoji_text, 4000);
        assert!(chunks.len() >= 2);
        // Verify each chunk respects UTF-16 limit (accounting for suffix)
        for chunk in &chunks {
            let content = chunk.split(" (").next().unwrap_or(chunk);
            assert!(
                utf16_len(content) <= 4000,
                "chunk exceeds UTF-16 limit: {} units",
                utf16_len(content)
            );
        }
    }

    #[test]
    fn test_chunk_text_newline_split_preference() {
        let text = format!("{}\n\n{}", "line".repeat(500), "next".repeat(500));
        let chunks = chunk_text(&text, 4000);
        assert!(chunks.len() >= 2);
        // Split should prefer \n\n boundary
        let first_content = chunks[0].split(" (").next().unwrap_or(&chunks[0]);
        assert!(
            first_content.ends_with("\n\n") || first_content.ends_with('\n'),
            "split didn't use newline boundary"
        );
    }

    #[test]
    fn test_utf16_len_ascii() {
        assert_eq!(utf16_len("hello"), 5);
    }

    #[test]
    fn test_utf16_len_emoji() {
        // Most emojis are 2 UTF-16 code units
        assert_eq!(utf16_len("🎉"), 2);
        assert_eq!(utf16_len("a🎉b"), 4);
    }

    #[test]
    fn test_utf16_len_mixed() {
        // "Hello " = 6, "🎉" = 2 utf16, " World" = 6 → total 14
        assert_eq!(utf16_len("Hello 🎉 World"), 14);
    }
}
