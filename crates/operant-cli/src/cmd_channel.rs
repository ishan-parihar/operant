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
                    .map(|(name, enabled)| serde_json::json!({"name": name, "enabled": enabled}))
                    .collect();
                println!("{}", serde_json::json!({"channels": items}));
            } else {
                println!("Channels:");
                for (name, enabled) in &channels {
                    println!("  {} {}", if *enabled { "✅" } else { "❌" }, name);
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
            let configured: Vec<_> = channels.iter().filter(|(_, e)| *e).collect();
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "total": channels.len(),
                        "enabled": configured.len(),
                        "channels": channels.iter().map(|(n, e)| serde_json::json!({"name": n, "enabled": e, "healthy": e})).collect::<Vec<_>>()
                    })
                );
            } else {
                println!(
                    "Channel health: {}/{} enabled",
                    configured.len(),
                    channels.len()
                );
                for (name, enabled) in &channels {
                    println!("  {} {}", if *enabled { "✅" } else { "⏭️" }, name);
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
                    serde_json::json!({"status":"added","channel": channel_type, "config": saved_at})
                );
            } else {
                println!("Added {channel_type} channel and persisted the config to {saved_at}.");
                println!("Run `operant channel doctor` to verify connectivity.");
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
    }
}

/// Normalize and validate a channel type name against the set this CLI can manage.
fn normalize_channel_type(raw: &str) -> Result<String> {
    let t = raw.trim().to_ascii_lowercase();
    const SUPPORTED: &[&str] = &[
        "telegram", "discord", "slack", "whatsapp", "email", "webhooks",
    ];
    if SUPPORTED.contains(&t.as_str()) {
        Ok(t)
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
        "webhooks" => {
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
fn build_channel_list(config: &AppConfig) -> Vec<(String, bool)> {
    let gw = &config.gateway;
    vec![
        ("CLI".to_string(), true),
        ("Telegram".to_string(), gw.telegram_enabled),
        ("Discord".to_string(), gw.discord_enabled),
        ("Slack".to_string(), gw.slack_enabled),
        ("WhatsApp".to_string(), gw.whatsapp_enabled),
        ("Email".to_string(), gw.email_enabled),
        ("Webhooks".to_string(), gw.webhooks_enabled),
    ]
}
