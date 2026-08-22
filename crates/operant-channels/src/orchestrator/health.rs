//! `health` — extracted verbatim from the former orchestrator/mod.rs monolith.
//! Re-exported from `orchestrator` so every import path is unchanged.

use anyhow::Result;
use operant_config::schema::Config;
use operant_memory;
use std::sync::Arc;
use std::time::Duration;

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChannelHealthState {
    Healthy,
    Unhealthy,
    Timeout,
}

pub(crate) fn classify_health_result(
    result: &std::result::Result<bool, tokio::time::error::Elapsed>,
) -> ChannelHealthState {
    match result {
        Ok(true) => ChannelHealthState::Healthy,
        Ok(false) => ChannelHealthState::Unhealthy,
        Err(_) => ChannelHealthState::Timeout,
    }
}

pub(crate) struct ConfiguredChannel {
    pub(crate) display_name: &'static str,
    pub(crate) channel: Arc<dyn Channel>,
}

pub(crate) fn collect_configured_channels(
    config: &Config,
    matrix_skip_context: &str,
    tool_specs: &[(String, String)],
) -> Vec<ConfiguredChannel> {
    let _ = matrix_skip_context;
    let _ = tool_specs;
    let mut channels = Vec::new();

    #[cfg(feature = "channel-telegram")]
    if let Some(ref tg) = config.channels.telegram {
        if tg.enabled {
            let ack = tg.ack_reactions.unwrap_or(config.channels.ack_reactions);
            channels.push(ConfiguredChannel {
                display_name: "Telegram",
                channel: Arc::new(
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
                    .with_proxy_url(tg.proxy_url.clone())
                    .with_tool_command_specs(tool_specs.to_vec())
                    .with_approval_timeout_secs(tg.approval_timeout_secs)
                    .with_dm_topics(tg.dm_topics_enabled, tg.dm_topic_name.clone())
                    .with_fallback_ips(tg.fallback_ips.clone()),
                ),
            });
        } else {
            tracing::info!("Telegram channel configured but disabled (enabled = false)");
        }
    }

    if let Some(ref dc) = config.channels.discord {
        if dc.enabled {
            channels.push(ConfiguredChannel {
                display_name: "Discord",
                channel: Arc::new(
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
                    .with_proxy_url(dc.proxy_url.clone())
                    .with_transcription(config.transcription.clone())
                    .with_stall_timeout(dc.stall_timeout_secs)
                    .with_approval_timeout_secs(dc.approval_timeout_secs),
                ),
            });
        } else {
            tracing::info!("Discord channel configured but disabled (enabled = false)");
        }
    }

    if let Some(ref dh) = config.channels.discord_history {
        if dh.enabled {
            match operant_memory::SqliteMemory::new_named(&config.workspace_dir, "discord") {
                Ok(discord_mem) => {
                    channels.push(ConfiguredChannel {
                        display_name: "Discord History",
                        channel: Arc::new(
                            DiscordHistoryChannel::new(
                                dh.bot_token.clone(),
                                dh.guild_id.clone(),
                                dh.allowed_users.clone(),
                                dh.channel_ids.clone(),
                                Arc::new(discord_mem),
                                dh.store_dms,
                                dh.respond_to_dms,
                            )
                            .with_proxy_url(dh.proxy_url.clone()),
                        ),
                    });
                }
                Err(e) => {
                    tracing::error!("discord_history: failed to open discord.db: {e}");
                }
            }
        } else {
            tracing::info!("Discord History channel configured but disabled (enabled = false)");
        }
    }

    if let Some(ref sl) = config.channels.slack {
        if !sl.enabled {
            tracing::info!("Slack channel configured but disabled (enabled = false)");
        } else if let Some(bot_token) = sl.bot_token.clone().filter(|s| !s.is_empty()) {
            channels.push(ConfiguredChannel {
                display_name: "Slack",
                channel: Arc::new(
                    SlackChannel::new(
                        bot_token,
                        sl.app_token.clone(),
                        sl.channel_ids.clone(),
                        sl.allowed_users.clone(),
                    )
                    .with_thread_replies(sl.thread_replies.unwrap_or(true))
                    .with_group_reply_policy(sl.mention_only, Vec::new())
                    .with_strict_mention_in_thread(sl.strict_mention_in_thread)
                    .with_workspace_dir(config.workspace_dir.clone())
                    .with_markdown_blocks(sl.use_markdown_blocks)
                    .with_proxy_url(sl.proxy_url.clone())
                    .with_transcription(config.transcription.clone())
                    .with_streaming(sl.stream_drafts, sl.draft_update_interval_ms)
                    .with_cancel_reaction(sl.cancel_reaction.clone())
                    .with_approval_timeout_secs(sl.approval_timeout_secs),
                ),
            });
        } else {
            tracing::error!(
                "Slack channel is enabled but no bot_token is configured. \
                 Set `bot_token` in `[channels.slack]` of config.toml, or \
                 export OPERANT_SLACK_BOT_TOKEN / SLACK_BOT_TOKEN before \
                 starting. Skipping Slack channel."
            );
        }
    }

    #[cfg(feature = "channel-mattermost")]
    if let Some(ref mm) = config.channels.mattermost {
        if mm.enabled {
            channels.push(ConfiguredChannel {
                display_name: "Mattermost",
                channel: Arc::new(
                    MattermostChannel::new(
                        mm.url.clone(),
                        mm.bot_token.clone(),
                        mm.channel_id.clone(),
                        mm.allowed_users.clone(),
                        mm.thread_replies.unwrap_or(true),
                        mm.mention_only.unwrap_or(false),
                    )
                    .with_proxy_url(mm.proxy_url.clone())
                    .with_transcription(config.transcription.clone()),
                ),
            });
        } else {
            tracing::info!("Mattermost channel configured but disabled (enabled = false)");
        }
    }

    #[cfg(feature = "channel-imessage")]
    if let Some(ref im) = config.channels.imessage {
        if im.enabled {
            channels.push(ConfiguredChannel {
                display_name: "iMessage",
                channel: Arc::new(IMessageChannel::new(im.allowed_contacts.clone())),
            });
        } else {
            tracing::info!("iMessage channel configured but disabled (enabled = false)");
        }
    }

    #[cfg(all(feature = "channel-matrix", feature = "channels-vendor"))]
    if let Some(ref mx) = config.channels.matrix {
        if mx.enabled {
            let state_dir = config
                .config_path
                .parent()
                .map(|p| p.join("state").join("matrix"))
                .unwrap_or_else(|| std::path::PathBuf::from(".operant/state/matrix"));
            match MatrixChannel::new(mx.clone(), state_dir) {
                Ok(channel) => {
                    let channel = channel
                        .with_transcription(config.transcription.clone())
                        .with_workspace_dir(config.workspace_dir.clone());
                    channels.push(ConfiguredChannel {
                        display_name: "Matrix",
                        channel: Arc::new(channel),
                    });
                }
                Err(e) => {
                    tracing::error!("Matrix channel construction failed: {e}");
                }
            }
        } else {
            tracing::info!("Matrix channel configured but disabled (enabled = false)");
        }
    }

    #[cfg(not(all(feature = "channel-matrix", feature = "channels-vendor")))]
    if config.channels.matrix.is_some() {
        tracing::warn!(
            "Matrix channel is configured but this build was compiled without `channel-matrix`; skipping Matrix {}.",
            matrix_skip_context
        );
    }

    #[cfg(feature = "channel-signal")]
    if let Some(ref sig) = config.channels.signal {
        if sig.enabled {
            channels.push(ConfiguredChannel {
                display_name: "Signal",
                channel: Arc::new(
                    SignalChannel::new(
                        sig.http_url.clone(),
                        sig.account.clone(),
                        sig.group_id.clone(),
                        sig.allowed_from.clone(),
                        sig.ignore_attachments,
                        sig.ignore_stories,
                    )
                    .with_proxy_url(sig.proxy_url.clone())
                    .with_approval_timeout_secs(sig.approval_timeout_secs),
                ),
            });
        } else {
            tracing::info!("Signal channel configured but disabled (enabled = false)");
        }
    }

    #[cfg(all(feature = "whatsapp-web", feature = "channels-vendor"))]
    if let Some(ref wa) = config.channels.whatsapp {
        if wa.enabled {
            if wa.is_ambiguous_config() {
                tracing::warn!(
                    "WhatsApp config has both phone_number_id and session_path set; preferring Cloud API mode. Remove one selector to avoid ambiguity."
                );
            }
            // Runtime negotiation: detect backend type from config
            match wa.backend_type() {
                "cloud" => {
                    // Cloud API mode: requires phone_number_id, access_token, verify_token
                    if wa.is_cloud_config() {
                        channels.push(ConfiguredChannel {
                            display_name: "WhatsApp",
                            channel: Arc::new(
                                WhatsAppChannel::new(
                                    wa.access_token.clone().unwrap_or_default(),
                                    wa.phone_number_id.clone().unwrap_or_default(),
                                    wa.verify_token.clone().unwrap_or_default(),
                                    wa.allowed_numbers.clone(),
                                )
                                .with_proxy_url(wa.proxy_url.clone())
                                .with_dm_mention_patterns(wa.dm_mention_patterns.clone())
                                .with_group_mention_patterns(wa.group_mention_patterns.clone())
                                .with_approval_timeout_secs(wa.approval_timeout_secs),
                            ),
                        });
                    } else {
                        tracing::warn!(
                            "WhatsApp Cloud API configured but missing required fields (phone_number_id, access_token, verify_token)"
                        );
                    }
                }
                "web" => {
                    // Web mode: requires session_path
                    #[cfg(all(feature = "whatsapp-web", feature = "channels-vendor"))]
                    if wa.is_web_config() {
                        channels.push(ConfiguredChannel {
                            display_name: "WhatsApp",
                            channel: Arc::new(
                                WhatsAppWebChannel::new(
                                    wa.session_path.clone().unwrap_or_default(),
                                    wa.pair_phone.clone(),
                                    wa.pair_code.clone(),
                                    wa.allowed_numbers.clone(),
                                    wa.mention_only,
                                    wa.mode.clone(),
                                    wa.dm_policy.clone(),
                                    wa.group_policy.clone(),
                                    wa.self_chat_mode,
                                )
                                .with_transcription(config.transcription.clone())
                                .with_tts(config.tts.clone())
                                .with_dm_mention_patterns(wa.dm_mention_patterns.clone())
                                .with_group_mention_patterns(wa.group_mention_patterns.clone()),
                            ),
                        });
                    } else {
                        tracing::warn!("WhatsApp Web configured but session_path not set");
                    }
                    #[cfg(not(all(feature = "whatsapp-web", feature = "channels-vendor")))]
                    {
                        tracing::warn!(
                            "WhatsApp Web backend requires 'whatsapp-web' feature. Build/run with --features whatsapp-web"
                        );
                        eprintln!(
                            "{}",
                            i18n::get_required_cli_string(
                                "channel-whatsapp-web-feature-missing-warning"
                            )
                        );
                        eprintln!(
                            "{}",
                            i18n::get_required_cli_string(
                                "channel-whatsapp-web-feature-missing-build"
                            )
                        );
                        eprintln!(
                            "{}",
                            i18n::get_required_cli_string(
                                "channel-whatsapp-web-feature-missing-install"
                            )
                        );
                    }
                }
                _ => {
                    tracing::warn!(
                        "WhatsApp config invalid: neither phone_number_id (Cloud API) nor session_path (Web) is set"
                    );
                }
            }
        } else {
            tracing::info!("WhatsApp channel configured but disabled (enabled = false)");
        }
    }

    #[cfg(feature = "channel-linq")]
    if let Some(ref lq) = config.channels.linq {
        if lq.enabled {
            channels.push(ConfiguredChannel {
                display_name: "Linq",
                channel: Arc::new(LinqChannel::new(
                    lq.api_token.clone(),
                    lq.from_phone.clone(),
                    lq.allowed_senders.clone(),
                )),
            });
        } else {
            tracing::info!("Linq channel configured but disabled (enabled = false)");
        }
    }

    #[cfg(feature = "channel-wati")]
    if let Some(ref wati_cfg) = config.channels.wati {
        if wati_cfg.enabled {
            let wati_channel = WatiChannel::new_with_proxy(
                wati_cfg.api_token.clone(),
                wati_cfg.api_url.clone(),
                wati_cfg.tenant_id.clone(),
                wati_cfg.allowed_numbers.clone(),
                wati_cfg.proxy_url.clone(),
            )
            .with_transcription(config.transcription.clone());

            channels.push(ConfiguredChannel {
                display_name: "WATI",
                channel: Arc::new(wati_channel),
            });
        } else {
            tracing::info!("WATI channel configured but disabled (enabled = false)");
        }
    }

    #[cfg(feature = "channel-nextcloud")]
    if let Some(ref nc) = config.channels.nextcloud_talk {
        if nc.enabled {
            channels.push(ConfiguredChannel {
                display_name: "Nextcloud Talk",
                channel: Arc::new(NextcloudTalkChannel::new_with_proxy(
                    nc.base_url.clone(),
                    nc.app_token.clone(),
                    nc.bot_name.clone().unwrap_or_default(),
                    nc.allowed_users.clone(),
                    nc.proxy_url.clone(),
                )),
            });
        } else {
            tracing::info!("Nextcloud Talk channel configured but disabled (enabled = false)");
        }
    }

    #[cfg(feature = "channel-email")]
    if let Some(ref email_cfg) = config.channels.email {
        if email_cfg.enabled {
            channels.push(ConfiguredChannel {
                display_name: "Email",
                channel: Arc::new(EmailChannel::new(email_cfg.clone())),
            });
        } else {
            tracing::info!("Email channel configured but disabled (enabled = false)");
        }
    }

    #[cfg(feature = "channel-email")]
    if let Some(ref gp_cfg) = config.channels.gmail_push
        && gp_cfg.enabled
    {
        channels.push(ConfiguredChannel {
            display_name: "Gmail Push",
            channel: Arc::new(GmailPushChannel::new(gp_cfg.clone())),
        });
    }

    #[cfg(feature = "channel-irc")]
    if let Some(ref irc) = config.channels.irc {
        if irc.enabled {
            channels.push(ConfiguredChannel {
                display_name: "IRC",
                channel: Arc::new(IrcChannel::new(crate::irc::IrcChannelConfig {
                    server: irc.server.clone(),
                    port: irc.port,
                    nickname: irc.nickname.clone(),
                    username: irc.username.clone(),
                    channels: irc.channels.clone(),
                    allowed_users: irc.allowed_users.clone(),
                    server_password: irc.server_password.clone(),
                    nickserv_password: irc.nickserv_password.clone(),
                    sasl_password: irc.sasl_password.clone(),
                    verify_tls: irc.verify_tls.unwrap_or(true),
                    mention_only: irc.mention_only,
                })),
            });
        } else {
            tracing::info!("IRC channel configured but disabled (enabled = false)");
        }
    }

    #[cfg(all(feature = "channel-lark", feature = "channels-vendor"))]
    if let Some(ref lk) = config.channels.lark {
        if lk.enabled {
            if lk.use_feishu {
                if config.channels.feishu.is_some() {
                    tracing::warn!(
                        "Both [channels_config.feishu] and legacy [channels_config.lark].use_feishu=true are configured; ignoring legacy Feishu fallback in lark."
                    );
                } else {
                    tracing::warn!(
                        "Using legacy [channels_config.lark].use_feishu=true compatibility path; prefer [channels_config.feishu]."
                    );
                    channels.push(ConfiguredChannel {
                        display_name: "Feishu",
                        channel: Arc::new(
                            LarkChannel::from_config(lk)
                                .with_transcription(config.transcription.clone()),
                        ),
                    });
                }
            } else {
                channels.push(ConfiguredChannel {
                    display_name: "Lark",
                    channel: Arc::new(
                        LarkChannel::from_lark_config(lk)
                            .with_transcription(config.transcription.clone()),
                    ),
                });
            }
        } else {
            tracing::info!("Lark channel configured but disabled (enabled = false)");
        }
    }

    #[cfg(all(feature = "channel-lark", feature = "channels-vendor"))]
    if let Some(ref fs) = config.channels.feishu {
        if fs.enabled {
            channels.push(ConfiguredChannel {
                display_name: "Feishu",
                channel: Arc::new(
                    LarkChannel::from_feishu_config(fs)
                        .with_transcription(config.transcription.clone()),
                ),
            });
        } else {
            tracing::info!("Feishu channel configured but disabled (enabled = false)");
        }
    }

    #[cfg(not(all(feature = "channel-lark", feature = "channels-vendor")))]
    if config.channels.lark.is_some() || config.channels.feishu.is_some() {
        tracing::warn!(
            "Lark/Feishu channel is configured but this build was compiled without `channel-lark`; skipping Lark/Feishu health check."
        );
    }

    #[cfg(all(feature = "channel-line", feature = "channels-vendor"))]
    if let Some(ref ln) = config.channels.line {
        if ln.enabled {
            channels.push(ConfiguredChannel {
                display_name: "LINE",
                channel: Arc::new(
                    LineChannel::from_config(ln).with_transcription(config.transcription.clone()),
                ),
            });
        } else {
            tracing::info!("LINE channel configured but disabled (enabled = false)");
        }
    }

    #[cfg(not(all(feature = "channel-line", feature = "channels-vendor")))]
    if config.channels.line.is_some() {
        tracing::warn!(
            "LINE channel is configured but this build was compiled without `channel-line`; skipping LINE health check."
        );
    }

    #[cfg(feature = "channel-dingtalk")]
    if let Some(ref dt) = config.channels.dingtalk {
        if dt.enabled {
            channels.push(ConfiguredChannel {
                display_name: "DingTalk",
                channel: Arc::new(
                    DingTalkChannel::new(
                        dt.client_id.clone(),
                        dt.client_secret.clone(),
                        dt.allowed_users.clone(),
                    )
                    .with_proxy_url(dt.proxy_url.clone()),
                ),
            });
        } else {
            tracing::info!("DingTalk channel configured but disabled (enabled = false)");
        }
    }

    #[cfg(feature = "channel-qq")]
    if let Some(ref qq) = config.channels.qq {
        if qq.enabled {
            channels.push(ConfiguredChannel {
                display_name: "QQ",
                channel: Arc::new(
                    QQChannel::new(
                        qq.app_id.clone(),
                        qq.app_secret.clone(),
                        qq.allowed_users.clone(),
                    )
                    .with_workspace_dir(config.workspace_dir.clone())
                    .with_proxy_url(qq.proxy_url.clone()),
                ),
            });
        } else {
            tracing::info!("QQ channel configured but disabled (enabled = false)");
        }
    }

    #[cfg(feature = "channel-twitter")]
    if let Some(ref tw) = config.channels.twitter {
        channels.push(ConfiguredChannel {
            display_name: "X/Twitter",
            channel: Arc::new(TwitterChannel::new(
                tw.bearer_token.clone(),
                tw.allowed_users.clone(),
            )),
        });
    }

    #[cfg(feature = "channel-mochat")]
    if let Some(ref mc) = config.channels.mochat {
        if mc.enabled {
            channels.push(ConfiguredChannel {
                display_name: "Mochat",
                channel: Arc::new(MochatChannel::new(
                    mc.api_url.clone(),
                    mc.api_token.clone(),
                    mc.allowed_users.clone(),
                    mc.poll_interval_secs,
                )),
            });
        } else {
            tracing::info!("Mochat channel configured but disabled (enabled = false)");
        }
    }

    #[cfg(feature = "channel-wecom")]
    if let Some(ref wc) = config.channels.wecom {
        if wc.enabled {
            channels.push(ConfiguredChannel {
                display_name: "WeCom",
                channel: Arc::new(WeComChannel::new(
                    wc.webhook_key.clone(),
                    wc.allowed_users.clone(),
                )),
            });
        } else {
            tracing::info!("WeCom channel configured but disabled (enabled = false)");
        }
    }

    #[cfg(all(feature = "channel-wechat", feature = "channels-vendor"))]
    if let Some(ref wechat) = config.channels.wechat {
        if wechat.enabled {
            match WeChatChannel::new(
                wechat.allowed_users.clone(),
                wechat.api_base_url.clone(),
                wechat.cdn_base_url.clone(),
                wechat.state_dir.as_ref().map(std::path::PathBuf::from),
            ) {
                Ok(channel) => {
                    channels.push(ConfiguredChannel {
                        display_name: "WeChat",
                        channel: Arc::new(channel.with_workspace_dir(config.workspace_dir.clone())),
                    });
                }
                Err(err) => {
                    tracing::warn!(
                        "WeChat channel configuration is invalid; skipping WeChat {matrix_skip_context}: {err}"
                    );
                }
            }
        } else {
            tracing::info!("WeChat channel configured but disabled (enabled = false)");
        }
    }

    #[cfg(not(all(feature = "channel-wechat", feature = "channels-vendor")))]
    if let Some(ref wechat) = config.channels.wechat
        && wechat.enabled
    {
        tracing::warn!(
            "WeChat channel is configured but this build was compiled without `channel-wechat`; skipping WeChat {matrix_skip_context}."
        );
    }

    #[cfg(feature = "channel-clawdtalk")]
    if let Some(ref ct) = config.channels.clawdtalk {
        if ct.enabled {
            channels.push(ConfiguredChannel {
                display_name: "ClawdTalk",
                channel: Arc::new(ClawdTalkChannel::new(ct.clone())),
            });
        } else {
            tracing::info!("ClawdTalk channel configured but disabled (enabled = false)");
        }
    }

    // Notion database poller channel
    #[cfg(feature = "channel-notion")]
    if config.notion.enabled && !config.notion.database_id.trim().is_empty() {
        let notion_api_key = if config.notion.api_key.trim().is_empty() {
            std::env::var("NOTION_API_KEY").unwrap_or_default()
        } else {
            config.notion.api_key.trim().to_string()
        };
        if notion_api_key.trim().is_empty() {
            tracing::warn!(
                "Notion channel enabled but no API key found (set notion.api_key or NOTION_API_KEY env var)"
            );
        } else {
            channels.push(ConfiguredChannel {
                display_name: "Notion",
                channel: Arc::new(NotionChannel::new(
                    notion_api_key,
                    config.notion.database_id.clone(),
                    config.notion.poll_interval_secs,
                    config.notion.status_property.clone(),
                    config.notion.input_property.clone(),
                    config.notion.result_property.clone(),
                    config.notion.max_concurrent,
                    config.notion.recover_stale,
                )),
            });
        }
    }

    #[cfg(feature = "channel-reddit")]
    if let Some(ref rd) = config.channels.reddit {
        channels.push(ConfiguredChannel {
            display_name: "Reddit",
            channel: Arc::new(RedditChannel::new(
                rd.client_id.clone(),
                rd.client_secret.clone(),
                rd.refresh_token.clone(),
                rd.username.clone(),
                rd.subreddit.clone(),
            )),
        });
    }

    #[cfg(feature = "channel-bluesky")]
    if let Some(ref bs) = config.channels.bluesky {
        channels.push(ConfiguredChannel {
            display_name: "Bluesky",
            channel: Arc::new(BlueskyChannel::new(
                bs.handle.clone(),
                bs.app_password.clone(),
            )),
        });
    }

    #[cfg(all(feature = "voice-wake", feature = "channels-vendor"))]
    if let Some(ref vw) = config.channels.voice_wake {
        channels.push(ConfiguredChannel {
            display_name: "VoiceWake",
            channel: Arc::new(VoiceWakeChannel::new(
                vw.clone(),
                config.transcription.clone(),
            )),
        });
    }

    #[cfg(all(feature = "channel-voice-call", feature = "channels-vendor"))]
    if let Some(ref vc) = config.channels.voice_call {
        if vc.enabled {
            channels.push(ConfiguredChannel {
                display_name: "Voice Call",
                channel: Arc::new(VoiceCallChannel::new(vc.clone())),
            });
        } else {
            tracing::info!("Voice Call channel configured but disabled (enabled = false)");
        }
    }

    if let Some(ref wh) = config.channels.webhook {
        if wh.enabled {
            channels.push(ConfiguredChannel {
                display_name: "Webhook",
                channel: Arc::new(WebhookChannel::new(
                    wh.port,
                    wh.listen_path.clone(),
                    wh.send_url.clone(),
                    wh.send_method.clone(),
                    wh.auth_header.clone(),
                    wh.secret.clone(),
                )),
            });
        } else {
            tracing::info!("Webhook channel configured but disabled (enabled = false)");
        }
    }

    channels
}

/// Run health checks for configured channels.
pub async fn doctor_channels(config: Config) -> Result<()> {
    #[allow(unused_mut)]
    let mut channels = collect_configured_channels(&config, "health check", &[]);

    #[cfg(all(feature = "channel-nostr", feature = "channels-vendor"))]
    if let Some(ref ns) = config.channels.nostr {
        channels.push(ConfiguredChannel {
            display_name: "Nostr",
            channel: Arc::new(
                NostrChannel::new(&ns.private_key, ns.relays.clone(), &ns.allowed_pubkeys).await?,
            ),
        });
    }

    if channels.is_empty() {
        println!("No real-time channels configured. Run `operant onboard` first.");
        return Ok(());
    }

    println!("🩺 Operant Channel Doctor");
    println!();

    let mut healthy = 0_u32;
    let mut unhealthy = 0_u32;
    let mut timeout = 0_u32;

    for configured in channels {
        let result =
            tokio::time::timeout(Duration::from_secs(10), configured.channel.health_check()).await;
        let state = classify_health_result(&result);

        match state {
            ChannelHealthState::Healthy => {
                healthy += 1;
                println!("  ✅ {:<9} healthy", configured.display_name);
            }
            ChannelHealthState::Unhealthy => {
                unhealthy += 1;
                println!(
                    "  ❌ {:<9} unhealthy (auth/config/network)",
                    configured.display_name
                );
            }
            ChannelHealthState::Timeout => {
                timeout += 1;
                println!("  ⏱️  {:<9} timed out (>10s)", configured.display_name);
            }
        }
    }

    if config.channels.webhook.is_some() {
        println!("  ℹ️  Webhook   check via `operant gateway` then GET /health");
    }

    println!();
    println!("Summary: {healthy} healthy, {unhealthy} unhealthy, {timeout} timed out");
    Ok(())
}
