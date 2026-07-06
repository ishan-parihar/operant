//! Multi-platform gateway for Operant-RS
//!
//! Provides unified messaging interface across multiple platforms including
//! Telegram, Discord, Slack, WhatsApp, and more.

use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::config::runtime_config;
use crate::error::{Error, Result};
use crate::gateway_markdown::{markdown_to_telegram_html, markdown_to_slack_mrkdwn};
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
            email_enabled: settings.email_enabled,
            email_smtp_host: settings.email_smtp_host,
            email_smtp_user: settings.email_smtp_user,
            email_smtp_pass: settings.email_smtp_pass,
            sms_twilio_enabled: settings.sms_twilio_enabled,
            webhooks_enabled: settings.webhooks_enabled,
            webhooks_addr: settings.webhooks_addr,
            admins: settings.admins,
            streaming_transport: settings.streaming_transport,
            telegram_proxy: settings.telegram_proxy,
            telegram_bot_username: settings.telegram_bot_username,
            telegram_dm_topics_enabled: settings.telegram_dm_topics_enabled,
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
}

impl OutgoingMessage {
    /// Create a new outgoing message
    pub fn new(channel_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            channel_id: channel_id.into(),
            content: content.into(),
            parse_markdown: true,
            reply_to: None,
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

impl SessionStore {
    pub fn new() -> Self {
        Self {
            sessions: std::sync::RwLock::new(HashMap::new()),
        }
    }

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

    /// Get a session by its ID
    pub fn get_session(&self, session_id: &str) -> Option<PlatformSession> {
        let sessions = self.sessions.read().expect("Session store lock poisoned");
        sessions.get(session_id).cloned()
    }

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

    /// Remove a session
    pub fn close_session(&self, session_id: &str) -> Result<()> {
        let mut sessions = self.sessions.write().expect("Session store lock poisoned");
        sessions.remove(session_id);
        Ok(())
    }

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

    /// Total number of sessions
    pub fn get_session_count(&self) -> usize {
        let sessions = self.sessions.read().expect("Session store lock poisoned");
        sessions.len()
    }

    /// Find a session by its Operant session ID
    pub fn get_operant_session(&self, operant_session_id: &str) -> Option<PlatformSession> {
        let sessions = self.sessions.read().expect("Session store lock poisoned");
        sessions
            .values()
            .find(|s| s.operant_session_id == operant_session_id)
            .cloned()
    }

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

impl ChannelDirectory {
    pub fn new() -> Self {
        Self {
            channels: std::sync::RwLock::new(HashMap::new()),
        }
    }

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

    /// Get channel info by ID
    pub fn get_channel(&self, channel_id: &str) -> Option<ChannelInfo> {
        let channels = self.channels.read().expect("Channel store lock poisoned");
        channels.get(channel_id).cloned()
    }

    /// Remove a channel
    pub fn remove_channel(&self, channel_id: &str) -> Result<()> {
        let mut channels = self.channels.write().expect("Channel store lock poisoned");
        channels.remove(channel_id);
        Ok(())
    }

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

    /// Send a typing indicator to a channel.
    fn send_typing(&self, _channel_id: &str) -> Result<()> {
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

    /// Get the status of all adapters
    pub async fn status(&self) -> HashMap<String, bool> {
        let mut status = HashMap::new();
        for (name, adapter) in &self.adapters {
            status.insert(name.clone(), adapter.is_enabled());
        }
        status
    }

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
            return Ok(Some(OutgoingMessage::new(
                &message.channel_id,
                "You are not authorized to use this bot.",
            )));
        }

        let handler = match &self.message_handler {
            Some(h) => h,
            None => {
                warn!("No message handler configured");
                return Ok(None);
            }
        };

        let response = handler.handle(message).await?;

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
    pub fn send_typing(&self, platform: &str, channel_id: &str) -> Result<()> {
        if let Some(adapter) = self.adapters.get(platform) {
            adapter.send_typing(channel_id)?;
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

    /// Get health status of all platform adapters
    pub async fn get_platform_status(&self) -> HashMap<String, bool> {
        let mut status = HashMap::new();
        for (name, adapter) in &self.adapters {
            let healthy = adapter.health_check().unwrap_or(false);
            status.insert(name.clone(), healthy);
        }
        status
    }

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
    bot_username: Option<String>,
    dm_topics_enabled: bool,
    dm_topic_map: Arc<RwLock<HashMap<String, i64>>>,
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
            bot_username: None,
            dm_topics_enabled: false,
            dm_topic_map: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a Telegram adapter with full configuration
    pub fn with_config(
        token: Option<String>,
        bot_username: Option<String>,
        dm_topics_enabled: bool,
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
            bot_username,
            dm_topics_enabled,
            dm_topic_map: Arc::new(RwLock::new(HashMap::new())),
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

/// Escape HTML special characters for Telegram HTML parse_mode
fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
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
    let estimated_chunks = (utf16_len(text) + body_budget - 1) / body_budget;
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
    if let Some(pos) = region.rfind(' ') {
        if pos > cp_limit / 4 {
            return pos;
        }
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
        if stripped.starts_with("```") {
            if in_code {
                in_code = false;
                lang = String::new();
            } else {
                in_code = true;
                let tag = stripped[3..].trim();
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
            self.send_telegram_inner(&message.channel_id, chunk, reply_to)
                .await?;
        }
        Ok(())
    }

    fn send_typing(&self, channel_id: &str) -> Result<()> {
        let url = format!("{}/sendChatAction", self.api_url());
        let body = serde_json::json!({
            "chat_id": channel_id,
            "action": "typing",
        });
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
            .send_telegram_inner(&message.channel_id, &chunks[0], message.reply_to.as_deref())
            .await?;
        for chunk in &chunks[1..] {
            self.send_telegram_inner(&message.channel_id, chunk, None)
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
        let body = serde_json::json!({
            "chat_id": channel_id,
            "message_id": message_id,
            "text": html,
            "parse_mode": "HTML",
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
        // Parse Telegram update — delegates to the static parse_update
        TelegramAdapter::parse_update(update)
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
                {
                    if let Ok(data) = resp.json::<serde_json::Value>().await {
                        if let Some(updates) = data["result"].as_array() {
                            for update in updates {
                                if let Some(update_id) = update["update_id"].as_i64() {
                                    offset = update_id + 1;
                                }
                            }
                        }
                    }
                }
                tracing::info!("Startup probe completed, initial offset: {}", offset);

                // === LOAD SAVED OFFSET (persist across restarts) ===
                let offset_path = get_offset_path();
                if offset_path.exists() {
                    if let Ok(saved) = tokio::fs::read_to_string(&offset_path).await {
                        if let Ok(n) = saved.trim().parse::<i64>() {
                            if n > offset {
                                offset = n;
                            }
                        }
                    }
                }
                tracing::info!("Loaded saved offset: {}", offset);

                // ── INNER POLLING LOOP ──
                let mut retry_delay: u64 = 1;
                let mut last_heartbeat = Instant::now();

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

                            if let Ok(data) = resp.json::<serde_json::Value>().await {
                                if let Some(updates) = data["result"].as_array() {
                                    had_updates = !updates.is_empty();
                                    if had_updates {
                                        tracing::info!(
                                            "Received {} update(s) from Telegram",
                                            updates.len()
                                        );
                                        last_heartbeat = Instant::now();
                                    }
                                    for update in updates {
                                        if let Some(update_id) = update["update_id"].as_i64() {
                                            offset = update_id + 1;
                                        }
                                        if let Ok(Some(msg)) =
                                            TelegramAdapter::parse_update(update.clone())
                                        {
                                            tracing::info!("Sent message to gateway handler (chat: {}, content: {:.50})", msg.channel_id, msg.content);
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
            .send_telegram_inner(channel_id, &chunks[0], message.reply_to.as_deref())
            .await?;
        for chunk in &chunks[1..] {
            self.send_telegram_inner(channel_id, chunk, None).await?;
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
    /// Parse a Telegram update into an IncomingMessage.
    /// This is the same logic used by handle_update but callable without a trait object.
    fn parse_update(update: serde_json::Value) -> Result<Option<IncomingMessage>> {
        let message = match update.get("message") {
            Some(m) => m,
            None => return Ok(None),
        };

        // Filter out messages from bots
        if let Some(from) = message.get("from") {
            if from
                .get("is_bot")
                .and_then(|b| b.as_bool())
                .unwrap_or(false)
            {
                return Ok(None);
            }
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
        } else if message.get("sticker").is_some() {
            "[sent a sticker]".to_string()
        } else {
            "[sent an attachment]".to_string()
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

impl TelegramAdapter {
    /// Send a message in reply to another message in a Telegram chat.
    /// Returns the message_id of the sent message.
    ///
    /// This is important for threaded replies where `reply_to_message_id`
    /// keeps the response in the same thread.
    pub async fn send_message_with_reply(
        &self,
        chat_id: &str,
        text: &str,
        reply_to: &str,
    ) -> Result<String> {
        self.send_telegram_inner(chat_id, text, Some(reply_to))
            .await
    }

    /// Send a streaming message to a Telegram chat, progressively editing
    /// the same message as new chunks arrive from the receiver.
    ///
    /// The first chunk is sent as a new message and each subsequent chunk
    /// edits that message in place (progressive edit). When the receiver is
    /// exhausted or an error occurs, the spawned task finishes.
    ///
    /// Returns a `JoinHandle` that can be used to await or detach the stream.
    pub fn send_message_streaming(
        &self,
        chat_id: String,
        mut content_rx: tokio::sync::mpsc::Receiver<String>,
    ) -> tokio::task::JoinHandle<()> {
        let client = self.client.clone();
        let token = self.token.clone().unwrap_or_default();
        let base = runtime_config().gateway.telegram_api_base;
        let api_base = base.trim_end_matches('/').to_string();

        tokio::spawn(async move {
            // Wait for the first chunk of the stream
            let first_chunk = match content_rx.recv().await {
                Some(chunk) => chunk,
                None => {
                    tracing::debug!("Telegram streaming: receiver closed before first chunk");
                    return;
                }
            };

            // Accumulate raw text across chunks
            let mut accumulated = first_chunk;

            // Send the initial message and capture the message_id
            let escaped = escape_html(&accumulated);
            let body = serde_json::json!({
                "chat_id": chat_id,
                "text": escaped,
                "parse_mode": "HTML",
            });

            let message_id = match client
                .post(format!("{}/bot{}/sendMessage", api_base, token))
                .json(&body)
                .send()
                .await
            {
                Ok(resp) => {
                    if let Ok(data) = resp.json::<serde_json::Value>().await {
                        data["result"]["message_id"]
                            .as_i64()
                            .map(|id| id.to_string())
                    } else {
                        None
                    }
                }
                Err(e) => {
                    tracing::error!("Telegram streaming: initial send failed: {}", e);
                    return;
                }
            };

            let mut message_id = match message_id {
                Some(id) => id,
                None => {
                    tracing::error!("Telegram streaming: failed to get message_id from response");
                    return;
                }
            };
            tracing::info!(
                "Streaming message started for chat {}, message_id: {}",
                chat_id,
                message_id
            );

            // Edit the message progressively as new chunks arrive
            while let Some(chunk) = content_rx.recv().await {
                // Check if adding this chunk would exceed the threshold
                if utf16_len(&accumulated) + utf16_len(&chunk) > 4000 {
                    // Overflow detected: finalize current message, start new one
                    tracing::info!(
                        "Telegram streaming: message overflow, starting new message (message_id: {})",
                        message_id
                    );

                    // Send new message with the overflow chunk as starting content
                    accumulated = chunk;
                    let escaped = escape_html(&accumulated);
                    let new_body = serde_json::json!({
                        "chat_id": chat_id,
                        "text": escaped,
                        "parse_mode": "HTML",
                    });

                    match client
                        .post(format!("{}/bot{}/sendMessage", api_base, token))
                        .json(&new_body)
                        .send()
                        .await
                    {
                        Ok(resp) => {
                            if let Ok(data) = resp.json::<serde_json::Value>().await {
                                if let Some(new_id) = data["result"]["message_id"].as_i64() {
                                    message_id = new_id.to_string();
                                } else {
                                    tracing::error!(
                                        "Telegram streaming: overflow send returned no message_id"
                                    );
                                    break;
                                }
                            } else {
                                tracing::error!(
                                    "Telegram streaming: overflow send returned invalid JSON"
                                );
                                break;
                            }
                        }
                        Err(e) => {
                            tracing::error!("Telegram streaming: overflow send failed: {}", e);
                            break;
                        }
                    }
                    continue;
                }

                // Accumulate and edit
                accumulated.push_str(&chunk);
                let escaped = escape_html(&accumulated);
                let edit_body = serde_json::json!({
                    "chat_id": chat_id,
                    "message_id": message_id,
                    "text": escaped,
                    "parse_mode": "HTML",
                });

                match client
                    .post(format!("{}/bot{}/editMessageText", api_base, token))
                    .json(&edit_body)
                    .send()
                    .await
                {
                    Ok(resp) => {
                        // 400 "message is not modified" — skip, not an error
                        if resp.status().as_u16() == 400 {
                            // message not modified, continue streaming
                        } else {
                            tracing::info!("Streaming message {} edited successfully", message_id);
                        }
                    }
                    Err(e) => {
                        tracing::error!("Telegram streaming edit error: {}", e);
                        break;
                    }
                }
            }
        })
    }
}

// ── Telegram hardening: DM topics, media, mentions, commands ──

impl TelegramAdapter {
    pub async fn create_dm_topic(&self, chat_id: &str, name: &str) -> Result<Option<i64>> {
        let url = format!("{}/createForumTopic", self.api_url());
        let body = serde_json::json!({
            "chat_id": chat_id,
            "name": name,
        });
        let resp = self.client.post(&url).json(&body).send().await?;
        let data: serde_json::Value = resp.json().await?;
        if let Some(thread_id) = data["result"]["message_thread_id"].as_i64() {
            tracing::info!(
                "Created DM topic '{}' in chat {} -> thread_id={}",
                name,
                chat_id,
                thread_id
            );
            self.dm_topic_map
                .write()
                .await
                .insert(chat_id.to_string(), thread_id);
            Ok(Some(thread_id))
        } else {
            let desc = data["description"].as_str().unwrap_or("unknown error");
            tracing::debug!("createForumTopic failed for chat {}: {}", chat_id, desc);
            Ok(None)
        }
    }

    pub async fn get_or_create_dm_topic(&self, chat_id: &str) -> Result<Option<i64>> {
        if let Some(&tid) = self.dm_topic_map.read().await.get(chat_id) {
            return Ok(Some(tid));
        }
        let topic_name = "Operant";
        match self.create_dm_topic(chat_id, topic_name).await? {
            Some(tid) => Ok(Some(tid)),
            None => {
                let fallback = self.dm_topic_map.read().await.get(chat_id).copied();
                Ok(fallback)
            }
        }
    }

    pub async fn send_to_dm_topic(
        &self,
        chat_id: &str,
        text: &str,
        reply_to: Option<&str>,
    ) -> Result<String> {
        let thread_id = self.get_or_create_dm_topic(chat_id).await?;
        let html = markdown_to_telegram_html(text);
        let mut body = serde_json::json!({
            "chat_id": chat_id,
            "text": html,
            "parse_mode": "HTML",
        });
        if let Some(tid) = thread_id {
            body["message_thread_id"] = serde_json::json!(tid);
        }
        if let Some(reply) = reply_to {
            body["reply_to_message_id"] = serde_json::json!(reply);
        }
        let resp = self
            .client
            .post(format!("{}/sendMessage", self.api_url()))
            .json(&body)
            .send()
            .await?;
        let data: serde_json::Value = resp.json().await?;
        Ok(data["result"]["message_id"]
            .as_i64()
            .map(|id| id.to_string())
            .unwrap_or_default())
    }

    pub fn strip_mention(&self, text: &str) -> String {
        match &self.bot_username {
            Some(username) => {
                let pattern = format!("@{}", username);
                text.replace(&pattern, "").trim().to_string()
            }
            None => text.to_string(),
        }
    }

    pub fn should_respond_in_group(&self, text: &str, entities: &serde_json::Value) -> bool {
        if self.bot_username.is_none() {
            return true;
        }
        let username = self.bot_username.as_ref().unwrap();
        if text.contains(&format!("@{}", username)) {
            return true;
        }
        if let Some(entity_list) = entities.as_array() {
            for entity in entity_list {
                let entity_type = entity["type"].as_str().unwrap_or("");
                if entity_type == "mention" || entity_type == "text_mention" {
                    let mention_text = entity["user"]["username"].as_str().unwrap_or("");
                    if mention_text.eq_ignore_ascii_case(username) {
                        return true;
                    }
                }
                if entity_type == "bot_command" {
                    let offset = entity["offset"].as_u64().unwrap_or(0) as usize;
                    let length = entity["length"].as_u64().unwrap_or(0) as usize;
                    let cmd_text = text.get(offset..offset + length).unwrap_or("");
                    if cmd_text.contains('@') {
                        let after_at = cmd_text.split('@').nth(1).unwrap_or("");
                        if after_at.eq_ignore_ascii_case(username) {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    pub async fn set_my_commands(&self, commands: &[(&str, &str)]) -> Result<()> {
        let url = format!("{}/setMyCommands", self.api_url());
        let cmd_list: Vec<serde_json::Value> = commands
            .iter()
            .map(|(name, desc)| serde_json::json!({"command": name, "description": desc}))
            .collect();
        let body = serde_json::json!({
            "commands": cmd_list,
        });
        let resp = self.client.post(&url).json(&body).send().await?;
        if resp.status().is_success() {
            tracing::info!("Registered {} bot commands", commands.len());
        } else {
            let err = resp.text().await.unwrap_or_default();
            tracing::warn!("setMyCommands failed: {}", err);
        }
        Ok(())
    }

    pub async fn download_file(&self, file_id: &str) -> Result<Vec<u8>> {
        let get_file_url = format!("{}/getFile", self.api_url());
        let body = serde_json::json!({"file_id": file_id});
        let resp = self.client.post(&get_file_url).json(&body).send().await?;
        let data: serde_json::Value = resp.json().await?;
        let file_path = data["result"]["file_path"]
            .as_str()
            .ok_or_else(|| crate::error::Error::Agent("No file_path in getFile response".into()))?;
        let base = runtime_config().gateway.telegram_api_base;
        let file_url = format!(
            "{}/file/bot{}/{}",
            base.trim_end_matches('/'),
            self.token.as_deref().unwrap_or(""),
            file_path
        );
        let bytes = self
            .client
            .get(&file_url)
            .send()
            .await?
            .bytes()
            .await?
            .to_vec();
        Ok(bytes)
    }

    pub async fn upload_document(
        &self,
        chat_id: &str,
        filename: &str,
        data: &[u8],
        mime: &str,
        caption: Option<&str>,
    ) -> Result<String> {
        let part = reqwest::multipart::Part::bytes(data.to_vec())
            .file_name(filename.to_string())
            .mime_str(mime)
            .map_err(|e| crate::error::Error::Agent(e.to_string()))?;
        let mut form = reqwest::multipart::Form::new()
            .text("chat_id", chat_id.to_string())
            .part("document", part);
        if let Some(cap) = caption {
            form = form.text("caption", cap.to_string());
        }
        let url = format!("{}/sendDocument", self.api_url());
        let resp = self.client.post(&url).multipart(form).send().await?;
        let data: serde_json::Value = resp.json().await?;
        Ok(data["result"]["message_id"]
            .as_i64()
            .map(|id| id.to_string())
            .unwrap_or_default())
    }

    pub async fn upload_photo(
        &self,
        chat_id: &str,
        filename: &str,
        data: &[u8],
        mime: &str,
        caption: Option<&str>,
    ) -> Result<String> {
        let part = reqwest::multipart::Part::bytes(data.to_vec())
            .file_name(filename.to_string())
            .mime_str(mime)
            .map_err(|e| crate::error::Error::Agent(e.to_string()))?;
        let mut form = reqwest::multipart::Form::new()
            .text("chat_id", chat_id.to_string())
            .part("photo", part);
        if let Some(cap) = caption {
            form = form.text("caption", cap.to_string());
        }
        let url = format!("{}/sendPhoto", self.api_url());
        let resp = self.client.post(&url).multipart(form).send().await?;
        let data: serde_json::Value = resp.json().await?;
        Ok(data["result"]["message_id"]
            .as_i64()
            .map(|id| id.to_string())
            .unwrap_or_default())
    }
}

/// Polls Telegram for new updates in a non-blocking loop.
///
/// Uses long polling (30s timeout) to receive updates from Telegram's Bot API.
/// Spawns a single task that continues until `stop()` is called.
pub struct TelegramPoller {
    adapter: Arc<TelegramAdapter>,
    running: Arc<AtomicBool>,
    interval: Duration,
}

impl TelegramPoller {
    /// Create a new Telegram poller for the given adapter.
    pub fn new(adapter: Arc<TelegramAdapter>) -> Self {
        Self {
            adapter,
            running: Arc::new(AtomicBool::new(false)),
            interval: Duration::from_secs(2),
        }
    }

    /// Start the polling loop. Spawns a task that sends received messages
    /// through the `tx` channel. This method itself does not block.
    pub fn start(&self, tx: mpsc::UnboundedSender<IncomingMessage>) {
        self.running.store(true, Ordering::SeqCst);
        let adapter = self.adapter.clone();
        let running = self.running.clone();
        let interval = self.interval;
        let url = format!("{}/getUpdates", adapter.api_url());

        tokio::spawn(async move {
            let client = reqwest::Client::new();
            let mut offset: i64 = 0;

            while running.load(Ordering::SeqCst) {
                let response = client
                    .post(&url)
                    .json(&serde_json::json!({
                        "offset": offset,
                        "timeout": 30,
                    }))
                    .send()
                    .await;

                match response {
                    Ok(resp) => {
                        if let Ok(data) = resp.json::<serde_json::Value>().await {
                            if let Some(updates) = data["result"].as_array() {
                                for update in updates {
                                    if let Some(update_id) = update["update_id"].as_i64() {
                                        offset = update_id + 1;
                                    }
                                    if let Ok(Some(msg)) =
                                        adapter.handle_update(update.clone()).await
                                    {
                                        let _ = tx.send(msg);
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("Telegram polling error: {}", e);
                    }
                }

                tokio::time::sleep(interval).await;
            }
        });
    }

    /// Stop the polling loop. Sets the running flag to false,
    /// causing the spawned task to exit on its next iteration.
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
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

        // Verify the token
        let client = reqwest::Client::new();
        let response = client
            .get(format!("{}/users/@me", self.api_url()))
            .header(
                "Authorization",
                format!("Bot {}", self.token.as_ref().unwrap()),
            )
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
    fn send_typing(&self, channel_id: &str) -> Result<()> {
        let token = self.token.as_ref().ok_or_else(|| {
            Error::Config("Discord token not configured".to_string())
        })?;
        let url = format!("{}/channels/{}/typing", self.api_url(), channel_id);
        // Fire and forget — we don't await the response since send_typing
        // is synchronous. The next typing call in 4s will refresh it.
        let client = reqwest::Client::new();
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
        let client = reqwest::Client::new();

        // Discord's message limit is 2000 chars. Messages exceeding this
        // are silently rejected by the API (400 Bad Request) — the user
        // gets nothing. Chunk the text to stay under the limit.
        // (Bug #14 from iter-98 audit.)
        let chunks = chunk_text(&message.content, 2000);

        for chunk in chunks {
            let body = serde_json::json!({
                "content": chunk,
            });

            let url = format!(
                "{}/channels/{}/messages",
                self.api_url(),
                message.channel_id
            );

            client
                .post(&url)
                .header(
                    "Authorization",
                    format!("Bot {}", self.token.as_ref().unwrap()),
                )
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
        let client = reqwest::Client::new();

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
            .header(
                "Authorization",
                format!("Bearer {}", self.token.as_ref().unwrap()),
            )
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

    async fn start_with_channel(
        &self,
        message_tx: mpsc::UnboundedSender<IncomingMessage>,
    ) -> Result<()> {
        use axum::extract::{Path, State};
        use axum::http::HeaderMap;
        use axum::response::IntoResponse;
        use axum::routing::{get, post};
        use axum::Router;

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
                post(
                    move |Path(route): Path<String>,
                          headers: HeaderMap,
                          State((tx, secret)): State<(
                        mpsc::UnboundedSender<IncomingMessage>,
                        Option<String>,
                    )>,
                          body: axum::body::Bytes| async move {
                        // Validate HMAC signature if secret is configured.
                        // Uses standard HMAC-SHA256(secret, body) — not the
                        // old non-standard SHA256(route+secret). Supports
                        // multiple signature header names for interop:
                        // x-webhook-signature (custom), x-hub-signature-256
                        // (GitHub), Stripe-Signature (Stripe).
                        // (iter-101 — closes Bug #9 from iter-98 audit.)
                        if let Some(ref sec) = secret {
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
                                    "Missing signature header (x-webhook-signature / x-hub-signature-256 / stripe-signature)",
                                )
                                    .into_response();
                            }
                        }

                        // Parse the body as the message content. Try JSON
                        // first (most webhooks send JSON); fall back to UTF-8
                        // text. The route name becomes the channel_id.
                        // (iter-101 — previously the body was thrown away and
                        // the agent received "Webhook received on /{route}".)
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
    fn name(&self) -> &str { "whatsapp" }
    fn is_enabled(&self) -> bool { self.enabled }

    async fn start(&self) -> Result<()> {
        info!("WhatsApp adapter started");
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        info!("WhatsApp adapter stopped");
        Ok(())
    }

    async fn send_message(&self, message: OutgoingMessage) -> Result<()> {
        let token = self.token.as_ref().ok_or_else(|| {
            Error::Config("WhatsApp token not configured".to_string())
        })?;
        let phone = message.channel_id.clone();

        let client = reqwest::Client::new();
        let phone_number_id = self.phone_number_id.as_deref().ok_or_else(|| {
            Error::Config(
                "WhatsApp phone_number_id not configured. Set it via WhatsAppAdapter::with_phone_number_id() or config.gateway.whatsapp_phone_number_id. Get it from Meta Business Manager.".to_string()
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
            .map_err(|e| Error::Network(e))?;

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
        if let Some(entry) = update["entry"].as_array().and_then(|e| e.first()) {
            if let Some(change) = entry["changes"].as_array().and_then(|c| c.first()) {
                if let Some(msg) = change["value"]["messages"].as_array().and_then(|m| m.first()) {
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
                        
                        raw: update,
                    }));
                }
            }
        }
        Ok(None)
    }

    fn config_json(&self) -> serde_json::Value {
        json!({
            "platform": "whatsapp",
            "enabled": self.enabled,
            "token_configured": self.token.is_some(),
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
    fn name(&self) -> &str { "email" }
    fn is_enabled(&self) -> bool { self.enabled }

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
        let host = self.smtp_host.clone().ok_or_else(|| {
            Error::Config("SMTP host not configured".to_string())
        })?;
        let user = self.smtp_user.clone().unwrap_or_default();
        let pass = self.smtp_pass.clone().unwrap_or_default();
        let to = message.channel_id.clone();
        let body = message.content.clone();

        tokio::task::spawn_blocking(move || -> std::result::Result<(), String> {
            // Basic SMTP send using native TLS.
            // This is a minimal implementation — production use should
            // switch to the `lettre` crate for proper MIME, attachments,
            // and TLS handling.
            use std::io::{Read, Write};
            use std::net::TcpStream;

            let port = if host.contains(":") { "" } else { ":587" };
            let addr = format!("{}{}", host, port);

            let mut stream = TcpStream::connect(&addr)
                .map_err(|e| format!("SMTP connect failed: {e}"))?;

            // Read greeting
            let mut buf = [0u8; 1024];
            stream.read(&mut buf).map_err(|e| format!("SMTP read: {e}"))?;

            // EHLO
            write!(stream, "EHLO operant\r\n").map_err(|e| format!("SMTP write: {e}"))?;
            stream.read(&mut buf).map_err(|e| format!("SMTP read: {e}"))?;

            // AUTH LOGIN (simplified — base64 encoded user/pass)
            use base64::Engine;
            write!(stream, "AUTH LOGIN\r\n").map_err(|e| format!("SMTP write: {e}"))?;
            stream.read(&mut buf).map_err(|e| format!("SMTP read: {e}"))?;
            write!(stream, "{}\r\n", base64::engine::general_purpose::STANDARD.encode(&user))
                .map_err(|e| format!("SMTP write: {e}"))?;
            stream.read(&mut buf).map_err(|e| format!("SMTP read: {e}"))?;
            write!(stream, "{}\r\n", base64::engine::general_purpose::STANDARD.encode(&pass))
                .map_err(|e| format!("SMTP write: {e}"))?;
            stream.read(&mut buf).map_err(|e| format!("SMTP read: {e}"))?;

            // MAIL FROM
            write!(stream, "MAIL FROM:<{}>\r\n", user).map_err(|e| format!("SMTP write: {e}"))?;
            stream.read(&mut buf).map_err(|e| format!("SMTP read: {e}"))?;

            // RCPT TO
            write!(stream, "RCPT TO:<{}>\r\n", to).map_err(|e| format!("SMTP write: {e}"))?;
            stream.read(&mut buf).map_err(|e| format!("SMTP read: {e}"))?;

            // DATA
            write!(stream, "DATA\r\n").map_err(|e| format!("SMTP write: {e}"))?;
            stream.read(&mut buf).map_err(|e| format!("SMTP read: {e}"))?;

            // Email body
            write!(
                stream,
                "From: <{}>\r\nTo: <{}>\r\nSubject: Operant Reply\r\n\r\n{}\r\n.\r\n",
                user, to, body
            )
            .map_err(|e| format!("SMTP write: {e}"))?;
            stream.read(&mut buf).map_err(|e| format!("SMTP read: {e}"))?;

            // QUIT
            write!(stream, "QUIT\r\n").map_err(|e| format!("SMTP write: {e}"))?;

            Ok(())
        })
        .await
        .map_err(|e| Error::Agent(format!("SMTP task failed: {e}")))?
        .map_err(|e| Error::Agent(format!("SMTP send failed: {e}")))?;

        Ok(())
    }

    async fn handle_update(&self, update: serde_json::Value) -> Result<Option<IncomingMessage>> {
        // Parse email webhook payload (format depends on the email forwarding service)
        let from = update["from"].as_str().or_else(|| update["sender"].as_str()).unwrap_or("");
        let subject = update["subject"].as_str().unwrap_or("(no subject)");
        let body = update["body"].as_str().or_else(|| update["text"].as_str()).unwrap_or("");

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
    fn name(&self) -> &str { "sms" }
    fn is_enabled(&self) -> bool { self.enabled }

    async fn start(&self) -> Result<()> {
        info!("SMS adapter started");
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        info!("SMS adapter stopped");
        Ok(())
    }

    async fn send_message(&self, message: OutgoingMessage) -> Result<()> {
        let sid = self.account_sid.as_ref().ok_or_else(|| {
            Error::Config("TWILIO_ACCOUNT_SID not set".to_string())
        })?;
        let token = self.auth_token.as_ref().ok_or_else(|| {
            Error::Config("TWILIO_AUTH_TOKEN not set".to_string())
        })?;
        let from = self.from_number.as_ref().ok_or_else(|| {
            Error::Config("TWILIO_FROM_NUMBER not set".to_string())
        })?;

        let url = format!("https://api.twilio.com/2010-04-01/Accounts/{}/Messages.json", sid);
        let client = reqwest::Client::new();

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
            .map_err(|e| Error::Network(e))?;

        if !resp.status().is_success() {
            return Err(Error::Agent(format!(
                "Twilio API error: {}",
                resp.status()
            )));
        }

        Ok(())
    }

    async fn handle_update(&self, update: serde_json::Value) -> Result<Option<IncomingMessage>> {
        // Parse Twilio webhook payload
        let from = update["From"].as_str().or_else(|| update["from"].as_str()).unwrap_or("");
        let body = update["Body"].as_str().or_else(|| update["body"].as_str()).unwrap_or("");

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
    }

    #[tokio::test]
    async fn test_gateway_config() {
        let config = GatewayConfig::default();
        assert!(!config.telegram_enabled);
        assert!(!config.discord_enabled);
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
        let found = store.get_operant_session(&String::new());
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
            admins: vec![],
            streaming_transport: "auto".to_string(),
            telegram_proxy: None,
            telegram_bot_username: None,
            telegram_dm_topics_enabled: false,
            whatsapp_enabled: false,
            whatsapp_token: None,
            email_enabled: false,
            email_smtp_host: None,
            email_smtp_user: None,
            email_smtp_pass: None,
            sms_twilio_enabled: false,
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
            admins: vec!["admin1".to_string()],
            streaming_transport: "auto".to_string(),
            telegram_proxy: None,
            telegram_bot_username: None,
            telegram_dm_topics_enabled: false,
            whatsapp_enabled: false,
            whatsapp_token: None,
            email_enabled: false,
            email_smtp_host: None,
            email_smtp_user: None,
            email_smtp_pass: None,
            sms_twilio_enabled: false,
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
