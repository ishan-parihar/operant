use anyhow::Result;
use clap::Subcommand;
use operant_core::config::AppConfig;
use std::path::PathBuf;

#[derive(Debug, Clone, Subcommand)]
pub enum ClawSubcommand {
    /// Migrate from the old OpenClaw system to Operant
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

fn skills_dir(config: &AppConfig) -> PathBuf {
    config.skills.root_dir.clone()
}

async fn cmd_migrate(
    config: &AppConfig,
    source: Option<String>,
    dry_run: bool,
    _preset: Option<String>,
    overwrite: bool,
    _migrate_secrets: bool,
    _no_backup: bool,
    _workspace_target: Option<String>,
    _skill_conflict: Option<String>,
    _yes: bool,
) -> Result<()> {
    let claw_dir = match crate::claw_migrate::detect_openclaw(source.as_deref()) {
        Some(dir) => dir,
        None => {
            println!("No OpenClaw directory found.");
            println!("Nothing to migrate.");
            return Ok(());
        }
    };

    let target_skills = skills_dir(config);

    let items = crate::claw_migrate::scan_openclaw(&claw_dir)?;
    println!("Found OpenClaw directory: {}", claw_dir.display());
    if items.is_empty() {
        println!("No migratable items found (expected: skills/, config/, etc.).");
        return Ok(());
    }
    println!("Discovered items: {}", items.join(", "));

    if dry_run {
        println!();
        println!("[DRY RUN] Preview of migration:");
        let result = crate::claw_migrate::dry_run_migrate(&claw_dir, &target_skills)?;
        for item in &result.migrated {
            println!(
                "  {}  [{}]  ->  {}",
                item.status, item.item_type, item.source
            );
        }
        println!();
        println!(
            "Skills would be imported to: {}/openclaw-imported",
            target_skills.display()
        );
        return Ok(());
    }

    println!();
    println!("Migrating OpenClaw skills...");
    println!("  Source:      {}/skills", claw_dir.display());
    println!(
        "  Destination: {}/openclaw-imported",
        target_skills.display()
    );

    let result = crate::claw_migrate::migrate_skills(&claw_dir, &target_skills, overwrite)?;

    let migrated_count = result
        .migrated
        .iter()
        .filter(|i| i.status == "migrated")
        .count();
    let skipped_count = result
        .migrated
        .iter()
        .filter(|i| i.status.starts_with("skipped"))
        .count();
    let error_count = result.errors.len();

    println!();
    if migrated_count > 0 {
        println!("✅ Migrated {} skill(s).", migrated_count);
        for item in result.migrated.iter().filter(|i| i.status == "migrated") {
            println!("   - {}", item.source);
        }
    }
    if skipped_count > 0 {
        println!(
            "⏭️  Skipped {} skill(s) (already exist, use --overwrite to replace).",
            skipped_count
        );
    }
    if error_count > 0 {
        println!("❌ {} error(s) during migration:", error_count);
        for err in &result.errors {
            println!("   - {}", err);
        }
    }

    if migrated_count == 0 && error_count == 0 {
        println!("No skills found in the OpenClaw skills directory.");
    }

    println!();
    println!(
        "Migration complete. Skills are available in: {}/openclaw-imported",
        target_skills.display()
    );
    println!(
        "You can run `operant claw cleanup` to remove the OpenClaw directory after verifying."
    );

    Ok(())
}

async fn cmd_cleanup(
    _config: &AppConfig,
    source: Option<String>,
    dry_run: bool,
    _yes: bool,
) -> Result<()> {
    let claw_dir = match crate::claw_migrate::detect_openclaw(source.as_deref()) {
        Some(dir) => dir,
        None => {
            println!("No OpenClaw directory found at ~/.openclaw.");
            println!("Nothing to clean up.");
            return Ok(());
        }
    };

    println!("OpenClaw directory: {}", claw_dir.display());
    println!();

    let messages = crate::claw_migrate::cleanup_openclaw(&claw_dir, dry_run)?;
    if dry_run {
        println!("[DRY RUN] Would perform the following:");
        for msg in &messages {
            println!("  {}", msg);
        }
        println!();
        println!("The OpenClaw directory will be MOVED to a backup location,");
        println!("not permanently deleted. You can restore it if needed.");
        return Ok(());
    }

    for msg in &messages {
        println!("{}", msg);
    }
    println!();
    println!("✅ OpenClaw directory has been backed up and removed.");
    println!("Your original ~/.openclaw was moved to ~/.openclaw.backup");

    Ok(())
}
