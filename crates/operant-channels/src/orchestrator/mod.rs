//! Channel subsystem for messaging platform integrations.
//!
//! This module provides the multi-channel messaging infrastructure that connects
//! Operant to external platforms. Each channel implements the [`Channel`] trait
//! defined in the `traits` submodule, which provides a uniform interface for
//! sending messages, listening for incoming messages, health checking, and typing
//! indicators.
//!
//! Channels are instantiated by [`start_channels`] based on the runtime configuration.
//! The subsystem manages per-sender conversation history, concurrent message processing
//! with configurable parallelism, and exponential-backoff reconnection for resilience.
//!
//! # Extension
//!
//! To add a new channel, implement [`Channel`] in a new top-level module in
//! `operant-channels/src/`, declare it in `lib.rs` behind the appropriate feature
//! gate, and wire it into [`start_channels`] here. See `AGENTS.md` §7.2 for the
//! full change playbook.

#[cfg(feature = "channel-acp-server")]
pub mod acp_server;
pub mod media_pipeline;
#[cfg(feature = "channel-mqtt")]
pub mod mqtt;

// Channel types imported directly from source crates (no shim files)
// Each gated to match the feature declarations in lib.rs
#[cfg(feature = "channel-bluesky")]
pub use crate::bluesky::BlueskyChannel;
#[cfg(feature = "channel-clawdtalk")]
pub use crate::clawdtalk::ClawdTalkChannel;
#[cfg(feature = "channel-dingtalk")]
pub use crate::dingtalk::DingTalkChannel;
pub use crate::discord::DiscordChannel;
pub use crate::discord_history::DiscordHistoryChannel;
#[cfg(feature = "channel-email")]
pub use crate::email_channel::EmailChannel;
#[cfg(feature = "channel-email")]
pub use crate::gmail_push::GmailPushChannel;
#[cfg(feature = "channel-imessage")]
pub use crate::imessage::IMessageChannel;
#[cfg(feature = "channel-irc")]
pub use crate::irc::IrcChannel;
#[cfg(feature = "channels-vendor")]
pub use crate::lark::LarkChannel;
#[cfg(feature = "channels-vendor")]
pub use crate::line::LineChannel;
#[cfg(feature = "channel-linq")]
pub use crate::linq::LinqChannel;
#[cfg(feature = "channel-mattermost")]
pub use crate::mattermost::MattermostChannel;
#[cfg(feature = "channel-mochat")]
pub use crate::mochat::MochatChannel;
#[cfg(feature = "channel-nextcloud")]
pub use crate::nextcloud_talk::NextcloudTalkChannel;
#[cfg(feature = "channels-vendor")]
pub use crate::nostr::NostrChannel;
#[cfg(feature = "channel-notion")]
pub use crate::notion::NotionChannel;
#[cfg(feature = "channel-qq")]
pub use crate::qq::QQChannel;
#[cfg(feature = "channel-reddit")]
pub use crate::reddit::RedditChannel;
#[cfg(feature = "channel-signal")]
pub use crate::signal::SignalChannel;
pub use crate::slack::SlackChannel;
pub use crate::transcription;
pub use crate::tts::{TtsManager, TtsProvider};
#[cfg(feature = "channel-twitter")]
pub use crate::twitter::TwitterChannel;
#[cfg(feature = "channels-vendor")]
pub use crate::voice_call::VoiceCallChannel;
#[cfg(feature = "channels-vendor")]
pub use crate::voice_wake::VoiceWakeChannel;
#[cfg(feature = "channel-wati")]
pub use crate::wati::WatiChannel;
pub use crate::webhook::WebhookChannel;
#[cfg(feature = "channels-vendor")]
pub use crate::wechat::WeChatChannel;
#[cfg(feature = "channel-wecom")]
pub use crate::wecom::WeComChannel;
#[cfg(feature = "channel-whatsapp-cloud")]
pub use crate::whatsapp::WhatsAppChannel;
pub use operant_api::channel::{Channel, ChannelMessage, SendMessage};
// Local channel types (in misc, not operant-channels)
pub use crate::cli::CliChannel;
pub use crate::link_enricher;
#[cfg(feature = "channels-vendor")]
pub use crate::matrix::MatrixChannel;
#[cfg(feature = "channel-telegram")]
pub use crate::telegram::TelegramChannel;
#[cfg(feature = "channels-vendor")]
pub use crate::whatsapp_web::WhatsAppWebChannel;
pub use operant_infra::debounce::MessageDebouncer;
pub use operant_infra::session_backend::SessionBackend;
pub use operant_infra::session_sqlite::SqliteSessionBackend;
pub use operant_infra::stall_watchdog::StallWatchdog;

use operant_memory;
use operant_providers::{self};
use operant_runtime::observability::Observer;
use operant_runtime::observability::traits::{ObserverEvent, ObserverMetric};
use operant_runtime::util::truncate_with_ellipsis;
use portable_atomic::Ordering;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

// Concern-group modules extracted verbatim from the former monolith (see BUGS.md).
mod commands;
mod consts;
mod dispatch;
mod factory;
mod health;
mod history;
mod identity;
mod memory_ctx;
mod prompts;
mod routing;
mod runtime_types;
mod sanitize;
mod startup;
mod supervision;
#[cfg(test)]
mod tests;

pub(crate) use commands::*;
pub(crate) use consts::*;
pub(crate) use dispatch::*;
pub use factory::*;
pub use health::*;
pub use history::*;
pub use identity::*;
pub(crate) use memory_ctx::*;
pub(crate) use prompts::*;
pub(crate) use routing::*;
pub(crate) use runtime_types::*;
pub(crate) use sanitize::*;
pub use startup::*;
pub(crate) use supervision::*;

/// Observer wrapper that forwards tool-call events to a channel sender
/// for real-time threaded notifications.
struct ChannelNotifyObserver {
    inner: Arc<dyn Observer>,
    tx: tokio::sync::mpsc::UnboundedSender<String>,
    tools_used: AtomicBool,
}

impl Observer for ChannelNotifyObserver {
    fn record_event(&self, event: &ObserverEvent) {
        if let ObserverEvent::ToolCallStart { tool, arguments } = event {
            self.tools_used.store(true, Ordering::Relaxed);
            let detail = match arguments {
                Some(args) if !args.is_empty() => {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(args) {
                        if let Some(cmd) = v.get("command").and_then(|c| c.as_str()) {
                            format!(": `{}`", truncate_with_ellipsis(cmd, 200))
                        } else if let Some(q) = v.get("query").and_then(|c| c.as_str()) {
                            format!(": {}", truncate_with_ellipsis(q, 200))
                        } else if let Some(p) = v.get("path").and_then(|c| c.as_str()) {
                            format!(": {p}")
                        } else if let Some(u) = v.get("url").and_then(|c| c.as_str()) {
                            format!(": {u}")
                        } else {
                            let s = args.to_string();
                            format!(": {}", truncate_with_ellipsis(&s, 120))
                        }
                    } else {
                        let s = args.to_string();
                        format!(": {}", truncate_with_ellipsis(&s, 120))
                    }
                }
                _ => String::new(),
            };
            let _ = self.tx.send(format!("\u{1F527} `{tool}`{detail}"));
        }
        self.inner.record_event(event);
    }
    fn record_metric(&self, metric: &ObserverMetric) {
        self.inner.record_metric(metric);
    }
    fn flush(&self) {
        self.inner.flush();
    }
    fn name(&self) -> &str {
        "channel-notify"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Deliver a cron job announcement to a configured channel.
/// Scans for credential leaks before delivery.
///
/// `thread_id` is forwarded to channels whose outbound `thread_id` is distinct
/// from the recipient (notably the webhook channel, which serialises both into
/// the JSON callback). For channels that do not honour `thread_ts` it is a
/// harmless no-op.
pub async fn deliver_announcement(
    config: &operant_config::schema::Config,
    channel: &str,
    target: &str,
    thread_id: Option<String>,
    output: &str,
) -> anyhow::Result<()> {
    use operant_api::channel::SendMessage;

    // Scan for credential leaks before delivering
    let leak_detector = operant_runtime::security::LeakDetector::new();
    let safe_output = match leak_detector.scan(output) {
        operant_runtime::security::LeakResult::Detected { redacted, .. } => redacted,
        operant_runtime::security::LeakResult::Clean => output.to_string(),
    };

    let make_msg = |s: &str| SendMessage::new(s, target).in_thread(thread_id.clone());

    // Use the live channel instance when available — critical for Matrix E2EE which must
    // reuse the authenticated client rather than re-running session restore per delivery.
    if let Some(registry) = CRON_CHANNEL_REGISTRY.get()
        && let Some(ch) = registry.get(channel.to_ascii_lowercase().as_str())
    {
        return ch.send(&make_msg(&safe_output)).await;
    }

    match channel.to_ascii_lowercase().as_str() {
        #[cfg(feature = "channel-telegram")]
        "telegram" => {
            let tg = config
                .channels
                .telegram
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("telegram channel not configured"))?;
            let ch = TelegramChannel::new(
                tg.bot_token.clone(),
                tg.allowed_users.clone(),
                tg.mention_only,
            );
            operant_api::channel::Channel::send(&ch, &make_msg(&safe_output)).await?;
        }
        "discord" => {
            let dc = config
                .channels
                .discord
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("discord channel not configured"))?;
            let ch = DiscordChannel::new(
                dc.bot_token.clone(),
                dc.guild_id.clone(),
                dc.allowed_users.clone(),
                dc.listen_to_bots,
                dc.mention_only,
            )
            .with_workspace_dir(config.workspace_dir.clone());
            operant_api::channel::Channel::send(&ch, &make_msg(&safe_output)).await?;
        }
        "slack" => {
            let sl = config
                .channels
                .slack
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("slack channel not configured"))?;
            let bot_token = sl
                .bot_token
                .clone()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Slack channel requires a bot token. Set `bot_token` in \
                         `[channels.slack]` of config.toml, or export \
                         OPERANT_SLACK_BOT_TOKEN / SLACK_BOT_TOKEN before \
                         starting Operant."
                    )
                })?;
            let ch = SlackChannel::new(
                bot_token,
                sl.app_token.clone(),
                sl.channel_ids.clone(),
                sl.allowed_users.clone(),
            )
            .with_workspace_dir(config.workspace_dir.clone());
            operant_api::channel::Channel::send(&ch, &make_msg(&safe_output)).await?;
        }
        #[cfg(feature = "channel-signal")]
        "signal" => {
            let sg = config
                .channels
                .signal
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("signal channel not configured"))?;
            let ch = SignalChannel::new(
                sg.http_url.clone(),
                sg.account.clone(),
                sg.group_id.clone(),
                sg.allowed_from.clone(),
                sg.ignore_attachments,
                sg.ignore_stories,
            );
            operant_api::channel::Channel::send(&ch, &make_msg(&safe_output)).await?;
        }
        #[cfg(all(feature = "channel-wechat", feature = "channels-vendor"))]
        "wechat" => {
            let wc = config
                .channels
                .wechat
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("wechat channel not configured"))?;
            let ch = WeChatChannel::new(
                wc.allowed_users.clone(),
                wc.api_base_url.clone(),
                wc.cdn_base_url.clone(),
                wc.state_dir.as_ref().map(std::path::PathBuf::from),
            )?
            .with_workspace_dir(config.workspace_dir.clone());
            operant_api::channel::Channel::send(&ch, &make_msg(&safe_output)).await?;
        }
        #[cfg(not(all(feature = "channel-wechat", feature = "channels-vendor")))]
        "wechat" => {
            anyhow::bail!("WeChat channel requires the `channel-wechat` feature");
        }
        "webhook" => {
            let wh = config
                .channels
                .webhook
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("webhook channel not configured"))?;
            let ch = WebhookChannel::new(
                wh.port,
                wh.listen_path.clone(),
                wh.send_url.clone(),
                wh.send_method.clone(),
                wh.auth_header.clone(),
                wh.secret.clone(),
            );
            operant_api::channel::Channel::send(&ch, &make_msg(&safe_output)).await?;
        }
        other => anyhow::bail!("unsupported delivery channel: {other}"),
    }
    Ok(())
}
