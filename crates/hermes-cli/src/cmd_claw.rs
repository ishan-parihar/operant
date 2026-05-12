use anyhow::Result;
use clap::Subcommand;
use hermes_core::config::AppConfig;
use std::path::PathBuf;

#[derive(Debug, Clone, Subcommand)]
pub enum ClawSubcommand {
    /// Migrate from the old OpenClaw system to Hermes
    Migrate {
        /// Source directory (default: ~/.openclaw)
        #[arg(long)]
        source: Option<String>,

        /// Preview what would happen without making changes
        #[arg(long, action = clap::ArgAction::SetTrue)]
        dry_run: bool,

        /// Use a named migration preset
        #[arg(long)]
        preset: Option<String>,

        /// Overwrite existing files without confirmation
        #[arg(long, action = clap::ArgAction::SetTrue)]
        overwrite: bool,

        /// Also migrate secrets (API keys, tokens)
        #[arg(long, action = clap::ArgAction::SetTrue)]
        migrate_secrets: bool,

        /// Skip creating a backup before migration
        #[arg(long, action = clap::ArgAction::SetTrue)]
        no_backup: bool,

        /// Target workspace for migration output
        #[arg(long)]
        workspace_target: Option<String>,

        /// How to handle skill conflicts (skip, overwrite, rename)
        #[arg(long)]
        skill_conflict: Option<String>,

        /// Skip confirmation prompts
        #[arg(long, action = clap::ArgAction::SetTrue)]
        yes: bool,
    },

    /// Clean up old Claw data, backups, and artifacts
    Cleanup {
        /// Source directory (default: ~/.openclaw)
        #[arg(long)]
        source: Option<String>,

        /// Preview what would be cleaned up without deleting
        #[arg(long, action = clap::ArgAction::SetTrue)]
        dry_run: bool,

        /// Skip confirmation prompts
        #[arg(long, action = clap::ArgAction::SetTrue)]
        yes: bool,
    },
}

pub async fn handle_claw_command(config: &AppConfig, cmd: ClawSubcommand) -> Result<()> {
    match cmd {
        ClawSubcommand::Migrate {
            source,
            dry_run,
            preset,
            overwrite,
            migrate_secrets,
            no_backup,
            workspace_target,
            skill_conflict,
            yes,
        } => {
            cmd_migrate(
                config,
                source,
                dry_run,
                preset,
                overwrite,
                migrate_secrets,
                no_backup,
                workspace_target,
                skill_conflict,
                yes,
            )
            .await
        }
        ClawSubcommand::Cleanup {
            source,
            dry_run,
            yes,
        } => cmd_cleanup(config, source, dry_run, yes).await,
    }
}

fn openclaw_dir(source: &Option<String>) -> PathBuf {
    match source {
        Some(path) => PathBuf::from(path),
        None => dirs::home_dir()
            .map(|h| h.join(".openclaw"))
            .unwrap_or_else(|| PathBuf::from(".openclaw")),
    }
}

async fn cmd_migrate(
    _config: &AppConfig,
    source: Option<String>,
    dry_run: bool,
    _preset: Option<String>,
    _overwrite: bool,
    _migrate_secrets: bool,
    _no_backup: bool,
    _workspace_target: Option<String>,
    _skill_conflict: Option<String>,
    _yes: bool,
) -> Result<()> {
    let claw_dir = openclaw_dir(&source);

    if !claw_dir.exists() {
        println!("No OpenClaw directory found at: {}", claw_dir.display());
        println!("Nothing to migrate.");
        return Ok(());
    }

    println!("Found OpenClaw directory: {}", claw_dir.display());

    if dry_run {
        println!("[DRY RUN] Would migrate from: {}", claw_dir.display());
        println!("[DRY RUN] The Python migration script is available at:");
        println!(
            "[DRY RUN]   <hermes-repo>/scripts/claw_migrate.py --source {}",
            claw_dir.display()
        );
        return Ok(());
    }

    println!(
        "To complete the migration, run the Python migration script:"
    );
    println!(
        "  python3 scripts/claw_migrate.py --source {}",
        claw_dir.display()
    );
    println!();
    println!("This script will transfer your Claw configuration, skills,");
    println!("workspaces, and data into the Hermes format.");

    Ok(())
}

async fn cmd_cleanup(
    _config: &AppConfig,
    source: Option<String>,
    dry_run: bool,
    _yes: bool,
) -> Result<()> {
    let claw_dir = openclaw_dir(&source);

    if !claw_dir.exists() {
        println!("No OpenClaw directory found at: {}", claw_dir.display());
        println!("Nothing to clean up.");
        return Ok(());
    }

    println!("OpenClaw directory: {}", claw_dir.display());

    if dry_run {
        println!("[DRY RUN] Would clean up the following:");
        println!("[DRY RUN]   - {}", claw_dir.display());
        println!("[DRY RUN]   - Backup files in the Hermes workspace");
        println!("[DRY RUN]   - Temporary migration artifacts");
        return Ok(());
    }

    println!("Cleanup of old Claw data is available via:");
    println!("  python3 scripts/claw_cleanup.py --source {}", claw_dir.display());
    println!();
    println!("This will remove old Claw configuration, backup files,");
    println!("and temporary migration artifacts.");

    Ok(())
}
