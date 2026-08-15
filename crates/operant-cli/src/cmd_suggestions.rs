//! Suggested automations CLI subcommand — hermes `/suggestions` parity.
//!
//! `operant suggestions list|accept|dismiss|catalog|clear` manages
//! ready-to-run cron job specs surfaced to the user. Accepting one creates
//! the real cron job through `CronDb`.

use anyhow::{Context, Result};
use clap::Subcommand;
use operant_core::config::AppConfig;
use operant_core::cronjobs::{CreateJobParams, CronDb, SuggestionStore};

/// Manage suggested automations
#[derive(Debug, Clone, Subcommand)]
pub enum SuggestionsSubcommand {
    /// List pending suggestions (numbered)
    List,
    /// Accept a suggestion by number or id — creates the cron job
    Accept {
        /// Suggestion number from `list`, or its id (e.g. `sug_ab12cd34`)
        id: String,
    },
    /// Dismiss a suggestion (latched — never re-offered)
    Dismiss {
        /// Suggestion number from `list`, or its id
        id: String,
    },
    /// Seed the curated starter automations as pending
    Catalog,
    /// Drop accepted records (housekeeping)
    Clear,
}

/// Dispatch a suggestions subcommand.
pub async fn handle_suggestions_command(
    config: &AppConfig,
    cmd: SuggestionsSubcommand,
    json: bool,
) -> Result<()> {
    let store = SuggestionStore::open().context("Failed to open suggestions store")?;
    match cmd {
        SuggestionsSubcommand::List => cmd_list(&store, json),
        SuggestionsSubcommand::Accept { id } => cmd_accept(config, &store, &id).await,
        SuggestionsSubcommand::Dismiss { id } => cmd_dismiss(&store, &id),
        SuggestionsSubcommand::Catalog => cmd_catalog(&store),
        SuggestionsSubcommand::Clear => cmd_clear(&store),
    }
}

/// Resolve a user-supplied selector (number or id) to a suggestion id.
fn resolve_id(store: &SuggestionStore, selector: &str) -> Result<Option<String>> {
    let pending = store.pending()?;
    // Number form: "3" → 1-based index into pending.
    if let Ok(n) = selector.parse::<usize>() {
        if n >= 1 && n <= pending.len() {
            return Ok(Some(pending[n - 1].id.clone()));
        }
        return Ok(None);
    }
    // Id form: exact match.
    Ok(pending.into_iter().find(|s| s.id == selector).map(|s| s.id))
}

fn cmd_list(store: &SuggestionStore, json: bool) -> Result<()> {
    let pending = store.pending()?;
    if json {
        let items: Vec<serde_json::Value> = pending
            .iter()
            .map(|s| {
                serde_json::json!({
                    "id": s.id,
                    "title": s.title,
                    "description": s.description,
                    "source": s.source,
                    "schedule": s.schedule,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&items)?);
        return Ok(());
    }
    if pending.is_empty() {
        println!("No suggested automations right now.");
        println!("Try `operant suggestions catalog` to see the curated starter set.");
        return Ok(());
    }
    println!("Suggested automations — `operant suggestions accept N` or `dismiss N`:\n");
    for (i, s) in pending.iter().enumerate() {
        println!("  {}. {}  [{}]  ({})", i + 1, s.title, s.schedule, s.source);
        if !s.description.is_empty() {
            println!("     {}", s.description);
        }
    }
    Ok(())
}

async fn cmd_accept(config: &AppConfig, store: &SuggestionStore, selector: &str) -> Result<()> {
    let id = resolve_id(store, selector)?
        .ok_or_else(|| anyhow::anyhow!("No pending suggestion matches '{}'", selector))?;
    let suggestion = store
        .accept(&id)?
        .ok_or_else(|| anyhow::anyhow!("Suggestion '{}' is not pending", id))?;
    let schedule = operant_core::cronjobs::normalize_schedule(&suggestion.schedule)
        .with_context(|| format!("invalid schedule '{}'", suggestion.schedule))?;
    let db = CronDb::init(config.database_path.clone()).context("Failed to open cron database")?;
    let job_id = db
        .create_job(CreateJobParams {
            name: suggestion.title.clone(),
            prompt: suggestion.prompt.clone(),
            schedule: schedule.clone(),
            schedule_display: schedule.clone(),
            repeat_times: None,
            deliver: "local".to_string(),
            origin_platform: None,
            origin_chat_id: None,
            origin_thread_id: None,
            skill: None,
            skills: None,
            model: None,
            provider: None,
            base_url: None,
            script: None,
            context_from: None,
            enabled_toolsets: None,
            workdir: None,
            no_agent: false,
        })
        .context("Failed to create cron job from suggestion")?;

    println!(
        "Accepted '{}' — created cron job {} [{}].",
        suggestion.title, job_id, suggestion.schedule
    );
    Ok(())
}

fn cmd_dismiss(store: &SuggestionStore, selector: &str) -> Result<()> {
    let id = resolve_id(store, selector)?
        .ok_or_else(|| anyhow::anyhow!("No pending suggestion matches '{}'", selector))?;
    if store.dismiss(&id)? {
        println!("Dismissed suggestion {}.", id);
    } else {
        println!("Suggestion {} is not pending.", id);
    }
    Ok(())
}

fn cmd_catalog(store: &SuggestionStore) -> Result<()> {
    let added = store.catalog().context("Failed to seed catalog")?;
    if added > 0 {
        println!(
            "Seeded {} curated suggestion(s). Run `operant suggestions list` to see them.",
            added
        );
    } else {
        println!("Catalog already seeded (dedup latch — nothing new added).");
    }
    Ok(())
}

fn cmd_clear(store: &SuggestionStore) -> Result<()> {
    let removed = store.clear().context("Failed to clear accepted records")?;
    println!("Removed {} accepted record(s).", removed);
    Ok(())
}
