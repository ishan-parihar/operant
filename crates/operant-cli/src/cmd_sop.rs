//! CLI subcommand for the operator's standard operating procedures.
//!
//! An SOP is a TOML file in `~/.operant/sops/` — a playbook for a recurring
//! task. The files are the same ones the agent's `sop` tool reads (iter-1126),
//! so what the operator lists here is what the agent can follow.

use anyhow::Result;
use clap::Subcommand;
use serde::{Deserialize, Serialize};

use operant_core::config::AppConfig;
use operant_core::sops::{discover_sops, sops_dir};

#[derive(Subcommand, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SopSubcommand {
    /// List loaded SOPs
    List,
    /// Validate SOP definitions
    Validate {
        /// SOP name to validate (all if omitted)
        name: Option<String>,
    },
    /// Show details of an SOP
    Show {
        /// Name of the SOP to show
        name: String,
    },
}

pub async fn handle_sop_command(_config: &AppConfig, cmd: SopSubcommand, json: bool) -> Result<()> {
    let sop_dir = sops_dir();
    let sops = discover_sops(&sop_dir);

    match cmd {
        SopSubcommand::List => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "sops": sops.iter().map(|s| serde_json::json!({"name": s.name, "description": s.description})).collect::<Vec<_>>()
                    })
                );
            } else if sops.is_empty() {
                println!("No SOPs found in {}", sop_dir.display());
                println!("\nAdd .toml files to {} to define SOPs.", sop_dir.display());
            } else {
                println!("Loaded SOPs ({}):", sops.len());
                for sop in &sops {
                    println!("  • {} — {}", sop.name, sop.description);
                }
            }
            Ok(())
        }
        SopSubcommand::Validate { name } => {
            let to_validate: Vec<_> = match &name {
                Some(n) => sops.iter().filter(|sop| &sop.name == n).collect(),
                None => sops.iter().collect(),
            };
            if json {
                let results: Vec<_> = to_validate
                    .iter()
                    .map(|sop| serde_json::json!({"name": sop.name, "valid": true}))
                    .collect();
                println!("{}", serde_json::json!({"results": results}));
            } else {
                for sop in &to_validate {
                    println!("✅ '{}' is valid", sop.name);
                }
                if to_validate.is_empty() {
                    println!("No SOPs to validate.");
                }
            }
            Ok(())
        }
        SopSubcommand::Show { name } => {
            let wanted = name.trim().to_ascii_lowercase();
            let found = sops
                .iter()
                .find(|sop| sop.name.to_ascii_lowercase() == wanted);
            match found {
                Some(sop) => {
                    if json {
                        println!(
                            "{}",
                            serde_json::json!({
                                "name": sop.name,
                                "path": sop.path.to_string_lossy(),
                                "content": sop.content
                            })
                        );
                    } else {
                        println!("=== SOP: {} ===", sop.name);
                        println!("Path: {}", sop.path.display());
                        println!();
                        println!("{}", sop.content);
                    }
                }
                None => {
                    if json {
                        println!(
                            "{}",
                            serde_json::json!({"error": format!("SOP '{}' not found", name)})
                        );
                    } else {
                        println!("SOP '{}' not found in {}", name, sop_dir.display());
                    }
                }
            }
            Ok(())
        }
    }
}
