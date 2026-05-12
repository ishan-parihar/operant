//! CLI subcommand for gateway management.
//!
//! Provides `hermes gateway <subcommand>` for inspecting gateway status,
//! sessions, channels, and statistics, plus webhook, hooks, and pairing
//! management.  This module works **without** a running gateway instance —
//! it is a read-only management view backed by configuration and (when
//! available) the in-process gateway state.

use anyhow::{Context, Result};
use clap::Subcommand;
use hermes_core::config::AppConfig;

// ── Sub-subcommand enums ────────────────────────────────────────────────

/// Webhook management actions.
#[derive(Debug, Clone, Subcommand)]
pub enum WebhookAction {
    /// List configured webhooks
    List,
    /// Test connectivity to a webhook URL
    Test {
        /// Webhook URL to test
        url: String,
    },
}

/// Hook (platform event hook) management actions.
#[derive(Debug, Clone, Subcommand)]
pub enum HooksAction {
    /// List configured hooks
    List,
    /// Check hook health
    Doctor,
}

/// Gateway pairing management actions.
#[derive(Debug, Clone, Subcommand)]
pub enum PairingAction {
    /// List pending pairings
    List,
    /// Approve a pending pairing
    Approve {
        /// Pairing code to approve
        code: String,
    },
    /// Revoke an existing pairing
    Revoke {
        /// Pairing ID to revoke
        id: String,
    },
}

// ── Top-level gateway subcommand ───────────────────────────────────────

/// Manage the multi-platform gateway.
///
/// Gateway connects Hermes to messaging platforms such as Telegram, Discord,
/// and Slack.  These commands inspect configuration and, when applicable,
/// query the in-process gateway state.
#[derive(Debug, Clone, Subcommand)]
pub enum GatewaySubcommand {
    /// Show gateway status (enabled platforms, runtime state)
    Status,
    /// List active gateway sessions
    Sessions,
    /// List registered channels
    Channels,
    /// Show gateway statistics
    Stats,
    /// Manage webhooks
    Webhook {
        #[command(subcommand)]
        action: WebhookAction,
    },
    /// Manage platform event hooks
    Hooks {
        #[command(subcommand)]
        action: HooksAction,
    },
    /// Manage gateway pairings
    Pairing {
        #[command(subcommand)]
        action: PairingAction,
    },
    /// Start the gateway and platform adapters
    Start,
    /// Stop the gateway and platform adapters
    Stop,
    /// Restart the gateway
    Restart,
}

// ── Public dispatcher ─────────────────────────────────────────────────

/// Dispatch and execute a gateway subcommand.
///
/// `config` is the currently active `AppConfig`.  Because the gateway
/// requires runtime adapter setup, this handler provides a management view
/// that works without an active gateway instance.
pub async fn handle_gateway_command(config: &AppConfig, cmd: GatewaySubcommand) -> Result<()> {
    match cmd {
        GatewaySubcommand::Status => cmd_status(config),
        GatewaySubcommand::Sessions => cmd_sessions(config),
        GatewaySubcommand::Channels => cmd_channels(config),
        GatewaySubcommand::Stats => cmd_stats(config),
        GatewaySubcommand::Webhook { action } => match action {
            WebhookAction::List => cmd_webhook_list(config),
            WebhookAction::Test { url } => cmd_webhook_test(config, &url),
        },
        GatewaySubcommand::Hooks { action } => match action {
            HooksAction::List => cmd_hooks_list(config),
            HooksAction::Doctor => cmd_hooks_doctor(config),
        },
        GatewaySubcommand::Pairing { action } => match action {
            PairingAction::List => cmd_pairing_list(config),
            PairingAction::Approve { code } => cmd_pairing_approve(config, &code),
            PairingAction::Revoke { id } => cmd_pairing_revoke(config, &id),
        },
        GatewaySubcommand::Start => {
            crate::gateway_runner::start_gateway(config)
                .await
                .map(|msg| {
                    println!("{}", msg);
                })
        }
        GatewaySubcommand::Stop => {
            crate::gateway_runner::stop_gateway()
                .await
                .map(|msg| {
                    println!("{}", msg);
                })
        }
        GatewaySubcommand::Restart => {
            crate::gateway_runner::restart_gateway(config)
                .await
                .map(|msg| {
                    println!("{}", msg);
                })
        }
    }
}

// ── Private handlers ──────────────────────────────────────────────────

/// Print gateway status from configuration.
///
/// Shows which platforms are enabled, the webhook state, and a reminder
/// that the gateway must be started for these to be active.
fn cmd_status(config: &AppConfig) -> Result<()> {
    let gw = &config.gateway;

    println!("Gateway Status (from config)");
    println!("────────────────────────────");
    println!();

    println!("Platforms:");
    println!(
        "  Telegram  {}",
        indicator(gw.telegram_enabled, gw.telegram_token.is_some())
    );
    println!(
        "  Discord   {}",
        indicator(gw.discord_enabled, gw.discord_token.is_some())
    );
    println!(
        "  Slack     {}",
        indicator(gw.slack_enabled, gw.slack_token.is_some())
    );
    println!();
    println!(
        "Webhooks:  {}",
        if gw.webhooks_enabled {
            let addr = gw
                .webhooks_addr
                .as_deref()
                .unwrap_or("localhost:0");
            format!("enabled (listen on {addr})")
        } else {
            "disabled".to_string()
        }
    );
    println!("Admins:    {}", if gw.admins.is_empty() {
        "none configured (all users allowed)".to_string()
    } else {
        gw.admins.join(", ")
    });
    println!();
    println!("Run `hermes gateway sessions` / `channels` / `stats`");
    println!("after starting the gateway to see live data.");
    println!();
    println!("Note: Gateway is not running — use the TUI or");
    println!("      programmatic API to start platform adapters.");

    Ok(())
}

/// Show the platform indicator badge.
fn indicator(enabled: bool, has_token: bool) -> String {
    match (enabled, has_token) {
        (true, true) => "✓ enabled, token configured".to_string(),
        (true, false) => "⚠ enabled, MISSING TOKEN".to_string(),
        (false, _) => "✗ disabled".to_string(),
    }
}

/// List active gateway sessions from the persistent session store.
fn cmd_sessions(config: &AppConfig) -> Result<()> {
    let db_path = config
        .database_path
        .to_str()
        .context("database_path is not valid UTF-8")?;

    let store =
        hermes_core::PersistentSessionStore::open(db_path).context("Failed to open session store")?;

    let sessions = store.list_active_sessions(None);

    if sessions.is_empty() {
        println!("No active gateway sessions.");
        return Ok(());
    }

    println!("Gateway Sessions");
    println!("────────────────");
    println!();
    println!("{:<22} {:<10} {:<28} {:<22} {:<22}", "Session ID", "Platform", "User ID", "Channel ID", "Created At");
    println!("{}", "-".repeat(110));
    for s in &sessions {
        println!(
            "{:<22} {:<10} {:<28} {:<22} {:<22}",
            truncate(&s.session_id, 20),
            truncate(&s.platform, 8),
            truncate(&s.platform_user_id, 26),
            truncate(&s.platform_channel_id, 20),
            fmt_ts_short(&s.created_at),
        );
    }
    println!();
    println!("Total: {} session(s)", sessions.len());
    Ok(())
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() > max_len {
        let boundary = (0..=max_len)
            .rev()
            .find(|&i| s.is_char_boundary(i))
            .unwrap_or(0);
        format!("{}…", &s[..boundary])
    } else {
        s.to_string()
    }
}

fn fmt_ts_short(rfc3339: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(rfc3339)
        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|_| rfc3339.to_string())
}

/// List registered gateway channels.
///
/// Without a running gateway instance this prints a placeholder message.
fn cmd_channels(config: &AppConfig) -> Result<()> {
    let gw = &config.gateway;

    let any_platform = gw.telegram_enabled || gw.discord_enabled || gw.slack_enabled;
    if !any_platform {
        println!("No gateway platforms are enabled in config.");
        println!("Enable a platform (telegram, discord, slack) in your config file first.");
        return Ok(());
    }

    println!("Gateway Channels");
    println!("────────────────");
    println!();
    println!("{:<8} {:<10} {:<28}", "#", "Platform", "Channel ID");
    println!("{}", "-".repeat(50));

    // Print platform-level channel info derived from config.
    if gw.telegram_enabled {
        println!("{:<8} {:<10} {:<28}", "—", "telegram", "(config defined)");
    }
    if gw.discord_enabled {
        println!("{:<8} {:<10} {:<28}", "—", "discord", "(config defined)");
    }
    if gw.slack_enabled {
        println!("{:<8} {:<10} {:<28}", "—", "slack", "(config defined)");
    }
    println!();
    println!("Channels are registered dynamically when platform adapters start.");
    println!("Enable the gateway or connect platforms to populate this list.");

    Ok(())
}

/// Show gateway statistics.
///
/// Statistics only become meaningful once a gateway instance is running.
fn cmd_stats(config: &AppConfig) -> Result<()> {
    let gw = &config.gateway;

    let enabled_count = [gw.telegram_enabled, gw.discord_enabled, gw.slack_enabled]
        .iter()
        .filter(|&&e| e)
        .count();

    println!("Gateway Statistics");
    println!("──────────────────");
    println!();
    println!("Enabled platforms:  {enabled_count}/3");
    println!("Webhooks:           {}", if gw.webhooks_enabled { "enabled" } else { "disabled" });
    println!("Admins configured:  {}", gw.admins.len());
    println!();
    println!("Runtime statistics are not available without an active");
    println!("gateway. Start the gateway to see live metrics:");

    if gw.telegram_enabled {
        println!("  • Telegram  {}", token_status(gw.telegram_token.as_deref()));
    }
    if gw.discord_enabled {
        println!("  • Discord   {}", token_status(gw.discord_token.as_deref()));
    }
    if gw.slack_enabled {
        println!("  • Slack     {}", token_status(gw.slack_token.as_deref()));
    }

    Ok(())
}

fn token_status(token: Option<&str>) -> &str {
    match token {
        Some(_) => "token configured",
        None => "TOKEN MISSING",
    }
}

/// List configured webhooks.
fn cmd_webhook_list(config: &AppConfig) -> Result<()> {
    let gw = &config.gateway;

    println!("Configured Webhooks");
    println!("──────────────────");
    println!();

    if !gw.webhooks_enabled {
        println!("Webhooks are not enabled in the config.");
        println!("Set `gateway.webhooks_enabled = true` in your config file.");
        return Ok(());
    }

    let addr = gw
        .webhooks_addr
        .as_deref()
        .unwrap_or("not configured");
    println!("Webhook listen address: {addr}");
    println!();
    println!("Webhooks are registered dynamically at runtime.");
    println!("Start the gateway to manage active webhooks.");

    Ok(())
}

/// Test a webhook URL for connectivity.
fn cmd_webhook_test(_config: &AppConfig, url: &str) -> Result<()> {
    println!("Testing webhook URL: {url}");
    println!();
    println!("Webhook testing requires an active gateway instance.");
    println!("Start the gateway and try again.");
    println!();
    println!("To test manually, send a POST request with:");
    println!("  curl -X POST {url} \\");
    println!("    -H \"Content-Type: application/json\" \\");
    println!("    -d '{{\"event\":\"ping\"}}'");

    Ok(())
}

/// List configured hooks.
fn cmd_hooks_list(config: &AppConfig) -> Result<()> {
    let gw = &config.gateway;

    println!("Configured Hooks");
    println!("───────────────");
    println!();

    if !gw.telegram_enabled && !gw.discord_enabled && !gw.slack_enabled {
        println!("No gateway platforms enabled — no hooks to display.");
        return Ok(());
    }

    println!("Platform-level hooks are managed at runtime.");
    println!("Enable the gateway to register and inspect hooks.");

    Ok(())
}

/// Check hook health.
fn cmd_hooks_doctor(_config: &AppConfig) -> Result<()> {
    println!("Hook Health Check");
    println!("────────────────");
    println!();
    println!("Hook diagnostics require a running gateway instance.");
    println!("Start the gateway and re-run `hermes gateway hooks doctor`.");
    println!();
    println!("A healthy hook should:");
    println!("  1. Be registered with a valid platform adapter");
    println!("  2. Respond to health-check pings");
    println!("  3. Have a non-expired authentication token");

    Ok(())
}

/// List pending pairings.
fn cmd_pairing_list(_config: &AppConfig) -> Result<()> {
    println!("Pending Pairings");
    println!("───────────────");
    println!();
    println!("No pending pairings.");
    println!();
    println!("Pairings are created when a user connects a new platform account.");
    println!("Start the gateway and enable platforms to accept pairings.");

    Ok(())
}

/// Approve a pending pairing by code.
fn cmd_pairing_approve(_config: &AppConfig, code: &str) -> Result<()> {
    println!("Approving pairing with code: {code}");
    println!();
    println!("Pairing approval requires an active gateway instance.");
    println!("Start the gateway and try again.");
    println!();
    println!("To approve manually, ensure the pairing code \"{code}\"");
    println!("matches a pending pairing request.");

    Ok(())
}

/// Revoke an existing pairing by ID.
fn cmd_pairing_revoke(_config: &AppConfig, id: &str) -> Result<()> {
    println!("Revoking pairing with ID: {id}");
    println!();
    println!("Pairing revocation requires an active gateway instance.");
    println!("Start the gateway and try again.");

    Ok(())
}
