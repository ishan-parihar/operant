//! CLI subcommand for managing webhook subscriptions.
//!
//! Provides `hermes webhook <subcommand>` for managing webhook subscriptions
//! backed by a JSON file at `~/.hermes/webhook_subscriptions.json`.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use hermes_core::config::AppConfig;
use serde::{Deserialize, Serialize};

/// A single webhook subscription entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct WebhookSubscription {
    url: String,
    prompt: Option<String>,
    events: Vec<String>,
    description: Option<String>,
    skills: Vec<String>,
    deliver: Option<String>,
    deliver_chat_id: Option<String>,
    secret: Option<String>,
    deliver_only: bool,
}

/// Path to the webhook subscriptions JSON file.
fn subscriptions_path() -> Result<PathBuf> {
    let dir = hermes_core::platform::hermes_data_dir();
    Ok(dir.join("webhook_subscriptions.json"))
}

/// Load subscriptions from disk.
fn load_subscriptions() -> Result<HashMap<String, WebhookSubscription>> {
    let path = subscriptions_path()?;
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let content =
        fs::read_to_string(&path).with_context(|| format!("Failed to read {}", path.display()))?;
    serde_json::from_str(&content).with_context(|| format!("Failed to parse {}", path.display()))
}

/// Save subscriptions to disk atomically.
fn save_subscriptions(subs: &HashMap<String, WebhookSubscription>) -> Result<()> {
    let path = subscriptions_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    // Write to a temp file first, then rename for atomicity.
    let tmp = path.with_extension("json.tmp");
    let content = serde_json::to_string_pretty(subs)?;
    fs::write(&tmp, &content)?;
    fs::rename(&tmp, &path)?;
    Ok(())
}

#[derive(Debug, Clone, Subcommand)]
pub enum WebhookSubcommand {
    /// Subscribe a webhook URL
    Subscribe(WebhookSubscribeArgs),
    /// List all webhook subscriptions
    List,
    /// Remove a webhook subscription by URL
    Remove {
        /// URL to remove
        url: String,
    },
    /// Test a webhook (send a ping event)
    Test {
        /// URL to test
        url: String,
    },
}

#[derive(Debug, Clone, Args)]
pub struct WebhookSubscribeArgs {
    /// Webhook URL to subscribe
    pub url: String,

    /// Prompt to use when handling webhook events
    #[arg(long)]
    pub prompt: Option<String>,

    /// Comma-separated list of events to subscribe to (e.g. "message,reaction")
    #[arg(long, default_value = "message")]
    pub events: String,

    /// Human-readable description for this subscription
    #[arg(long)]
    pub description: Option<String>,

    /// Comma-separated list of skill names to enable
    #[arg(long)]
    pub skills: Option<String>,

    /// Delivery target (e.g. "telegram:chat_id")
    #[arg(long)]
    pub deliver: Option<String>,

    /// Delivery chat ID override
    #[arg(long)]
    pub deliver_chat_id: Option<String>,

    /// Optional secret for HMAC signing
    #[arg(long)]
    pub secret: Option<String>,

    /// Only deliver via webhook (don't process via agent)
    #[arg(long)]
    pub deliver_only: bool,
}

pub async fn handle_webhook_command(_config: &AppConfig, cmd: WebhookSubcommand) -> Result<()> {
    match cmd {
        WebhookSubcommand::Subscribe(args) => cmd_subscribe(args).await?,
        WebhookSubcommand::List => cmd_list().await?,
        WebhookSubcommand::Remove { url } => cmd_remove(&url).await?,
        WebhookSubcommand::Test { url } => cmd_test(&url).await?,
    }
    Ok(())
}

async fn cmd_subscribe(args: WebhookSubscribeArgs) -> Result<()> {
    let mut subs = load_subscriptions()?;

    let subscription = WebhookSubscription {
        url: args.url.clone(),
        prompt: args.prompt,
        events: args
            .events
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        description: args.description,
        skills: args
            .skills
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        deliver: args.deliver,
        deliver_chat_id: args.deliver_chat_id,
        secret: args.secret,
        deliver_only: args.deliver_only,
    };

    subs.insert(args.url.clone(), subscription);
    save_subscriptions(&subs)?;
    println!("Subscribed webhook: {}", args.url);
    Ok(())
}

async fn cmd_list() -> Result<()> {
    let subs = load_subscriptions()?;
    if subs.is_empty() {
        println!("No webhook subscriptions.");
        return Ok(());
    }

    println!("Webhook Subscriptions");
    println!("─────────────────────");
    for (url, sub) in &subs {
        println!();
        println!("  URL:         {}", url);
        if let Some(desc) = &sub.description {
            println!("  Description: {}", desc);
        }
        println!("  Events:      {}", sub.events.join(", "));
        if let Some(prompt) = &sub.prompt {
            println!("  Prompt:      {}", truncate(prompt, 60));
        }
        if !sub.skills.is_empty() {
            println!("  Skills:      {}", sub.skills.join(", "));
        }
        if sub.deliver_only {
            println!("  Mode:        deliver-only");
        }
        if let Some(deliver) = &sub.deliver {
            println!("  Deliver:     {}", deliver);
        }
    }
    Ok(())
}

async fn cmd_remove(url: &str) -> Result<()> {
    let mut subs = load_subscriptions()?;
    if subs.remove(url).is_some() {
        save_subscriptions(&subs)?;
        println!("Removed webhook subscription: {}", url);
    } else {
        println!("No subscription found for URL: {}", url);
    }
    Ok(())
}

async fn cmd_test(url: &str) -> Result<()> {
    println!("Testing webhook: {}", url);
    println!();
    println!("This would send a POST request with an HMAC-SHA256 signature");
    println!("to the webhook URL using the configured secret (if any).");
    println!();
    println!("To test manually:");
    println!("  curl -X POST {} \\", url);
    println!("    -H \"Content-Type: application/json\" \\");
    println!("    -d '{{\"event\":\"ping\",\"source\":\"hermes-cli\"}}'");
    Ok(())
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}
