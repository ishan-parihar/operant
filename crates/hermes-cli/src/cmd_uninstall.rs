use anyhow::Result;
use clap::Subcommand;
use hermes_core::config::AppConfig;

#[derive(Debug, Clone, Subcommand)]
pub enum UninstallSubcommand {
    /// Remove Hermes data only (keep config)
    Data {
        /// Skip confirmation prompt
        #[arg(short, long)]
        yes: bool,
    },
    /// Remove everything including configuration
    Full {
        /// Skip confirmation prompt
        #[arg(short, long)]
        yes: bool,
    },
}

pub async fn handle_uninstall_command(config: &AppConfig, cmd: UninstallSubcommand) -> Result<()> {
    match cmd {
        UninstallSubcommand::Data { yes } => cmd_remove(config, false, yes).await,
        UninstallSubcommand::Full { yes } => cmd_remove(config, true, yes).await,
    }
}

fn remove_dir(path: &std::path::Path, label: &str) -> Result<()> {
    if path.exists() {
        std::fs::remove_dir_all(path)?;
        println!("Removed {}: {}", label, path.display());
    } else {
        println!("  {} not found: {}", label, path.display());
    }
    Ok(())
}

async fn cmd_remove(_config: &AppConfig, full: bool, yes: bool) -> Result<()> {
    if !yes {
        let scope = if full { "config and data" } else { "data" };
        eprintln!("This will remove Hermes {} directories.", scope);
        eprintln!("Use --yes to skip this warning and proceed.");
        return Ok(());
    }

    remove_dir(&hermes_core::platform::hermes_data_dir(), "data directory")?;
    if full {
        remove_dir(&hermes_core::platform::hermes_config_dir(), "config directory")?;
    }
    Ok(())
}
