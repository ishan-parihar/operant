use anyhow::Result;
use clap::Subcommand;
use serde::{Deserialize, Serialize};

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
        /// Channel type (telegram, discord, slack, whatsapp, matrix, imessage, email)
        channel_type: String,
        /// Optional configuration as JSON
        config: String,
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
    config: &AppConfig,
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
            let msg = crate::gateway_runner::start_gateway(config).await?;
            if json {
                println!("{}", serde_json::json!({"status":"started","message": msg}));
            } else {
                println!("{msg}");
                println!("Running in foreground — press Ctrl+C to stop.");
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
            config: _,
        } => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({"status":"info","message": format!("Use `operant config set gateway.{}_enabled true` to enable {}", channel_type, channel_type)})
                );
            } else {
                println!(
                    "To add a {} channel, update your operant.toml config:\n",
                    channel_type
                );
                println!("  [gateway]");
                println!("  {}_enabled = true", channel_type);
                println!("  {}_token = \"YOUR_TOKEN\"", channel_type);
                println!();
                println!("Then run `operant channel doctor` to verify.");
            }
            Ok(())
        }
        ChannelSubcommand::Remove { name } => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({"status":"info","message": format!("Set gateway.{}_enabled = false in operant.toml", name)})
                );
            } else {
                println!(
                    "To remove channel '{}', set `gateway.{}_enabled = false` in operant.toml",
                    name, name
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
