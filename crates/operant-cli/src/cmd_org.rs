//! `operant org` — the org surface, and the `--reason` mandate (Wave 1).
//!
//! Spec: `docs/WAVE1-DECISIONS.md` §3.5. Every mutating subcommand in this
//! module carries a **required, long-only** `--reason` declared on the clap
//! variant, so clap rejects a missing one before any I/O happens. The second
//! half of the mandate — a reason that is present but empty or whitespace is
//! also not a reason — is [`require_reason`], called at the top of every
//! mutating handler before the database is opened.
//!
//! ## Why parsed-and-discarded is the failure mode
//!
//! The organism's post-mortem (AD-032) is that 1,311 logged decisions were
//! read by nobody. A `--reason` flag that clap accepts and the handler then
//! throws away is *worse* than no flag: it makes the command look auditable
//! while writing nothing. So every write here persists the value into the
//! same row and the same transaction as its effect — §3.1's
//! `reason TEXT NOT NULL`, §3.5's "the reason and the effect are one
//! transaction".
//!
//! ## Where the reason is persisted
//!
//! | Command | Persisted in the same transaction as the effect |
//! |---|---|
//! | `org employee create` | `employees.reason` (§3.1, NOT NULL) |
//! | `org employee update` | `employees.reason` + `updated_at` |
//! | `org employee retire` | `employees.reason` + `status` + `updated_at` |
//! | `org notice post` | `notices.reason` (§3.3, NOT NULL) |
//! | `org notice ack` | an `ack`-tagged `notices` row, same `correlation_id`, **plus** the parent row's `acked_by`/`acked_at` (§3.3) |
//! | `org notice pin` / `unpin` | a `pin`/`unpin`-tagged `notices` row, plus the parent row's `pinned` |
//! | `org worklog append` | the `artifacts` JSON of the appended row — §3.4's 20 fields have no `reason` column, so inventing a 21st was rejected; see [`operator_entry`] |
//! | `org grant give` / `org grant revoke` | `authority_grants.reason` / `revocation_reason` — written only through the `SeatApprover` (F1, ORGANISM-ARCHITECTURE §6: one authority-write path) |
//! | `org sync` | `employees.reason` + `employee_cron_jobs`, one row per backfilled job (§3.1) |
//! | `org check` / `org list` / `org worklog list` | read-only — no reason |
//!
//! `org import` parses and validates a reason and then writes **nothing**, by
//! design and loudly: §4.3 of the decision document records importing a
//! foreign organism tree as an open *product* decision the owner has not
//! made. See [`cmd_import`]. (`org sync`, which once sat here too, is now
//! wired to the real `EmployeeDb::backfill_from_cron_jobs`.)
//!
//! ## Storage — one schema, owned by operant-core
//!
//! `operant-core/src/org/` owns the DDL for all four tables
//! (`EmployeeDb`/`NoticeBoard`/`WorklogDb`), and this module calls those
//! stores. It does **not** carry its own copy of the schema.
//!
//! That is not a style preference. An earlier revision of this file
//! transcribed the DDL locally, on the grounds that `operant-core` did not
//! compile yet. By the time it did, the two copies had already diverged on
//! the `worklog` table: the local copy had `created_at`/`epoch`/
//! `result_summary`/`sender`/`tags`/`reason`, while core's had the
//! 20-column §3.4 shape (`ts`/`ts_iso`/`employee`/`what_done`/`outcome`/
//! `next_intent`/`improvement_proposal`/…). Because every statement was
//! `CREATE TABLE IF NOT EXISTS`, whichever side ran first would define the
//! table and the other would fail at insert time with "no such column" — a
//! latent failure whose only symptom was a working CLI and a dead
//! `WorklogDb`, or the reverse, depending on boot order. One schema, in one
//! place, removes that class of bug entirely.
//!
//! ## Known limits
//!
//! 1. `org employee update` cannot clear a field back to NULL, because the
//!    update path cannot distinguish "not passed" from "passed as null" over
//!    the CLI. Documented on [`OrgEmployeeAction::Update`]. The workaround
//!    (a `--clear-*` flag per column) is scope this packet was not asked for.
//! 2. `org worklog append` goes through `WorklogDb::append` and the sealed
//!    `TurnObservation::into_entry` constructor rather than raw SQL, so an
//!    operator's row is shaped exactly like a framework row. See
//!    [`operator_entry`].
//! 3. `org import` writes nothing, by design — see the note at the top.

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use operant_core::config::AppConfig;
use rusqlite::OptionalExtension;
use std::path::{Path, PathBuf};

use crate::cmd_budget::BudgetSubcommand;

/// Reject a reason that is present but is not a reason.
///
/// §3.5 makes `--reason` required at the clap layer, which catches a
/// *missing* flag. It does not catch `--reason ""` or `--reason "   "`, and
/// a zero-length string satisfies a `NOT NULL` column perfectly well — it
/// is a reason-shaped value that carries no causal information, which is
/// exactly what AD-032 already caught operant doing in
/// `cmd_kanban.rs`'s `"Blocked via CLI"` default.
///
/// This lives at the write boundary rather than in a clap value parser for
/// two reasons: it can name the specific command in the error, and it cannot
/// be bypassed by any future call path that reaches a handler directly.
///
/// Returns the reason **unchanged**, not trimmed: the audit record should
/// carry what the operator actually typed. Emptiness is judged on the
/// trimmed view, but the stored value is the raw input — trimming on the
/// way in would quietly alter the audit record.
pub fn require_reason<'a>(command: &str, reason: &'a str) -> Result<&'a str> {
    if reason.trim().is_empty() {
        anyhow::bail!(
            "--reason is required and must not be empty or whitespace-only \
             (command: `{}`). An empty reason satisfies the NOT NULL \
             constraint while recording nothing about why this write \
             happened, which is the AD-032 failure mode. State the cause, \
             e.g. --reason \"upstream API returned 429\".",
            command
        );
    }
    Ok(reason)
}

/// `require_reason` **and bind** the validated value.
///
/// Every mutating handler should use this rather than `require_reason(..)?`
/// followed by reading the original binding: the check trims to decide
/// emptiness but hands back the *untrimmed* string, so a handler that stored
/// it verbatim would persist the operator's shell padding along with their
/// reason. Binding the returned value makes "validated" and "what gets
/// written" the same string by construction.
fn validated_reason<'a>(command: &str, reason: &'a str) -> Result<&'a str> {
    require_reason(command, reason).map(|r| r.trim())
}

/// Arguments for `org department set`.
///
/// A separate, boxed `Args` struct rather than inline enum fields, for two
/// reasons. It keeps `OrgDepartmentAction` a reasonable size: this form
/// carries eight `String`/`Option<String>` fields (~208 bytes) while the other
/// two variants carry at most a `bool`, so inlining it would make every
/// `List`/`Findings` match pay for the form's stack frame — hence the `Box`.
/// And a partial update needs a distinguishable "not mentioned" for every
/// optional field, which reads far better as one struct than as eight enum
/// fields.
#[derive(Debug, Clone, Args)]
pub struct OrgDepartmentSetArgs {
    /// Canonical department slug (primary key)
    pub dept_key: String,
    /// Human label (defaults to the slug)
    #[arg(long)]
    pub display_name: Option<String>,
    /// What this department exists to do
    #[arg(long)]
    pub mandate: Option<String>,
    /// Working protocols, comma-separated (§3.4 injects these into member prompts)
    #[arg(long, default_value = "")]
    pub protocols: String,
    /// Hard constraints, comma-separated (enforced, not advisory)
    #[arg(long, default_value = "")]
    pub rules: String,
    /// Capabilities the seat set must cover, comma-separated
    #[arg(long, default_value = "")]
    pub required_capabilities: String,
    /// Employee id occupying the head seat (omit to leave it vacant)
    #[arg(long)]
    pub head_employee_id: Option<String>,
    /// Planned size
    #[arg(long)]
    pub target_headcount: Option<i64>,
    /// Why this change is being made (required; stored in departments.reason)
    #[arg(long, value_name = "REASON")]
    pub reason: String,
}
///
/// The `org` subcommands, per the §3.5 table.
///
/// Mutating variants carry `#[arg(long, value_name = "REASON")] reason: String`.
/// A bare `String` in a clap derive variant is already required — that is
/// the pattern §3.5 prescribes verbatim, and it means a missing reason is
/// rejected by clap before any I/O, not by a handler check.
#[derive(Debug, Clone, Subcommand)]
pub enum OrgSubcommand {
    /// Per-seat budget overrides — the provisioning surface for the
    /// enforcement machinery (personal operator seats; org-mode caps come
    /// from Holon's `policy.json.budget`).
    #[command(subcommand)]
    Budget(BudgetSubcommand),

    /// Views over a Holon organization through its published read API
    /// (docs/agentic-os/ phase B). Read-only; requires [holon] enabled.
    /// `org holon archive` is the one-time org-store snapshot (phase G).
    #[command(subcommand)]
    Holon(HolonAction),
}

fn rfc3339_now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn now_epoch() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Dispatch an `org` subcommand.
pub async fn handle_org_command(config: &AppConfig, cmd: OrgSubcommand) -> Result<()> {
    match cmd {
        OrgSubcommand::Budget(cmd) => crate::cmd_budget::handle_budget_command(config, cmd).await,
        OrgSubcommand::Holon(action) => handle_holon_action(config, action).await,
    }
}

/// Views over the Holon organization (docs/agentic-os/ phase B): the
/// read-API panels at the CLI. Every one is read-only — the published
/// api_* views are the only surface, and writes to the org go through
/// Holon itself (boundary law S1). Off unless [holon] enabled (invariant 5).
#[derive(Debug, Clone, Subcommand)]
pub enum HolonAction {
    /// The roster: employees with departments and managers
    Roster {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// The board: ready and running stages
    Board {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Pending approvals with their plans
    Approvals {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Unacked messages
    Messages {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Routine triggers (cron patterns today; events land in phase F)
    Triggers {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// One compact org overview: departments, board, approvals
    Overview,

    /// Phase E (CONTRACT §11): one supervision pass — claim the next ready
    /// stage for the employee, run the body, hand it in. `--loop` keeps
    /// supervising until interrupted. Runs regardless of [holon] supervise
    /// (that flag gates the gateway's resident organ; the CLI loop is a
    /// manual foreground tool) — it needs supervise_employees in the
    /// config, or the employee ids as arguments.
    Supervise {
        /// The employee to serve (defaults to [holon] supervise_employees,
        /// served in order)
        #[arg(long, value_name = "EMP_ID")]
        employee: Option<String>,
        /// Claim only this stage
        #[arg(long)]
        stage: Option<String>,
        /// Keep supervising until interrupted (claim → run → hand in, repeat)
        #[arg(long)]
        r#loop: bool,
    },

    /// Phase E: fire the kernel's heartbeat once (`holon tick`) — sweeps,
    /// notifications, due schedules. Wire to a cron or systemd timer;
    /// every 2 minutes is the packaged cadence.
    Tick,

    /// Phase G (docs/agentic-os/): the one-time archive of the org stores —
    /// copy, never delete (ORG-LAYER-DISPOSITION.md §4). Writes
    /// <operant home>/archive/org-<date>/ with a checksum manifest.
    /// Date defaults to today (UTC). One snapshot per date.
    Archive {
        /// The snapshot's date tag (YYYY-MM-DD); defaults to today
        #[arg(long, value_name = "YYYY-MM-DD")]
        date: Option<String>,
    },
}

/// Open the Holon read API or explain precisely why org mode is off.
fn holon_api(config: &AppConfig) -> Result<operant_os::api::HolonApi> {
    if !config.holon.enabled {
        anyhow::bail!(
            "org mode is off: set [holon] enabled = true in operant.toml \
             (docs/agentic-os/README.md — invariant 5: off is byte-identical personal mode)"
        );
    }
    if config.holon.home.as_os_str().is_empty() {
        anyhow::bail!("org mode needs [holon] home set to the Holon instance ($HOLON_HOME)");
    }
    operant_os::api::HolonApi::open(std::path::Path::new(&config.holon.home))
        .map_err(|e| anyhow::anyhow!("holon read API unavailable: {e}"))
}

async fn handle_holon_action(config: &AppConfig, action: HolonAction) -> Result<()> {
    use operant_os::api::HolonApi;

    // Phase G: the archive touches only the OPERANT home — the read API
    // (and its live-instance gate) is irrelevant, and a not-yet-imported
    // box archives before anything else.
    if let HolonAction::Archive { date } = action {
        let operant_home = operant_core::platform::operant_home();
        let date = date.unwrap_or_else(|| chrono::Utc::now().format("%Y-%m-%d").to_string());
        let dest = operant_os::archive::archive_org_stores(&operant_home, &date)?;
        println!(
            "org stores archived to {} (MANIFEST.sha256 beside them)",
            dest.display()
        );
        println!("the originals are untouched — their writers go dark in phase G+1");
        return Ok(());
    }

    let api = holon_api(config)?;
    match action {
        HolonAction::Roster { json } => {
            let rows = api.employees()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&rows)?);
            } else {
                println!(
                    "{:<14} {:<14} {:<22} {:<10} {:<8}",
                    "ID", "NAME", "TITLE", "DEPT", "STATUS"
                );
                for e in rows {
                    println!(
                        "{:<14} {:<14} {:<22} {:<10} {:<8}",
                        e.id, e.name, e.title, e.department, e.status
                    );
                }
            }
        }
        HolonAction::Board { json } => {
            let rows = api.board()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&rows)?);
            } else {
                println!(
                    "{:<8} {:<30} {:<8} {:<8} {:<12} {:<10}",
                    "STAGE", "WORK", "STATUS", "GRADE", "DEPT", "ASSIGNEE"
                );
                for r in rows {
                    println!(
                        "{:<8} {:<30} {:<8} {:<8} {:<12} {:<10}",
                        r.stage,
                        truncate(&r.title, 30),
                        r.status,
                        r.grade
                            .map(|g| format!("G{g}"))
                            .unwrap_or_else(|| "-".into()),
                        r.department.as_deref().unwrap_or("-"),
                        r.employee_id.as_deref().unwrap_or("-"),
                    );
                }
            }
        }
        HolonAction::Approvals { json } => {
            let rows = api.approvals()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&rows)?);
            } else {
                if rows.is_empty() {
                    println!("no pending approvals");
                }
                for a in rows {
                    println!(
                        "{} · G{} · {} — {}",
                        a.id,
                        a.grade.unwrap_or(4),
                        a.department.as_deref().unwrap_or("-"),
                        truncate(&a.title, 60)
                    );
                }
            }
        }
        HolonAction::Messages { json } => {
            let rows = api.messages()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&rows)?);
            } else {
                for m in rows {
                    println!(
                        "{} {} -> {}: {}",
                        m.ts,
                        m.sender,
                        m.to_ref,
                        truncate(&m.body, 70)
                    );
                }
            }
        }
        HolonAction::Triggers { json } => {
            let rows = api.triggers()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&rows)?);
            } else {
                for t in rows {
                    println!(
                        "{} · {} · {} — {}{}",
                        t.id,
                        t.trigger_kind,
                        t.pattern,
                        t.employee.as_deref().unwrap_or("-"),
                        if t.enabled { "" } else { " (disabled)" }
                    );
                }
            }
        }
        HolonAction::Overview => {
            let depts = api.departments()?;
            let board = api.board()?;
            let approvals = api.approvals()?;
            let employees = api.employees()?;
            println!(
                "{} departments · {} employees · {} stages on the board · {} approvals pending",
                depts.len(),
                employees.len(),
                board.len(),
                approvals.len()
            );
            for d in &depts {
                let head = d.head.as_deref().unwrap_or("-");
                let stages: usize = board
                    .iter()
                    .filter(|b| b.department.as_deref() == Some(d.slug.as_str()))
                    .count();
                println!("  {} — head {} · {} active stage(s)", d.slug, head, stages);
            }
        }
        HolonAction::Supervise {
            employee,
            stage,
            r#loop,
        } => {
            let home = config.holon.home.clone();
            let bin = config.holon.bin.clone();
            let employees: Vec<String> = employee
                .map(|e| vec![e])
                .unwrap_or_else(|| config.holon.supervise_employees.clone());
            if employees.is_empty() {
                anyhow::bail!(
                    "no employee to serve: pass --employee or set [holon] supervise_employees"
                );
            }
            let mut errored = false;
            loop {
                // one pass: every configured employee in order — the kernel
                // enforces the employee's own max_concurrency on the claim;
                // [holon] max_concurrent_runs is the daemon-side budget for
                // the parallel mount
                let mut ran = false;
                for emp in &employees {
                    // sequential pass: the kernel enforces the employee's own
                    // max_concurrency on the claim; [holon] max_concurrent_runs
                    // is the daemon-side budget for the parallel mount
                    match operant_os::supervisor::supervise_one(
                        &home,
                        bin.as_deref(),
                        emp,
                        stage.as_deref(),
                    )
                    .await
                    {
                        Ok(true) => ran = true,
                        Ok(false) => {}
                        Err(e) => {
                            eprintln!("supervise {emp}: {e}");
                            // O-12: an error is NOT progress — a persistent
                            // failure (missing holon, bad home) used to set
                            // ran = true and spin with no sleep, re-spawning
                            // the kernel in a tight loop. Errors back off.
                            errored = true;
                        }
                    }
                }
                if !r#loop {
                    break;
                }
                if errored {
                    // O-12: back off on failure — a missing kernel must not
                    // become a hot loop of prepare attempts
                    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                } else if !ran {
                    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                }
            }
        }
        HolonAction::Tick => {
            let home = config.holon.home.clone();
            let bin = config.holon.bin.clone();
            operant_os::run::tick(&home, bin.as_deref()).await?;
            println!("tick fired");
        }
        // (the Archive arm runs BEFORE the read-API gate above — see the
        // handler head; this match never sees it)
        HolonAction::Archive { date: _ } => unreachable!("handled before the read-API gate"),
    }
    Ok(())
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}
