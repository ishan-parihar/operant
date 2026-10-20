use anyhow::{Context, Result, bail};
use clap::Subcommand;
use serde::{Deserialize, Serialize};
use std::path::Path;

use operant_core::config::AppConfig;

#[derive(Subcommand, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ChannelSubcommand {
    /// List all configured channels
    List,
    /// Start all configured channels
    Start,
    /// Run health checks for configured channels
    Doctor,
    /// Add a new channel configuration
    Add {
        /// Channel type (telegram, discord, slack, whatsapp, email, webhooks)
        channel_type: String,
        /// Optional configuration as JSON (e.g. {"api_key":"..."})
        #[arg(value_name = "CONFIG")]
        config_json: Option<String>,
        /// Bot token for the channel (do NOT use the global --api-key — that
        /// flag overrides the LLM API key and must never be persisted here)
        #[arg(long, value_name = "TOKEN")]
        token: Option<String>,
    },
    /// Remove a channel configuration
    Remove {
        /// Channel name to remove
        name: String,
    },
    /// Bind a Telegram identity (username or numeric user ID) into allowlist
    BindTelegram {
        /// Telegram identity to allow (username without '@' or numeric user ID)
        identity: String,
    },
    /// Send a message to a configured channel
    Send {
        /// Message text to send
        message: String,
        /// Channel config name (e.g. telegram, discord, slack)
        #[arg(long)]
        channel_id: String,
        /// Recipient identifier (platform-specific, e.g. Telegram chat ID)
        #[arg(long)]
        recipient: String,
    },
    /// Delete a message operant sent earlier (platform support varies)
    DeleteMessage {
        /// Channel config name (e.g. telegram)
        #[arg(long)]
        channel_id: String,
        /// Recipient/chat identifier the message was sent to (e.g. Telegram chat ID)
        #[arg(long)]
        recipient: String,
        /// Platform message ID (Telegram returns it from sendMessage)
        #[arg(long)]
        message_id: String,
    },
}

pub async fn handle_channel_command(
    config: &mut AppConfig,
    config_path: Option<&Path>,
    cmd: ChannelSubcommand,
    json: bool,
) -> Result<()> {
    match cmd {
        ChannelSubcommand::List => {
            let channels = build_channel_list(config);
            if json {
                let items: Vec<serde_json::Value> = channels
                    .iter()
                    .map(|c| {
                        serde_json::json!({
                            "name": c.label,
                            "enabled": c.enabled,
                            "inbound": c.can_receive(),
                        })
                    })
                    .collect();
                println!("{}", serde_json::json!({"channels": items}));
            } else {
                println!("Channels:");
                for c in &channels {
                    if c.is_platform() {
                        println!(
                            "  {} {:<9} {}",
                            if c.enabled { "✅" } else { "❌" },
                            c.label,
                            if c.can_receive() {
                                "receives messages"
                            } else {
                                "send-only (no inbound)"
                            }
                        );
                    } else {
                        println!("  {} {}", if c.enabled { "✅" } else { "❌" }, c.label);
                    }
                }
                println!("\nTo start channels: `operant channel start`");
                println!("To check health:   `operant channel doctor`");
            }
            Ok(())
        }
        ChannelSubcommand::Start => {
            // R15-2: `operant daemon` does not exist — wire to the real
            // gateway runner (the same path `operant gateway run` uses).
            // Mirror cmd_run's lifecycle: `start_gateway` spawns the gateway
            // tasks, so the process MUST await a signal and then stop the
            // gateway — otherwise the runtime drops the spawned tasks the
            // instant the command returns and the gateway dies immediately
            // (reviewer-caught bug).
            let msg = crate::gateway_runner::start_gateway(config).await?;
            if json {
                println!("{}", serde_json::json!({"status":"started","message": msg}));
                println!(
                    "{}",
                    serde_json::json!({"status":"running","message": "Gateway running. Send SIGINT or SIGTERM to stop."})
                );
            } else {
                println!("{msg}");
                println!("Running in foreground — press Ctrl+C (SIGINT) or send SIGTERM to stop.");
            }

            // `tokio::signal::unix` does not exist on Windows, so this is a
            // compile error there rather than a runtime one. Same cfg split, and
            // the same shape, as `cmd_gateway::cmd_run` and
            // `operant_runtime::daemon::wait_for_exit_signal`.
            #[cfg(unix)]
            {
                let mut sigint =
                    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
                let mut sigterm =
                    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
                tokio::select! {
                    _ = sigint.recv() => {
                        if json {
                            println!(r#"{{"status":"stopping","reason":"SIGINT"}}"#);
                        } else {
                            println!("\nReceived SIGINT (Ctrl+C). Shutting down gateway...");
                        }
                    }
                    _ = sigterm.recv() => {
                        if json {
                            println!(r#"{{"status":"stopping","reason":"SIGTERM"}}"#);
                        } else {
                            println!("\nReceived SIGTERM. Shutting down gateway...");
                        }
                    }
                }
            }

            // Windows has no SIGTERM; Ctrl+C is the only graceful stop there.
            // The `reason` field changes accordingly rather than claiming a
            // signal this platform never delivered.
            #[cfg(not(unix))]
            {
                tokio::signal::ctrl_c().await?;
                if json {
                    println!(r#"{{"status":"stopping","reason":"Ctrl+C"}}"#);
                } else {
                    println!("\nReceived Ctrl+C. Shutting down gateway...");
                }
            }
            let stop = crate::gateway_runner::stop_gateway().await?;
            if json {
                println!(
                    "{}",
                    serde_json::json!({"status":"stopped","message": stop})
                );
            } else {
                println!("{stop}");
            }
            Ok(())
        }
        ChannelSubcommand::Doctor => {
            let channels = build_channel_list(config);
            let configured: Vec<_> = channels.iter().filter(|c| c.enabled).collect();
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "total": channels.len(),
                        "enabled": configured.len(),
                        "channels": channels.iter().map(|c| serde_json::json!({
                            "name": c.label,
                            "enabled": c.enabled,
                            // The ENABLED flag, not a probe: the doctor does
                            // not reach the platform, so it cannot know
                            // whether the channel works. Renamed from
                            // "healthy", which claimed a health the
                            // command never checked.
                            "configured": c.enabled,
                            "health": "not probed",
                            // Whether the agent can RECEIVE on this
                            // channel. A send-only platform has a complete
                            // inbound parser that nothing calls, so a
                            // message sent to it never reaches the agent.
                            "inbound": c.can_receive(),
                        })).collect::<Vec<_>>()
                    })
                );
            } else {
                println!(
                    "Channel health: {}/{} enabled",
                    configured.len(),
                    channels.len()
                );
                for c in &channels {
                    if c.is_platform() {
                        println!(
                            "  {} {:<9} {}",
                            if c.enabled { "✅" } else { "⏭️" },
                            c.label,
                            if c.can_receive() {
                                "receives messages"
                            } else {
                                "send-only (no inbound)"
                            }
                        );
                    } else {
                        println!("  {} {}", if c.enabled { "✅" } else { "⏭️" }, c.label);
                    }
                }
            }
            Ok(())
        }
        ChannelSubcommand::Add {
            channel_type,
            config_json,
            token,
        } => {
            let channel_type = normalize_channel_type(&channel_type)?;
            let token =
                resolve_channel_token(&channel_type, token.as_deref(), config_json.as_deref())?;
            set_channel_enabled(config, &channel_type, true, token.as_deref())?;
            persist_config(config, config_path)?;
            let saved_at = config_path
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| "<in-memory>".to_string());
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "status": "added",
                        "channel": channel_type,
                        "config": saved_at,
                        // Whether the agent can RECEIVE on this channel.
                        // A platform that is send-only has a complete
                        // inbound parser that nothing calls, so a message
                        // sent to it never reaches the agent — and the
                        // operator would otherwise believe the opposite.
                        "inbound": operant_core::gateway::platform_can_receive(&channel_type),
                    })
                );
            } else {
                println!("Added {channel_type} channel and persisted the config to {saved_at}.");
                if !operant_core::gateway::platform_can_receive(&channel_type) {
                    println!(
                        "NOTE: {channel_type} is SEND-ONLY — operant can post to it, but \
                         messages sent to it never reach the agent."
                    );
                }
                // The doctor does NOT reach the platform — it reports the
                // configuration, not a probe — so telling the operator to
                // "verify connectivity" with it was a false instruction.
                println!("The doctor reports the CONFIGURED channels; it does not probe them.");
            }
            Ok(())
        }
        ChannelSubcommand::Remove { name } => {
            let name = normalize_channel_type(&name)?;
            set_channel_enabled(config, &name, false, None)?;
            persist_config(config, config_path)?;
            let saved_at = config_path
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| "<in-memory>".to_string());
            if json {
                println!(
                    "{}",
                    serde_json::json!({"status":"removed","channel": name, "config": saved_at})
                );
            } else {
                println!(
                    "Removed {name} channel (disabled) and persisted the config to {saved_at}."
                );
            }
            Ok(())
        }
        ChannelSubcommand::BindTelegram { identity } => {
            // R15-2: this command previously claimed success without doing
            // anything (no allowlist persistence existed for it). Report the
            // truth: the live gateway enforces `[gateway] admins`, not a
            // per-channel allowlist.
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "status": "not-applied",
                        "identity": identity,
                        "message": "The gateway does not support a per-user Telegram allowlist. Add the identity to [gateway] admins in operant.toml instead."
                    })
                );
            } else {
                println!("Cannot bind Telegram identity: {}", identity);
                println!(
                    "The gateway does not support a per-user Telegram allowlist.\nAdd the identity to `[gateway] admins` in operant.toml instead."
                );
            }
            Ok(())
        }
        ChannelSubcommand::Send {
            message,
            channel_id,
            recipient,
        } => {
            // R15-2: this previously printed a fake "sent" status without
            // delivering anything. Wire to the real gateway sender.
            let result = crate::gateway_runner::send_channel_message(
                config,
                &channel_id,
                &recipient,
                &message,
            )
            .await;
            match result {
                Ok(detail) => {
                    if json {
                        println!(
                            "{}",
                            serde_json::json!({"status":"sent","channel": channel_id, "recipient": recipient, "detail": detail})
                        );
                    } else {
                        println!("{detail}");
                    }
                }
                Err(e) => {
                    if json {
                        println!(
                            "{}",
                            serde_json::json!({"status":"error","channel": channel_id, "recipient": recipient, "message": e.to_string()})
                        );
                    } else {
                        println!("Failed to send: {e:#}");
                    }
                }
            }
            Ok(())
        }
        ChannelSubcommand::DeleteMessage {
            channel_id,
            recipient,
            message_id,
        } => {
            let result = crate::gateway_runner::delete_channel_message(
                config,
                &channel_id,
                &recipient,
                &message_id,
            )
            .await;
            match result {
                Ok(()) => {
                    if json {
                        println!(
                            "{}",
                            serde_json::json!({"status":"deleted","channel": channel_id, "recipient": recipient, "message_id": message_id})
                        );
                    } else {
                        println!("Deleted message {message_id} on {channel_id} ({recipient}).");
                    }
                }
                Err(e) => {
                    if json {
                        println!(
                            "{}",
                            serde_json::json!({"status":"error","channel": channel_id, "recipient": recipient, "message_id": message_id, "message": e.to_string()})
                        );
                    } else {
                        println!("Failed to delete: {e:#}");
                    }
                    // Non-zero exit: a retraction that did not happen must not
                    // look like one in a script.
                    std::process::exit(1);
                }
            }
            Ok(())
        }
    }
}

/// Normalize and validate a channel type name against the set this CLI can manage.
fn normalize_channel_type(raw: &str) -> Result<String> {
    let t = raw.trim().to_ascii_lowercase();
    // The CANONICAL platform keys — the ones the gateway's registry and
    // the config use.
    //
    // "webhooks" was here, and it is not a platform key: the key is
    // "webhook" (`webhooks_enabled`). So `operant channel add webhooks`
    // wrote a key the config does not have, and `operant channel add
    // webhook` — the correct spelling — was REJECTED as unsupported.
    // The plural is accepted as an ALIAS and normalised to the
    // canonical key, so both spellings work and the one that is written
    // is the one the config reads.
    //
    // "sms" was missing entirely, so RT-118's fix — SMS in the channel
    // LIST — had no way to be configured: `operant channel add sms` was
    // rejected. The registry has SEVEN adapters and the list shows all
    // seven; the write path now does too.
    const SUPPORTED: &[&str] = &[
        "telegram", "discord", "slack", "whatsapp", "email", "sms", "webhook",
    ];
    let canonical = if t == "webhooks" {
        "webhook"
    } else {
        t.as_str()
    };
    if SUPPORTED.contains(&canonical) {
        Ok(canonical.to_string())
    } else {
        bail!(
            "Unsupported channel type '{t}'. Supported: {}",
            SUPPORTED.join(", ")
        )
    }
}

/// Resolve the bot token from the --token flag or the CONFIG JSON (`api_key`,
/// `token`, or `bot_token` keys). Token-backed platforms require one; email and
/// webhooks do not.
fn resolve_channel_token(
    channel_type: &str,
    token_flag: Option<&str>,
    config_json: Option<&str>,
) -> Result<Option<String>> {
    if let Some(key) = token_flag.filter(|k| !k.trim().is_empty()) {
        return Ok(Some(key.trim().to_string()));
    }
    if let Some(raw) = config_json {
        let raw = raw.trim();
        if !raw.is_empty() {
            let value: serde_json::Value =
                serde_json::from_str(raw).with_context(|| format!("Invalid CONFIG JSON: {raw}"))?;
            for key in ["api_key", "token", "bot_token"] {
                if let Some(s) = value
                    .get(key)
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                {
                    return Ok(Some(s.trim().to_string()));
                }
            }
        }
    }
    if matches!(channel_type, "telegram" | "discord" | "slack" | "whatsapp") {
        bail!(
            "A token is required for {channel_type}: pass --token <TOKEN> or a CONFIG JSON with an api_key field"
        )
    }
    Ok(None)
}

/// Enable/disable a channel on the runtime GatewayConfig and (for token-backed
/// platforms) set or clear its token.
fn set_channel_enabled(
    config: &mut AppConfig,
    channel_type: &str,
    enabled: bool,
    token: Option<&str>,
) -> Result<()> {
    let gw = &mut config.gateway;
    let token = token.map(str::to_string);
    match channel_type {
        "telegram" => {
            gw.telegram_enabled = enabled;
            gw.telegram_token = token;
        }
        "discord" => {
            gw.discord_enabled = enabled;
            gw.discord_token = token;
        }
        "slack" => {
            gw.slack_enabled = enabled;
            gw.slack_token = token;
        }
        "whatsapp" => {
            gw.whatsapp_enabled = enabled;
            gw.whatsapp_token = token;
        }
        "email" => {
            gw.email_enabled = enabled;
        }
        "sms" => {
            // The SMS adapter reads its Twilio credentials from the
            // environment (`TWILIO_ACCOUNT_SID` and friends), so there
            // is no token to store here — only the switch.
            gw.sms_twilio_enabled = enabled;
        }
        "webhook" => {
            gw.webhooks_enabled = enabled;
        }
        other => bail!("Unsupported channel type '{other}'"),
    }
    // Syntactic sanity check only — never calls the platform API.
    if enabled && channel_type == "telegram" {
        let Some(tok) = gw.telegram_token.as_deref() else {
            return Ok(());
        };
        let looks_like_bot_token = tok
            .split(':')
            .next()
            .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
            && tok.contains(':')
            && !tok.contains(' ');
        if !looks_like_bot_token {
            eprintln!(
                "warning: token '{tok}' does not look like a Telegram bot token (expected digits:alphanumeric); storing anyway"
            );
        }
    }
    Ok(())
}

/// Persist the mutated config back to the file that was loaded (requires -c).
fn persist_config(config: &AppConfig, config_path: Option<&Path>) -> Result<()> {
    let Some(path) = config_path else {
        bail!("No config file to persist to. Pass -c/--config <PATH> so the change can be saved.")
    };
    let content = toml::to_string(config).context("Failed to serialize config")?;
    std::fs::write(path, content)
        .with_context(|| format!("Failed to write config to {}", path.display()))?;
    Ok(())
}

/// Build a list of (channel_name, is_enabled) from config.
fn build_channel_list(config: &AppConfig) -> Vec<ChannelEntry> {
    let gw = &config.gateway;
    // Every platform in the gateway's registry. SMS was missing: the
    // registry has SEVEN adapters and the list showed six, so an operator
    // with `sms_twilio_enabled = true` saw no SMS in the channel list.
    //
    // The entry carries the platform KEY as well as the display label:
    // the inbound lookup needs the key ("webhook"), and the label is
    // "Webhooks" — looking the table up by the label silently missed it.
    vec![
        ChannelEntry::new("cli", "CLI", true),
        ChannelEntry::new("telegram", "Telegram", gw.telegram_enabled),
        ChannelEntry::new("discord", "Discord", gw.discord_enabled),
        ChannelEntry::new("slack", "Slack", gw.slack_enabled),
        ChannelEntry::new("whatsapp", "WhatsApp", gw.whatsapp_enabled),
        ChannelEntry::new("email", "Email", gw.email_enabled),
        ChannelEntry::new("sms", "SMS", gw.sms_twilio_enabled),
        ChannelEntry::new("webhook", "Webhooks", gw.webhooks_enabled),
    ]
}

/// One row of the channel list: the platform KEY (for the inbound
/// lookup), the display LABEL, and whether it is enabled.
struct ChannelEntry {
    key: &'static str,
    label: &'static str,
    enabled: bool,
}

impl ChannelEntry {
    fn new(key: &'static str, label: &'static str, enabled: bool) -> Self {
        Self {
            key,
            label,
            enabled,
        }
    }

    /// Whether the agent can RECEIVE on this channel.
    ///
    /// The CLI is the operator's own terminal, not a messaging platform,
    /// so it is not in the table — and it is not a send-only channel
    /// either. It is simply not a channel, and the column does not apply
    /// to it.
    fn can_receive(&self) -> bool {
        if self.key == "cli" {
            return true;
        }
        operant_core::gateway::platform_can_receive(self.key)
    }

    /// Whether the inbound column applies to this row at all.
    fn is_platform(&self) -> bool {
        self.key != "cli"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every platform the gateway's registry has must be configurable.
    ///
    /// The list showed SMS (RT-118) but the WRITE path did not: the
    /// supported set had no "sms", so `operant channel add sms` was
    /// rejected. The list and the write path had drifted.
    #[test]
    fn every_platform_in_the_registry_is_configurable() {
        for platform in [
            "telegram", "discord", "slack", "whatsapp", "email", "sms", "webhook",
        ] {
            let normalized = normalize_channel_type(platform)
                .unwrap_or_else(|e| panic!("{platform} must be configurable: {e}"));
            assert_eq!(&normalized, platform, "{platform} must normalise to itself");
        }
    }

    /// The platform key is "webhook" — the config's key is
    /// `webhooks_enabled`, so the plural "webhooks" was never a valid
    /// key. It is accepted as an ALIAS and normalised, so
    /// `channel add webhooks` works AND `channel add webhook` (the
    /// correct spelling) is not rejected.
    #[test]
    fn the_webhook_platform_key_is_canonicalised() {
        assert_eq!(
            normalize_channel_type("webhook").expect("webhook is supported"),
            "webhook"
        );
        assert_eq!(
            normalize_channel_type("webhooks").expect("the plural is an alias"),
            "webhook",
            "the plural must normalise to the canonical key"
        );
        assert_eq!(
            normalize_channel_type("WebHooks").expect("the case is ignored"),
            "webhook"
        );
    }

    /// An unknown platform is still rejected.
    #[test]
    fn an_unknown_platform_is_rejected() {
        assert!(normalize_channel_type("carrier-pigeon").is_err());
    }

    /// The switch must write the config key the gateway reads.
    #[test]
    fn the_sms_switch_writes_the_config_key() {
        let mut config = AppConfig::default();
        set_channel_enabled(&mut config, "sms", true, None).expect("sms is supported");
        assert!(
            config.gateway.sms_twilio_enabled,
            "the SMS switch must set sms_twilio_enabled"
        );
    }

    /// The webhook switch must write `webhooks_enabled` — the key the
    /// config has — for the canonical platform name.
    #[test]
    fn the_webhook_switch_writes_the_config_key() {
        let mut config = AppConfig::default();
        set_channel_enabled(&mut config, "webhook", true, None).expect("webhook is supported");
        assert!(
            config.gateway.webhooks_enabled,
            "the webhook switch must set webhooks_enabled"
        );
    }
}
