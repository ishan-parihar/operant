//! Checkpoints CLI subcommand
//!
//! Provides `hermes checkpoints status`, `hermes checkpoints list`,
//! `hermes checkpoints prune`, and `hermes checkpoints clear`.

use anyhow::{Context, Result};
use clap::Subcommand;
use hermes_core::config::AppConfig;
use hermes_core::database::Database;
use hermes_core::platform::hermes_home;

/// Manage filesystem checkpoints
#[derive(Debug, Clone, Subcommand)]
pub enum CheckpointsSubcommand {
    /// Show checkpoint system status (enabled/disabled, storage info)
    Status,
    /// List available checkpoints in a table
    List,
    /// Prune old checkpoints, keeping the N most recent (default: 10)
    Prune {
        /// Number of most recent checkpoints to keep
        keep: Option<usize>,
    },
    /// Clear all checkpoints for the current directory
    Clear,
}

/// Dispatch a checkpoints subcommand.
pub async fn handle_checkpoints_command(
    config: &AppConfig,
    cmd: CheckpointsSubcommand,
) -> Result<()> {
    match cmd {
        CheckpointsSubcommand::Status => cmd_status(config).await,
        CheckpointsSubcommand::List => cmd_list(config).await,
        CheckpointsSubcommand::Prune { keep } => cmd_prune(config, keep.unwrap_or(10)).await,
        CheckpointsSubcommand::Clear => cmd_clear(config).await,
    }
}

fn working_dir() -> Result<String> {
    let dir = std::env::current_dir().context("Failed to determine current directory")?;
    Ok(dir.to_string_lossy().to_string())
}

fn dir_size(path: &std::path::Path) -> std::io::Result<u64> {
    let mut total = 0u64;
    if path.is_dir() {
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            let child = entry.path();
            if child.is_dir() {
                total += dir_size(&child)?;
            } else {
                total += entry.metadata()?.len();
            }
        }
    }
    Ok(total)
}

fn format_size(size: u64) -> String {
    if size >= 1_000_000_000 {
        format!("{:.2} GB", size as f64 / 1_000_000_000.0)
    } else if size >= 1_000_000 {
        format!("{:.2} MB", size as f64 / 1_000_000.0)
    } else if size >= 1_000 {
        format!("{:.2} KB", size as f64 / 1_000.0)
    } else {
        format!("{} bytes", size)
    }
}

async fn cmd_status(config: &AppConfig) -> Result<()> {
    let storage = hermes_home().join("checkpoints");
    let enabled = storage.exists();

    println!("Checkpoint System Status");
    println!("{}", "=".repeat(40));
    println!(
        "Status:           {}",
        if enabled { "Enabled" } else { "Disabled" }
    );
    println!("Storage:          {}", storage.display());

    if enabled {
        match dir_size(&storage) {
            Ok(size) => println!("Storage size:     {}", format_size(size)),
            Err(e) => println!("Storage size:     <error: {}>", e),
        }
    } else {
        println!("Storage size:     N/A (directory does not exist yet)");
    }

    let db = Database::init(config.database_path.clone()).context("Failed to open database")?;

    let dir = working_dir()?;
    let checkpoints = db
        .list_checkpoints(&dir)
        .context("Failed to list checkpoints")?;

    println!("Working directory: {}", dir);
    println!();
    println!("Database: {}", config.database_path.display());

    if let Ok(meta) = std::fs::metadata(&config.database_path) {
        println!("Database size: {}", format_size(meta.len()));
    }

    println!("Checkpoints (this dir): {}", checkpoints.len());
    println!();

    Ok(())
}

async fn cmd_list(config: &AppConfig) -> Result<()> {
    let db = Database::init(config.database_path.clone()).context("Failed to open database")?;

    let dir = working_dir()?;
    let checkpoints = db
        .list_checkpoints(&dir)
        .context("Failed to list checkpoints")?;

    if checkpoints.is_empty() {
        println!("No checkpoints found for this directory.");
        return Ok(());
    }

    println!("Checkpoints for: {}", dir);
    println!();
    println!("{:<12} {:<28}  Reason", "Hash", "Timestamp");
    println!("{}", "-".repeat(88));

    for cp in &checkpoints {
        let short = if cp.hash.len() > 8 {
            &cp.hash[..8]
        } else {
            &cp.hash
        };
        let reason = cp.reason.as_deref().unwrap_or("-");
        let display = if reason.len() > 50 {
            format!("{}…", &reason[..49])
        } else {
            reason.to_string()
        };
        println!("{:<12} {:<28}  {}", short, cp.timestamp, display);
    }

    println!();
    println!("Total: {} checkpoint(s)", checkpoints.len());
    Ok(())
}

async fn cmd_prune(config: &AppConfig, keep: usize) -> Result<()> {
    let db = Database::init(config.database_path.clone()).context("Failed to open database")?;

    let dir = working_dir()?;
    let checkpoints = db
        .list_checkpoints(&dir)
        .context("Failed to list checkpoints")?;

    if checkpoints.len() <= keep {
        println!(
            "Only {} checkpoint(s) found (keep={}), nothing to prune.",
            checkpoints.len(),
            keep,
        );
        return Ok(());
    }

    // checkpoints sorted by timestamp DESC so first `keep` are newest
    let to_delete = &checkpoints[keep..];
    for cp in to_delete {
        db.delete_checkpoint(&cp.hash)
            .with_context(|| format!("Failed to delete checkpoint {}", cp.hash))?;
    }

    println!("Pruned {} checkpoint(s), kept {}.", to_delete.len(), keep,);
    Ok(())
}

async fn cmd_clear(config: &AppConfig) -> Result<()> {
    let db = Database::init(config.database_path.clone()).context("Failed to open database")?;

    let dir = working_dir()?;
    let checkpoints = db
        .list_checkpoints(&dir)
        .context("Failed to list checkpoints")?;

    let count = checkpoints.len();
    if count == 0 {
        println!("No checkpoints to clear.");
        return Ok(());
    }

    for cp in &checkpoints {
        db.delete_checkpoint(&cp.hash)
            .with_context(|| format!("Failed to delete checkpoint {}", cp.hash))?;
    }

    println!("Cleared {} checkpoint(s).", count);
    Ok(())
}
