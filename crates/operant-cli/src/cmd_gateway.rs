//! CLI subcommand for gateway management.
//!
//! Provides `operant gateway <subcommand>` for inspecting gateway status,
//! sessions, channels, and statistics, plus webhook, hooks, and pairing
//! management.  This module works **without** a running gateway instance —
//! it is a read-only management view backed by configuration and (when
//! available) the in-process gateway state.

use anyhow::{Context, Result};
use clap::Subcommand;
use operant_core::config::AppConfig;
use tokio::signal::unix::{SignalKind, signal};

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
/// Gateway connects Operant to messaging platforms such as Telegram, Discord,
/// and Slack.  These commands inspect configuration and, when applicable,
/// query the in-process gateway state.
#[derive(Debug, Clone, Subcommand)]
pub enum GatewaySubcommand {
    /// Run the gateway in foreground (blocking, Ctrl+C to stop)
    Run,
    /// Show gateway status (enabled platforms, runtime state)
    Status {
        /// Show detailed service status including systemd info
        #[arg(long)]
        deep: bool,
    },
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
    /// Start the gateway service (via systemd)
    Start,
    /// Stop the gateway service
    Stop,
    /// Restart the gateway service
    Restart,
    /// Install systemd service for automatic gateway startup
    Install {
        /// Force reinstall even if already installed
        #[arg(long)]
        force: bool,
        /// Install as system-wide service (requires root)
        #[arg(long)]
        system: bool,
    },
    /// Uninstall the gateway systemd service
    Uninstall,
    /// List gateway profiles
    List,
    /// Remove legacy service units
    MigrateLegacy,
}

// ── Public dispatcher ─────────────────────────────────────────────────

/// Dispatch and execute a gateway subcommand.
///
/// `config` is the currently active `AppConfig`.  Because the gateway
/// requires runtime adapter setup, this handler provides a management view
/// that works without an active gateway instance.
pub async fn handle_gateway_command(
    config: &AppConfig,
    cmd: GatewaySubcommand,
    json: bool,
) -> Result<()> {
    match cmd {
        GatewaySubcommand::Run => cmd_run(config).await,
        GatewaySubcommand::Status { deep } => cmd_status(config, deep, json),
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
        GatewaySubcommand::Start => cmd_start().await,
        GatewaySubcommand::Stop => cmd_stop().await,
        GatewaySubcommand::Restart => cmd_restart().await,
        GatewaySubcommand::Install { force, system } => cmd_install(config, force, system),
        GatewaySubcommand::Uninstall => cmd_uninstall().await,
        GatewaySubcommand::List => cmd_list(config),
        GatewaySubcommand::MigrateLegacy => cmd_migrate_legacy(),
    }
}

// ── Private handlers ──────────────────────────────────────────────────

/// Print gateway status from configuration.
///
/// Shows which platforms are enabled, the webhook state, and a reminder
/// that the gateway must be started for these to be active.
fn cmd_status(config: &AppConfig, deep: bool, json: bool) -> Result<()> {
    let gw = &config.gateway;

    // Cross-process running check via PID file
    let pid_path = operant_core::platform::operant_home().join("gateway.pid");
    let running = std::fs::read_to_string(&pid_path)
        .ok()
        .and_then(|pid_str| pid_str.trim().parse::<u32>().ok())
        .map(|pid| {
            std::process::Command::new("kill")
                .arg("-0")
                .arg(pid.to_string())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        })
        .unwrap_or(false);

    if !running && pid_path.exists() {
        let _ = std::fs::remove_file(&pid_path);
    }

    if json {
        let status = serde_json::json!({
            "platforms": {
                "telegram": {"enabled": gw.telegram_enabled, "token_configured": gw.telegram_token.is_some()},
                "discord": {"enabled": gw.discord_enabled, "token_configured": gw.discord_token.is_some()},
                "slack": {"enabled": gw.slack_enabled, "token_configured": gw.slack_token.is_some()},
                "whatsapp": {"enabled": gw.whatsapp_enabled, "token_configured": gw.whatsapp_token.is_some()},
                "email": {"enabled": gw.email_enabled, "smtp_host_configured": gw.email_smtp_host.is_some()},
                "sms": {"enabled": gw.sms_twilio_enabled},
            },
            "webhooks": {
                "enabled": gw.webhooks_enabled,
                "listen_addr": gw.webhooks_addr.as_deref().unwrap_or("localhost:0"),
            },
            "admins": gw.admins,
            "running": running,
            "pid_file": pid_path.display().to_string(),
        });
        println!("{}", serde_json::to_string_pretty(&status)?);
        return Ok(());
    }

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
    println!(
        "  WhatsApp  {}",
        indicator(gw.whatsapp_enabled, gw.whatsapp_token.is_some())
    );
    println!(
        "  Email     {}",
        indicator(gw.email_enabled, gw.email_smtp_host.is_some())
    );
    println!("  SMS       {}", indicator(gw.sms_twilio_enabled, false));
    println!();
    println!(
        "Webhooks:  {}",
        if gw.webhooks_enabled {
            let addr = gw.webhooks_addr.as_deref().unwrap_or("localhost:0");
            format!("enabled (listen on {addr})")
        } else {
            "disabled".to_string()
        }
    );
    println!(
        "Admins:    {}",
        if gw.admins.is_empty() {
            "none configured (all users allowed)".to_string()
        } else {
            gw.admins.join(", ")
        }
    );
    println!();

    if running {
        println!("Runtime: running (PID from {})", pid_path.display());
    } else {
        println!("Runtime: not running");
    }

    if deep {
        println!();
        println!("Service Status (systemd):");
        let sysd = std::process::Command::new("systemctl")
            .args(["--user", "is-active", "operant-gateway"])
            .output();
        match sysd {
            Ok(out) => {
                let status = String::from_utf8_lossy(&out.stdout).trim().to_string();
                println!("  Active: {}", status);
                if let Ok(enabled) = std::process::Command::new("systemctl")
                    .args(["--user", "is-enabled", "operant-gateway"])
                    .output()
                {
                    let enabled_str = String::from_utf8_lossy(&enabled.stdout).trim().to_string();
                    println!("  Enabled: {}", enabled_str);
                }
            }
            Err(_) => {
                println!("  systemd not available");
            }
        }
    }

    println!();
    println!("Run `operant gateway run` to start in foreground.");
    println!("Run `operant gateway start` to start via systemd.");
    println!("Run `operant gateway stop` to stop it.");

    Ok(())
}

/// Install systemd service for automatic gateway startup.
fn cmd_install(_config: &AppConfig, force: bool, system: bool) -> Result<()> {
    let operant_bin = std::env::current_exe().context("Failed to determine operant binary path")?;

    let (unit_dir, scope_label) = if system {
        ("/etc/systemd/system".to_string(), "system")
    } else {
        let home = std::env::var("HOME").context("HOME not set")?;
        (
            std::path::PathBuf::from(&home)
                .join(".config")
                .join("systemd")
                .join("user")
                .to_string_lossy()
                .to_string(),
            "user",
        )
    };

    let unit_path = std::path::PathBuf::from(&unit_dir).join("operant-gateway.service");

    if unit_path.exists() && !force {
        println!(
            "Gateway service already installed at: {}",
            unit_path.display()
        );
        println!("Use --force to reinstall.");
        println!();
        println!("To enable and start:");
        println!("  systemctl --{} enable operant-gateway", scope_label);
        println!("  systemctl --{} start operant-gateway", scope_label);
        return Ok(());
    }

    std::fs::create_dir_all(&unit_dir).context("Failed to create systemd unit directory")?;

    let unit_content = format!(
        r#"[Unit]
Description=Operant Multi-Platform Gateway
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart={bin} gateway run
Restart=on-failure
RestartSec=5
RestartSteps=3
RestartMaxDelaySec=30
TimeoutStopSec=30

[Install]
WantedBy=default.target
"#,
        bin = operant_bin.display()
    );

    std::fs::write(&unit_path, unit_content.as_bytes())
        .with_context(|| format!("Failed to write {}", unit_path.display()))?;

    println!("Gateway systemd service installed:");
    println!("  {}", unit_path.display());
    println!("  Scope: {}", scope_label);
    println!();
    println!("To enable and start:");
    println!("  systemctl daemon-reload");
    println!("  systemctl --{} enable operant-gateway", scope_label);
    println!("  systemctl --{} start operant-gateway", scope_label);
    println!();
    println!("To view logs:");
    println!("  journalctl --{} -u operant-gateway -f", scope_label);

    Ok(())
}

/// Run the gateway in foreground.
async fn cmd_run(config: &AppConfig) -> Result<()> {
    let msg = crate::gateway_runner::start_gateway(config).await?;
    println!("{}", msg);
    println!("Press Ctrl+C (SIGINT) or send SIGTERM to stop the gateway.");

    let mut sigint = signal(SignalKind::interrupt())?;
    let mut sigterm = signal(SignalKind::terminate())?;

    tokio::select! {
        _ = sigint.recv() => {
            println!("\nReceived SIGINT (Ctrl+C). Shutting down gateway...");
        }
        _ = sigterm.recv() => {
            println!("\nReceived SIGTERM. Shutting down gateway...");
        }
    }

    crate::gateway_runner::stop_gateway().await?;
    println!("Gateway stopped.");
    Ok(())
}

/// Start the gateway service via systemd.
async fn cmd_start() -> Result<()> {
    let out = tokio::process::Command::new("systemctl")
        .args(["--user", "start", "operant-gateway"])
        .output()
        .await;

    match out {
        Ok(output) if output.status.success() => {
            println!("Gateway service started via systemd.");
            Ok(())
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("not found") || stderr.contains("No such") {
                println!("Gateway service is not installed.");
                println!("Run `operant gateway install` to install it first.");
            } else {
                println!("Failed to start gateway service: {}", stderr.trim());
                println!("Try `operant gateway run` for foreground mode.");
            }
            Ok(())
        }
        Err(_) => {
            println!("systemd is not available on this system.");
            println!("Try `operant gateway run` for foreground mode.");
            Ok(())
        }
    }
}

/// Stop the gateway service.
async fn cmd_stop() -> Result<()> {
    let sysd = tokio::process::Command::new("systemctl")
        .args(["--user", "stop", "operant-gateway"])
        .output()
        .await;

    if let Ok(output) = &sysd
        && output.status.success()
    {
        println!("Gateway service stopped via systemd.");
        return Ok(());
    }

    let pid_path = operant_core::platform::operant_home().join("gateway.pid");
    if let Ok(pid_str) = std::fs::read_to_string(&pid_path)
        && let Ok(pid) = pid_str.trim().parse::<u32>()
    {
        let kill = tokio::process::Command::new("kill")
            .arg(pid.to_string())
            .output()
            .await;
        if let Ok(k) = kill
            && k.status.success()
        {
            println!("Gateway process ({}) killed.", pid);
            let _ = std::fs::remove_file(&pid_path);
            return Ok(());
        }
    }

    let msg = crate::gateway_runner::stop_gateway().await?;
    println!("{}", msg);
    Ok(())
}

/// Restart the gateway service.
async fn cmd_restart() -> Result<()> {
    let out = tokio::process::Command::new("systemctl")
        .args(["--user", "restart", "operant-gateway"])
        .output()
        .await;

    match out {
        Ok(output) if output.status.success() => {
            println!("Gateway service restarted via systemd.");
            Ok(())
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("not found") || stderr.contains("No such") {
                println!("Gateway service is not installed.");
                println!("Run `operant gateway install` to install it first.");
            } else {
                println!("Failed to restart gateway service: {}", stderr.trim());
                println!("Try stopping first with `operant gateway stop`");
                println!("then starting with `operant gateway run`.");
            }
            Ok(())
        }
        Err(_) => {
            println!("systemd is not available on this system.");
            println!("Use `operant gateway run` for foreground mode.");
            Ok(())
        }
    }
}

/// Uninstall the gateway systemd service.
async fn cmd_uninstall() -> Result<()> {
    let _ = tokio::process::Command::new("systemctl")
        .args(["--user", "stop", "operant-gateway"])
        .output()
        .await;
    let _ = tokio::process::Command::new("systemctl")
        .args(["--user", "disable", "operant-gateway"])
        .output()
        .await;

    let home = std::env::var("HOME").unwrap_or_default();
    let unit_path = std::path::PathBuf::from(&home)
        .join(".config")
        .join("systemd")
        .join("user")
        .join("operant-gateway.service");

    if unit_path.exists() {
        std::fs::remove_file(&unit_path).context("Failed to remove service unit")?;
        println!("Removed: {}", unit_path.display());
    }

    let _ = tokio::process::Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .output()
        .await;

    let pid_path = operant_core::platform::operant_home().join("gateway.pid");
    if pid_path.exists() {
        let _ = std::fs::remove_file(&pid_path);
    }

    println!("Gateway service uninstalled.");
    Ok(())
}

/// List gateway profiles with status.
fn cmd_list(config: &AppConfig) -> Result<()> {
    let gw = &config.gateway;

    println!("Gateway Profiles");
    println!("────────────────");
    println!();

    let enabled_count = [
        gw.telegram_enabled,
        gw.discord_enabled,
        gw.slack_enabled,
        gw.whatsapp_enabled,
        gw.email_enabled,
        gw.sms_twilio_enabled,
        gw.webhooks_enabled,
    ]
    .iter()
    .filter(|&&e| e)
    .count();

    println!("Active profile: default");
    println!("  Platforms:    {}/7 enabled", enabled_count);
    println!(
        "  Telegram:     {}",
        if gw.telegram_enabled { "✓" } else { "✗" }
    );
    println!(
        "  Discord:      {}",
        if gw.discord_enabled { "✓" } else { "✗" }
    );
    println!(
        "  Slack:        {}",
        if gw.slack_enabled { "✓" } else { "✗" }
    );

    let pid_path = operant_core::platform::operant_home().join("gateway.pid");
    let running = std::fs::read_to_string(&pid_path)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .map(|pid| {
            std::process::Command::new("kill")
                .arg("-0")
                .arg(pid.to_string())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        })
        .unwrap_or(false);

    println!(
        "  Status:       {}",
        if running { "running" } else { "stopped" }
    );
    println!();
    println!("Run `operant gateway run` to start in foreground.");
    Ok(())
}

/// Remove legacy service units from previous versions.
fn cmd_migrate_legacy() -> Result<()> {
    let home = std::env::var("HOME").context("HOME not set")?;
    let user_unit_dir = std::path::PathBuf::from(&home)
        .join(".config")
        .join("systemd")
        .join("user");

    let legacy_names = [
        "operant.service",
        "operant-agent.service",
        "operant-gateway.service",
    ];
    let mut found = false;

    for name in &legacy_names {
        let path = user_unit_dir.join(name);
        if path.exists() {
            println!("Found legacy unit: {}", path.display());
            found = true;
        }
    }

    if !found {
        println!("No legacy service units found.");
        return Ok(());
    }

    println!();
    println!("To clean up legacy units manually:");
    for name in &legacy_names {
        let path = user_unit_dir.join(name);
        if path.exists() {
            println!("  rm {}", path.display());
        }
    }
    println!();
    println!("Then run:");
    println!("  systemctl --user daemon-reload");

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

    let store = operant_core::PersistentSessionStore::open(db_path)
        .context("Failed to open session store")?;

    let sessions = store.list_active_sessions(None);

    if sessions.is_empty() {
        println!("No active gateway sessions.");
        return Ok(());
    }

    println!("Gateway Sessions");
    println!("────────────────");
    println!();
    println!(
        "{:<22} {:<10} {:<28} {:<22} {:<22}",
        "Session ID", "Platform", "User ID", "Channel ID", "Created At"
    );
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

    let any_platform = gw.telegram_enabled
        || gw.discord_enabled
        || gw.slack_enabled
        || gw.whatsapp_enabled
        || gw.email_enabled
        || gw.sms_twilio_enabled
        || gw.webhooks_enabled;
    if !any_platform {
        println!("No gateway platforms are enabled in config.");
        println!(
            "Enable a platform (telegram, discord, slack, whatsapp, email, sms, webhooks) in your config file first."
        );
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

    let enabled_count = [
        gw.telegram_enabled,
        gw.discord_enabled,
        gw.slack_enabled,
        gw.whatsapp_enabled,
        gw.email_enabled,
        gw.sms_twilio_enabled,
        gw.webhooks_enabled,
    ]
    .iter()
    .filter(|&&e| e)
    .count();

    println!("Gateway Statistics");
    println!("──────────────────");
    println!();
    println!("Enabled platforms:  {enabled_count}/7");
    println!(
        "Webhooks:           {}",
        if gw.webhooks_enabled {
            "enabled"
        } else {
            "disabled"
        }
    );
    println!("Admins configured:  {}", gw.admins.len());
    println!();
    println!("Runtime statistics are not available without an active");
    println!("gateway. Start the gateway to see live metrics:");

    if gw.telegram_enabled {
        println!(
            "  • Telegram  {}",
            token_status(gw.telegram_token.as_deref())
        );
    }
    if gw.discord_enabled {
        println!(
            "  • Discord   {}",
            token_status(gw.discord_token.as_deref())
        );
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

    let addr = gw.webhooks_addr.as_deref().unwrap_or("not configured");
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
    println!("Start the gateway and re-run `operant gateway hooks doctor`.");
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
