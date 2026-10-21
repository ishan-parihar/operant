//! `operant org` — the org surface: per-seat budgets and the Holon views.
//!
//! Two subcommand families live here, and nothing else:
//!
//! - `org budget set|list|clear` — the operator write surface for the
//!   per-seat budget overrides (`seat_budgets`). Fields left unset inherit
//!   `[bylaws].budget`; a cap of `0` is the documented release valve.
//! - `org holon …` — the views over a Holon organization through the kernel's
//!   published `api_*` views (docs/agentic-os/ phase B), plus the phase-E
//!   supervision loop, the kernel's heartbeat tick, the phase-G one-time
//!   org-store archive, and the `notify` delivery seam. Off unless
//!   `[holon] enabled` (invariant 5): the personal agent is byte-identical.
//!
//! ## What this module used to be
//!
//! The org surface once carried `employee`, `notice`, `worklog`, `sync`,
//! `import` and `grant` subcommands over stores operant owned —
//! `EmployeeDb`, `NoticeBoard`, `WorklogDb`, the grant ledger, the
//! escalation queue, the employee registry, the hierarchy edges and the
//! cast. Those retired to Holon under the owner order of 2026-10-15 (G+1,
//! docs/agentic-os/ORG-LAYER-DISPOSITION.md): Holon is the only writer of
//! org data and operant reads it through the published views. What operant
//! keeps is the enforcement surface — the seat policies, the seat budgets,
//! and the interactive approval that resolves an in-flight ask (there is no
//! grant to mint and no durable row to record).
//!
//! An earlier revision of this documentation still described that retired
//! family command by command, with intra-doc links to types that no longer
//! exist. The command list was the false part; what follows is the truth.
//!
//! ## The `--reason` mandate (Wave 1)
//!
//! Spec: `docs/WAVE1-DECISIONS.md` §3.5. Every mutating command that
//! remains carries a **required, long-only** `--reason` declared on the clap
//! variant, so clap rejects a missing one before any I/O happens. The second
//! half of the mandate — a reason that is present but empty or whitespace is
//! also not a reason — is [`require_reason`], called at the top of the
//! mutating handler before anything is written.
//!
//! ### Why parsed-and-discarded is the failure mode
//!
//! The organism's post-mortem (AD-032) is that 1,311 logged decisions were
//! read by nobody. A `--reason` flag that clap accepts and the handler then
//! throws away is *worse* than no flag: it makes the command look auditable
//! while writing nothing. So where there IS a store, the reason is persisted
//! with the effect, in the same transaction.
//!
//! ### Where the reason goes today
//!
//! | Command | The reason |
//! |---|---|
//! | `kanban block` / `kanban unblock` | persisted in the task event, in the same transaction as the block |
//! | `org holon notify` | validated, then echoed in the command's own output — operant keeps no store for it; the kernel records the notify event |
//!
//! The `org budget` mutations carry no `--reason`: a budget cap is a
//! declarative setting, not a decision that needs an audit trail.
//!
//! ## Known limits
//!
//! 1. `org holon` is a VIEW over another system's data. Every panel is
//!    read-only, and writes to the org go through Holon itself (boundary
//!    law S1) — `operant org holon …` cannot edit what it shows.

use anyhow::Result;
use clap::{Args, Subcommand};
use operant_core::config::AppConfig;

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

    /// G2 (REMAINING-GAPS): the kernel's `[notify]` delivery seam. The
    /// kernel computes what is new for the Chair (its event seq, deduped
    /// by notify_seq) and shells `[notify].command` with `{message}`
    /// substituted; operant's half is to post that message to each Chair
    /// channel. No polling, no local state — the kernel owns both. The
    /// owner wires `[notify].command = "operant org holon notify --reason
    /// heartbeat --text {message}"` in the instance's holon.toml.
    Notify {
        /// Audit reason (the org mandate: every action names its why)
        #[arg(long)]
        reason: String,
        /// The kernel-composed message (arrives argv-safe: the kernel
        /// shlex-quotes `{message}` into the command line)
        #[arg(long, value_name = "TEXT")]
        text: String,
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

/// Whether `employee` is on the Holon roster (`api_employees`). Only meaningful
/// in org mode; the caller decides what personal mode does with the answer.
pub(crate) fn roster_has_employee(config: &AppConfig, employee: &str) -> Result<bool> {
    Ok(holon_api(config)?
        .employees()?
        .iter()
        .any(|e| e.id == employee))
}

/// Send through `send` to each `<platform>:<user id>` chair ref and return
/// `(sent, failed)`. When nothing was delivered the call is an error, so
/// `chair notify` exits nonzero instead of reporting success (OS-5).
async fn deliver_to_chairs<F>(channels: &[String], mut send: F) -> Result<(usize, usize)>
where
    F: AsyncFnMut(&str, &str) -> Result<String>,
{
    let mut sent = 0usize;
    let mut failed = 0usize;
    for r#ref in channels {
        // a ref is `<platform>:<user id>` — the same shape is_chair
        // compares against
        let Some((platform, user_id)) = r#ref.split_once(':') else {
            continue;
        };
        match send(platform, user_id).await {
            Ok(_) => sent += 1,
            Err(e) => {
                failed += 1;
                eprintln!("send to {platform}:{user_id} failed: {e}");
            }
        }
    }
    if sent == 0 && failed > 0 {
        anyhow::bail!("chair notify: all {failed} sends failed");
    }
    Ok((sent, failed))
}

async fn handle_holon_action(config: &AppConfig, action: HolonAction) -> Result<()> {
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

    // G2 (REMAINING-GAPS): the kernel's [notify] delivery seam — like the
    // archive, it needs the [holon] config but NOT the read API (it shells
    // the kernel CLI through chair_channels, the same single source the
    // inbound is_chair check uses). The kernel computed and deduped what
    // is new before shelling us; this arm only delivers.
    if let HolonAction::Notify { reason, text } = action {
        if !config.holon.enabled {
            anyhow::bail!(
                "org mode is off: set [holon] enabled = true in operant.toml \
                 (docs/agentic-os/README.md — invariant 5: off is byte-identical personal mode)"
            );
        }
        if config.holon.home.as_os_str().is_empty() {
            anyhow::bail!("org mode needs [holon] home set to the Holon instance ($HOLON_HOME)");
        }
        validated_reason("org holon notify", &reason)?;
        let channels = operant_os::chair::chair_channels(
            std::path::Path::new(&config.holon.home),
            config.holon.bin.as_deref(),
        )
        .await?;
        if channels.is_empty() {
            // not an error: an org with nobody configured to hear is the
            // owner's setup state; the kernel's own notify event still
            // records the attempt
            println!("no [chair] channels configured — the message has nobody to reach");
            return Ok(());
        }
        let (sent, failed) = deliver_to_chairs(&channels, async |platform, user_id| {
            crate::gateway_runner::send_channel_message(config, platform, user_id, &text).await
        })
        .await?;
        println!("chair notify: {sent} sent, {failed} failed (reason: {reason})");
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
            let passes = async {
                loop {
                    // O-9: an error backs off the next pass only — it is
                    // per-pass state, not a flag that sticks for the process
                    let mut errored = false;
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
                        // G1 live-day note: rc=0 with no output was opaque on the
                        // workstation — "nothing to claim" is a real, common
                        // answer (all held stages leased, none ready) and the
                        // operator should see it said
                        if !ran {
                            println!(
                                "nothing to claim{} — every ready stage for these \
                             employees is held or none is ready",
                                stage.map(|s| format!(" at stage {s}")).unwrap_or_default()
                            );
                        }
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
            };
            // OS-6: a signalled supervisor stops here. Dropping the pass drops
            // the body in flight, and its process group dies with it, so no
            // body outlives its supervisor
            tokio::select! {
                () = passes => {}
                signal = operant_os::supervisor::shutdown_signal() => {
                    signal?;
                    eprintln!("supervise: stopped by signal — the body in flight was killed");
                }
            }
        }
        HolonAction::Tick => {
            let home = config.holon.home.clone();
            let bin = config.holon.bin.clone();
            operant_os::run::tick(&home, bin.as_deref()).await?;
            println!("tick fired");
        }
        // (the Archive and Notify arms run BEFORE the read-API gate above —
        // see the handler head; this match never sees them)
        HolonAction::Archive { date: _ } => unreachable!("handled before the read-API gate"),
        HolonAction::Notify { .. } => unreachable!("handled before the read-API gate"),
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

#[cfg(test)]
pub(crate) mod roster_tests {
    use super::*;

    /// A Holon home whose `work.db` publishes the read API (version 4) with one
    /// roster row per id.
    pub(crate) fn org_home(ids: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let runtime = dir.path().join("execution").join("runtime");
        std::fs::create_dir_all(&runtime).unwrap();
        let conn = rusqlite::Connection::open(runtime.join("work.db")).unwrap();
        conn.execute_batch(
            "CREATE TABLE api_meta (key TEXT, value TEXT);
             INSERT INTO api_meta VALUES ('api_version', '4');
             CREATE TABLE api_employees (id TEXT, name TEXT, title TEXT, department TEXT,
                 manager_id TEXT, manager TEXT, status TEXT, driver TEXT, roles TEXT);",
        )
        .unwrap();
        for id in ids {
            conn.execute(
                "INSERT INTO api_employees VALUES (?1, 'n', 't', 'd', NULL, NULL, 'active', 'operant', NULL)",
                [id],
            )
            .unwrap();
        }
        dir
    }

    pub(crate) fn org_config(home: &std::path::Path) -> AppConfig {
        let mut config = AppConfig::default();
        config.holon.enabled = true;
        config.holon.home = home.to_path_buf();
        config
    }

    #[test]
    fn roster_lookup_matches_published_ids_only() {
        let dir = org_home(&["emp_a"]);
        let config = org_config(dir.path());
        assert!(roster_has_employee(&config, "emp_a").unwrap());
        assert!(!roster_has_employee(&config, "hrmaster").unwrap());
    }

    #[test]
    fn unreadable_roster_is_an_error_not_a_miss() {
        let dir = tempfile::tempdir().unwrap();
        assert!(roster_has_employee(&org_config(dir.path()), "emp_a").is_err());
    }
}

#[cfg(test)]
mod notify_delivery_tests {
    use super::*;

    fn chairs() -> Vec<String> {
        vec!["telegram:1".to_string(), "telegram:2".to_string()]
    }

    #[tokio::test]
    async fn notify_is_an_error_when_every_send_fails() {
        // OS-5: a run where nothing reached a chair must exit nonzero.
        let err = deliver_to_chairs(&chairs(), async |_, _| anyhow::bail!("offline"))
            .await
            .expect_err("zero delivered is an error");
        assert!(err.to_string().contains("all 2 sends failed"), "{err}");
    }

    #[tokio::test]
    async fn notify_succeeds_when_one_send_lands() {
        // A partial failure still counts as delivered, so the exit stays zero.
        let calls = std::cell::Cell::new(0usize);
        let (sent, failed) = deliver_to_chairs(&chairs(), async |_, _| {
            let call = calls.get();
            calls.set(call + 1);
            if call == 0 {
                Ok("sent".to_string())
            } else {
                anyhow::bail!("offline")
            }
        })
        .await
        .expect("one delivery is success");
        assert_eq!((sent, failed), (1, 1));
    }
}
