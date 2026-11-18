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
use operant_core::mcp::McpManager;
use operant_core::text_slice::char_prefix;

/// Manage cron jobs
#[derive(Debug, Clone, Subcommand)]
pub enum CronSubcommand {
    /// List all cron jobs
    List,
    /// Create a new cron job
    Create {
        /// Name of the cron job
        name: String,
        /// Cron schedule expression: cron forms ("0 9 * * *") evaluate in
        /// UTC (09:00 UTC in the example); "every N<s|m|h|d|w>" forms are
        /// fixed durations ("every 6h" = every 6 hours exactly)
        schedule: String,
        /// Command or prompt to execute when triggered
        command: String,
        /// Run at most this many times, then mark the job completed (default: infinite)
        #[arg(long)]
        repeat: Option<i32>,
        /// Wave 5 (ORGANISM-ARCHITECTURE §4): the seat mode the automaton's
        /// policy row is created under. Defaults to
        /// `[bylaws].unrestricted_default` when absent — the documented
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
        /// Override the default schedule (e.g. "0 9 * * *" for 09:00 UTC
        /// daily; cron expressions evaluate in UTC)
        #[arg(long)]
        schedule: Option<String>,
    },
}

/// Cron DB path — MUST match `main.rs` (`db_dir.join("operant_cron.db")`).
/// The runtime scheduler reads/writes this dedicated file; pointing the CLI at
/// the shared `database_path` made CLI-created jobs invisible to the scheduler
/// and tripped the shared-PRAGMA migration guard (R39-7).
pub fn cron_db_path(config: &AppConfig) -> std::path::PathBuf {
    // The core owns the rule: every cron reader and writer must open the
    // SAME file, or a reader sees an empty database and reports "nothing
    // references this skill" (RT-267).
    operant_core::cronjobs::db_path(&config.database_path)
}

// ── Z31: the gateway-daemon lock probe ──────────────────────────────────
//
// The scheduled cron dispatcher lives INSIDE the gateway daemon, which holds
// an exclusive `flock` on `<operant home>/gateway.lock` for the life of the
// process (`gateway_runner::acquire_gateway_daemon_lock`). `operant cron run
// <id>` builds its OWN `CronDb`, agent and scheduler and runs the job in a
// second process, where the `CronDb` connection mutex (`std::sync::Mutex`) is
// per-process and serializes nothing. The file lock is the only machine-wide
// statement that the scheduler is already running here.
//
// WHY THIS PROBES AND DOES NOT ACQUIRE. The daemon holds the lock for its
// whole life; acquiring it here would either fail (correct detection, but then
// the answer would have to be inferred from the failure) or succeed and then
// release it the instant this function returned — a probe with a use-after-free
// shape. The kernel answers the question directly: `LOCK_EX | LOCK_NB` on the
// same inode succeeds ONLY when nobody else holds it, and the fd is closed
// before returning, so this process never becomes a holder.
//
// The PATH comes from the core (`cronjobs::lock::gateway_daemon_lock_path`) so
// the daemon and this probe cannot disagree about which file is the lock —
// the same one-definition rule `cron_db_path` follows.
//
// The result is ADVISORY: see `cmd_run` for the refusal and why a daemon that
// is running does not make a manual run impossible.

/// Whether a live gateway daemon holds the machine-wide cron lock.
///
/// `Ok(true)` = the lock is held by another process, so the gateway daemon
/// (and with it the scheduled cron dispatcher) is running on this machine.
/// `Err` is a real failure to answer the question (the lock file cannot be
/// opened) — the caller decides how to degrade, never silently "no".
/// The probe at an explicit path — the seam a test drives, so the test does
/// not depend on whether a real daemon happens to be running on the host.
#[cfg(unix)]
fn gateway_daemon_holds_lock_at(path: &std::path::Path) -> std::io::Result<bool> {
    Ok(try_hold_gateway_lock_at(path)?.is_none())
}

/// Take the gateway's lock for the DURATION of a manual run (iter-2645).
///
/// `None` = a live daemon holds it and the run must be refused. `Some(guard)`
/// = this process holds it; the daemon cannot start until the guard drops.
///
/// The probe this replaces held the lock only long enough to answer, then
/// released it, and `run_job_now` mints its own occurrence key — so a daemon
/// starting in that window dispatched the same cadence a second time, and the
/// claim ledger could not collide the two keys. Holding is the same three
/// lines the probe already had; the difference is that the fd stays open
/// across the await.
#[cfg(unix)]
fn try_hold_gateway_lock_at(path: &std::path::Path) -> std::io::Result<Option<std::fs::File>> {
    use std::os::fd::AsRawFd;

    if let Some(parent) = path.parent() {
        // A missing home is NOT a held lock: create the directory so the open
        // below is probing the lock rather than reporting a filesystem error.
        // The daemon's own path creation does the same thing.
        std::fs::create_dir_all(parent)?;
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(path)?;
    // SAFETY: `file` owns a live fd; it is either returned to the caller (and
    // the lock is released when THAT drops) or dropped here, which releases
    // anything this call took. The path is never unlinked: `flock` protects
    // the inode, so removing it would let a second starter lock a new one.
    let taken = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0;
    Ok(if taken { Some(file) } else { None })
}

/// Take the gateway lock for a manual run, or report who holds it.
#[cfg(unix)]
fn gateway_daemon_lock_guard() -> std::io::Result<Option<std::fs::File>> {
    try_hold_gateway_lock_at(&operant_core::cronjobs::gateway_daemon_lock_path())
}

/// Non-unix stand-in: there is no `flock`, so the daemon's own lock is the
/// best-effort pid-file shape and this probe cannot answer. The permissive
/// answer is "hold nothing, run anyway", exactly as before.
#[cfg(not(unix))]
fn gateway_daemon_lock_guard() -> std::io::Result<Option<std::fs::File>> {
    Ok(None)
}

/// The refusal text for a manual run while the daemon is live. Pure so the
/// wording is testable without a lock file.
fn daemon_lock_refusal(job_id: &str, lock_path: &std::path::Path) -> String {
    format!(
        "The gateway daemon is running on this machine and it owns the cron scheduler.\n\
         \x20 Job '{job_id}' would be dispatched by a SECOND scheduler in this process, \
         so this run was refused rather than risking a double fire.\n\
         \x20 Lock held: {}\n\
         \x20 Either wait for the daemon's next tick, or stop it first \
         (`operant gateway stop`) if you need to run the job by hand.",
        lock_path.display()
    )
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
            // Validated BEFORE the org-mode branch, not inside `cmd_create`.
            // The org-mode branch never reaches `cmd_create`, which is where
            // `normalize_schedule` used to live — so in org mode an
            // unusable schedule ("not-a-schedule", "99 99 99 99 99 99 99")
            // was accepted and proposed to the Chair verbatim. The proposal
            // must carry a schedule that can actually fire.
            operant_core::cronjobs::normalize_schedule(&schedule)
                .with_context(|| format!("invalid schedule '{schedule}'"))?;

            // Phase G (docs/agentic-os/): in org mode recurring work belongs
            // to the organization — `operant cron add` proposes a Holon
            // routine instead of creating a local cron. One command printed,
            // ready to paste (the kernel's classifier vets the proposal).
            if config.holon.enabled {
                // O-23: Holon's real argv — a positional title, --intent
                // and --dept required, --cron the 5/6-field schedule.
                let dept = "eng";
                let mut proposal = format!(
                    "holon routine add {name:?} --intent {command:?} --dept {dept} --cron {schedule:?}"
                );
                if let Some(n) = repeat {
                    proposal.push_str(&format!(" --recurring {n}"));
                }
                if json {
                    // stdout must carry EXACTLY one JSON document — the same
                    // rule `doctor --json` follows (H10) — so the human
                    // prose above is suppressed in JSON mode.
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "created": false,
                            "reason": "org_mode",
                            "proposal": proposal,
                        }))?
                    );
                } else {
                    println!("[holon] org mode: recurring work is the organization's.");
                    println!("Propose it as a routine (the Chair approves):");
                    println!("  {proposal}");
                    println!("No local cron job was created.");
                }
                // It printed a proposal and returned Ok — so a script that
                // gated on the exit code believed the job existed, and
                // `cron list` showed no such row. The command did NOT do
                // what it was asked, and it must say so.
                anyhow::bail!(
                    "org mode: no cron job was created — recurring work is the organization's. \
                     Propose it as a routine with the command printed above."
                );
            }
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

/// The next run to DISPLAY for a job, from its stored value and schedule.
///
/// The stored `next_run_at` is the value the scheduler will use, but it is
/// only refreshed when the scheduler ticks — so on a host where the gateway
/// is not running it can name a moment in the PAST. The column then reads
/// as "this fires in the past", which is worse than useless: the operator
/// sees a job that looks broken when the truth is that nothing has
/// refreshed it. A moment that has already gone by is labelled, and a job
/// that was never scheduled gets the next occurrence its schedule implies.
fn display_next_run(stored: Option<&str>, schedule: &str) -> String {
    match stored {
        Some(stored) => {
            let overdue = chrono::DateTime::parse_from_rfc3339(stored)
                .map(|at| at <= chrono::Utc::now())
                .unwrap_or(false);
            if overdue {
                format!("{stored} (overdue)")
            } else {
                stored.to_string()
            }
        }
        None => operant_core::cronjobs::schedule::next_run_from_schedule(schedule)
            .unwrap_or_else(|| "—".to_string()),
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
                    "next_run": display_next_run(j.next_run_at.as_deref(), &j.schedule),
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
        let next_run = display_next_run(job.next_run_at.as_deref(), &job.schedule);
        let last_output = job.last_status.as_deref().unwrap_or("—");

        // Char-boundary cut: a job name is operator-supplied text, so a
        // multi-byte char at byte 25 panicked `operant cron list` (RT-301 class).
        let display_name = if job.name.len() > 26 {
            format!("{}…", char_prefix(&job.name, 25))
        } else {
            job.name.clone()
        };

        let display_schedule = if job.schedule_display.len() > 18 {
            format!("{}…", char_prefix(&job.schedule_display, 17))
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

#[expect(
    clippy::too_many_arguments,
    reason = "mirrors the `cron create` flag set one-to-one"
)]
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

/// G+1 (docs/agentic-os/): the employee registry retired to Holon — a
/// personal cron seat now provisions only its policy row (the enforcement
/// surface that stays). The derived seat id is unchanged, so an existing
/// operator's rows keep applying.
#[allow(clippy::too_many_arguments)]
fn provision_cron_seat(
    config: &AppConfig,
    job_id: &str,
    seat_mode: Option<&str>,
    allow: &[String],
    deny: &[String],
) -> Result<String> {
    use operant_core::org::seat_policy::SeatPolicy;
    use operant_core::org::seat_policy_db::SeatPolicyDb;

    let cron_db = CronDb::init(cron_db_path(config)).context("Failed to open cron database")?;
    let job = cron_db
        .get_job(job_id)
        .context("Failed to read the new cron job")?
        .with_context(|| format!("cron job {job_id} vanished before seat provisioning"))?;

    let employee_id = operant_core::org::worklog::derive_employee_id(&job.id);
    let mode_raw = seat_mode
        .map(|m| m.to_string())
        .unwrap_or_else(|| config.bylaws.unrestricted_default.clone());
    let mode = std::str::FromStr::from_str(&mode_raw)
        .map_err(|e| anyhow::anyhow!("invalid --seat-mode '{mode_raw}': {e}"))?;
    // the org-sibling rule: the policy store lives in operant_kanban.db
    // beside the app db (the pre-G+1 convention, kept for personal seats)
    let org_db = operant_core::org::org_sibling_kanban_path(&config.database_path);
    let policies = SeatPolicyDb::init(org_db).context("Failed to open the seat policy store")?;
    policies
        .upsert(
            &employee_id,
            &SeatPolicy {
                mode,
                allow: allow.to_vec(),
                deny: deny.to_vec(),
            },
        )
        .context("Failed to seat the automaton's policy row")?;

    Ok(format!(
        "Seat provisioned: employee `{employee_id}`, policy mode `{}` (allow {}, deny {})",
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
    // Global emergency stop (hermes estop parity): while engaged, the manual
    // run path refuses to dispatch — mirroring CronScheduler::tick() and
    // CronScheduler::run_job_now(). (iter-170 established the CLI-side check.)
    if operant_core::estop::is_engaged() {
        println!("⏸️ Operant is paused — cron dispatch is suspended.");
        println!("   Resume with `operant resume`.");
        return Ok(());
    }

    // ── Z31: the daemon-lock gate ───────────────────────────────────────
    // The scheduled dispatcher is a task inside the gateway daemon, and the
    // daemon names itself with an exclusive flock on
    // `<operant home>/gateway.lock`. A manual run from this process is a
    // SECOND scheduler over the same rows, and the `CronDb` mutex that
    // serializes writers is per-process — it cannot see the daemon. So the
    // lock being held means "the scheduler you would race is live", and the
    // run is refused with a message that names the daemon and the way out.
    //
    // This is a deliberate behaviour change: `cron run` used to work with a
    // live daemon. The double fire it allowed is the defect (two agents, one
    // `next_run_at` row, `repeat_completed` bumped twice for one cadence
    // tick). The claim ledger (Z29) now also refuses a duplicate OCCURRENCE
    // even across processes, but it cannot stop a manual run from firing an
    // occurrence the daemon has already finished and whose `next_run_at` has
    // moved on — that is a second dispatch of the same cadence, and only this
    // gate sees it.
    //
    // The probe is advisory in the failure direction only: if the lock file
    // cannot be read the run PROCEEDS (fail-open) — refusing every manual run
    // because of a filesystem error would be a worse outage than the race it
    // prevents, and the claim ledger is still there as the backstop.
    // (iter-2645) HELD, not probed: the guard lives until this function
    // returns, so no daemon can start mid-run and dispatch the same cadence.
    let _gateway_lock = match gateway_daemon_lock_guard() {
        Ok(Some(guard)) => Some(guard),
        Ok(None) => {
            let path = operant_core::cronjobs::gateway_daemon_lock_path();
            println!("{}", daemon_lock_refusal(id, &path));
            return Ok(());
        }
        Err(e) => {
            eprintln!(
                "Warning: could not read the gateway daemon lock ({}): {e} — \
                 proceeding; a concurrent scheduled run is still possible.",
                operant_core::cronjobs::gateway_daemon_lock_path().display()
            );
            None
        }
    };

    // RT-135: this used to mark the job "triggered_manually" and print
    // "Manual execution via CLI is not yet implemented" — a bookkeeping write
    // standing in for a run, so an operator testing a job saw nothing happen
    // until the next tick. Worse, the row was marked BEFORE the run, so a
    // failed attempt still left a trigger behind.
    //
    // It now runs the job. Two things make that safe rather than a feature
    // fork, and both are load-bearing:
    //
    // 1. `CronScheduler::run_job_now` rides the scheduler's OWN `run_job`, so
    //    the budget gate, the script-vs-agent arms, the outcome row, the run
    //    history, delivery and the transient-retry backoff apply to a manual
    //    run exactly as they do to a scheduled one. A manual run is not a
    //    special case.
    // 2. The job's row is marked by `run_job` AFTER the attempt, from the
    //    attempt's real outcome.
    let db = CronDb::init(cron_db_path(config)).context("Failed to open cron database")?;
    let job = db
        .get_job(id)
        .context("Failed to get cron job")?
        .ok_or_else(|| anyhow::anyhow!("Cron job '{}' not found.", id))?;

    println!("Running cron job '{}' now.", job.name);
    println!("  Prompt: {}", job.prompt);
    println!("  Schedule: {}", job.schedule_display);
    if let Some(ref script) = job.script {
        println!("  Script: {}", script);
    }

    // The scheduler needs a live agent for agent-backed jobs. The construction
    // mirrors `autonomous`'s executor (autonomous.rs:224): a plain agent over
    // the configured model with no event channel, which is what a
    // foreground, one-shot CLI run wants.
    let mcp_manager = McpManager::new();
    let agent =
        crate::create_agent_without_events(config, None, &mcp_manager, &config.skills.root_dir)
            .await
            .context("Failed to build the agent for the manual cron run")?;

    let scheduler = operant_core::cronjobs::CronScheduler::new(
        std::sync::Arc::new(db),
        std::sync::Arc::new(agent),
    )
    .with_retry(config.agent.retry.clone());

    scheduler.run_job_now(id).await.context("Cron job failed")?;

    // Read the row back so the CLI reports what was RECORDED, not what the
    // run felt like. A second miss here (the row is gone) means a concurrent
    // deletion — reported, not silently passed.
    let db = CronDb::init(cron_db_path(config)).context("Failed to reopen cron database")?;
    let refreshed = db
        .get_job(id)
        .context("Failed to read the job back after the run")?;
    match refreshed {
        Some(j) => {
            let status = j.last_status.as_deref().unwrap_or("(none)");
            println!("Cron job '{}' ran — status: {}.", j.name, status);
            // The scheduler's vocabulary is "ok" / "error" (cronjobs/db.rs
            // mark_job_run writes them literally), not "success". Comparing
            // against the wrong word would print the error block after every
            // successful run.
            if status != "ok" {
                if let Some(err) = j.last_error.as_deref() {
                    println!("  Error: {}", err);
                }
                println!("  History: `operant cron history {}`", id);
            }
        }
        None => println!(
            "Cron job '{}' ran, but its row is gone (deleted concurrently).",
            job.name
        ),
    }
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
        "{:<26} {:<9} {:<11} ERROR",
        "FINISHED AT", "OUTCOME", "ORIGIN"
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A config with Holon org mode ON — the posture in which `cron create`
    /// proposes a routine instead of creating a local job.
    fn org_mode_config() -> AppConfig {
        let mut config = AppConfig::default();
        config.holon.enabled = true;
        config
    }

    fn create(name: &str, schedule: &str) -> CronSubcommand {
        CronSubcommand::Create {
            name: name.to_string(),
            schedule: schedule.to_string(),
            command: "echo hi".to_string(),
            repeat: None,
            seat_mode: None,
            allow: Vec::new(),
            deny: Vec::new(),
        }
    }

    #[tokio::test]
    async fn org_mode_create_creates_nothing_and_does_not_report_success() {
        // It used to print a proposal and return Ok — so a script that
        // gated on the exit code believed the job existed, and `cron list`
        // showed no such row.
        let outcome =
            handle_cron_command(&org_mode_config(), create("probe", "0 * * * *"), false).await;
        let err = outcome
            .expect_err("org mode must not report that a job was created")
            .to_string();
        assert!(
            err.contains("no cron job was created"),
            "the refusal must say what happened: {err}"
        );
    }

    #[tokio::test]
    async fn org_mode_create_still_validates_the_schedule() {
        // The org-mode branch never reached `cmd_create`, where the
        // validation lives — so an unusable schedule was proposed to the
        // Chair verbatim.
        let outcome =
            handle_cron_command(&org_mode_config(), create("probe", "not-a-schedule"), false).await;
        let err = outcome
            .expect_err("an unusable schedule must fail here")
            .to_string();
        assert!(
            err.contains("invalid schedule"),
            "the schedule must be rejected before the proposal: {err}"
        );
    }

    #[test]
    fn a_next_run_in_the_past_is_labelled_overdue() {
        // The column used to print the stored moment verbatim, so a job
        // whose scheduler had not ticked read as "fires in the past".
        let past = "2001-01-01T00:00:00+00:00";
        let shown = display_next_run(Some(past), "0 * * * *");
        assert!(
            shown.starts_with(past) && shown.contains("overdue"),
            "a moment that has gone by must be labelled: {shown}"
        );
    }

    #[test]
    fn a_next_run_in_the_future_is_shown_as_it_is() {
        let future = "2099-01-01T00:00:00+00:00";
        assert_eq!(display_next_run(Some(future), "0 * * * *"), future);
    }

    #[test]
    fn a_job_that_was_never_scheduled_shows_the_next_occurrence() {
        let shown = display_next_run(None, "0 0 * * *");
        assert!(
            !shown.is_empty() && shown != "—",
            "an unscheduled job still has a next occurrence: {shown}"
        );
    }

    // ── Z31: the daemon-lock refusal ──

    /// The refusal must NAME the daemon and give the operator a way out. A
    /// bare "busy" would leave the operator with a job they cannot run and no
    /// idea that the gateway is the thing holding it.
    #[test]
    fn the_daemon_lock_refusal_names_the_daemon_the_lock_and_the_way_out() {
        let msg = daemon_lock_refusal(
            "cron_abc123",
            std::path::Path::new("/home/u/.operant/gateway.lock"),
        );
        assert!(
            msg.contains("gateway daemon"),
            "it must say WHAT is running: {msg}"
        );
        assert!(
            msg.contains("cron_abc123"),
            "it must name the job the operator asked for: {msg}"
        );
        assert!(
            msg.contains("double fire") || msg.contains("SECOND scheduler"),
            "it must say WHY the run is refused, not just that it is: {msg}"
        );
        assert!(
            msg.contains("/home/u/.operant/gateway.lock"),
            "it must show the lock file, so the holder is checkable: {msg}"
        );
        assert!(
            msg.contains("operant gateway stop"),
            "it must name the command that changes the situation: {msg}"
        );
    }

    /// The probe reads the same path the core hands the daemon, so the two
    /// cannot disagree about which file is the lock. A probe against a
    /// DIFFERENT path would report "free" while the daemon holds the real
    /// one — the failure this one-definition rule exists to prevent.
    #[test]
    fn the_probe_uses_the_core_lock_path_not_a_second_guess() {
        let path = operant_core::cronjobs::gateway_daemon_lock_path();
        assert!(
            path.ends_with("gateway.lock"),
            "the probe and the daemon must name the same file: {}",
            path.display()
        );
        assert!(
            path.starts_with(operant_core::platform::operant_home()),
            "and it must live under the resolved operant home: {}",
            path.display()
        );
    }

    /// (iter-2645) The guard HOLDS while it lives. The old probe released the
    /// lock the instant it answered, so a daemon could start in the window
    /// between the check and the dispatch and run the same cadence twice.
    #[cfg(unix)]
    #[test]
    fn the_guard_holds_the_lock_until_it_is_dropped() {
        let dir = tempfile::tempdir().expect("tempdir");
        let lock = dir.path().join("gateway.lock");
        let guard = try_hold_gateway_lock_at(&lock)
            .expect("taking a free lock")
            .expect("a free lock is taken");
        assert!(
            gateway_daemon_holds_lock_at(&lock).expect("read while held"),
            "the guard must still be holding: a probe is not a leak, but this \
             fd is exactly what a starting daemon would see"
        );
        drop(guard);
        assert!(
            !gateway_daemon_holds_lock_at(&lock).expect("read after drop"),
            "dropping the guard releases the lock for the next starter"
        );
    }

    /// The probe's answer, both ways, over a lock file this test owns. This is
    /// the whole Z31 mechanism: the same `flock` the daemon holds, read
    /// non-blocking, so "held" can only mean "a live daemon".
    #[cfg(unix)]
    #[test]
    fn the_probe_reports_held_while_a_daemon_style_holder_exists() {
        use std::os::fd::AsRawFd;

        let dir = tempfile::tempdir().expect("tempdir");
        let lock = dir.path().join("gateway.lock");

        // Nobody holds it: the probe says free, and probing does not TAKE it
        // (a probe that leaked the lock would refuse every later caller).
        assert!(
            !gateway_daemon_holds_lock_at(&lock).expect("first probe"),
            "an unheld lock reads free"
        );
        assert!(
            !gateway_daemon_holds_lock_at(&lock).expect("second probe"),
            "the probe is not a leak — the second read is still free"
        );

        // A holder that behaves exactly like the daemon: an open fd with
        // LOCK_EX held for the life of the guard.
        let file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&lock)
            .expect("open the lock file");
        // SAFETY: `file` owns the fd; LOCK_EX without LOCK_NB blocks only if
        // a peer holds it, which this test has just proven nobody does.
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
        assert_eq!(rc, 0, "this test takes the lock the daemon shape takes");

        assert!(
            gateway_daemon_holds_lock_at(&lock).expect("held probe"),
            "a held lock reads held — the refusal's whole trigger"
        );

        // Release: the same kernel call the daemon's process exit performs.
        // SAFETY: the fd is still owned by `file`.
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
        assert_eq!(rc, 0, "release the lock");
        drop(file);
        assert!(
            !gateway_daemon_holds_lock_at(&lock).expect("released probe"),
            "after the holder goes away the lock reads free again — a crashed \
             daemon never wedges manual runs"
        );
    }
}
