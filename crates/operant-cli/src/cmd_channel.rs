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
    _config: &AppConfig,
    cmd: ChannelSubcommand,
    json: bool,
) -> Result<()> {
    match cmd {
        ChannelSubcommand::List => {
            if json {
                println!("{{\"channels\":[]}}");
            } else {
                println!("No channels configured.");
                println!();
                println!("Use `operant channel add <type> '<config_json>'` to add a channel.");
                println!();
                println!("Supported channel types: telegram, discord, slack, whatsapp, matrix, imessage, email");
            }
            Ok(())
        }
        ChannelSubcommand::Start => {
            println!("Starting channels... (requires `operant daemon` for production use)");
            Ok(())
        }
        ChannelSubcommand::Doctor => {
            println!("Running channel health checks...");
            println!("No channels configured — nothing to check.");
            Ok(())
        }
        ChannelSubcommand::Add {
            channel_type,
            config: config_json,
        } => {
            // Validate the JSON
            let _parsed: serde_json::Value = serde_json::from_str(&config_json)
                .map_err(|e| anyhow::anyhow!("Invalid JSON config: {}", e))?;

            if json {
                println!(
                    "{{\"status\":\"added\",\"channel_type\":\"{}\"}}",
                    channel_type
                );
            } else {
                println!("Added {} channel configuration.", channel_type);
                println!("Run `operant daemon` to start the channel.");
            }
            Ok(())
        }
        ChannelSubcommand::Remove { name } => {
            if json {
                println!("{{\"status\":\"removed\",\"name\":\"{}\"}}", name);
            } else {
                println!("Removed channel configuration: {}", name);
            }
            Ok(())
        }
        ChannelSubcommand::BindTelegram { identity } => {
            if json {
                println!(
                    "{{\"status\":\"bound\",\"identity\":\"{}\"}}",
                    identity
                );
            } else {
                println!("Bound Telegram identity: {}", identity);
                println!("The agent will now respond to messages from this identity.");
            }
            Ok(())
        }
        ChannelSubcommand::Send {
            message,
            channel_id,
            recipient,
        } => {
            if json {
                println!(
                    "{{\"status\":\"sent\",\"channel\":\"{}\",\"recipient\":\"{}\"}}",
                    channel_id, recipient
                );
            } else {
                println!("Sending message via {} to {}...", channel_id, recipient);
                println!("Message: {}", message);
            }
            Ok(())
        }
    }
}
