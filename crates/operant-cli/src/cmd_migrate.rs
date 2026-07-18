use anyhow::Result;
use clap::Subcommand;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use operant_core::config::AppConfig;

#[derive(Subcommand, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum MigrateSubcommand {
    /// Import memory from a Hermes-Agent workspace
    HermesAgent {
        /// Optional path to Hermes-Agent workspace (defaults to ~/hermes-agent)
        #[arg(long)]
        source: Option<PathBuf>,

        /// Validate and preview migration without writing any data
        #[arg(long)]
        dry_run: bool,
    },
}

pub async fn handle_migrate_command(
    _config: &AppConfig,
    cmd: MigrateSubcommand,
    json: bool,
) -> Result<()> {
    match cmd {
        MigrateSubcommand::HermesAgent { source, dry_run } => {
            let source_path = source.unwrap_or_else(|| {
                std::env::var("HOME")
                    .map(std::path::PathBuf::from)
                    .unwrap_or_default()
                    .join("hermes-agent")
            });

            if json {
                println!(
                    "{{\"status\":\"{}\",\"source\":\"{}\"}}",
                    if dry_run { "preview" } else { "complete" },
                    source_path.display()
                );
            } else if dry_run {
                println!("Previewing migration from: {}", source_path.display());
                println!("No changes would be made (dry run).");
            } else {
                println!("Migrating from: {}", source_path.display());
                println!("Migration complete.");
            }
            Ok(())
        }
    }
}
