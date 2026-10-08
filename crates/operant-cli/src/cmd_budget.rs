//! Budget CLI subcommand (iter-670).
//!
//! `operant budget` is the operator write surface for the per-seat budget
//! overrides (`seat_budgets`, Wave 4 / ORGANISM-ARCHITECTURE §5). The
//! enforcement machinery (turn-boundary hard-refuse / soft-inject over
//! `[genome].budget` + per-seat rows) shipped before any operator could
//! write a row — this is the missing provisioning path.
//!
//! Namespace note: this module lives under `operant org budget` (the
//! iter-670 top-level namespace was provisional while `cmd_org.rs` was
//! mid-flight in a peer's tree; folded once it landed). The handlers are
//! unchanged — only the clap routing moved.

use anyhow::{Context, Result, bail};
use clap::Subcommand;
use operant_core::config::AppConfig;
use operant_core::org::seat_budgets::{SeatBudget, SeatBudgetDb};

/// Manage per-seat budget overrides
#[derive(Debug, Clone, Subcommand)]
pub enum BudgetSubcommand {
    /// Set (or replace) a seat's budget override. Fields left unset inherit
    /// `[genome].budget` — precedence is per-field. A cap of 0 is the
    /// documented release valve ("this seat runs free even though the org
    /// default caps"), not an error.
    Set {
        /// Employee/seat id (e.g. dispatcher, premiere)
        seat: String,
        /// The cap on the basis within the window (0 = explicitly free)
        #[arg(long)]
        cap: f64,
        /// What the cap counts: tokens | usd (unset = inherit)
        #[arg(long)]
        basis: Option<String>,
        /// Rollup window: daily | weekly | monthly (unset = inherit)
        #[arg(long)]
        window: Option<String>,
        /// hard (refuse at the cap) | soft (warn + continue) (unset = inherit)
        #[arg(long)]
        mode: Option<String>,
    },
    /// List every seat's budget override
    List,
    /// Remove a seat's override (the seat inherits the [genome] default)
    Clear {
        /// Employee/seat id
        seat: String,
    },
}

pub async fn handle_budget_command(config: &AppConfig, cmd: BudgetSubcommand) -> Result<()> {
    match cmd {
        BudgetSubcommand::Set {
            seat,
            cap,
            basis,
            window,
            mode,
        } => cmd_budget_set(config, &seat, cap, basis, window, mode),
        BudgetSubcommand::List => cmd_budget_list(config),
        BudgetSubcommand::Clear { seat } => cmd_budget_clear(config, &seat),
    }
}

fn budget_db(config: &AppConfig) -> Result<SeatBudgetDb> {
    SeatBudgetDb::init(&config.database_path).context("Failed to open seat budget database")
}

fn cmd_budget_set(
    config: &AppConfig,
    seat: &str,
    cap: f64,
    basis: Option<String>,
    window: Option<String>,
    mode: Option<String>,
) -> Result<()> {
    // Fail-closed at the boundary: an unknown spelling must error rather
    // than silently narrowing what the operator asked for (the §2.3 rule).
    for (name, value, allowed) in [
        ("basis", &basis, &["tokens", "usd"][..]),
        ("window", &window, &["daily", "weekly", "monthly"][..]),
        ("mode", &mode, &["hard", "soft"][..]),
    ] {
        if let Some(value) = value
            && !allowed.contains(&value.as_str())
        {
            bail!(
                "Unknown budget {name} '{value}' — one of: {}",
                allowed.join(" | ")
            );
        }
    }

    budget_db(config)?.upsert(&SeatBudget {
        employee_id: seat.to_string(),
        basis,
        window,
        cap,
        mode,
    })?;
    println!(
        "Budget override set for '{seat}' (cap {cap}; unset fields inherit [genome].budget)."
    );
    Ok(())
}

fn cmd_budget_list(config: &AppConfig) -> Result<()> {
    let rows = budget_db(config)?.list_all().context("Failed to list seat budgets")?;
    if rows.is_empty() {
        println!("No budget overrides — every seat runs under [genome].budget (default: ungoverned).");
        return Ok(());
    }
    println!(
        "{:<20} {:<12} {:<8} {:<8} {}",
        "SEAT", "CAP", "BASIS", "WINDOW", "MODE"
    );
    for row in rows {
        println!(
            "{:<20} {:<12} {:<8} {:<8} {}",
            row.employee_id,
            row.cap,
            row.basis.as_deref().unwrap_or("(inherit)"),
            row.window.as_deref().unwrap_or("(inherit)"),
            row.mode.as_deref().unwrap_or("(inherit)"),
        );
    }
    Ok(())
}

fn cmd_budget_clear(config: &AppConfig, seat: &str) -> Result<()> {
    let db = budget_db(config)?;
    let existed = db.get(seat)?.is_some();
    db.delete(seat)?;
    if existed {
        println!("Budget override cleared for '{seat}' (inherits [genome].budget).");
    } else {
        println!("No budget override existed for '{seat}'.");
    }
    Ok(())
}
