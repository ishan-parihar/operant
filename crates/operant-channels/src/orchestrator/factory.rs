//! `factory` — extracted verbatim from the former orchestrator/mod.rs monolith.
//! Re-exported from `orchestrator` so every import path is unchanged.

use anyhow::{Context, Result};
use operant_config::schema::Config;
use operant_memory::{self};
use std::sync::Arc;

use super::*;

/// Build a single channel instance by config section name (e.g. "telegram").
pub(crate) fn build_channel_by_id(config: &Config, channel_id: &str) -> Result<Arc<dyn Channel>> {
    match channel_id {
        #[cfg(feature = "channel-telegram")]
        "telegram" => {
            let tg = config
                .channels
                .telegram
                .as_ref()
                .context("Telegram channel is not configured")?;
            let ack = tg.ack_reactions.unwrap_or(config.channels.ack_reactions);
            Ok(Arc::new(
                TelegramChannel::new(
                    tg.bot_token.clone(),
                    tg.allowed_users.clone(),
                    tg.mention_only,
                )
                .with_ack_reactions(ack)
                .with_streaming(tg.stream_mode, tg.draft_update_interval_ms)
                .with_transcription(config.transcription.clone())
                .with_tts(config.tts.clone())
                .with_workspace_dir(config.workspace_dir.clone())
                .with_approval_timeout_secs(tg.approval_timeout_secs)
                .with_dm_topics(tg.dm_topics_enabled, tg.dm_topic_name.clone())
                .with_link_previews(!tg.disable_link_previews)
                .with_typing_cooldown_secs(tg.typing_cooldown_seconds)
                .with_fallback_ips(tg.fallback_ips.clone()),
            ))
        }
        "discord" => {
            let dc = config
                .channels
                .discord
                .as_ref()
                .context("Discord channel is not configured")?;
            Ok(Arc::new(
                DiscordChannel::new(
                    dc.bot_token.clone(),
                    dc.guild_id.clone(),
                    dc.allowed_users.clone(),
                    dc.listen_to_bots,
                    dc.mention_only,
                )
                .with_workspace_dir(config.workspace_dir.clone())
                .with_streaming(
                    dc.stream_mode,
                    dc.draft_update_interval_ms,
                    dc.multi_message_delay_ms,
                )
                .with_transcription(config.transcription.clone())
                .with_stall_timeout(dc.stall_timeout_secs)
                .with_approval_timeout_secs(dc.approval_timeout_secs),
            ))
        }
        "slack" => {
            let sl = config
                .channels
                .slack
                .as_ref()
                .context("Slack channel is not configured")?;
            let bot_token = sl.bot_token.clone().filter(|s| !s.is_empty()).context(
                "Slack channel requires a bot token. Set `bot_token` in \
                     `[channels.slack]` of config.toml, or export \
                     `OPERANT_SLACK_BOT_TOKEN` / `SLACK_BOT_TOKEN` before \
                     starting Operant.",
            )?;
            Ok(Arc::new(
                SlackChannel::new(
                    bot_token,
                    sl.app_token.clone(),
                    sl.channel_ids.clone(),
                    sl.allowed_users.clone(),
                )
                .with_workspace_dir(config.workspace_dir.clone())
                .with_markdown_blocks(sl.use_markdown_blocks)
                .with_transcription(config.transcription.clone())
                .with_streaming(sl.stream_drafts, sl.draft_update_interval_ms)
                .with_cancel_reaction(sl.cancel_reaction.clone())
                .with_approval_timeout_secs(sl.approval_timeout_secs),
            ))
        }
        #[cfg(feature = "channel-mattermost")]
        "mattermost" => {
            let mm = config
                .channels
                .mattermost
                .as_ref()
                .context("Mattermost channel is not configured")?;
            Ok(Arc::new(MattermostChannel::new(
                mm.url.clone(),
                mm.bot_token.clone(),
                mm.channel_id.clone(),
                mm.allowed_users.clone(),
                mm.thread_replies.unwrap_or(true),
                mm.mention_only.unwrap_or(false),
            )))
        }
        #[cfg(not(feature = "channel-mattermost"))]
        "mattermost" => {
            anyhow::bail!("Mattermost channel requires the `channel-mattermost` feature");
        }
        #[cfg(feature = "channel-signal")]
        "signal" => {
            let sg = config
                .channels
                .signal
                .as_ref()
                .context("Signal channel is not configured")?;
            Ok(Arc::new(
                SignalChannel::new(
                    sg.http_url.clone(),
                    sg.account.clone(),
                    sg.group_id.clone(),
                    sg.allowed_from.clone(),
                    sg.ignore_attachments,
                    sg.ignore_stories,
                )
                .with_approval_timeout_secs(sg.approval_timeout_secs),
            ))
        }
        #[cfg(not(feature = "channel-signal"))]
        "signal" => {
            anyhow::bail!("Signal channel requires the `channel-signal` feature");
        }
        "matrix" => {
            #[cfg(all(feature = "channel-matrix", feature = "channels-vendor"))]
            {
                let mx = config
                    .channels
                    .matrix
                    .as_ref()
                    .context("Matrix channel is not configured")?;
                let state_dir = config
                    .config_path
                    .parent()
                    .map(|p| p.join("state").join("matrix"))
                    .unwrap_or_else(|| std::path::PathBuf::from(".operant/state/matrix"));
                Ok(Arc::new(
                    MatrixChannel::new(mx.clone(), state_dir)?
                        .with_transcription(config.transcription.clone())
                        .with_workspace_dir(config.workspace_dir.clone()),
                ))
            }
            #[cfg(not(all(feature = "channel-matrix", feature = "channels-vendor")))]
            {
                anyhow::bail!("Matrix channel requires the `channel-matrix` feature");
            }
        }
        "whatsapp" | "whatsapp-web" | "whatsapp_web" => {
            #[cfg(all(feature = "whatsapp-web", feature = "channels-vendor"))]
            {
                let wa = config
                    .channels
                    .whatsapp
                    .as_ref()
                    .context("WhatsApp channel is not configured")?;
                if !wa.is_web_config() {
                    anyhow::bail!(
                        "WhatsApp channel send requires Web mode (session_path must be set)"
                    );
                }
                Ok(Arc::new(WhatsAppWebChannel::new(
                    wa.session_path.clone().unwrap_or_default(),
                    wa.pair_phone.clone(),
                    wa.pair_code.clone(),
                    wa.allowed_numbers.clone(),
                    wa.mention_only,
                    wa.mode.clone(),
                    wa.dm_policy.clone(),
                    wa.group_policy.clone(),
                    wa.self_chat_mode,
                )))
            }
            #[cfg(not(all(feature = "whatsapp-web", feature = "channels-vendor")))]
            {
                anyhow::bail!("WhatsApp channel requires the `whatsapp-web` feature");
            }
        }
        #[cfg(feature = "channel-qq")]
        "qq" => {
            let qq = config
                .channels
                .qq
                .as_ref()
                .context("QQ channel is not configured")?;
            Ok(Arc::new(QQChannel::new(
                qq.app_id.clone(),
                qq.app_secret.clone(),
                qq.allowed_users.clone(),
            )))
        }
        #[cfg(not(feature = "channel-qq"))]
        "qq" => {
            anyhow::bail!("QQ channel requires the `channel-qq` feature");
        }
        "lark" => {
            #[cfg(all(feature = "channel-lark", feature = "channels-vendor"))]
            {
                let lk = config
                    .channels
                    .lark
                    .as_ref()
                    .context("Lark channel is not configured")?;
                Ok(Arc::new(LarkChannel::from_lark_config(lk)))
            }
            #[cfg(not(all(feature = "channel-lark", feature = "channels-vendor")))]
            {
                anyhow::bail!("Lark channel requires the `channel-lark` feature");
            }
        }
        "feishu" => {
            #[cfg(all(feature = "channel-lark", feature = "channels-vendor"))]
            {
                if let Some(ref fs) = config.channels.feishu {
                    return Ok(Arc::new(LarkChannel::from_feishu_config(fs)));
                }
                // Legacy: [channels_config.lark] with use_feishu = true
                let lk = config
                    .channels
                    .lark
                    .as_ref()
                    .context("Feishu channel is not configured")?;
                Ok(Arc::new(LarkChannel::from_config(lk)))
            }
            #[cfg(not(all(feature = "channel-lark", feature = "channels-vendor")))]
            {
                anyhow::bail!("Feishu channel requires the `channel-lark` feature");
            }
        }
        #[cfg(feature = "channel-dingtalk")]
        "dingtalk" => {
            let dt = config
                .channels
                .dingtalk
                .as_ref()
                .context("DingTalk channel is not configured")?;
            Ok(Arc::new(
                DingTalkChannel::new(
                    dt.client_id.clone(),
                    dt.client_secret.clone(),
                    dt.allowed_users.clone(),
                )
                .with_proxy_url(dt.proxy_url.clone()),
            ))
        }
        #[cfg(not(feature = "channel-dingtalk"))]
        "dingtalk" => {
            anyhow::bail!("DingTalk channel requires the `channel-dingtalk` feature");
        }
        #[cfg(feature = "channel-wecom")]
        "wecom" => {
            let wc = config
                .channels
                .wecom
                .as_ref()
                .context("WeCom channel is not configured")?;
            Ok(Arc::new(WeComChannel::new(
                wc.webhook_key.clone(),
                wc.allowed_users.clone(),
            )))
        }
        #[cfg(not(feature = "channel-wecom"))]
        "wecom" => {
            anyhow::bail!("WeCom channel requires the `channel-wecom` feature");
        }
        #[cfg(all(feature = "channel-wechat", feature = "channels-vendor"))]
        "wechat" => {
            let wc = config
                .channels
                .wechat
                .as_ref()
                .context("WeChat channel is not configured")?;
            Ok(Arc::new(
                WeChatChannel::new(
                    wc.allowed_users.clone(),
                    wc.api_base_url.clone(),
                    wc.cdn_base_url.clone(),
                    wc.state_dir.as_ref().map(std::path::PathBuf::from),
                )?
                .with_workspace_dir(config.workspace_dir.clone()),
            ))
        }
        #[cfg(not(all(feature = "channel-wechat", feature = "channels-vendor")))]
        "wechat" => {
            anyhow::bail!("WeChat channel requires the `channel-wechat` feature");
        }
        #[cfg(feature = "channel-nextcloud")]
        "nextcloud_talk" | "nextcloud-talk" => {
            let nc = config
                .channels
                .nextcloud_talk
                .as_ref()
                .context("Nextcloud Talk channel is not configured")?;
            Ok(Arc::new(
                NextcloudTalkChannel::new_with_proxy(
                    nc.base_url.clone(),
                    nc.app_token.clone(),
                    nc.bot_name.clone().unwrap_or_default(),
                    nc.allowed_users.clone(),
                    nc.proxy_url.clone(),
                )
                .with_streaming(nc.stream_mode, nc.draft_update_interval_ms),
            ))
        }
        #[cfg(not(feature = "channel-nextcloud"))]
        "nextcloud_talk" | "nextcloud-talk" => {
            anyhow::bail!("Nextcloud Talk channel requires the `channel-nextcloud` feature");
        }
        #[cfg(feature = "channel-wati")]
        "wati" => {
            let wati_cfg = config
                .channels
                .wati
                .as_ref()
                .context("WATI channel is not configured")?;
            Ok(Arc::new(WatiChannel::new_with_proxy(
                wati_cfg.api_token.clone(),
                wati_cfg.api_url.clone(),
                wati_cfg.tenant_id.clone(),
                wati_cfg.allowed_numbers.clone(),
                wati_cfg.proxy_url.clone(),
            )))
        }
        #[cfg(not(feature = "channel-wati"))]
        "wati" => {
            anyhow::bail!("WATI channel requires the `channel-wati` feature");
        }
        #[cfg(feature = "channel-linq")]
        "linq" => {
            let lq = config
                .channels
                .linq
                .as_ref()
                .context("Linq channel is not configured")?;
            Ok(Arc::new(LinqChannel::new(
                lq.api_token.clone(),
                lq.from_phone.clone(),
                lq.allowed_senders.clone(),
            )))
        }
        #[cfg(not(feature = "channel-linq"))]
        "linq" => {
            anyhow::bail!("Linq channel requires the `channel-linq` feature");
        }
        #[cfg(feature = "channel-email")]
        "email" => {
            let em = config
                .channels
                .email
                .as_ref()
                .context("Email channel is not configured")?;
            Ok(Arc::new(EmailChannel::new(em.clone())))
        }
        #[cfg(feature = "channel-email")]
        "gmail_push" | "gmail-push" => {
            let gp = config
                .channels
                .gmail_push
                .as_ref()
                .context("Gmail Push channel is not configured")?;
            Ok(Arc::new(GmailPushChannel::new(gp.clone())))
        }
        #[cfg(feature = "channel-irc")]
        "irc" => {
            let irc_cfg = config
                .channels
                .irc
                .as_ref()
                .context("IRC channel is not configured")?;
            Ok(Arc::new(IrcChannel::new(crate::irc::IrcChannelConfig {
                server: irc_cfg.server.clone(),
                port: irc_cfg.port,
                nickname: irc_cfg.nickname.clone(),
                username: irc_cfg.username.clone(),
                channels: irc_cfg.channels.clone(),
                allowed_users: irc_cfg.allowed_users.clone(),
                server_password: irc_cfg.server_password.clone(),
                nickserv_password: irc_cfg.nickserv_password.clone(),
                sasl_password: irc_cfg.sasl_password.clone(),
                verify_tls: irc_cfg.verify_tls.unwrap_or(true),
                mention_only: irc_cfg.mention_only,
            })))
        }
        #[cfg(not(feature = "channel-irc"))]
        "irc" => {
            anyhow::bail!("IRC channel requires the `channel-irc` feature");
        }
        #[cfg(feature = "channel-twitter")]
        "twitter" => {
            let tw = config
                .channels
                .twitter
                .as_ref()
                .context("X/Twitter channel is not configured")?;
            Ok(Arc::new(TwitterChannel::new(
                tw.bearer_token.clone(),
                tw.allowed_users.clone(),
            )))
        }
        #[cfg(not(feature = "channel-twitter"))]
        "twitter" => {
            anyhow::bail!("X/Twitter channel requires the `channel-twitter` feature");
        }
        #[cfg(feature = "channel-mochat")]
        "mochat" => {
            let mc = config
                .channels
                .mochat
                .as_ref()
                .context("Mochat channel is not configured")?;
            Ok(Arc::new(MochatChannel::new(
                mc.api_url.clone(),
                mc.api_token.clone(),
                mc.allowed_users.clone(),
                mc.poll_interval_secs,
            )))
        }
        #[cfg(not(feature = "channel-mochat"))]
        "mochat" => {
            anyhow::bail!("Mochat channel requires the `channel-mochat` feature");
        }
        #[cfg(feature = "channel-discord")]
        "discord_history" | "discord-history" => {
            let dh = config
                .channels
                .discord_history
                .as_ref()
                .context("Discord History channel is not configured")?;
            let discord_mem =
                operant_memory::SqliteMemory::new_named(&config.workspace_dir, "discord")
                    .context("Discord History: failed to open discord.db")?;
            Ok(Arc::new(DiscordHistoryChannel::new(
                dh.bot_token.clone(),
                dh.guild_id.clone(),
                dh.allowed_users.clone(),
                dh.channel_ids.clone(),
                Arc::new(discord_mem),
                dh.store_dms,
                dh.respond_to_dms,
            )))
        }
        #[cfg(not(feature = "channel-discord"))]
        "discord_history" | "discord-history" => {
            anyhow::bail!("Discord History channel requires the `channel-discord` feature");
        }
        #[cfg(feature = "channel-imessage")]
        "imessage" => {
            let im = config
                .channels
                .imessage
                .as_ref()
                .context("iMessage channel is not configured")?;
            Ok(Arc::new(IMessageChannel::new(im.allowed_contacts.clone())))
        }
        #[cfg(not(feature = "channel-imessage"))]
        "imessage" => {
            anyhow::bail!("iMessage channel requires the `channel-imessage` feature");
        }
        "line" => {
            #[cfg(all(feature = "channel-line", feature = "channels-vendor"))]
            {
                let ln = config
                    .channels
                    .line
                    .as_ref()
                    .context("LINE channel is not configured")?;
                Ok(Arc::new(LineChannel::from_config(ln)))
            }
            #[cfg(not(all(feature = "channel-line", feature = "channels-vendor")))]
            {
                anyhow::bail!("LINE channel requires the `channel-line` feature");
            }
        }
        "voice-call" => {
            #[cfg(all(feature = "channel-voice-call", feature = "channels-vendor"))]
            {
                let vc = config
                    .channels
                    .voice_call
                    .as_ref()
                    .context("Voice Call channel is not configured")?;
                Ok(Arc::new(VoiceCallChannel::new(vc.clone())))
            }
            #[cfg(not(all(feature = "channel-voice-call", feature = "channels-vendor")))]
            {
                anyhow::bail!("Voice Call channel requires the `channel-voice-call` feature");
            }
        }
        other => anyhow::bail!(
            "Unknown channel '{other}'. Supported: telegram, discord, slack, mattermost, signal, \
            matrix, whatsapp, qq, lark, feishu, dingtalk, wecom, nextcloud_talk, wati, linq, \
            email, gmail_push, irc, twitter, mochat, discord_history, imessage, line, voice-call"
        ),
    }
}

/// Send a one-off message to a configured channel.
pub async fn send_channel_message(
    config: &Config,
    channel_id: &str,
    recipient: &str,
    message: &str,
) -> Result<()> {
    let channel = build_channel_by_id(config, channel_id)?;
    let msg = SendMessage::new(message, recipient);
    channel
        .send(&msg)
        .await
        .with_context(|| format!("Failed to send message via {channel_id}"))?;
    println!("Message sent via {channel_id}.");
    Ok(())
}
