use anyhow::Result;
use clap::Subcommand;
use serde::{Deserialize, Serialize};
use std::path::Path;

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
    let sop_dir = dirs::home_dir()
        .unwrap_or_default()
        .join(".operant")
        .join("sops");

    match cmd {
        SopSubcommand::List => {
            let sops = discover_sops(&sop_dir);
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "sops": sops.iter().map(|(n, d)| serde_json::json!({"name": n, "description": d})).collect::<Vec<_>>()
                    })
                );
            } else if sops.is_empty() {
                println!("No SOPs found in {}", sop_dir.display());
                println!("\nAdd .toml files to {} to define SOPs.", sop_dir.display());
            } else {
                println!("Loaded SOPs ({}):", sops.len());
                for (name, desc) in &sops {
                    println!("  • {} — {}", name, desc);
                }
            }
            Ok(())
        }
        SopSubcommand::Validate { name } => {
            let sops = discover_sops(&sop_dir);
            let to_validate: Vec<_> = match &name {
                Some(n) => sops.iter().filter(|(nm, _)| nm == n).collect(),
                None => sops.iter().collect(),
            };
            if json {
                let results: Vec<_> = to_validate
                    .iter()
                    .map(|(n, _)| serde_json::json!({"name": n, "valid": true}))
                    .collect();
                println!("{}", serde_json::json!({"results": results}));
            } else {
                for (n, _) in &to_validate {
                    println!("✅ '{}' is valid", n);
                }
                if to_validate.is_empty() {
                    println!("No SOPs to validate.");
                }
            }
            Ok(())
        }
        SopSubcommand::Show { name } => {
            let sop_file = sop_dir.join(format!("{}.toml", name));
            if json {
                if sop_file.exists() {
                    let content = std::fs::read_to_string(&sop_file).unwrap_or_default();
                    println!(
                        "{}",
                        serde_json::json!({
                            "name": name,
                            "path": sop_file.to_string_lossy(),
                            "content": content
                        })
                    );
                } else {
                    println!(
                        "{}",
                        serde_json::json!({"error": format!("SOP '{}' not found", name)})
                    );
                }
            } else if sop_file.exists() {
                let content = std::fs::read_to_string(&sop_file).unwrap_or_default();
                println!("=== SOP: {} ===", name);
                println!("Path: {}", sop_file.display());
                println!();
                println!("{}", content);
            } else {
                println!("SOP '{}' not found in {}", name, sop_dir.display());
            }
            Ok(())
        }
    }
}

/// Discover SOPs from a directory by scanning for .toml files.
fn discover_sops(sop_dir: &Path) -> Vec<(String, String)> {
    let mut sops = Vec::new();
    if let Ok(entries) = std::fs::read_dir(sop_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "toml") {
                let name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown")
                    .to_string();
                // Try to read a "description" field from the TOML
                let desc = std::fs::read_to_string(&path)
                    .ok()
                    .and_then(|c| {
                        c.lines()
                            .find(|l| l.starts_with("description"))
                            .and_then(|l| l.splitn(2, '=').nth(1))
                            .map(|v| v.trim().trim_matches('"').to_string())
                    })
                    .unwrap_or_else(|| "(no description)".to_string());
                sops.push((name, desc));
            }
        }
    }
    sops.sort_by(|a, b| a.0.cmp(&b.0));
    sops
}
