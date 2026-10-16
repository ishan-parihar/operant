use anyhow::{Context, Result};
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

            // Discover what can be migrated
            let memory_file = source_path.join("MEMORY.md");
            let user_file = source_path.join("USER.md");
            let has_memory = memory_file.exists();
            let has_user = user_file.exists();

            if dry_run {
                if json {
                    println!(
                        "{}",
                        serde_json::json!({
                            "status": "preview",
                            "source": source_path.to_string_lossy(),
                            "can_migrate": has_memory || has_user,
                            "files": {
                                "MEMORY.md": has_memory,
                                "USER.md": has_user
                            }
                        })
                    );
                } else {
                    println!("Migration preview from: {}", source_path.display());
                    println!(
                        "  MEMORY.md: {}",
                        if has_memory {
                            "✅ found"
                        } else {
                            "❌ not found"
                        }
                    );
                    println!(
                        "  USER.md:   {}",
                        if has_user {
                            "✅ found"
                        } else {
                            "❌ not found"
                        }
                    );
                    if has_memory || has_user {
                        println!("\nRun without --dry-run to migrate.");
                    } else {
                        println!("\nNo migratable files found.");
                    }
                }
            } else {
                // (plan 08) migrate IMPORTS into memory-wire — the one
                // engine — instead of copying MEMORY.md/USER.md into
                // ~/.operant where nothing would ever read them again.
                let mut migrated = 0u32;
                let provider = operant_core::memory_wire::MemoryWireProvider::new(
                    operant_core::platform::operant_home(),
                )?;

                for (found, path, label) in [
                    (has_memory, &memory_file, "MEMORY.md"),
                    (has_user, &user_file, "USER.md"),
                ] {
                    if !found {
                        continue;
                    }
                    let text = std::fs::read_to_string(path)
                        .with_context(|| format!("reading {}", path.display()))?;
                    let mut count = 0u32;
                    for line in text.lines() {
                        let body = line.trim().trim_start_matches('-').trim();
                        if body.is_empty() {
                            continue;
                        }
                        if provider
                            .retain_fact(body, &["migrated".to_string()])
                            .is_some()
                        {
                            count += 1;
                        }
                    }
                    migrated += 1;
                    if !json {
                        println!("✅ Imported {label}: {count} memories → memory-wire");
                    }
                }

                if json {
                    println!(
                        "{}",
                        serde_json::json!({
                            "status": "complete",
                            "migrated": migrated,
                            "source": source_path.to_string_lossy()
                        })
                    );
                } else if migrated == 0 {
                    println!("No files to migrate from {}", source_path.display());
                } else {
                    println!("\nMigration complete: {} files.", migrated);
                }
            }
            Ok(())
        }
    }
}
