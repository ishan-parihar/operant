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
        /// Run at most this many times, then mark the job completed (default: infinite)
        #[arg(long)]
        repeat: Option<i32>,
        /// Wave 5 (ORGANISM-ARCHITECTURE §4): the seat mode the automaton's
        /// policy row is created under. Defaults to
        /// `[genome].unrestricted_default` when absent — the documented
        /// ungoverned posture. One of: yolo | standard | scoped | lockdown.
        #[arg(long)]
        seat_mode: Option<String>,
        /// Allow-list globs for the seat's policy row (repeatable).
        #[arg(long)]
        allow: Vec<String>,
        /// Deny-list globs for the seat's policy row (repeatable; deny is
        /// absolute — it outranks allow and grants).
        #[arg(long)]
        deny: Vec<String>,
    },
    /// Show details of a specific cron job
    Get {
        /// Cron job ID
        id: String,
    },
    /// Show a job's recent run history (iter-668 cron_runs)
    History {
        /// Cron job ID
        id: String,
        /// How many recent runs to show (default 20)
        #[arg(long)]
        limit: Option<usize>,
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

/// Cron DB path — MUST match `main.rs` (`db_dir.join("operant_cron.db")`).
/// The runtime scheduler reads/writes this dedicated file; pointing the CLI at
/// the shared `database_path` made CLI-created jobs invisible to the scheduler
/// and tripped the shared-PRAGMA migration guard (R39-7).
pub fn cron_db_path(config: &AppConfig) -> std::path::PathBuf {
    config
        .database_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("operant_cron.db")
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
            repeat,
            seat_mode,
            allow,
            deny,
        } => {
            cmd_create(
                config, &name, &schedule, &command, repeat, seat_mode, allow, deny,
            )
            .await
        }
        CronSubcommand::Get { id } => cmd_get(config, &id).await,
        CronSubcommand::History { id, limit } => cmd_history(config, &id, limit).await,
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
    let db = CronDb::init(cron_db_path(config)).context("Failed to open cron database")?;
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

async fn cmd_create(
    config: &AppConfig,
    name: &str,
    schedule: &str,
    command: &str,
    repeat: Option<i32>,
    seat_mode: Option<String>,
    allow: Vec<String>,
    deny: Vec<String>,
) -> Result<()> {
    let db = CronDb::init(cron_db_path(config)).context("Failed to open cron database")?;
    // Normalize + validate the schedule up front: the cron crate only parses
    // 6-field expressions, so 5-field ("0 9 * * *") and interval ("every 6h")
    // forms are converted here. Invalid schedules fail at creation instead of
    // silently never firing.
    let schedule = operant_core::cronjobs::normalize_schedule(schedule)
        .with_context(|| format!("invalid schedule '{schedule}'"))?;
    // Repeat is now enforced by the scheduler (R16): when repeat_completed
    // reaches repeat_times the job is marked completed and disabled. Negative
    // values mean "infinite" — same semantics as a None repeat.
    let repeat_times = repeat.filter(|n| *n > 0);
    let id = db
        .create_job(operant_core::cronjobs::db::CreateJobParams {
            name: name.to_string(),
            prompt: command.to_string(),
            schedule: schedule.clone(),
            schedule_display: schedule.to_string(),
            repeat_times,
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
        .context("Failed to create cron job")?;

    // Wave 5 (ORGANISM-ARCHITECTURE §4): the onboarding transaction. Job
    // creation and seat provisioning are one step — an automaton must not
    // exist ungoverned by accident. Cross-file sqlite cannot be one
    // transaction, so the honest shape is job-first-then-provision with a
    // DELETE rollback if provisioning fails: the operator either gets a
    // governed automaton or no automaton, never an ungoverned one.
    let provisioned = provision_cron_seat(config, &id, seat_mode.as_deref(), &allow, &deny);
    match provisioned {
        Ok(report) => {
            println!("Cron job created successfully.");
            println!("ID: {}", id);
            println!("{report}");
        }
        Err(e) => {
            // Rollback: the job exists but its seat does not — that is the
            // ungoverned automaton this transaction exists to prevent.
            let _ = db.delete_job(&id);
            return Err(e).context(
                "Seat provisioning failed — the cron job was rolled back \
                 (no ungoverned automaton was left behind)",
            );
        }
    }
    Ok(())
}

/// Wave 5: provision employee + policy row for a freshly created cron job.
/// Employee shape comes from the registry's own backfill (single source of
/// truth); the policy row defaults to `[genome].unrestricted_default` —
/// the documented posture — unless `--seat-mode` names one.
#[allow(clippy::too_many_arguments)]
fn provision_cron_seat(
    config: &AppConfig,
    job_id: &str,
    seat_mode: Option<&str>,
    allow: &[String],
    deny: &[String],
) -> Result<String> {
    use operant_core::org::employee_db::EmployeeDb;
    use operant_core::org::seat_policy::SeatPolicy;
    use operant_core::org::seat_policy_db::SeatPolicyDb;

    let cron_db = CronDb::init(cron_db_path(config)).context("Failed to open cron database")?;
    let job = cron_db
        .get_job(job_id)
        .context("Failed to read the new cron job")?
        .with_context(|| format!("cron job {job_id} vanished before seat provisioning"))?;

    let org_db = operant_core::org::employee_db::org_db_path(&config.database_path);
    let registry =
        EmployeeDb::open_at(org_db.clone()).context("Failed to open the employee registry")?;
    let now = chrono::Utc::now().to_rfc3339();
    let report = registry
        .backfill_from_cron_jobs(
            std::slice::from_ref(&job),
            &now,
            "wave5: onboarded at cron register",
        )
        .context("Failed to provision the automaton's employee row")?;

    let employee_id = operant_core::org::employee::derive_employee_id(&job.id);
    let mode_raw = seat_mode
        .map(|m| m.to_string())
        .unwrap_or_else(|| config.genome.unrestricted_default.clone());
    let mode = std::str::FromStr::from_str(&mode_raw)
        .map_err(|e| anyhow::anyhow!("invalid --seat-mode '{mode_raw}': {e}"))?;
    let policies = SeatPolicyDb::init(org_db).context("Failed to open the seat policy store")?;
    policies
        .upsert(
            &employee_id,
            &SeatPolicy {
                mode,
                allow: allow.to_vec(),
                deny: deny.to_vec(),
            delegation: None,
            },
        )
        .context("Failed to seat the automaton's policy row")?;

    Ok(format!(
        "Seat provisioned: employee `{employee_id}` (registry backfill wrote {} row(s)), policy mode `{}` (allow {}, deny {})",
        report.employees_written,
        mode.as_str(),
        allow.len(),
        deny.len(),
    ))
}

async fn cmd_get(config: &AppConfig, id: &str) -> Result<()> {
    let db = CronDb::init(cron_db_path(config)).context("Failed to open cron database")?;
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
    let db = CronDb::init(cron_db_path(config)).context("Failed to open cron database")?;

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
    let db = CronDb::init(cron_db_path(config)).context("Failed to open cron database")?;
    let deleted = db.delete_job(id).context("Failed to delete cron job")?;

    if deleted {
        println!("Cron job '{}' deleted successfully.", id);
    } else {
        println!("Cron job '{}' not found.", id);
    }

    Ok(())
}

async fn cmd_pause(config: &AppConfig, id: &str) -> Result<()> {
    let db = CronDb::init(cron_db_path(config)).context("Failed to open cron database")?;

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
    let db = CronDb::init(cron_db_path(config)).context("Failed to open cron database")?;

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
    let db = CronDb::init(cron_db_path(config)).context("Failed to open cron database")?;

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
    let db = CronDb::init(cron_db_path(config)).context("Failed to open cron database")?;
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
    // Global emergency stop (hermes estop parity): while engaged, the manual
    // tick path refuses to dispatch — mirroring CronScheduler::tick(), which
    // skips due jobs while paused. (iter-170)
    if operant_core::estop::is_engaged() {
        println!("⏸️ Operant is paused — cron dispatch is suspended.");
        println!("   Resume with `operant resume`.");
        return Ok(());
    }

    let db = CronDb::init(cron_db_path(config)).context("Failed to open cron database")?;
    // Self-heal legacy schedules/next_run before checking due jobs.
    let healed = db
        .repair_schedules()
        .context("Failed to repair cron schedules")?;
    if healed > 0 {
        println!("Repaired {} legacy cron job schedule(s).", healed);
    }
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
    // Blueprint defaults are 5-field expressions — normalize to the 6-field
    // form the scheduler's cron crate parses ("0 8 * * *" → "0 0 8 * * *").
    let schedule = operant_core::cronjobs::normalize_schedule(&schedule)
        .with_context(|| format!("invalid schedule '{schedule}'"))?;

    let db = CronDb::init(cron_db_path(config)).context("Failed to open cron database")?;
    let id = db
        .create_job(operant_core::cronjobs::db::CreateJobParams {
            name: display_name.to_string(),
            prompt,
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

/// iter-668: `operant cron history <id>` — the per-run forensics surface
/// over the cron_runs table. Failures used to vanish when the next ok
/// cleared last_error; history keeps every attempt.
async fn cmd_history(config: &AppConfig, id: &str, limit: Option<usize>) -> Result<()> {
    let db = CronDb::init(cron_db_path(config)).context("Failed to open cron database")?;
    let runs = db
        .list_recent_runs(id, limit.unwrap_or(20))
        .context("Failed to list cron run history")?;

    if runs.is_empty() {
        println!("No recorded runs for '{id}'.");
        return Ok(());
    }

    println!("Recent runs for '{id}' (newest first):");
    println!(
        "{:<26} {:<9} {:<11} {}",
        "FINISHED AT", "OUTCOME", "ORIGIN", "ERROR"
    );
    for run in runs {
        println!(
            "{:<26} {:<9} {:<11} {}",
            run.finished_at,
            if run.success { "ok" } else { "error" },
            run.origin,
            run.error.unwrap_or_default()
        );
    }
    Ok(())
}
