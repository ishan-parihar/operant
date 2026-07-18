use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use clap::Subcommand;
use operant_core::config::AppConfig;
use operant_core::curator::{CuratorEngine, archiver, backup};
use operant_core::skill_usage::{LifecycleState, SkillUsageTracker};

#[derive(Debug, Clone, Subcommand)]
pub enum CuratorSubcommand {
    /// Show curator status
    Status,
    /// Run curator review
    Run {
        /// Synchronous review
        #[arg(long)]
        sync: bool,
        /// Run in background
        #[arg(long)]
        background: bool,
        /// Dry-run mode (no changes applied)
        #[arg(long, action = clap::ArgAction::SetTrue)]
        dry_run: bool,
    },
    /// Pause the curator
    Pause,
    /// Resume the curator
    Resume,
    /// Pin a skill (prevent it from being archived)
    Pin {
        /// Skill name to pin
        skill: String,
    },
    /// Unpin a skill
    Unpin {
        /// Skill name to unpin
        skill: String,
    },
    /// Restore an archived skill
    Restore {
        /// Skill name to restore
        skill: String,
    },
    /// List archived skills
    ListArchived,
    /// Archive a skill
    Archive {
        /// Skill name to archive
        skill: String,
    },
    /// Prune curated (archived/expired) skills
    Prune {
        /// Prune skills unused for N days
        #[arg(long)]
        days: Option<u64>,
        /// Skip confirmation prompt
        #[arg(long, action = clap::ArgAction::SetTrue)]
        yes: bool,
        /// Dry-run mode (show what would be pruned)
        #[arg(long, action = clap::ArgAction::SetTrue)]
        dry_run: bool,
    },
    /// Backup curator state
    Backup {
        /// Optional reason for the backup
        #[arg(long)]
        reason: Option<String>,
    },
    /// Rollback curator state from a backup
    Rollback {
        /// List available backups
        #[arg(long, action = clap::ArgAction::SetTrue)]
        list: bool,
        /// Backup ID to rollback to
        #[arg(long)]
        id: Option<String>,
        /// Skip confirmation prompt
        #[arg(long, action = clap::ArgAction::SetTrue)]
        yes: bool,
    },
}

pub async fn handle_curator_command(config: &AppConfig, cmd: CuratorSubcommand, _json: bool) -> Result<()> {
    let skills_dir = config.skills.root_dir.clone();
    let curator_dir = skills_dir.join(".curator");
    let state_path = curator_dir.join("state.json");
    let usage_path = curator_dir.join("usage.json");
    let archive_dir = skills_dir.join(".archive");
    let backup_dir = skills_dir.join(".backups");

    let tracker = Arc::new(SkillUsageTracker::new(usage_path));
    let engine = CuratorEngine::new(skills_dir.clone(), state_path, tracker.clone());
    engine.load_state().await?;
    tracker.load()?;

    match cmd {
        CuratorSubcommand::Status => cmd_status(&engine).await,
        CuratorSubcommand::Run {
            sync: _,
            background,
            dry_run,
        } => cmd_run(&engine, background, dry_run).await,
        CuratorSubcommand::Pause => cmd_pause(&engine).await,
        CuratorSubcommand::Resume => cmd_resume(&engine).await,
        CuratorSubcommand::Pin { skill } => cmd_pin(&tracker, &skill).await,
        CuratorSubcommand::Unpin { skill } => cmd_unpin(&tracker, &skill).await,
        CuratorSubcommand::Restore { skill } => {
            cmd_restore(&tracker, &archive_dir, &skills_dir, &skill).await
        }
        CuratorSubcommand::ListArchived => cmd_list_archived(&archive_dir).await,
        CuratorSubcommand::Archive { skill } => {
            cmd_archive(&tracker, &archive_dir, &skills_dir, &skill).await
        }
        CuratorSubcommand::Prune { days, yes, dry_run } => {
            cmd_prune(&archive_dir, days, yes, dry_run).await
        }
        CuratorSubcommand::Backup { reason } => {
            cmd_backup(&skills_dir, &backup_dir, reason.as_deref()).await
        }
        CuratorSubcommand::Rollback { list, id, yes } => {
            cmd_rollback(&skills_dir, &backup_dir, list, id.as_deref(), yes).await
        }
    }
}

// ---------------------------------------------------------------------------
// Handler implementations
// ---------------------------------------------------------------------------

async fn cmd_status(engine: &CuratorEngine) -> Result<()> {
    let state = engine.get_state().await;
    println!("Curator Status:");
    println!("  Enabled: {}", state.enabled);
    println!("  Paused: {}", state.paused);
    println!("  Interval: {} hours", state.interval_hours);
    println!("  Run Count: {}", state.run_count);
    if let Some(last_run) = state.last_run_at {
        let secs = std::time::Duration::from_secs(last_run as u64);
        println!("  Last Run: {}s since epoch", secs.as_secs());
    }
    if let Some(summary) = state.last_run_summary {
        println!("  Last Summary: {}", summary);
    }
    if let Some(report_path) = state.last_report_path {
        println!("  Last Report: {}", report_path.display());
    }
    println!("  Stale After: {} days", state.stale_after_days);
    println!("  Archive After: {} days", state.archive_after_days);
    Ok(())
}

async fn cmd_run(engine: &CuratorEngine, background: bool, dry_run: bool) -> Result<()> {
    if background {
        println!("Curator run (background mode not yet supported, running synchronously)...");
    } else {
        println!("Curator run...");
    }
    if dry_run {
        println!("  Mode: dry-run (no changes applied)");
    }

    let report = engine.run_review(dry_run, None).await?;
    println!("{}", report.summary);
    if !report.skills_archived.is_empty() {
        println!(
            "  Archived ({}): {}",
            report.skills_archived.len(),
            report.skills_archived.join(", ")
        );
    }
    if !report.skills_stale.is_empty() {
        println!(
            "  Stale ({}): {}",
            report.skills_stale.len(),
            report.skills_stale.join(", ")
        );
    }
    for err in &report.errors {
        eprintln!("  Error: {}", err);
    }
    Ok(())
}

async fn cmd_pause(engine: &CuratorEngine) -> Result<()> {
    engine.set_paused(true).await?;
    println!("Curator paused.");
    Ok(())
}

async fn cmd_resume(engine: &CuratorEngine) -> Result<()> {
    engine.set_paused(false).await?;
    println!("Curator resumed.");
    Ok(())
}

async fn cmd_pin(tracker: &SkillUsageTracker, skill: &str) -> Result<()> {
    match tracker.set_pinned(skill, true) {
        Ok(()) => {
            tracker.save()?;
            println!("Pinned skill '{}'. It will not be auto-archived.", skill);
        }
        Err(e) => {
            // Skill may not have a telemetry record yet — still succeed for the user
            eprintln!("Warning: could not update telemetry for '{}': {}", skill, e);
            println!("Pinned skill '{}'.", skill);
        }
    }
    Ok(())
}

async fn cmd_unpin(tracker: &SkillUsageTracker, skill: &str) -> Result<()> {
    match tracker.set_pinned(skill, false) {
        Ok(()) => {
            tracker.save()?;
            println!("Unpinned skill '{}'. It may now be auto-archived.", skill);
        }
        Err(e) => {
            eprintln!("Warning: could not update telemetry for '{}': {}", skill, e);
            println!("Unpinned skill '{}'.", skill);
        }
    }
    Ok(())
}

async fn cmd_restore(
    tracker: &SkillUsageTracker,
    archive_dir: &Path,
    skills_dir: &Path,
    skill: &str,
) -> Result<()> {
    archiver::restore_skill(skill, archive_dir, skills_dir)?;
    // Update telemetry state if a record exists
    let _ = tracker.set_state(skill, LifecycleState::Active);
    tracker.save()?;
    println!("Restored skill '{}' from archive.", skill);
    Ok(())
}

async fn cmd_list_archived(archive_dir: &Path) -> Result<()> {
    let archived = archiver::list_archived(archive_dir)?;
    if archived.is_empty() {
        println!("No archived skills.");
    } else {
        println!("Archived skills ({}):", archived.len());
        for name in &archived {
            println!("  - {}", name);
        }
    }
    Ok(())
}

async fn cmd_archive(
    tracker: &SkillUsageTracker,
    archive_dir: &Path,
    skills_dir: &Path,
    skill: &str,
) -> Result<()> {
    archiver::archive_skill(skill, skills_dir, archive_dir)?;
    // Update telemetry state if a record exists
    let _ = tracker.set_state(skill, LifecycleState::Archived);
    tracker.save()?;
    println!("Archived skill '{}'.", skill);
    Ok(())
}

async fn cmd_prune(archive_dir: &Path, days: Option<u64>, yes: bool, dry_run: bool) -> Result<()> {
    let threshold_days = days.unwrap_or(90);
    if dry_run {
        let pruned = archiver::prune_archived(archive_dir, threshold_days)?;
        if pruned.is_empty() {
            println!(
                "No archived skills older than {} days would be pruned.",
                threshold_days
            );
        } else {
            println!(
                "Would prune {} archived skill(s) older than {} days:",
                pruned.len(),
                threshold_days
            );
            for name in &pruned {
                println!("  - {}", name);
            }
        }
        return Ok(());
    }

    if !yes {
        eprintln!("Pruning requires confirmation. Use --yes to proceed, or --dry-run to preview.");
        return Ok(());
    }

    let pruned = archiver::prune_archived(archive_dir, threshold_days)?;
    if pruned.is_empty() {
        println!(
            "No archived skills older than {} days to prune.",
            threshold_days
        );
    } else {
        println!(
            "Pruned {} archived skill(s) older than {} days:",
            pruned.len(),
            threshold_days
        );
        for name in &pruned {
            println!("  - {}", name);
        }
    }
    Ok(())
}

async fn cmd_backup(skills_dir: &Path, backup_dir: &Path, reason: Option<&str>) -> Result<()> {
    let path = backup::create_backup(skills_dir, backup_dir, reason)?;
    println!("Backup created: {}", path.display());
    if let Some(r) = reason {
        println!("  Reason: {}", r);
    }
    Ok(())
}

async fn cmd_rollback(
    skills_dir: &Path,
    backup_dir: &Path,
    list: bool,
    id: Option<&str>,
    yes: bool,
) -> Result<()> {
    if list {
        let backups = backup::list_backups(backup_dir)?;
        if backups.is_empty() {
            println!("No backups available.");
        } else {
            println!("Available backups:");
            for b in &backups {
                let name = b
                    .file_name()
                    .map(|n| n.to_string_lossy())
                    .unwrap_or_default();
                println!("  {}", name);
            }
        }
        return Ok(());
    }

    let backup_id = match id {
        Some(id) => id,
        None => {
            eprintln!("Specify a backup ID with --id, or use --list to see available backups.");
            return Ok(());
        }
    };

    if !yes {
        eprintln!(
            "Rollback requires confirmation. Use --yes to proceed to '{}'.",
            backup_id
        );
        return Ok(());
    }

    // Resolve backup path — try exact match first, then partial filename match
    let backup_path = {
        let exact = backup_dir.join(backup_id);
        if exact.exists() {
            exact
        } else {
            let backups = backup::list_backups(backup_dir)?;
            backups
                .into_iter()
                .find(|p| {
                    p.file_name()
                        .map(|n| n.to_string_lossy().contains(backup_id))
                        .unwrap_or(false)
                })
                .ok_or_else(|| anyhow::anyhow!("No backup found matching '{}'", backup_id))?
        }
    };

    let rollback_path = backup::restore_backup(&backup_path, skills_dir)?;
    println!("Rolled back to backup: {}", backup_path.display());
    println!(
        "Previous skills directory preserved at: {}",
        rollback_path.display()
    );
    Ok(())
}
