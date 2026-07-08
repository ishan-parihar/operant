//! Cron jobs CLI subcommand
//!
//! Provides `operant cron list`, `operant cron create`, `operant cron get`,
//! `operant cron update`, `operant cron delete`, `operant cron pause`,
//! `operant cron resume`, `operant cron run`, `operant cron status`,
//! and `operant cron tick`.

use std::collections::HashMap;

use anyhow::{Context, Result};
use clap::Subcommand;
use operant_core::config::AppConfig;
use operant_core::cronjobs::CronDb;

/// Manage cron jobs
#[derive(Debug, Clone, Subcommand)]
pub enum CronSubcommand {
    /// List all cron jobs
    List,
    /// Create a new cron job
    Create {
        /// Name of the cron job
        name: String,
        /// Cron schedule expression (e.g. "every 6h", "0 9 * * *")
        schedule: String,
        /// Command or prompt to execute when triggered
        command: String,
    },
    /// Show details of a specific cron job
    Get {
        /// Cron job ID
        id: String,
    },
    /// Update a cron job
    Update {
        /// Cron job ID
        id: String,
        /// New name
        name: Option<String>,
        /// New schedule expression
        schedule: Option<String>,
        /// New command or prompt
        command: Option<String>,
    },
    /// Delete a cron job
    Delete {
        /// Cron job ID
        id: String,
    },
    /// Pause a cron job
    Pause {
        /// Cron job ID
        id: String,
    },
    /// Resume a paused cron job
    Resume {
        /// Cron job ID
        id: String,
    },
    /// Manually trigger a cron job run
    Run {
        /// Cron job ID
        id: String,
    },
    /// Show cron subsystem status (total, active, paused counts)
    Status,
    /// Tick the cron scheduler (check for due jobs)
    Tick,
    /// Create a cron job from a pre-built blueprint.
    Blueprint {
        /// Blueprint name: morning-brief | weekly-digest | reflection
        name: String,
        /// Override the default schedule (e.g. "0 9 * * *" for 9am daily)
        #[arg(long)]
        schedule: Option<String>,
    },
}

/// Dispatch a cron subcommand.
pub async fn handle_cron_command(
    config: &AppConfig,
    cmd: CronSubcommand,
    json: bool,
) -> Result<()> {
    match cmd {
        CronSubcommand::List => cmd_list(config, json).await,
        CronSubcommand::Create {
            name,
            schedule,
            command,
        } => cmd_create(config, &name, &schedule, &command).await,
        CronSubcommand::Get { id } => cmd_get(config, &id).await,
        CronSubcommand::Update {
            id,
            name,
            schedule,
            command,
        } => cmd_update(config, &id, name, schedule, command).await,
        CronSubcommand::Delete { id } => cmd_delete(config, &id).await,
        CronSubcommand::Pause { id } => cmd_pause(config, &id).await,
        CronSubcommand::Resume { id } => cmd_resume(config, &id).await,
        CronSubcommand::Run { id } => cmd_run(config, &id).await,
        CronSubcommand::Status => cmd_status(config).await,
        CronSubcommand::Tick => cmd_tick(config).await,
        CronSubcommand::Blueprint { name, schedule } => {
            cmd_blueprint(config, &name, schedule).await
        }
    }
}

async fn cmd_list(config: &AppConfig, json: bool) -> Result<()> {
    let db = CronDb::init(config.database_path.clone()).context("Failed to open cron database")?;
    let jobs = db.list_jobs(true).context("Failed to list cron jobs")?;

    if json {
        let items: Vec<serde_json::Value> = jobs
            .iter()
            .map(|j| {
                let status = if j.state == "paused" {
                    "paused"
                } else if j.enabled {
                    "active"
                } else {
                    "disabled"
                };
                serde_json::json!({
                    "id": j.id,
                    "name": j.name,
                    "schedule": j.schedule_display,
                    "status": status,
                    "next_run": j.next_run_at,
                    "last_status": j.last_status,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&items)?);
        return Ok(());
    }

    if jobs.is_empty() {
        println!("No cron jobs found.");
        return Ok(());
    }

    println!(
        "{:<22} {:<28} {:<20} {:<10} {:<28} {:>15}",
        "ID", "Name", "Schedule", "Status", "Next Run", "Last Output"
    );
    println!("{}", "-".repeat(130));

    for job in &jobs {
        let status = if job.state == "paused" {
            "Paused"
        } else if job.enabled {
            "Active"
        } else {
            "Disabled"
        };
        let next_run = job.next_run_at.as_deref().unwrap_or("—");
        let last_output = job.last_status.as_deref().unwrap_or("—");

        let display_name = if job.name.len() > 26 {
            format!("{}…", &job.name[..25])
        } else {
            job.name.clone()
        };

        let display_schedule = if job.schedule_display.len() > 18 {
            format!("{}…", &job.schedule_display[..17])
        } else {
            job.schedule_display.clone()
        };

        println!(
            "{:<22} {:<28} {:<20} {:<10} {:<28} {:>15}",
            job.id, display_name, display_schedule, status, next_run, last_output,
        );
    }

    Ok(())
}

async fn cmd_create(config: &AppConfig, name: &str, schedule: &str, command: &str) -> Result<()> {
    let db = CronDb::init(config.database_path.clone()).context("Failed to open cron database")?;
    let id = db
        .create_job(
            name.to_string(),
            command.to_string(),
            schedule.to_string(),
            schedule.to_string(),
            None,
            "local".to_string(),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .context("Failed to create cron job")?;
    println!("Cron job created successfully.");
    println!("ID: {}", id);
    Ok(())
}

async fn cmd_get(config: &AppConfig, id: &str) -> Result<()> {
    let db = CronDb::init(config.database_path.clone()).context("Failed to open cron database")?;
    let job = db
        .get_job(id)
        .context("Failed to get cron job")?
        .ok_or_else(|| anyhow::anyhow!("Cron job '{}' not found.", id))?;

    println!("ID:              {}", job.id);
    println!("Name:            {}", job.name);
    println!("Prompt:          {}", job.prompt);
    println!("Schedule:        {}", job.schedule);
    println!("Schedule (disp): {}", job.schedule_display);
    println!("Enabled:         {}", job.enabled);
    println!("State:           {}", job.state);
    println!("Created At:      {}", job.created_at);
    println!(
        "Next Run At:     {}",
        job.next_run_at.as_deref().unwrap_or("—")
    );
    println!(
        "Last Run At:     {}",
        job.last_run_at.as_deref().unwrap_or("—")
    );
    println!(
        "Last Status:     {}",
        job.last_status.as_deref().unwrap_or("—")
    );
    println!(
        "Last Error:      {}",
        job.last_error.as_deref().unwrap_or("—")
    );
    Ok(())
}

async fn cmd_update(
    config: &AppConfig,
    id: &str,
    name: Option<String>,
    schedule: Option<String>,
    command: Option<String>,
) -> Result<()> {
    let db = CronDb::init(config.database_path.clone()).context("Failed to open cron database")?;

    db.get_job(id)
        .context("Failed to get cron job")?
        .ok_or_else(|| anyhow::anyhow!("Cron job '{}' not found.", id))?;

    let mut updates: HashMap<String, Option<serde_json::Value>> = HashMap::new();
    if let Some(name) = name {
        updates.insert("name".to_string(), Some(serde_json::Value::String(name)));
    }
    if let Some(schedule) = schedule {
        updates.insert(
            "schedule".to_string(),
            Some(serde_json::Value::String(schedule.clone())),
        );
        updates.insert(
            "schedule_display".to_string(),
            Some(serde_json::Value::String(schedule)),
        );
    }
    if let Some(command) = command {
        updates.insert(
            "prompt".to_string(),
            Some(serde_json::Value::String(command)),
        );
    }

    let updated = db
        .update_job(id, updates)
        .context("Failed to update cron job")?;

    match updated {
        Some(job) => {
            println!("Cron job updated successfully.");
            println!("ID:     {}", job.id);
            println!("Name:   {}", job.name);
            println!("Status: {}", job.state);
        }
        None => {
            println!("Cron job '{}' not found.", id);
        }
    }

    Ok(())
}

async fn cmd_delete(config: &AppConfig, id: &str) -> Result<()> {
    let db = CronDb::init(config.database_path.clone()).context("Failed to open cron database")?;
    let deleted = db.delete_job(id).context("Failed to delete cron job")?;

    if deleted {
        println!("Cron job '{}' deleted successfully.", id);
    } else {
        println!("Cron job '{}' not found.", id);
    }

    Ok(())
}

async fn cmd_pause(config: &AppConfig, id: &str) -> Result<()> {
    let db = CronDb::init(config.database_path.clone()).context("Failed to open cron database")?;

    let job = db
        .get_job(id)
        .context("Failed to get cron job")?
        .ok_or_else(|| anyhow::anyhow!("Cron job '{}' not found.", id))?;

    if !job.enabled || job.state == "paused" {
        println!("Cron job '{}' is already paused.", id);
        return Ok(());
    }

    let mut updates: HashMap<String, Option<serde_json::Value>> = HashMap::new();
    updates.insert(
        "enabled".to_string(),
        Some(serde_json::Value::Number(serde_json::Number::from(0))),
    );
    updates.insert(
        "state".to_string(),
        Some(serde_json::Value::String("paused".to_string())),
    );
    db.update_job(id, updates)
        .context("Failed to pause cron job")?;

    println!("Cron job '{}' paused successfully.", id);
    Ok(())
}

async fn cmd_resume(config: &AppConfig, id: &str) -> Result<()> {
    let db = CronDb::init(config.database_path.clone()).context("Failed to open cron database")?;

    let job = db
        .get_job(id)
        .context("Failed to get cron job")?
        .ok_or_else(|| anyhow::anyhow!("Cron job '{}' not found.", id))?;

    if job.enabled && job.state != "paused" {
        println!("Cron job '{}' is already active.", id);
        return Ok(());
    }

    let mut updates: HashMap<String, Option<serde_json::Value>> = HashMap::new();
    updates.insert(
        "enabled".to_string(),
        Some(serde_json::Value::Number(serde_json::Number::from(1))),
    );
    updates.insert(
        "state".to_string(),
        Some(serde_json::Value::String("scheduled".to_string())),
    );
    db.update_job(id, updates)
        .context("Failed to resume cron job")?;

    println!("Cron job '{}' resumed successfully.", id);
    Ok(())
}

async fn cmd_run(config: &AppConfig, id: &str) -> Result<()> {
    let db = CronDb::init(config.database_path.clone()).context("Failed to open cron database")?;

    let job = db
        .get_job(id)
        .context("Failed to get cron job")?
        .ok_or_else(|| anyhow::anyhow!("Cron job '{}' not found.", id))?;

    // Mark the job as "triggered" (not "success") — actual execution requires
    // the cron scheduler or the gateway. Previously this marked the job as
    // "ran successfully" without executing anything, which was misleading.
    db.mark_job_run(
        id,
        false,
        Some("triggered_manually".to_string()),
        None,
        None,
    )
    .context("Failed to mark cron job run")?;

    println!("Cron job '{}' triggered.", job.name);
    println!("  Prompt: {}", job.prompt);
    println!("  Schedule: {}", job.schedule_display);
    if let Some(ref script) = job.script {
        println!("  Script: {}", script);
    }
    println!();
    println!("  Note: Manual execution via CLI is not yet implemented.");
    println!(
        "  The job has been marked as 'triggered' and will execute on the next scheduler tick."
    );
    println!(
        "  To run the prompt now, use: operant run --query \"{}\"",
        job.prompt
    );
    Ok(())
}

async fn cmd_status(config: &AppConfig) -> Result<()> {
    let db = CronDb::init(config.database_path.clone()).context("Failed to open cron database")?;
    let all_jobs = db.list_jobs(true).context("Failed to list cron jobs")?;

    let total = all_jobs.len();
    let active = all_jobs
        .iter()
        .filter(|j| j.enabled && j.state != "paused")
        .count();
    let paused = all_jobs.iter().filter(|j| j.state == "paused").count();

    println!("Cron Subsystem Status");
    println!("  Total jobs:  {}", total);
    println!("  Active jobs: {}", active);
    println!("  Paused jobs: {}", paused);
    Ok(())
}

async fn cmd_tick(config: &AppConfig) -> Result<()> {
    let db = CronDb::init(config.database_path.clone()).context("Failed to open cron database")?;
    let due_jobs = db.get_due_jobs().context("Failed to get due cron jobs")?;

    if due_jobs.is_empty() {
        println!("No cron jobs due for execution.");
    } else {
        println!("Found {} cron job(s) due for execution:", due_jobs.len());
        for job in &due_jobs {
            println!("  - {} ({})", job.name, job.id);
        }
    }

    Ok(())
}

/// Create a cron job from a pre-built blueprint.
/// (iter-107 — the #1 transformative feature from the UX audit.)
async fn cmd_blueprint(
    config: &AppConfig,
    name: &str,
    schedule_override: Option<String>,
) -> Result<()> {
    let (display_name, default_schedule, prompt): (&str, &str, String) = match name {
        "morning-brief" => (
            "Morning Brief",
            "0 8 * * *",
            "You are delivering the morning brief. Review your memory of recent conversations with this user.\n\nSurface exactly three things, formatted as a short message (under 200 words total):\n\n1. Pattern: One thing you have noticed the user doing repeatedly. Be specific and observational.\n\n2. Insight: One observation the user might not have about themselves. This should come from connecting dots across sessions.\n\n3. Question: One question that invites reflection. Not a task - a question that makes the user think about their direction.\n\nKeep the tone warm, specific, and brief. If you do not have enough memory yet, say so honestly.".to_string(),
        ),
        "weekly-digest" => (
            "Weekly Digest",
            "0 18 * * 5",
            "You are delivering the weekly digest. Review all conversations from the past 7 days.\n\nSummarize: 1) Themes, 2) Progress, 3) Friction, 4) Growth. Under 300 words. End with one question for the week ahead.".to_string(),
        ),
        "reflection" => (
            "Daily Reflection",
            "0 21 * * *",
            "You are guiding a daily reflection. Ask these 3 questions one at a time: 1) What went well today? 2) What did not go as you hoped? 3) What will you do differently tomorrow? After all 3 answers, synthesize a one-sentence summary.".to_string(),
        ),
        _ => {
            anyhow::bail!("Unknown blueprint '{}'. Available: morning-brief, weekly-digest, reflection", name);
        }
    };

    let schedule = schedule_override.unwrap_or_else(|| default_schedule.to_string());

    let db = CronDb::init(config.database_path.clone()).context("Failed to open cron database")?;
    let id = db
        .create_job(
            display_name.to_string(),
            prompt,
            schedule.clone(),
            schedule.clone(),
            None,
            "local".to_string(),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            false,
        )
        .context("Failed to create cron job")?;

    println!("Blueprint '{}' created successfully!", display_name);
    println!();
    println!("   Schedule: {}", schedule);
    println!("   Job ID:   {}", id);
    println!();
    println!("   The agent will run this prompt on schedule and deliver");
    println!("   the result via your configured gateway (Telegram, Discord, etc.)");
    println!("   or in the TUI if no gateway is running.");
    println!();
    println!("   To test it now: operant cron run {}", id);
    println!(
        "   To customize:   operant cron update {} --command <your prompt>",
        id
    );
    println!("   To delete:       operant cron delete {}", id);

    Ok(())
}
