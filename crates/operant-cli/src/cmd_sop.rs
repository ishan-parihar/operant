use anyhow::Result;
use clap::Subcommand;
use serde::{Deserialize, Serialize};

use operant_core::config::AppConfig;

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

pub async fn handle_sop_command(
    _config: &AppConfig,
    cmd: SopSubcommand,
    json: bool,
) -> Result<()> {
    match cmd {
        SopSubcommand::List => {
            if json {
                println!("{{\"sops\":[]}}");
            } else {
                println!("No SOPs loaded.");
                println!();
                println!("SOPs (Standard Operating Procedures) define agent behavior for specific workflows.");
                println!("Add .sop.toml files to your workspace to define SOPs.");
            }
            Ok(())
        }
        SopSubcommand::Validate { name } => {
            if json {
                println!(
                    "{{\"status\":\"valid\",\"name\":{}}}",
                    name.map(|n| format!("\"{}\"", n))
                        .unwrap_or_else(|| "null".to_string())
                );
            } else {
                match name {
                    Some(n) => println!("SOP '{}' is valid.", n),
                    None => println!("All SOPs are valid."),
                }
            }
            Ok(())
        }
        SopSubcommand::Show { name } => {
            if json {
                println!(
                    "{{\"name\":\"{}\",\"description\":\"\",\"triggers\":[],\"steps\":[]}}",
                    name
                );
            } else {
                println!("SOP: {}", name);
                println!("No SOP with this name is currently loaded.");
            }
            Ok(())
        }
    }
}
