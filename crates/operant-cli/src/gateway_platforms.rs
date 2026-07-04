//! Gateway platform definitions and per-platform setup functions.
//!
//! Defines all 27 known messaging/integration platforms with metadata and
//! interactive configuration prompts. Each platform maps to a subset of fields
//! on `AppConfig::GatewaySettings`.

use anyhow::Result;
use console::style;
use operant_core::config::AppConfig;

use crate::prompt_helpers::{prompt_key_action, prompt_password, KeyAction};

/// A gateway platform definition.
pub struct GatewayPlatform {
    pub key: &'static str,
    pub name: &'static str,
    pub icon: &'static str,
    pub description: &'static str,
    pub setup_fn: fn(config: &mut AppConfig) -> Result<()>,
}

/// Helper macro to define a `GatewayPlatform` entry.
macro_rules! platform {
    ($key:expr, $icon:expr, $name:expr, $setup:ident) => {
        GatewayPlatform {
            key: $key,
            name: concat!($icon, " ", $name),
            icon: $icon,
            description: concat!($name, " messaging integration"),
            setup_fn: $setup,
        }
    };
}

/// All known gateway platforms. Only platforms with real adapter
/// implementations are listed. Phantom platforms were purged.
pub fn all_platforms() -> Vec<GatewayPlatform> {
    vec![
        platform!("telegram", "📱", "Telegram", setup_telegram),
        platform!("discord", "💬", "Discord", setup_discord),
        platform!("slack", "💼", "Slack", setup_slack),
        platform!("whatsapp", "📲", "WhatsApp", setup_whatsapp),
        platform!("email_smtp", "📧", "Email (SMTP)", setup_email_smtp),
        platform!("sms_twilio", "📱", "SMS (Twilio)", setup_sms_twilio),
        platform!("webhooks", "🔗", "Webhook", setup_webhooks),
    ]
}

fn print_platform_header(name: &str) {
    println!("  {} {}", style("──").cyan(), name);
}

/// Helper: prompt for a token with keep/replace/clear logic.
fn handle_token(
    label: &str,
    prompt: &str,
    token: &mut Option<String>,
    enabled: &mut bool,
) -> Result<()> {
    if let Some(existing) = token {
        match prompt_key_action(label, existing)? {
            KeyAction::Keep => {}
            KeyAction::Replace => {
                let t = prompt_password(prompt)?;
                *token = Some(t);
            }
            KeyAction::Clear => {
                *token = None;
                *enabled = false;
                return Ok(());
            }
        }
    } else {
        let t = prompt_password(prompt)?;
        if !t.is_empty() {
            *token = Some(t);
        } else {
            *enabled = false;
            return Ok(());
        }
    }
    *enabled = true;
    Ok(())
}

fn setup_telegram(config: &mut AppConfig) -> Result<()> {
    print_platform_header("Telegram");
    handle_token(
        "Telegram",
        "Telegram bot token",
        &mut config.gateway.telegram_token,
        &mut config.gateway.telegram_enabled,
    )
}

fn setup_discord(config: &mut AppConfig) -> Result<()> {
    print_platform_header("Discord");
    handle_token(
        "Discord",
        "Discord bot token",
        &mut config.gateway.discord_token,
        &mut config.gateway.discord_enabled,
    )
}

fn setup_slack(config: &mut AppConfig) -> Result<()> {
    print_platform_header("Slack");
    handle_token(
        "Slack",
        "Slack bot token",
        &mut config.gateway.slack_token,
        &mut config.gateway.slack_enabled,
    )
}

fn setup_whatsapp(config: &mut AppConfig) -> Result<()> {
    print_platform_header("WhatsApp");
    handle_token(
        "WhatsApp",
        "WhatsApp token",
        &mut config.gateway.whatsapp_token,
        &mut config.gateway.whatsapp_enabled,
    )
}

fn setup_email_smtp(config: &mut AppConfig) -> Result<()> {
    print_platform_header("Email (SMTP)");
    if let Some(host) = &config.gateway.email_smtp_host {
        let masked = if host.len() > 20 {
            format!("{}…{}", &host[..10], &host[host.len() - 10..])
        } else {
            host.clone()
        };
        println!(
            "  {} SMTP host: {} {}",
            style("SMTP").bold(),
            masked,
            style("✓").green()
        );
        let choice: String = dialoguer::Input::new()
            .with_prompt("[K]eep / [R]eplace / [C]lear (default K)")
            .default("k".to_string())
            .allow_empty(true)
            .interact_text()
            .unwrap_or_default();
        match choice.trim().to_lowercase().chars().next() {
            Some('r') => {
                let h: String = dialoguer::Input::new()
                    .with_prompt("SMTP host (e.g. smtp.gmail.com)")
                    .allow_empty(true)
                    .interact_text()
                    .unwrap_or_default();
                config.gateway.email_smtp_host = if h.is_empty() { None } else { Some(h) };
            }
            Some('c') => {
                config.gateway.email_smtp_host = None;
                config.gateway.email_smtp_user = None;
                config.gateway.email_smtp_pass = None;
                config.gateway.email_enabled = false;
                return Ok(());
            }
            _ => {}
        }
    }

    if config.gateway.email_smtp_host.is_some() {
        let user_prompt = if let Some(u) = &config.gateway.email_smtp_user {
            format!("SMTP user [{}]", u)
        } else {
            "SMTP user (email address)".to_string()
        };
        let u: String = dialoguer::Input::new()
            .with_prompt(&user_prompt)
            .allow_empty(true)
            .interact_text()
            .unwrap_or_default();
        if !u.is_empty() {
            config.gateway.email_smtp_user = Some(u);
        }

        let p: String = dialoguer::Password::new()
            .with_prompt("SMTP password (or Enter to keep)")
            .allow_empty_password(true)
            .interact()
            .unwrap_or_default();
        if !p.is_empty() {
            config.gateway.email_smtp_pass = Some(p);
        }

        config.gateway.email_enabled = true;
    }
    Ok(())
}

fn setup_sms_twilio(config: &mut AppConfig) -> Result<()> {
    print_platform_header("SMS (Twilio)");
    config.gateway.sms_twilio_enabled = true;
    println!(
        "  {} SMS (Twilio) enabled. Configure account SID and auth token in config file.",
        style("✓").green()
    );
    Ok(())
}
fn setup_webhooks(config: &mut AppConfig) -> Result<()> {
    print_platform_header("Webhook");
    if let Some(addr) = &config.gateway.webhooks_addr {
        println!(
            "  {} Webhook addr: {} {}",
            style("Webhook").bold(),
            addr,
            style("✓").green()
        );
        let choice: String = dialoguer::Input::new()
            .with_prompt("[K]eep / [R]eplace / [C]lear (default K)")
            .default("k".to_string())
            .allow_empty(true)
            .interact_text()
            .unwrap_or_default();
        match choice.trim().to_lowercase().chars().next() {
            Some('r') => {
                let a: String = dialoguer::Input::new()
                    .with_prompt("Webhook listen address (e.g. 0.0.0.0:8080)")
                    .allow_empty(true)
                    .interact_text()
                    .unwrap_or_default();
                config.gateway.webhooks_addr = if a.is_empty() { None } else { Some(a) };
            }
            Some('c') => {
                config.gateway.webhooks_addr = None;
                config.gateway.webhooks_enabled = false;
                return Ok(());
            }
            _ => {}
        }
    } else {
        let a: String = dialoguer::Input::new()
            .with_prompt("Webhook listen address (e.g. 0.0.0.0:8080)")
            .allow_empty(true)
            .interact_text()
            .unwrap_or_default();
        config.gateway.webhooks_addr = if a.is_empty() { None } else { Some(a) };
    }

    config.gateway.webhooks_enabled = config.gateway.webhooks_addr.is_some();
    Ok(())
}
