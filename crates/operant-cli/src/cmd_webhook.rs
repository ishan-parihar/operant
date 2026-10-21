//! CLI subcommand for managing webhook subscriptions (`operant webhook`).
//!
//! A subscription records an INBOUND route: the path segment under
//! `/webhook/` that an external service posts to, the prompt and skills
//! that shape how the agent handles the event, and where the answer goes
//! (`deliver` / `deliver_chat_id`). The store lives in core
//! (`operant_core::gateway::webhook_subscriptions`) so the CLI, the
//! inbound adapter and the agent read the same file.
//!
//! `operant webhook test <url>` REALLY posts the ping (signed with the
//! subscription's secret when it has one) and reports the answer — before
//! iter-1127 it only printed what it would have done.

use std::collections::HashMap;

use anyhow::Result;
use clap::{Args, Subcommand};
use operant_core::config::AppConfig;
use operant_core::gateway::webhook_subscriptions::{
    WebhookSubscription, load_subscriptions, post_json, route_from_url, save_subscriptions,
};

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
    /// Public URL of the endpoint (the subscription's key). Its last path
    /// segment is the route the subscription claims, unless `--route`
    /// names one.
    pub url: String,

    /// The path segment under `/webhook/` the subscription answers to.
    /// Defaults to the last segment of the URL.
    #[arg(long)]
    pub route: Option<String>,

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
    let mut subs = load_subscriptions().map_err(anyhow::Error::msg)?;

    // The route is the path segment under `/webhook/` that the external
    // service posts to. Derived from the URL when not named explicitly.
    let route = args
        .route
        .clone()
        .or_else(|| route_from_url(&args.url))
        .map(|r| r.trim().trim_matches('/').to_string())
        .filter(|r| !r.is_empty());

    let subscription = WebhookSubscription {
        url: args.url.clone(),
        route: route.clone(),
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

    // Computed BEFORE the subscription is moved into the store.
    let delivery_target = subscription.delivery_target();
    let deliver_only = subscription.deliver_only;

    subs.insert(args.url.clone(), subscription);
    save_subscriptions(&subs).map_err(anyhow::Error::msg)?;
    println!("Subscribed webhook: {}", args.url);
    match &route {
        Some(route) => {
            println!("  Route:       {route}  (POST /webhook/{route})");
            if deliver_only {
                println!(
                    "  Mode:        deliver-only — the event is delivered, the agent is not run."
                );
            }
            match delivery_target {
                Some(target) => {
                    println!("  Answer goes: {}:{}", target.platform, target.channel_id)
                }
                None => println!(
                    "  Answer goes: nowhere — the inbound channel cannot carry it. \
                     Set --deliver <platform:chat_id>."
                ),
            }
            println!("  The gateway dispatches this route; no restart is needed.");
        }
        None => println!(
            "  NOTE: no route — the subscription is recorded only. Name one with \
             `--route <name>` (or use a URL whose last path segment is a plain \
             token) for the gateway to dispatch it."
        ),
    }
    println!(
        "  Use `operant webhook test {}` to check that the endpoint answers.",
        args.url
    );
    Ok(())
}

async fn cmd_list() -> Result<()> {
    let subs = load_subscriptions().map_err(anyhow::Error::msg)?;
    if subs.is_empty() {
        println!("No webhook subscriptions.");
        return Ok(());
    }

    println!("Webhook Subscriptions");
    println!("─────────────────────");
    for (url, sub) in &subs {
        println!();
        println!("  URL:         {}", url);
        match &sub.route {
            Some(route) => println!("  Route:       {route}  (POST /webhook/{route})"),
            None => println!("  Route:       — (recorded only; the gateway dispatches nothing)"),
        }
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
    let mut subs = load_subscriptions().map_err(anyhow::Error::msg)?;
    if subs.remove(url).is_some() {
        save_subscriptions(&subs).map_err(anyhow::Error::msg)?;
        println!("Removed webhook subscription: {}", url);
    } else {
        println!("No subscription found for URL: {}", url);
    }
    Ok(())
}

/// POST the ping to the URL and report what came back. When the URL is a
/// known subscription, its secret signs the request.
async fn cmd_test(url: &str) -> Result<()> {
    let subs: HashMap<String, WebhookSubscription> =
        load_subscriptions().map_err(anyhow::Error::msg)?;
    let secret = subs.get(url).and_then(|sub| sub.secret.clone());
    let body = serde_json::json!({
        "event": "ping",
        "source": "operant-cli",
        "timestamp": chrono_now(),
    })
    .to_string();

    println!("Testing webhook: {url}");
    match post_json(url, &body, secret.as_deref()).await {
        Ok(outcome) => {
            if outcome.status >= 200 && outcome.status < 300 {
                println!("  HTTP {} — the endpoint answered.", outcome.status);
            } else {
                println!(
                    "  HTTP {} — the endpoint rejected the ping.",
                    outcome.status
                );
            }
            if !outcome.body.trim().is_empty() {
                println!("  Body: {}", outcome.body.trim());
            }
        }
        Err(err) => {
            println!("  FAILED: {err}");
        }
    }
    Ok(())
}

/// Seconds since the epoch — the payload's timestamp.
fn chrono_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}
