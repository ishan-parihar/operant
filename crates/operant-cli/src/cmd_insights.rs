use anyhow::{Context, Result};
use clap::Subcommand;
use operant_core::agent::insights::InsightsEngine;
use operant_core::config::AppConfig;
use operant_core::database::Database;

#[derive(Debug, Clone, Subcommand)]
pub enum InsightsSubcommand {
    /// Show session statistics
    Sessions {
        /// Number of recent days to analyze
        #[arg(short, long, default_value = "7")]
        days: u64,
    },
    // (iter-154: All variant deleted — just called cmd_sessions(config, 7))
}

pub async fn handle_insights_command(config: &AppConfig, cmd: InsightsSubcommand) -> Result<()> {
    match cmd {
        InsightsSubcommand::Sessions { days } => cmd_sessions(config, days).await,
    }
}

/// The session-insights report, rendered by the engine's terminal formatter.
///
/// This command used to bypass `InsightsEngine` entirely and print two session
/// counts, while `docs/library/observability.md` named `operant insights` as a
/// surface of the insights engine — so the documented surface showed none of
/// the engine's output, and the engine's own terminal renderer had no caller
/// at all. The pub-fn surface audit is what surfaced it: `format_terminal`
/// had exactly one reference in the workspace, its own definition.
async fn cmd_sessions(config: &AppConfig, days: u64) -> Result<()> {
    let db = Database::init(config.database_path.clone()).context("Failed to open database")?;
    let engine = InsightsEngine::new(&db);
    let report = engine.generate(days as u32, None);

    // RT-294: a FAILED query must not read as an empty window. The engine
    // names the failure on the report, so a broken database stays
    // distinguishable from a quiet week — and the report is still printed,
    // because a partial report is more useful than none.
    if let Some(err) = &report.error {
        eprintln!("warning: {err}");
    }

    println!("{}", engine.format_terminal(&report));
    Ok(())
}
