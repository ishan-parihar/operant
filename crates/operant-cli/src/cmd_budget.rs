//! Budget CLI subcommand (iter-670).
//!
//! `operant budget` is the operator write surface for the per-seat budget
//! overrides (`seat_budgets`, Wave 4 / ORGANISM-ARCHITECTURE §5). The
//! enforcement machinery (turn-boundary hard-refuse / soft-inject over
//! `[bylaws].budget` + per-seat rows) shipped before any operator could
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
    /// `[bylaws].budget` — precedence is per-field. A cap of 0 is the
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
    /// Remove a seat's override (the seat inherits the `[bylaws]` default)
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
    // The ORG SIBLING of the app database — the same file the gateway's
    // turn gate and the cron scheduler's budget gate read. Opening the app
    // database instead wrote the override into a table the enforcement
    // never sees, so the command printed "Budget override set" and nothing
    // was enforced (RT-288).
    SeatBudgetDb::init_org_sibling(&config.database_path)
        .context("Failed to open seat budget database")
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
    println!("Budget override set for '{seat}' (cap {cap}; unset fields inherit [bylaws].budget).");
    Ok(())
}

fn cmd_budget_list(config: &AppConfig) -> Result<()> {
    let rows = budget_db(config)?
        .list_all()
        .context("Failed to list seat budgets")?;
    if rows.is_empty() {
        println!(
            "No budget overrides — every seat runs under [bylaws].budget (default: ungoverned)."
        );
        return Ok(());
    }
    println!(
        "{:<20} {:<12} {:<8} {:<8} MODE",
        "SEAT", "CAP", "BASIS", "WINDOW"
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
        println!("Budget override cleared for '{seat}' (inherits [bylaws].budget).");
    } else {
        println!("No budget override existed for '{seat}'.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RT-288: the command the operator runs must write into the store the
    /// ENFORCEMENT reads.
    ///
    /// `operant budget set` wrote into the app database while both
    /// enforcement paths read the org sibling — so the command printed
    /// "Budget override set for '<seat>'" and nothing was enforced.
    #[test]
    fn the_budget_command_writes_to_the_store_the_enforcement_reads() {
        let dir = std::env::temp_dir().join(format!(
            "operant_budget_cmd_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let config = AppConfig {
            database_path: dir.join("database.db"),
            ..AppConfig::default()
        };

        let db = budget_db(&config).expect("the command's store");
        // The command must not touch the APP database at all: the store it
        // opens is the org sibling, and the app database is not even
        // created (the migration reads it read-only, and only when it
        // exists).
        assert!(
            !config.database_path.exists(),
            "the budget command must not create the app database"
        );
        db.upsert(&SeatBudget {
            employee_id: "dispatcher".to_string(),
            basis: None,
            window: None,
            cap: 50_000.0,
            mode: None,
        })
        .expect("upsert");

        // What the gateway's turn gate and the cron scheduler's budget gate
        // mount.
        let enforcement =
            SeatBudgetDb::init_org_sibling(&config.database_path).expect("the enforcement store");
        let row = enforcement
            .get("dispatcher")
            .expect("read the override")
            .expect("the override the operator set must be visible to enforcement");
        assert_eq!(row.cap, 50_000.0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
