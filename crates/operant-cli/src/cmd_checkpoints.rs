//! Checkpoints CLI subcommand
//!
//! Provides `operant checkpoints status`, `operant checkpoints list`,
//! `operant checkpoints prune`, and `operant checkpoints clear`.

use anyhow::{Context, Result};
use clap::Subcommand;
use operant_core::config::AppConfig;
use operant_core::platform::operant_home;
use operant_core::tools::checkpoint_tool::Checkpoint;
use operant_core::tools::get_checkpoint_manager;

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

/// Every snapshot this directory holds, across the session-less store and
/// every session-scoped store (RT-539), newest first.
///
/// THE SOURCE OF THIS LIST IS THE SHADOW STORE, NOT THE DATABASE. It used to
/// be the database's own checkpoint reader, over a `checkpoints` table that
/// one writer fills and nothing in the workspace calls — so
/// `operant checkpoints list` always printed "No checkpoints found",
/// `prune` always reported nothing to prune, and `clear` returned before it
/// reached `clear_store`, leaving a multi-gigabyte store on disk. Every row
/// the TOOL wrote went to `checkpoint_tool`'s git shadow stores, which is
/// what this now reads.
fn live_checkpoints(dir: &str) -> Vec<Checkpoint> {
    let manager = get_checkpoint_manager();
    let mut all = manager.list_checkpoints(dir, None);
    // Session-scoped stores are keyed by a hash the caller does not have, so
    // the manager's own directory-wide entry point is the only one that can
    // see them. It is the session-less store; the per-session ones are found
    // by listing each directory's store set, which only the manager can do.
    all.extend(manager.all_checkpoints_in(dir));
    all.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    all
}

async fn cmd_status(config: &AppConfig) -> Result<()> {
    let storage = operant_home().join("checkpoints");
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

    let dir = working_dir()?;
    let checkpoints = live_checkpoints(&dir);

    println!("Working directory: {}", dir);
    println!();
    // The database line is gone with the table it described: the checkpoint
    // store is a git shadow directory, so the size that matters is its own.
    // (`config` is still taken by every arm so the command's signature and
    // its call site stay uniform.)
    let _ = config;

    println!("Checkpoints (this dir): {}", checkpoints.len());
    println!();

    Ok(())
}

async fn cmd_list(config: &AppConfig) -> Result<()> {
    let _ = config;

    let dir = working_dir()?;
    let checkpoints = live_checkpoints(&dir);

    if checkpoints.is_empty() {
        println!("No checkpoints found for this directory. (Checkpoints are");
        println!("filesystem snapshots taken by the checkpoint tool; enable");
        println!("them with [tools.checkpoint] enabled = true in the config.)");
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
        let reason = if cp.reason.is_empty() {
            "-"
        } else {
            cp.reason.as_str()
        };
        // CHAR-safe: the reason is free text, and a byte slice panics when
        // the cut lands inside a multi-byte character (the same defect that
        // killed `operant sessions list` on the operator's own em-dash).
        let display = if reason.chars().count() > 50 {
            let mut cut: String = reason.chars().take(49).collect();
            cut.push('…');
            cut
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
    let _ = config;

    let dir = working_dir()?;
    let checkpoints = live_checkpoints(&dir);

    if checkpoints.len() <= keep {
        println!(
            "Only {} checkpoint(s) found (keep={}), nothing to prune.",
            checkpoints.len(),
            keep,
        );
        return Ok(());
    }

    // `prune_to` keeps the newest `keep` commits IN EACH STORE and reclaims
    // the dropped objects. The count it reports is commits, which is what the
    // operator asked about, rather than a database row count that never moved.
    let dropped = get_checkpoint_manager()
        .prune_to(&dir, keep)
        .context("Failed to prune checkpoints")?;

    println!("Pruned {} checkpoint(s), kept {} per store.", dropped, keep);
    Ok(())
}

async fn cmd_clear(config: &AppConfig) -> Result<()> {
    let _ = config;

    let dir = working_dir()?;
    let checkpoints = live_checkpoints(&dir);

    let count = checkpoints.len();
    if count == 0 {
        // Still reclaim the bytes: a store whose commits were all pruned, or
        // one written by an older build, holds packed objects with no row to
        // count. The directory is the thing the operator asked to clear.
        let reclaimed = get_checkpoint_manager().clear_store(&dir).unwrap_or(0);
        if reclaimed > 0 {
            println!(
                "Cleared no checkpoints, but reclaimed {} MiB of the shadow store.",
                reclaimed / (1024 * 1024)
            );
        } else {
            println!("No checkpoints to clear.");
        }
        return Ok(());
    }

    // The ROWS were never the store. `clear_store` removes every session's
    // shadow directory for this working directory and reports the bytes it
    // reclaimed — a store that had reached 1.5 GB of packed objects used to
    // report "cleared N checkpoints" and reclaim nothing, because the rows
    // it counted were in a table the tool never wrote to.
    let reclaimed = operant_core::tools::get_checkpoint_manager()
        .clear_store(&dir)
        .context("Failed to clear the checkpoint store")?;

    if reclaimed > 0 {
        println!(
            "Cleared {} checkpoint(s) and reclaimed {} MiB of the shadow store.",
            count,
            reclaimed / (1024 * 1024)
        );
    } else {
        println!("Cleared {} checkpoint(s).", count);
    }
    Ok(())
}
