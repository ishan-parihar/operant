//! Gateway platform definitions and per-platform setup functions.
//!
//! Defines all 27 known messaging/integration platforms with metadata and
//! interactive configuration prompts. Each platform maps to a subset of fields
//! on `AppConfig::GatewaySettings`.

use anyhow::Result;
use console::style;
use hermes_core::config::AppConfig;

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

/// All known gateway platforms. Keep this list in sync with `AppConfig::GatewaySettings` fields.
pub fn all_platforms() -> Vec<GatewayPlatform> {
    vec![
        platform!("telegram", "📱", "Telegram", setup_telegram),
        platform!("discord", "💬", "Discord", setup_discord),
        platform!("slack", "💼", "Slack", setup_slack),
        platform!("matrix", "🔐", "Matrix", setup_matrix),
        platform!("mattermost", "💬", "Mattermost", setup_mattermost),
        platform!("whatsapp", "📲", "WhatsApp", setup_whatsapp),
        platform!("signal", "📡", "Signal", setup_signal),
        platform!("email_smtp", "📧", "Email (SMTP)", setup_email_smtp),
        platform!("sms_twilio", "📱", "SMS (Twilio)", setup_sms_twilio),
        platform!("dingtalk", "💬", "DingTalk", setup_dingtalk),
        platform!("feishu_lark", "", "Feishu / Lark", setup_feishu_lark),
        platform!("wecom", "💬", "WeCom (Enterprise WeChat)", setup_wecom),
        platform!(
            "wecom_callback",
            "💬",
            "WeCom Callback (Self-Built App)",
            setup_wecom_callback
        ),
        platform!("imessage", "💬", "BlueBubbles (iMessage)", setup_imessage),
        platform!("qq_bot", "🐧", "QQ Bot", setup_qq_bot),
        platform!("yuanbao", "💎", "Yuanbao", setup_yuanbao),
        platform!("google_chat", "💬", "Google Chat", setup_google_chat),
        platform!("irc", "💬", "IRC", setup_irc),
        platform!("line", "💚", "LINE", setup_line),
        platform!(
            "microsoft_teams",
            "💼",
            "Microsoft Teams",
            setup_microsoft_teams
        ),
        platform!(
            "facebook_messenger",
            "💬",
            "Facebook Messenger",
            setup_facebook_messenger
        ),
        platform!("wechat", "💬", "WeChat", setup_wechat),
        platform!("viber", "💬", "Viber", setup_viber),
        platform!(
            "google_business_messages",
            "💬",
            "Google Business Messages",
            setup_google_business
        ),
        platform!("twitter", "🐦", "Twitter / X", setup_twitter),
        platform!("instagram", "📷", "Instagram", setup_instagram),
        platform!("webhooks", "🔗", "Webhook", setup_webhooks),
    ]
}

// ---------------------------------------------------------------------------
// Per-platform setup helpers
// ---------------------------------------------------------------------------

/// Helper: print a platform header line.
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

fn setup_matrix(config: &mut AppConfig) -> Result<()> {
    print_platform_header("Matrix");
    handle_token(
        "Matrix",
        "Matrix access token",
        &mut config.gateway.matrix_token,
        &mut config.gateway.matrix_enabled,
    )
}

fn setup_mattermost(config: &mut AppConfig) -> Result<()> {
    print_platform_header("Mattermost");
    handle_token(
        "Mattermost",
        "Mattermost token",
        &mut config.gateway.mattermost_token,
        &mut config.gateway.mattermost_enabled,
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

fn setup_signal(config: &mut AppConfig) -> Result<()> {
    print_platform_header("Signal");
    handle_token(
        "Signal",
        "Signal token",
        &mut config.gateway.signal_token,
        &mut config.gateway.signal_enabled,
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

fn setup_dingtalk(config: &mut AppConfig) -> Result<()> {
    print_platform_header("DingTalk");
    config.gateway.dingtalk_enabled = true;
    println!(
        "  {} DingTalk enabled. Configure webhook URL and secret in config file.",
        style("✓").green()
    );
    Ok(())
}

fn setup_feishu_lark(config: &mut AppConfig) -> Result<()> {
    print_platform_header("Feishu / Lark");
    config.gateway.feishu_lark_enabled = true;
    println!(
        "  {} Feishu / Lark enabled. Configure webhook URL and secret in config file.",
        style("✓").green()
    );
    Ok(())
}

fn setup_wecom(config: &mut AppConfig) -> Result<()> {
    print_platform_header("WeCom (Enterprise WeChat)");
    config.gateway.wecom_enabled = true;
    println!(
        "  {} WeCom enabled. Configure webhook URL in config file.",
        style("✓").green()
    );
    Ok(())
}

fn setup_wecom_callback(config: &mut AppConfig) -> Result<()> {
    print_platform_header("WeCom Callback (Self-Built App)");
    config.gateway.wecom_callback_enabled = true;
    println!(
        "  {} WeCom Callback enabled. Configure token and encoding key in config file.",
        style("✓").green()
    );
    Ok(())
}

fn setup_imessage(config: &mut AppConfig) -> Result<()> {
    print_platform_header("BlueBubbles (iMessage)");
    config.gateway.imessage_enabled = true;
    println!(
        "  {} iMessage enabled (uses native macOS or BlueBubbles bridge).",
        style("✓").green()
    );
    Ok(())
}

fn setup_qq_bot(config: &mut AppConfig) -> Result<()> {
    print_platform_header("QQ Bot");
    config.gateway.qq_bot_enabled = true;
    println!(
        "  {} QQ Bot enabled. Configure bot credentials in config file.",
        style("✓").green()
    );
    Ok(())
}

fn setup_yuanbao(config: &mut AppConfig) -> Result<()> {
    print_platform_header("Yuanbao");
    config.gateway.yuanbao_enabled = true;
    println!(
        "  {} Yuanbao enabled. Configure API endpoint in config file.",
        style("✓").green()
    );
    Ok(())
}

fn setup_google_chat(config: &mut AppConfig) -> Result<()> {
    print_platform_header("Google Chat");
    config.gateway.google_chat_enabled = true;
    println!(
        "  {} Google Chat enabled. Configure webhook URL in config file.",
        style("✓").green()
    );
    Ok(())
}

fn setup_irc(config: &mut AppConfig) -> Result<()> {
    print_platform_header("IRC");
    config.gateway.irc_enabled = true;
    println!(
        "  {} IRC enabled. Configure server, port, and nickname in config file.",
        style("✓").green()
    );
    Ok(())
}

fn setup_line(config: &mut AppConfig) -> Result<()> {
    print_platform_header("LINE");
    handle_token(
        "LINE",
        "LINE channel token",
        &mut config.gateway.line_token,
        &mut config.gateway.line_enabled,
    )
}

fn setup_microsoft_teams(config: &mut AppConfig) -> Result<()> {
    print_platform_header("Microsoft Teams");
    config.gateway.microsoft_teams_enabled = true;
    println!(
        "  {} Microsoft Teams enabled. Configure webhook URL in config file.",
        style("✓").green()
    );
    Ok(())
}

fn setup_facebook_messenger(config: &mut AppConfig) -> Result<()> {
    print_platform_header("Facebook Messenger");
    handle_token(
        "Facebook Messenger",
        "Facebook Messenger access token",
        &mut config.gateway.facebook_messenger_token,
        &mut config.gateway.facebook_messenger_enabled,
    )
}

fn setup_wechat(config: &mut AppConfig) -> Result<()> {
    print_platform_header("WeChat");
    handle_token(
        "WeChat",
        "WeChat token",
        &mut config.gateway.wechat_token,
        &mut config.gateway.wechat_enabled,
    )
}

fn setup_viber(config: &mut AppConfig) -> Result<()> {
    print_platform_header("Viber");
    handle_token(
        "Viber",
        "Viber auth token",
        &mut config.gateway.viber_token,
        &mut config.gateway.viber_enabled,
    )
}

fn setup_google_business(config: &mut AppConfig) -> Result<()> {
    print_platform_header("Google Business Messages");
    handle_token(
        "Google Business Messages",
        "Google Business Messages token",
        &mut config.gateway.google_business_messages_token,
        &mut config.gateway.google_business_messages_enabled,
    )
}

fn setup_twitter(config: &mut AppConfig) -> Result<()> {
    print_platform_header("Twitter / X");
    handle_token(
        "Twitter",
        "Twitter bearer token",
        &mut config.gateway.twitter_token,
        &mut config.gateway.twitter_enabled,
    )
}

fn setup_instagram(config: &mut AppConfig) -> Result<()> {
    print_platform_header("Instagram");
    handle_token(
        "Instagram",
        "Instagram access token",
        &mut config.gateway.instagram_token,
        &mut config.gateway.instagram_enabled,
    )
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
