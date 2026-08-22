//! `channels_cfg` configuration surface — extracted verbatim from the
//! former schema.rs monolith (dedup pass). Placement is navigational;
//! every item is re-exported from `schema::`.

use crate::traits::ChannelConfig;
use crate::validation_bail;
use anyhow::Result;
use operant_macros::Configurable;
#[cfg(feature = "schema-export")]
use serde::{Deserialize, Serialize};

use super::*;

// ── LinkedIn ────────────────────────────────────────────────────

/// LinkedIn integration configuration (`[linkedin]` section).
///
/// When enabled, the `linkedin` tool is registered in the agent tool surface.
/// Requires `LINKEDIN_*` credentials in the workspace `.env` file.
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "linkedin"]
pub struct LinkedInConfig {
    /// Enable the LinkedIn tool.
    #[serde(default)]
    pub enabled: bool,

    /// LinkedIn REST API version header (YYYYMM format).
    #[serde(default = "default_linkedin_api_version")]
    pub api_version: String,

    /// Content strategy for automated posting.
    #[serde(default)]
    #[nested]
    pub content: LinkedInContentConfig,

    /// Image generation for posts (`[linkedin.image]`).
    #[serde(default)]
    #[nested]
    pub image: LinkedInImageConfig,
}

impl Default for LinkedInConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            api_version: default_linkedin_api_version(),
            content: LinkedInContentConfig::default(),
            image: LinkedInImageConfig::default(),
        }
    }
}

/// Content strategy configuration for LinkedIn auto-posting (`[linkedin.content]`).
///
/// The agent reads this via the `linkedin get_content_strategy` action to know
/// what feeds to check, which repos to highlight, and how to write posts.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "linkedin.content"]
pub struct LinkedInContentConfig {
    /// RSS feed URLs to monitor for topic inspiration (titles only).
    #[serde(default)]
    pub rss_feeds: Vec<String>,

    /// GitHub usernames whose public activity to reference.
    #[serde(default)]
    pub github_users: Vec<String>,

    /// GitHub repositories to highlight (format: `owner/repo`).
    #[serde(default)]
    pub github_repos: Vec<String>,

    /// Topics of expertise and interest for post themes.
    #[serde(default)]
    pub topics: Vec<String>,

    /// Professional persona description (name, role, expertise).
    #[serde(default)]
    pub persona: String,

    /// Freeform posting instructions for the AI agent.
    #[serde(default)]
    pub instructions: String,
}

/// Image generation configuration for LinkedIn posts (`[linkedin.image]`).
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "linkedin.image"]
pub struct LinkedInImageConfig {
    /// Enable image generation for posts.
    #[serde(default)]
    pub enabled: bool,

    /// Provider priority order. Tried in sequence; first success wins.
    #[serde(default = "default_image_providers")]
    pub providers: Vec<String>,

    /// Generate a branded SVG text card when all AI providers fail.
    #[serde(default = "default_true")]
    pub fallback_card: bool,

    /// Accent color for the fallback card (CSS hex).
    #[serde(default = "default_card_accent_color")]
    pub card_accent_color: String,

    /// Temp directory for generated images, relative to workspace.
    #[serde(default = "default_image_temp_dir")]
    pub temp_dir: String,

    /// Stability AI provider settings.
    #[serde(default)]
    #[nested]
    pub stability: ImageProviderStabilityConfig,

    /// Google Imagen (Vertex AI) provider settings.
    #[serde(default)]
    #[nested]
    pub imagen: ImageProviderImagenConfig,

    /// OpenAI DALL-E provider settings.
    #[serde(default)]
    #[nested]
    pub dalle: ImageProviderDalleConfig,

    /// Flux (fal.ai) provider settings.
    #[serde(default)]
    #[nested]
    pub flux: ImageProviderFluxConfig,
}

impl Default for LinkedInImageConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            providers: default_image_providers(),
            fallback_card: true,
            card_accent_color: default_card_accent_color(),
            temp_dir: default_image_temp_dir(),
            stability: ImageProviderStabilityConfig::default(),
            imagen: ImageProviderImagenConfig::default(),
            dalle: ImageProviderDalleConfig::default(),
            flux: ImageProviderFluxConfig::default(),
        }
    }
}

/// Top-level channel configurations (`[channels]` section).
///
/// Each channel sub-section (e.g. `telegram`, `discord`) is optional;
/// setting it to `Some(...)` enables that channel.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "channels"]
pub struct ChannelsConfig {
    /// Enable the CLI interactive channel. Default: `true`.
    #[serde(default = "default_true")]
    pub cli: bool,
    /// Telegram bot channel configuration.
    #[nested]
    #[display_name = "Telegram"]
    #[description = "Bot API — long-polling"]
    pub telegram: Option<TelegramConfig>,
    /// Discord bot channel configuration.
    #[nested]
    #[display_name = "Discord"]
    #[description = "Servers, channels & DMs"]
    pub discord: Option<DiscordConfig>,
    /// Discord history channel — logs ALL messages and forwards @mentions to agent.
    #[nested]
    #[display_name = "Discord History"]
    #[description = "Logs all messages, forwards mentions to the agent"]
    pub discord_history: Option<DiscordHistoryConfig>,
    /// Slack bot channel configuration.
    #[nested]
    #[display_name = "Slack"]
    #[description = "Workspace apps via Web API"]
    pub slack: Option<SlackConfig>,
    /// Mattermost bot channel configuration.
    #[nested]
    #[display_name = "Mattermost"]
    #[description = "Self-hosted team chat"]
    pub mattermost: Option<MattermostConfig>,
    /// Webhook channel configuration.
    #[nested]
    #[display_name = "Webhooks"]
    #[description = "HTTP endpoint for triggers"]
    pub webhook: Option<WebhookConfig>,
    /// iMessage channel configuration (macOS only).
    #[nested]
    #[display_name = "iMessage"]
    #[description = "macOS AppleScript bridge"]
    pub imessage: Option<IMessageConfig>,
    /// Matrix channel configuration.
    #[nested]
    #[display_name = "Matrix"]
    #[description = "Matrix protocol (Element)"]
    pub matrix: Option<MatrixConfig>,
    /// Signal channel configuration.
    #[nested]
    #[display_name = "Signal"]
    #[description = "Privacy-focused via signal-cli"]
    pub signal: Option<SignalConfig>,
    /// WhatsApp channel configuration (Cloud API or Web mode).
    #[nested]
    #[display_name = "WhatsApp"]
    #[description = "Meta Cloud API or Web mode"]
    pub whatsapp: Option<WhatsAppConfig>,
    /// Linq Partner API channel configuration.
    #[nested]
    #[display_name = "Linq"]
    #[description = "Linq Partner API for iMessage/RCS/SMS"]
    pub linq: Option<LinqConfig>,
    /// WATI WhatsApp Business API channel configuration.
    #[nested]
    #[display_name = "WATI"]
    #[description = "WhatsApp Business API gateway"]
    pub wati: Option<WatiConfig>,
    /// Nextcloud Talk bot channel configuration.
    #[nested]
    #[display_name = "Nextcloud Talk"]
    #[description = "Self-hosted Nextcloud chat"]
    pub nextcloud_talk: Option<NextcloudTalkConfig>,
    /// Email channel configuration.
    #[nested]
    #[display_name = "Email"]
    #[description = "IMAP / SMTP inbox bridge"]
    pub email: Option<crate::scattered_types::EmailConfig>,
    /// Gmail Pub/Sub push notification channel configuration.
    #[nested]
    #[display_name = "Gmail Push"]
    #[description = "Pub/Sub push notifications for Gmail"]
    pub gmail_push: Option<crate::scattered_types::GmailPushConfig>,
    /// IRC channel configuration.
    #[nested]
    #[display_name = "IRC"]
    #[description = "Classic IRC with SASL / NickServ"]
    pub irc: Option<IrcConfig>,
    /// Lark channel configuration.
    #[nested]
    #[display_name = "Lark"]
    #[description = "ByteDance Lark / Feishu international"]
    pub lark: Option<LarkConfig>,
    /// LINE Messaging API channel configuration.
    #[nested]
    #[display_name = "LINE"]
    #[description = "LINE Messaging API"]
    pub line: Option<LineConfig>,
    /// Feishu channel configuration.
    #[nested]
    #[display_name = "Feishu"]
    #[description = "ByteDance Feishu (China)"]
    pub feishu: Option<FeishuConfig>,
    /// DingTalk channel configuration.
    #[nested]
    #[display_name = "DingTalk"]
    #[description = "DingTalk Stream Mode"]
    pub dingtalk: Option<DingTalkConfig>,
    /// WeCom (WeChat Enterprise) Bot Webhook channel configuration.
    #[nested]
    #[display_name = "WeCom"]
    #[description = "WeChat Enterprise Bot Webhook"]
    pub wecom: Option<WeComConfig>,
    /// WeChat personal iLink Bot channel configuration (QR code login).
    #[nested]
    #[display_name = "WeChat"]
    #[description = "WeChat personal iLink Bot (QR login)"]
    pub wechat: Option<WeChatConfig>,
    /// QQ Official Bot channel configuration.
    #[nested]
    #[display_name = "QQ Official"]
    #[description = "Tencent QQ Bot SDK"]
    pub qq: Option<QQConfig>,
    /// X/Twitter channel configuration.
    #[nested]
    #[display_name = "X / Twitter"]
    #[description = "X / Twitter API"]
    pub twitter: Option<TwitterConfig>,
    /// Mochat customer service channel configuration.
    #[nested]
    #[display_name = "Mochat"]
    #[description = "Mochat customer service"]
    pub mochat: Option<MochatConfig>,
    #[cfg(feature = "channel-nostr")]
    #[nested]
    #[display_name = "Nostr"]
    #[description = "Decentralized DMs (NIP-04)"]
    /// Nostr channel configuration (decentralized DMs, NIP-04).
    pub nostr: Option<NostrConfig>,
    /// ClawdTalk voice channel configuration.
    #[nested]
    #[display_name = "ClawdTalk"]
    #[description = "ClawdTalk voice channel"]
    pub clawdtalk: Option<crate::scattered_types::ClawdTalkConfig>,
    /// Reddit channel configuration (OAuth2 bot).
    #[nested]
    #[display_name = "Reddit"]
    #[description = "Reddit OAuth2 bot"]
    pub reddit: Option<RedditConfig>,
    /// Bluesky channel configuration (AT Protocol).
    #[nested]
    #[display_name = "Bluesky"]
    #[description = "Bluesky / AT Protocol"]
    pub bluesky: Option<BlueskyConfig>,
    /// Voice call channel configuration (Twilio/Telnyx/Plivo).
    #[nested]
    #[display_name = "Voice Call"]
    #[description = "Twilio / Telnyx / Plivo voice calls"]
    pub voice_call: Option<crate::scattered_types::VoiceCallConfig>,
    /// Voice wake word detection channel configuration.
    #[cfg(feature = "voice-wake")]
    #[nested]
    #[display_name = "Voice Wake"]
    #[description = "Local wake-word detection"]
    pub voice_wake: Option<VoiceWakeConfig>,
    /// Voice duplex configuration (full-duplex voice over WebSocket).
    #[nested]
    #[display_name = "Voice Duplex"]
    #[description = "Full-duplex voice over WebSocket"]
    pub voice_duplex: Option<VoiceDuplexConfig>,
    /// MQTT channel configuration (SOP listener).
    #[nested]
    #[display_name = "MQTT"]
    #[description = "MQTT SOP listener"]
    pub mqtt: Option<MqttConfig>,
    /// Base timeout in seconds for processing a single channel message (LLM + tools).
    /// Runtime uses this as a per-turn budget that scales with tool-loop depth
    /// (up to 4x, capped) so one slow/retried model call does not consume the
    /// entire conversation budget.
    /// Default: 300s for on-device LLMs (Ollama) which are slower than cloud APIs.
    #[serde(default = "default_channel_message_timeout_secs")]
    pub message_timeout_secs: u64,
    /// Whether to add acknowledgement reactions (👀 on receipt, ✅/⚠️ on
    /// completion) to incoming channel messages. Default: `true`.
    #[serde(default = "default_true")]
    pub ack_reactions: bool,
    /// Whether to send tool-call notification messages (e.g. `🔧 web_search_tool: …`)
    /// to channel users. When `false`, tool calls are still logged server-side but
    /// not forwarded as individual channel messages. Default: `false`.
    #[serde(default = "default_false")]
    pub show_tool_calls: bool,
    /// Persist channel conversation history to JSONL files so sessions survive
    /// daemon restarts. Files are stored in `{workspace}/sessions/`. Default: `true`.
    #[serde(default = "default_true")]
    pub session_persistence: bool,
    /// Session persistence backend: `"jsonl"` (legacy) or `"sqlite"` (new default).
    /// SQLite provides FTS5 search, metadata tracking, and TTL cleanup.
    #[serde(default = "default_session_backend")]
    pub session_backend: String,
    /// Auto-archive stale sessions older than this many hours. `0` disables. Default: `0`.
    #[serde(default)]
    pub session_ttl_hours: u32,
    /// Inbound message debounce window in milliseconds. When a sender fires
    /// multiple messages within this window, they are accumulated and dispatched
    /// as a single concatenated message. `0` disables debouncing. Default: `0`.
    #[serde(default)]
    pub debounce_ms: u64,
}

impl ChannelsConfig {
    /// Backfill `enabled = true` for channel sections present in the raw TOML
    /// that don't have an explicit `enabled` key. This preserves backward
    /// compatibility: configs written before `enabled` was introduced continue
    /// to activate their channels.
    pub fn backfill_enabled(&mut self, raw_toml: &str) {
        let mut table = match raw_toml.parse::<toml::Table>() {
            Ok(t) => t,
            Err(_) => return,
        };
        crate::migration::prepare_table(&mut table);
        let channels = match table.get("channels").and_then(|v| v.as_table()) {
            Some(t) => t,
            None => return,
        };
        for (key, value) in channels {
            let is_section = value.as_table().is_some();
            let has_explicit_enabled = value.as_table().is_some_and(|t| t.contains_key("enabled"));
            if is_section && !has_explicit_enabled {
                // Section exists without explicit `enabled` — backfill true
                let prop_path = format!("channels.{}.enabled", key.replace('_', "-"));
                if let Err(e) = self.set_prop(&prop_path, "true") {
                    tracing::warn!("backfill_enabled: failed to set {prop_path}: {e}");
                }
            }
        }
    }

    /// get channels' metadata and `.is_some()`, except webhook
    #[rustfmt::skip]
    pub fn channels_except_webhook(&self) -> Vec<(Box<dyn crate::traits::ConfigHandle>, bool)> {
        vec![
            (
                Box::new(ConfigWrapper::new(self.telegram.as_ref())),
                self.telegram.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.discord.as_ref())),
                self.discord.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.slack.as_ref())),
                self.slack.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.mattermost.as_ref())),
                self.mattermost.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.imessage.as_ref())),
                self.imessage.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.matrix.as_ref())),
                self.matrix.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.signal.as_ref())),
                self.signal.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.whatsapp.as_ref())),
                self.whatsapp.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.linq.as_ref())),
                self.linq.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.wati.as_ref())),
                self.wati.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.nextcloud_talk.as_ref())),
                self.nextcloud_talk.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.email.as_ref())),
                self.email.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.gmail_push.as_ref())),
                self.gmail_push.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.irc.as_ref())),
                self.irc.is_some()
            ),
            (
                Box::new(ConfigWrapper::new(self.lark.as_ref())),
                self.lark.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.feishu.as_ref())),
                self.feishu.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.dingtalk.as_ref())),
                self.dingtalk.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.wecom.as_ref())),
                self.wecom.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.wechat.as_ref())),
                self.wechat.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.qq.as_ref())),
                self.qq.is_some()
            ),
            #[cfg(feature = "channel-nostr")]
            (
                Box::new(ConfigWrapper::new(self.nostr.as_ref())),
                self.nostr.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.clawdtalk.as_ref())),
                self.clawdtalk.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.reddit.as_ref())),
                self.reddit.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.bluesky.as_ref())),
                self.bluesky.is_some(),
            ),
            #[cfg(feature = "voice-wake")]
            (
                Box::new(ConfigWrapper::new(self.voice_wake.as_ref())),
                self.voice_wake.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.mqtt.as_ref())),
                self.mqtt.is_some(),
            ),
        ]
    }

    /// All channel configs (including webhook) as `(handle, enabled)` pairs.
    pub fn channels(&self) -> Vec<(Box<dyn crate::traits::ConfigHandle>, bool)> {
        let mut ret = self.channels_except_webhook();
        ret.push((
            Box::new(ConfigWrapper::new(self.webhook.as_ref())),
            self.webhook.is_some(),
        ));
        ret
    }
}

impl Default for ChannelsConfig {
    fn default() -> Self {
        Self {
            cli: true,
            telegram: None,
            discord: None,
            discord_history: None,
            slack: None,
            mattermost: None,
            webhook: None,
            imessage: None,
            matrix: None,
            signal: None,
            whatsapp: None,
            linq: None,
            wati: None,
            nextcloud_talk: None,
            email: None,
            gmail_push: None,
            irc: None,
            lark: None,
            line: None,
            feishu: None,
            dingtalk: None,
            wecom: None,
            wechat: None,
            qq: None,
            twitter: None,
            mochat: None,
            #[cfg(feature = "channel-nostr")]
            nostr: None,
            clawdtalk: None,
            reddit: None,
            bluesky: None,
            voice_call: None,
            #[cfg(feature = "voice-wake")]
            voice_wake: None,
            voice_duplex: None,
            mqtt: None,
            message_timeout_secs: default_channel_message_timeout_secs(),
            ack_reactions: true,
            show_tool_calls: false,
            session_persistence: true,
            session_backend: default_session_backend(),
            session_ttl_hours: 0,
            debounce_ms: 0,
        }
    }
}

/// Telegram bot channel configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "channels.telegram"]
pub struct TelegramConfig {
    /// Whether this channel is active (must be explicitly enabled). Default: false.
    #[serde(default)]
    pub enabled: bool,
    /// Telegram Bot API token (from @BotFather).
    #[secret]
    pub bot_token: String,
    /// Allowed Telegram user IDs or usernames. Empty = deny all.
    pub allowed_users: Vec<String>,
    /// Streaming mode for progressive response delivery via message edits.
    #[serde(default)]
    pub stream_mode: StreamMode,
    /// Minimum interval (ms) between draft message edits to avoid rate limits.
    #[serde(default = "default_draft_update_interval_ms")]
    pub draft_update_interval_ms: u64,
    /// When true, a newer Telegram message from the same sender in the same chat
    /// cancels the in-flight request and starts a fresh response with preserved history.
    #[serde(default)]
    pub interrupt_on_new_message: bool,
    /// When true, only respond to messages that @-mention the bot in groups.
    /// Direct messages are always processed.
    #[serde(default)]
    pub mention_only: bool,
    /// Override for the top-level `ack_reactions` setting. When `None`, the
    /// channel falls back to `[channels].ack_reactions`. When set
    /// explicitly, it takes precedence.
    #[serde(default)]
    pub ack_reactions: Option<bool>,
    /// Per-channel proxy URL (http, https, socks5, socks5h).
    /// Overrides the global `[proxy]` setting for this channel only.
    #[serde(default)]
    pub proxy_url: Option<String>,
    /// How long (seconds) to wait for the operator to tap an inline-keyboard
    /// button on a tool approval prompt before auto-denying. Default: 120.
    #[serde(default = "default_telegram_approval_timeout_secs")]
    pub approval_timeout_secs: u64,
    /// When true, each DM chat gets its own forum topic (created via
    /// `createForumTopic`, thread id persisted to a state file across
    /// restarts) and replies are routed into that topic. Hermes
    /// `_setup_dm_topics` / `ensure_dm_topic` parity. Default: false.
    #[serde(default)]
    pub dm_topics_enabled: bool,
    /// Name for the per-chat DM topic. Default: "General".
    #[serde(default = "default_telegram_dm_topic_name")]
    pub dm_topic_name: String,
    /// When true, Telegram link previews are disabled on outbound messages
    /// (`link_preview_options.is_disabled`). Hermes `disable_link_previews`
    /// parity. Default: false (previews on).
    #[serde(default)]
    pub disable_link_previews: bool,
    /// Seconds to suppress Telegram typing-indicator refreshes for a chat
    /// after a transient send failure (rate limit, timeout). Hermes
    /// `typing_cooldown_seconds` parity. Default: 30.
    #[serde(default = "default_telegram_typing_cooldown_secs")]
    pub typing_cooldown_seconds: f64,
    /// Fallback IPs for the Bot API host (e.g. `api.telegram.org`), used to
    /// pin the connection when DNS is broken or poisoned (hermes
    /// `fallback_ips` parity). Each entry is an IP literal; entries are
    /// rotated through on client rebuilds. Default: empty (use DNS).
    #[serde(default)]
    pub fallback_ips: Vec<String>,
}

impl ChannelConfig for TelegramConfig {
    fn name() -> &'static str {
        "Telegram"
    }
    fn desc() -> &'static str {
        "connect your bot"
    }
}

/// Discord bot channel configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "channels.discord"]
#[allow(clippy::struct_excessive_bools)]
pub struct DiscordConfig {
    /// Whether this channel is active (must be explicitly enabled). Default: false.
    #[serde(default)]
    pub enabled: bool,
    /// Discord bot token (from Discord Developer Portal).
    #[secret]
    pub bot_token: String,
    /// Optional guild (server) ID to restrict the bot to a single guild.
    pub guild_id: Option<String>,
    /// Allowed Discord user IDs. Empty = deny all.
    #[serde(default)]
    pub allowed_users: Vec<String>,
    /// When true, process messages from other bots (not just humans).
    /// The bot still ignores its own messages to prevent feedback loops.
    #[serde(default)]
    pub listen_to_bots: bool,
    /// When true, a newer Discord message from the same sender in the same channel
    /// cancels the in-flight request and starts a fresh response with preserved history.
    #[serde(default)]
    pub interrupt_on_new_message: bool,
    /// When true, only respond to messages that @-mention the bot.
    /// Other messages in the guild are silently ignored.
    #[serde(default)]
    pub mention_only: bool,
    /// Per-channel proxy URL (http, https, socks5, socks5h).
    /// Overrides the global `[proxy]` setting for this channel only.
    #[serde(default)]
    pub proxy_url: Option<String>,
    /// Streaming mode for progressive response delivery.
    /// `off` (default): single message. `partial`: editable draft updates.
    /// `multi_message`: split response into separate messages at paragraph boundaries.
    #[serde(default)]
    pub stream_mode: StreamMode,
    /// Minimum interval (ms) between draft message edits to avoid rate limits.
    /// Only used when `stream_mode = "partial"`.
    #[serde(default = "default_draft_update_interval_ms")]
    pub draft_update_interval_ms: u64,
    /// Delay (ms) between sending each message chunk in multi-message mode.
    /// Only used when `stream_mode = "multi_message"`.
    #[serde(default = "default_multi_message_delay_ms")]
    pub multi_message_delay_ms: u64,
    /// Stall-watchdog timeout in seconds. When non-zero, the bot will abort
    /// and retry if no progress is made within this duration. 0 = disabled.
    #[serde(default)]
    pub stall_timeout_secs: u64,
    /// Seconds to wait for operator approval on `always_ask` tools before auto-denying.
    #[serde(default = "default_channel_approval_timeout_secs")]
    pub approval_timeout_secs: u64,
}

impl ChannelConfig for DiscordConfig {
    fn name() -> &'static str {
        "Discord"
    }
    fn desc() -> &'static str {
        "connect your bot"
    }
}

/// Discord history channel — logs ALL messages to discord.db and forwards @mentions to the agent.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "channels.discord-history"]
pub struct DiscordHistoryConfig {
    /// Whether this channel is active (must be explicitly enabled). Default: false.
    #[serde(default)]
    pub enabled: bool,
    /// Discord bot token (from Discord Developer Portal).
    #[secret]
    pub bot_token: String,
    /// Optional guild (server) ID to restrict logging to a single guild.
    pub guild_id: Option<String>,
    /// Allowed Discord user IDs. Empty = allow all (open logging).
    #[serde(default)]
    pub allowed_users: Vec<String>,
    /// Discord channel IDs to watch. Empty = watch all channels.
    #[serde(default)]
    pub channel_ids: Vec<String>,
    /// When true (default), store Direct Messages in discord.db.
    #[serde(default = "default_true")]
    pub store_dms: bool,
    /// When true (default), respond to @mentions in Direct Messages.
    #[serde(default = "default_true")]
    pub respond_to_dms: bool,
    /// Per-channel proxy URL (http, https, socks5, socks5h).
    #[serde(default)]
    pub proxy_url: Option<String>,
}

impl ChannelConfig for DiscordHistoryConfig {
    fn name() -> &'static str {
        "Discord History"
    }
    fn desc() -> &'static str {
        "log all messages and forward @mentions"
    }
}

/// Slack bot channel configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "channels.slack"]
#[allow(clippy::struct_excessive_bools)]
pub struct SlackConfig {
    /// Whether this channel is active (must be explicitly enabled). Default: false.
    #[serde(default)]
    pub enabled: bool,
    /// Slack bot OAuth token (xoxb-...). When omitted from `config.toml`,
    /// resolved at startup from `OPERANT_SLACK_BOT_TOKEN` then
    /// `SLACK_BOT_TOKEN`. Channel construction fails with a clear error
    /// if the token is supplied through neither path. See #6237.
    #[secret]
    #[serde(default)]
    pub bot_token: Option<String>,
    /// Slack app-level token for Socket Mode (xapp-...).
    #[secret]
    pub app_token: Option<String>,
    /// Explicit list of channel IDs to watch.
    /// Empty = listen across all accessible channels.
    /// Migrated from the legacy `channel_id` singular field.
    #[serde(default)]
    pub channel_ids: Vec<String>,
    /// Allowed Slack user IDs. Empty = deny all.
    #[serde(default)]
    pub allowed_users: Vec<String>,
    /// When true, a newer Slack message from the same sender in the same channel
    /// cancels the in-flight request and starts a fresh response with preserved history.
    #[serde(default)]
    pub interrupt_on_new_message: bool,
    /// When true (default), replies stay in the originating Slack thread.
    /// When false, replies go to the channel root instead.
    #[serde(default)]
    pub thread_replies: Option<bool>,
    /// When true, only respond to messages that @-mention the bot in groups.
    /// Direct messages remain allowed.
    #[serde(default)]
    pub mention_only: bool,
    /// When true (and `mention_only` is also true), messages inside a Slack
    /// thread must also @-mention the bot to trigger a response. By default,
    /// thread replies are allowed through without a mention so the bot can
    /// keep a back-and-forth going without the user repeating @-mentions.
    /// Set this to true in channels shared with human discussion where the
    /// bot should stay silent unless explicitly addressed.
    #[serde(default)]
    pub strict_mention_in_thread: bool,
    /// Use the newer Slack `markdown` block type (12 000 char limit, richer formatting).
    /// Defaults to false (uses universally supported `section` blocks with `mrkdwn`).
    /// Enable this only if your Slack workspace supports the `markdown` block type.
    #[serde(default)]
    pub use_markdown_blocks: bool,
    /// Per-channel proxy URL (http, https, socks5, socks5h).
    /// Overrides the global `[proxy]` setting for this channel only.
    #[serde(default)]
    pub proxy_url: Option<String>,
    /// Enable progressive draft message streaming via `chat.update`.
    #[serde(default)]
    pub stream_drafts: bool,
    /// Minimum interval (ms) between draft message edits to avoid Slack rate limits.
    #[serde(default = "default_slack_draft_update_interval_ms")]
    pub draft_update_interval_ms: u64,
    /// Emoji reaction name (without colons) that cancels an in-flight request.
    /// For example, `"x"` means reacting with `:x:` cancels the task.
    /// Leave unset to disable reaction-based cancellation.
    #[serde(default)]
    pub cancel_reaction: Option<String>,
    /// Seconds to wait for operator approval on `always_ask` tools before auto-denying.
    #[serde(default = "default_channel_approval_timeout_secs")]
    pub approval_timeout_secs: u64,
}

impl ChannelConfig for SlackConfig {
    fn name() -> &'static str {
        "Slack"
    }
    fn desc() -> &'static str {
        "connect your bot"
    }
}

/// iMessage channel configuration (macOS only).
#[derive(Debug, Clone, Default, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "channels.imessage"]
pub struct IMessageConfig {
    /// Whether this channel is active (must be explicitly enabled). Default: false.
    #[serde(default)]
    pub enabled: bool,
    /// Allowed iMessage contacts (phone numbers or email addresses). Empty = deny all.
    pub allowed_contacts: Vec<String>,
}

impl ChannelConfig for IMessageConfig {
    fn name() -> &'static str {
        "iMessage"
    }
    fn desc() -> &'static str {
        "macOS only"
    }
}

/// Matrix channel configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "channels.matrix"]
pub struct MatrixConfig {
    /// Whether this channel is active (must be explicitly enabled). Default: false.
    #[serde(default)]
    pub enabled: bool,
    /// Matrix homeserver URL (e.g. `"https://matrix.org"`).
    pub homeserver: String,
    /// Matrix access token for the bot account.
    #[secret]
    pub access_token: String,
    /// Optional Matrix user ID (e.g. `"@bot:matrix.org"`).
    #[serde(default)]
    pub user_id: Option<String>,
    /// Optional Matrix device ID.
    #[serde(default)]
    pub device_id: Option<String>,
    /// Allowed Matrix user IDs. Empty = deny all.
    pub allowed_users: Vec<String>,
    /// Allowed Matrix room IDs or aliases. Empty = allow all rooms.
    /// Supports canonical room IDs (`!abc:server`) and aliases (`#room:server`).
    #[serde(default)]
    pub allowed_rooms: Vec<String>,
    /// Whether to interrupt an in-flight agent response when a new message arrives.
    #[serde(default)]
    pub interrupt_on_new_message: bool,
    /// Streaming mode for progressive response delivery.
    /// `"off"` (default): single message. `"partial"`: edit-in-place draft.
    /// `"multi_message"`: paragraph-split delivery.
    #[serde(default)]
    pub stream_mode: StreamMode,
    /// Minimum interval (ms) between draft message edits in Partial mode.
    #[serde(default = "default_matrix_draft_update_interval_ms")]
    pub draft_update_interval_ms: u64,
    /// Delay (ms) between sending each paragraph in MultiMessage mode.
    #[serde(default = "default_multi_message_delay_ms")]
    pub multi_message_delay_ms: u64,
    /// When true, only respond to messages that @-mention the bot in groups.
    /// Direct messages are always processed.
    #[serde(default)]
    pub mention_only: bool,
    /// Optional Matrix recovery key for automatic E2EE key backup restore.
    /// When set, Operant recovers room keys and cross-signing secrets on startup.
    #[secret]
    #[serde(default)]
    pub recovery_key: Option<String>,
    /// Optional login password for Matrix account (used for initial login flow).
    #[secret]
    #[serde(default)]
    pub password: Option<String>,
    /// Seconds to wait for operator approval on `always_ask` tools before auto-denying.
    #[serde(default = "default_channel_approval_timeout_secs")]
    pub approval_timeout_secs: u64,
    /// When true (default), replies are sent as thread replies. Starts a new thread from the
    /// incoming message when none exists. When false, only continues existing threads.
    #[serde(default = "default_true")]
    pub reply_in_thread: bool,
    /// When true (default), the bot sends acknowledgement reactions while processing
    /// (👀 on receipt, ✅ on completion). Disable to keep rooms reaction-free.
    #[serde(default = "default_true")]
    pub ack_reactions: bool,
}

impl ChannelConfig for MatrixConfig {
    fn name() -> &'static str {
        "Matrix"
    }
    fn desc() -> &'static str {
        "self-hosted chat"
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "channels.signal"]
/// Signal messenger channel configuration (via signal-cli HTTP daemon).
pub struct SignalConfig {
    /// Whether this channel is active (must be explicitly enabled). Default: false.
    #[serde(default)]
    pub enabled: bool,
    /// Base URL for the signal-cli HTTP daemon (e.g. `"http://127.0.0.1:8686"`).
    pub http_url: String,
    /// E.164 phone number of the signal-cli account (e.g. "+1234567890").
    pub account: String,
    /// Optional group ID to filter messages.
    /// - `None` or omitted: accept all messages (DMs and groups)
    /// - `"dm"`: only accept direct messages
    /// - Specific group ID: only accept messages from that group
    #[serde(default)]
    pub group_id: Option<String>,
    /// Allowed sender phone numbers (E.164) or "*" for all.
    #[serde(default)]
    pub allowed_from: Vec<String>,
    /// Skip messages that are attachment-only (no text body).
    #[serde(default)]
    pub ignore_attachments: bool,
    /// Skip incoming story messages.
    #[serde(default)]
    pub ignore_stories: bool,
    /// Per-channel proxy URL (http, https, socks5, socks5h).
    /// Overrides the global `[proxy]` setting for this channel only.
    #[serde(default)]
    pub proxy_url: Option<String>,
    /// Seconds to wait for operator approval on `always_ask` tools before auto-denying.
    #[serde(default = "default_channel_approval_timeout_secs")]
    pub approval_timeout_secs: u64,
}

impl ChannelConfig for SignalConfig {
    fn name() -> &'static str {
        "Signal"
    }
    fn desc() -> &'static str {
        "An open-source, encrypted messaging service"
    }
}

/// WhatsApp Web usage mode.
///
/// `Personal` treats the account as a personal phone — the bot only responds to
/// incoming messages that pass the DM/group/self-chat policy filters.
/// `Business` (default) responds to all incoming messages, subject only to the
/// `allowed_numbers` allowlist.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum WhatsAppWebMode {
    /// Respond to all messages passing the allowlist (default).
    #[default]
    Business,
    /// Apply per-chat-type policies (dm_policy, group_policy, self_chat_mode).
    Personal,
}

/// Policy for a particular WhatsApp chat type (DMs or groups) when
/// `mode = "personal"`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum WhatsAppChatPolicy {
    /// Only respond to senders on the `allowed_numbers` list (default).
    #[default]
    Allowlist,
    /// Ignore all messages in this chat type.
    Ignore,
    /// Respond to every message regardless of allowlist.
    All,
}

/// WhatsApp channel configuration (Cloud API or Web mode).
///
/// Set `phone_number_id` for Cloud API mode, or `session_path` for Web mode.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "channels.whatsapp"]
pub struct WhatsAppConfig {
    /// Whether this channel is active (must be explicitly enabled). Default: false.
    #[serde(default)]
    pub enabled: bool,
    /// Access token from Meta Business Suite (Cloud API mode)
    #[serde(default)]
    #[secret]
    pub access_token: Option<String>,
    /// Phone number ID from Meta Business API (Cloud API mode)
    #[serde(default)]
    pub phone_number_id: Option<String>,
    /// Webhook verify token (you define this, Meta sends it back for verification)
    /// Only used in Cloud API mode
    #[serde(default)]
    #[secret]
    pub verify_token: Option<String>,
    /// App secret from Meta Business Suite (for webhook signature verification)
    /// Can also be set via `OPERANT_WHATSAPP_APP_SECRET` environment variable
    /// Only used in Cloud API mode
    #[serde(default)]
    #[secret]
    pub app_secret: Option<String>,
    /// Session database path for WhatsApp Web client (Web mode)
    /// When set, enables native WhatsApp Web mode with wa-rs
    #[serde(default)]
    pub session_path: Option<String>,
    /// Phone number for pair code linking (Web mode, optional)
    /// Format: country code + number (e.g., "15551234567")
    /// If not set, QR code pairing will be used
    #[serde(default)]
    pub pair_phone: Option<String>,
    /// Custom pair code for linking (Web mode, optional)
    /// Leave empty to let WhatsApp generate one
    #[serde(default)]
    pub pair_code: Option<String>,
    /// Allowed phone numbers (E.164 format: +1234567890) or "*" for all
    #[serde(default)]
    pub allowed_numbers: Vec<String>,
    /// When true, only respond to messages that @-mention the bot in groups (Web mode only).
    /// Direct messages are always processed.
    /// Bot identity is resolved from the wa-rs device at runtime; `pair_phone` seeds it on first connect.
    #[serde(default)]
    pub mention_only: bool,
    /// Usage mode for WhatsApp Web: "business" (default) or "personal".
    /// In personal mode the bot applies dm_policy, group_policy, and
    /// self_chat_mode to decide which chats to respond in.
    #[serde(default)]
    pub mode: WhatsAppWebMode,
    /// Policy for direct messages when mode = "personal".
    /// "allowlist" (default) | "ignore" | "all".
    #[serde(default)]
    pub dm_policy: WhatsAppChatPolicy,
    /// Policy for group chats when mode = "personal".
    /// "allowlist" (default) | "ignore" | "all".
    #[serde(default)]
    pub group_policy: WhatsAppChatPolicy,
    /// When true and mode = "personal", always respond to messages in the
    /// user's own self-chat (Notes to Self). Defaults to false.
    #[serde(default)]
    pub self_chat_mode: bool,
    /// Regex patterns for DM mention gating (case-insensitive).
    /// When non-empty, only direct messages matching at least one pattern are
    /// processed; matched fragments are stripped from the forwarded content.
    /// Example: `["@?Operant", "\\+?15555550123"]`
    #[serde(default)]
    pub dm_mention_patterns: Vec<String>,
    /// Regex patterns for group-chat mention gating (case-insensitive).
    /// When non-empty, only group messages matching at least one pattern are
    /// processed; matched fragments are stripped from the forwarded content.
    /// Example: `["@?Operant", "\\+?15555550123"]`
    #[serde(default)]
    pub group_mention_patterns: Vec<String>,
    /// Per-channel proxy URL (http, https, socks5, socks5h).
    /// Overrides the global `[proxy]` setting for this channel only.
    #[serde(default)]
    pub proxy_url: Option<String>,
    /// Seconds to wait for operator approval on `always_ask` tools before auto-denying.
    #[serde(default = "default_channel_approval_timeout_secs")]
    pub approval_timeout_secs: u64,
}

impl ChannelConfig for WhatsAppConfig {
    fn name() -> &'static str {
        "WhatsApp"
    }
    fn desc() -> &'static str {
        "Business Cloud API"
    }
}

/// WATI WhatsApp Business API channel configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "channels.wati"]
pub struct WatiConfig {
    /// Whether this channel is active (must be explicitly enabled). Default: false.
    #[serde(default)]
    pub enabled: bool,
    /// WATI API token (Bearer auth).
    #[secret]
    pub api_token: String,
    /// WATI API base URL (default: <https://live-mt-server.wati.io>).
    #[serde(default = "default_wati_api_url")]
    pub api_url: String,
    /// Tenant ID for multi-channel setups (optional).
    #[serde(default)]
    pub tenant_id: Option<String>,
    /// Allowed phone numbers (E.164 format) or "*" for all.
    #[serde(default)]
    pub allowed_numbers: Vec<String>,
    /// Per-channel proxy URL (http, https, socks5, socks5h).
    /// Overrides the global `[proxy]` setting for this channel only.
    #[serde(default)]
    pub proxy_url: Option<String>,
}

impl ChannelConfig for WatiConfig {
    fn name() -> &'static str {
        "WATI"
    }
    fn desc() -> &'static str {
        "WhatsApp via WATI Business API"
    }
}

impl WhatsAppConfig {
    /// Detect which backend to use based on config fields.
    /// Returns "cloud" if phone_number_id is set, "web" if session_path is set.
    pub fn backend_type(&self) -> &'static str {
        if self.phone_number_id.is_some() {
            "cloud"
        } else if self.session_path.is_some() {
            "web"
        } else {
            // Default to Cloud API for backward compatibility
            "cloud"
        }
    }

    /// Check if this is a valid Cloud API config
    pub fn is_cloud_config(&self) -> bool {
        self.phone_number_id.is_some() && self.access_token.is_some() && self.verify_token.is_some()
    }

    /// Check if this is a valid Web config
    pub fn is_web_config(&self) -> bool {
        self.session_path.is_some()
    }

    /// Returns true when both Cloud and Web selectors are present.
    ///
    /// Runtime currently prefers Cloud mode in this case for backward compatibility.
    pub fn is_ambiguous_config(&self) -> bool {
        self.phone_number_id.is_some() && self.session_path.is_some()
    }
}

/// MQTT channel configuration (SOP listener).
///
/// Subscribes to MQTT topics and dispatches incoming messages
/// to the SOP engine for processing.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "channels.mqtt"]
pub struct MqttConfig {
    /// Whether this channel is active (must be explicitly enabled). Default: false.
    #[serde(default)]
    pub enabled: bool,
    /// MQTT broker URL (e.g., `mqtt://localhost:1883` or `mqtts://broker.example.com:8883`).
    /// Use `mqtt://` for plain connections or `mqtts://` for TLS.
    pub broker_url: String,
    /// MQTT client ID (must be unique per broker).
    pub client_id: String,
    /// Topics to subscribe to (e.g., `sensors/#`, `alerts/+/critical`).
    /// At least one topic is required.
    #[serde(default)]
    pub topics: Vec<String>,
    /// MQTT QoS level (0 = at-most-once, 1 = at-least-once, 2 = exactly-once). Default: 1.
    #[serde(default = "default_mqtt_qos")]
    pub qos: u8,
    /// Username for authentication (optional).
    pub username: Option<String>,
    /// Password for authentication (optional).
    #[secret]
    pub password: Option<String>,
    /// Enable TLS encryption. Must match the broker_url scheme:
    /// - `mqtt://` → `use_tls: false`
    /// - `mqtts://` → `use_tls: true`
    #[serde(default)]
    pub use_tls: bool,
    /// Keep-alive interval in seconds (default: 30). Prevents broker disconnect on idle.
    #[serde(default = "default_mqtt_keep_alive_secs")]
    pub keep_alive_secs: u64,
}

impl MqttConfig {
    /// Validate the MQTT configuration.
    ///
    /// Checks:
    /// - QoS is 0, 1, or 2
    /// - broker_url uses valid scheme (`mqtt://` or `mqtts://`)
    /// - `use_tls` flag matches broker_url scheme
    /// - At least one topic is configured
    /// - client_id is non-empty
    pub fn validate(&self) -> anyhow::Result<()> {
        // QoS validation
        if self.qos > 2 {
            anyhow::bail!("qos must be 0, 1, or 2, got {}", self.qos);
        }

        // Broker URL validation
        let is_tls_scheme = self.broker_url.starts_with("mqtts://");
        let is_mqtt_scheme = self.broker_url.starts_with("mqtt://");

        if !is_tls_scheme && !is_mqtt_scheme {
            anyhow::bail!(
                "broker_url must start with 'mqtt://' or 'mqtts://', got: {}",
                self.broker_url
            );
        }

        // TLS flag validation
        if is_mqtt_scheme && self.use_tls {
            anyhow::bail!("use_tls is true but broker_url uses 'mqtt://' (not 'mqtts://')");
        }

        if is_tls_scheme && !self.use_tls {
            anyhow::bail!(
                "use_tls is false but broker_url uses 'mqtts://' (requires use_tls: true)"
            );
        }

        // Topics validation
        if self.topics.is_empty() {
            anyhow::bail!("at least one topic must be configured");
        }

        // Client ID validation
        if self.client_id.is_empty() {
            validation_bail!(
                RequiredFieldEmpty,
                "client_id",
                "client_id must not be empty"
            );
        }

        Ok(())
    }
}

impl ChannelConfig for MqttConfig {
    fn name() -> &'static str {
        "MQTT"
    }
    fn desc() -> &'static str {
        "MQTT SOP Listener"
    }
}

/// IRC channel configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "channels.irc"]
pub struct IrcConfig {
    /// Whether this channel is active (must be explicitly enabled). Default: false.
    #[serde(default)]
    pub enabled: bool,
    /// IRC server hostname
    pub server: String,
    /// IRC server port (default: 6697 for TLS)
    #[serde(default = "default_irc_port")]
    pub port: u16,
    /// Bot nickname
    pub nickname: String,
    /// Username (defaults to nickname if not set)
    pub username: Option<String>,
    /// Channels to join on connect
    #[serde(default)]
    pub channels: Vec<String>,
    /// Allowed nicknames (case-insensitive) or "*" for all
    #[serde(default)]
    pub allowed_users: Vec<String>,
    /// Server password (for bouncers like ZNC)
    #[secret]
    pub server_password: Option<String>,
    /// NickServ IDENTIFY password
    #[secret]
    pub nickserv_password: Option<String>,
    /// SASL PLAIN password (IRCv3)
    #[secret]
    pub sasl_password: Option<String>,
    /// Verify TLS certificate (default: true)
    pub verify_tls: Option<bool>,
    /// When true, only respond to messages that mention the bot.
    /// Other messages in the channel are silently ignored.
    #[serde(default)]
    pub mention_only: bool,
}

impl ChannelConfig for IrcConfig {
    fn name() -> &'static str {
        "IRC"
    }
    fn desc() -> &'static str {
        "IRC over TLS"
    }
}

/// Feishu configuration for messaging integration.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "channels.feishu"]
pub struct FeishuConfig {
    /// Whether this channel is active (must be explicitly enabled). Default: false.
    #[serde(default)]
    pub enabled: bool,
    /// App ID from Feishu developer console
    pub app_id: String,
    /// App Secret from Feishu developer console
    #[secret]
    pub app_secret: String,
    /// Encrypt key for webhook message decryption (optional)
    #[serde(default)]
    #[secret]
    pub encrypt_key: Option<String>,
    /// Verification token for webhook validation (optional)
    #[serde(default)]
    #[secret]
    pub verification_token: Option<String>,
    /// Allowed user IDs or union IDs (empty = deny all, "*" = allow all)
    #[serde(default)]
    pub allowed_users: Vec<String>,
    /// When true, only respond to messages that @-mention the bot in groups.
    /// Direct messages are always processed.
    #[serde(default)]
    pub mention_only: bool,
    /// Event receive mode: "websocket" (default) or "webhook"
    #[serde(default)]
    pub receive_mode: LarkReceiveMode,
    /// HTTP port for webhook mode only. Must be set when receive_mode = "webhook".
    /// Not required (and ignored) for websocket mode.
    #[serde(default)]
    pub port: Option<u16>,
    /// Per-channel proxy URL (http, https, socks5, socks5h).
    /// Overrides the global `[proxy]` setting for this channel only.
    #[serde(default)]
    pub proxy_url: Option<String>,
}

impl ChannelConfig for FeishuConfig {
    fn name() -> &'static str {
        "Feishu"
    }
    fn desc() -> &'static str {
        "Feishu Bot"
    }
}

/// Nevis IAM integration configuration.
///
/// When `enabled` is true, Operant validates incoming requests against a Nevis
/// Security Suite instance and maps Nevis roles to tool/workspace permissions.
#[derive(Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "security.nevis"]
#[serde(deny_unknown_fields)]
pub struct NevisConfig {
    /// Enable Nevis IAM integration. Defaults to false for backward compatibility.
    #[serde(default)]
    pub enabled: bool,

    /// Base URL of the Nevis instance (e.g. `https://nevis.example.com`).
    #[serde(default)]
    pub instance_url: String,

    /// Nevis realm to authenticate against.
    #[serde(default = "default_nevis_realm")]
    pub realm: String,

    /// OAuth2 client ID registered in Nevis.
    #[serde(default)]
    pub client_id: String,

    /// OAuth2 client secret. Encrypted via SecretStore when stored on disk.
    #[serde(default)]
    #[secret]
    pub client_secret: Option<String>,

    /// Token validation strategy: `"local"` (JWKS) or `"remote"` (introspection).
    #[serde(default = "default_nevis_token_validation")]
    pub token_validation: String,

    /// JWKS endpoint URL for local token validation.
    #[serde(default)]
    pub jwks_url: Option<String>,

    /// Nevis role to Operant permission mappings.
    #[serde(default)]
    pub role_mapping: Vec<NevisRoleMappingConfig>,

    /// Require MFA verification for all Nevis-authenticated requests.
    #[serde(default)]
    pub require_mfa: bool,

    /// Session timeout in seconds.
    #[serde(default = "default_nevis_session_timeout_secs")]
    pub session_timeout_secs: u64,
}

impl std::fmt::Debug for NevisConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NevisConfig")
            .field("enabled", &self.enabled)
            .field("instance_url", &self.instance_url)
            .field("realm", &self.realm)
            .field("client_id", &self.client_id)
            .field(
                "client_secret",
                &self.client_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .field("token_validation", &self.token_validation)
            .field("jwks_url", &self.jwks_url)
            .field("role_mapping", &self.role_mapping)
            .field("require_mfa", &self.require_mfa)
            .field("session_timeout_secs", &self.session_timeout_secs)
            .finish()
    }
}

impl NevisConfig {
    /// Validate that required fields are present when Nevis is enabled.
    ///
    /// Call at config load time to fail fast on invalid configuration rather
    /// than deferring errors to the first authentication request.
    pub fn validate(&self) -> Result<(), String> {
        if !self.enabled {
            return Ok(());
        }

        if self.instance_url.trim().is_empty() {
            return Err("nevis.instance_url is required when Nevis IAM is enabled".into());
        }

        if self.client_id.trim().is_empty() {
            return Err("nevis.client_id is required when Nevis IAM is enabled".into());
        }

        if self.realm.trim().is_empty() {
            return Err("nevis.realm is required when Nevis IAM is enabled".into());
        }

        match self.token_validation.as_str() {
            "local" | "remote" => {}
            other => {
                return Err(format!(
                    "nevis.token_validation has invalid value '{other}': \
                     expected 'local' or 'remote'"
                ));
            }
        }

        if self.token_validation == "local" && self.jwks_url.is_none() {
            return Err("nevis.jwks_url is required when token_validation is 'local'".into());
        }

        if self.session_timeout_secs == 0 {
            return Err("nevis.session_timeout_secs must be greater than 0".into());
        }

        Ok(())
    }
}

impl Default for NevisConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            instance_url: String::new(),
            realm: default_nevis_realm(),
            client_id: String::new(),
            client_secret: None,
            token_validation: default_nevis_token_validation(),
            jwks_url: None,
            role_mapping: Vec::new(),
            require_mfa: false,
            session_timeout_secs: default_nevis_session_timeout_secs(),
        }
    }
}

/// Maps a Nevis role to Operant tool permissions and workspace access.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct NevisRoleMappingConfig {
    /// Nevis role name (case-insensitive).
    pub nevis_role: String,

    /// Tool names this role can access. Use `"all"` for unrestricted tool access.
    #[serde(default)]
    pub operant_permissions: Vec<String>,

    /// Workspace names this role can access. Use `"all"` for unrestricted.
    #[serde(default)]
    pub workspace_access: Vec<String>,
}

/// DingTalk configuration for Stream Mode messaging
#[derive(Debug, Clone, Default, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "channels.dingtalk"]
pub struct DingTalkConfig {
    /// Whether this channel is active (must be explicitly enabled). Default: false.
    #[serde(default)]
    pub enabled: bool,
    /// Client ID (AppKey) from DingTalk developer console
    pub client_id: String,
    /// Client Secret (AppSecret) from DingTalk developer console
    #[secret]
    pub client_secret: String,
    /// Allowed user IDs (staff IDs). Empty = deny all, "*" = allow all
    #[serde(default)]
    pub allowed_users: Vec<String>,
    /// Per-channel proxy URL (http, https, socks5, socks5h).
    /// Overrides the global `[proxy]` setting for this channel only.
    #[serde(default)]
    pub proxy_url: Option<String>,
}

impl ChannelConfig for DingTalkConfig {
    fn name() -> &'static str {
        "DingTalk"
    }
    fn desc() -> &'static str {
        "DingTalk Stream Mode"
    }
}

/// QQ Official Bot configuration (Tencent QQ Bot SDK)
#[derive(Debug, Clone, Default, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "channels.qq"]
pub struct QQConfig {
    /// Whether this channel is active (must be explicitly enabled). Default: false.
    #[serde(default)]
    pub enabled: bool,
    /// App ID from QQ Bot developer console
    pub app_id: String,
    /// App Secret from QQ Bot developer console
    #[secret]
    pub app_secret: String,
    /// Allowed user IDs. Empty = deny all, "*" = allow all
    #[serde(default)]
    pub allowed_users: Vec<String>,
    /// Per-channel proxy URL (http, https, socks5, socks5h).
    /// Overrides the global `[proxy]` setting for this channel only.
    #[serde(default)]
    pub proxy_url: Option<String>,
}

impl ChannelConfig for QQConfig {
    fn name() -> &'static str {
        "QQ Official"
    }
    fn desc() -> &'static str {
        "Tencent QQ Bot"
    }
}

/// Mochat channel configuration (Mochat customer service API)
#[derive(Debug, Clone, Default, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "channels.mochat"]
pub struct MochatConfig {
    /// Whether this channel is active (must be explicitly enabled). Default: false.
    #[serde(default)]
    pub enabled: bool,
    /// Mochat API base URL
    pub api_url: String,
    /// Mochat API token
    #[secret]
    pub api_token: String,
    /// Allowed user IDs. Empty = deny all, "*" = allow all
    #[serde(default)]
    pub allowed_users: Vec<String>,
    /// Poll interval in seconds for new messages. Default: 5
    #[serde(default = "default_mochat_poll_interval")]
    pub poll_interval_secs: u64,
}

impl ChannelConfig for MochatConfig {
    fn name() -> &'static str {
        "Mochat"
    }
    fn desc() -> &'static str {
        "Mochat Customer Service"
    }
}

// -- Notion --

/// Notion integration configuration (`[notion]`).
///
/// When `enabled = true`, the agent polls a Notion database for pending tasks
/// and exposes a `notion` tool for querying, reading, creating, and updating pages.
/// Requires `api_key` (or the `NOTION_API_KEY` env var) and `database_id`.
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "notion"]
pub struct NotionConfig {
    /// Whether the Notion integration is enabled. Default: `false`.
    #[serde(default)]
    pub enabled: bool,
    /// Notion integration API key (secret).
    #[serde(default)]
    #[secret]
    pub api_key: String,
    /// Notion database ID the agent polls for tasks.
    #[serde(default)]
    pub database_id: String,
    /// Poll interval in seconds. Default: `5`.
    #[serde(default = "default_notion_poll_interval")]
    pub poll_interval_secs: u64,
    /// Database property holding task status. Default: `"Status"`.
    #[serde(default = "default_notion_status_prop")]
    pub status_property: String,
    /// Database property receiving task inputs. Default: `"Input"`.
    #[serde(default = "default_notion_input_prop")]
    pub input_property: String,
    /// Database property receiving task results. Default: `"Result"`.
    #[serde(default = "default_notion_result_prop")]
    pub result_property: String,
    /// Maximum concurrent task executions. Default: `1`.
    #[serde(default = "default_notion_max_concurrent")]
    pub max_concurrent: usize,
    /// Recover stale in-flight tasks on startup. Default: `false`.
    #[serde(default = "default_notion_recover_stale")]
    pub recover_stale: bool,
}

impl Default for NotionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            api_key: String::new(),
            database_id: String::new(),
            poll_interval_secs: default_notion_poll_interval(),
            status_property: default_notion_status_prop(),
            input_property: default_notion_input_prop(),
            result_property: default_notion_result_prop(),
            max_concurrent: default_notion_max_concurrent(),
            recover_stale: default_notion_recover_stale(),
        }
    }
}

/// Jira integration configuration (`[jira]`).
///
/// When `enabled = true`, registers the `jira` tool which can get tickets,
/// search with JQL, and add comments. Requires `base_url` and `api_token`
/// (or the `JIRA_API_TOKEN` env var).
///
/// ## Defaults
/// - `enabled`: `false`
/// - `allowed_actions`: `["get_ticket"]` — read-only by default.
///   Add `"search_tickets"` or `"comment_ticket"` to unlock them.
/// - `timeout_secs`: `30`
///
/// ## Auth
/// Jira Cloud uses HTTP Basic auth: `email` + `api_token`.
/// Jira Server/Data Center uses Bearer token auth: omit `email` and set
/// `api_token` to a personal access token.
/// `api_token` is stored encrypted at rest; set it here or via `JIRA_API_TOKEN`.
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "jira"]
pub struct JiraConfig {
    /// Enable the `jira` tool. Default: `false`.
    #[serde(default)]
    pub enabled: bool,
    /// Atlassian instance base URL, e.g. `https://yourco.atlassian.net`.
    #[serde(default)]
    pub base_url: String,
    /// Jira account email used for Basic auth (Cloud).
    /// Omit for Server/DC deployments using Bearer token auth.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    /// Jira API token. Encrypted at rest. Falls back to `JIRA_API_TOKEN` env var.
    #[serde(default)]
    #[secret]
    pub api_token: String,
    /// Actions the agent is permitted to call.
    /// Valid values: `"get_ticket"`, `"search_tickets"`, `"comment_ticket"`.
    /// Defaults to `["get_ticket"]` (read-only).
    #[serde(default = "default_jira_allowed_actions")]
    pub allowed_actions: Vec<String>,
    /// Request timeout in seconds. Default: `30`.
    #[serde(default = "default_jira_timeout_secs")]
    pub timeout_secs: u64,
}

impl Default for JiraConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            base_url: String::new(),
            email: None,
            api_token: String::new(),
            allowed_actions: default_jira_allowed_actions(),
            timeout_secs: default_jira_timeout_secs(),
        }
    }
}
