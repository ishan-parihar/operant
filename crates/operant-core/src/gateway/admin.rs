//! `admin` — extracted verbatim from gateway/mod.rs.

use crate::error::Result;

use super::*;

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
