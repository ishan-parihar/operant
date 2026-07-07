use anyhow::{Context, Result};
use clap::Subcommand;
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

async fn cmd_sessions(config: &AppConfig, _days: u64) -> Result<()> {
    let db = Database::init(config.database_path.clone()).context("Failed to open database")?;
    let total = db
        .get_session_count()
        .context("Failed to get session count")?;
    println!("Total sessions: {}", total);
    let sessions = db.list_sessions(50).context("Failed to list sessions")?;
    if sessions.is_empty() {
        println!("No sessions recorded yet.");
        return Ok(());
    }
    let recent_count = sessions.len();
    let msg_sum: i64 = sessions.iter().map(|s| s.message_count as i64).sum();
    println!("Recent sessions (last {}): {}", 50, recent_count);
    println!("Total messages in recent sessions: {}", msg_sum);
    if !sessions.is_empty() {
        println!("First recorded: {}", sessions.last().unwrap().created_at);
        println!("Last recorded:  {}", sessions.first().unwrap().updated_at);
    }
    Ok(())
}
