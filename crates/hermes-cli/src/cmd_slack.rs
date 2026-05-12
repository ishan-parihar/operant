use anyhow::{Context, Result};
use clap::Subcommand;
use hermes_core::config::AppConfig;
use serde_json;

#[derive(Debug, Clone, Subcommand)]
pub enum SlackSubcommand {
    /// Generate a Slack app manifest JSON
    Manifest {
        /// Write manifest to a file instead of stdout
        #[arg(long)]
        write: Option<String>,

        /// Override the app name
        #[arg(long)]
        name: Option<String>,

        /// Override the app description
        #[arg(long)]
        description: Option<String>,

        /// Output only the slash_commands section
        #[arg(long, action = clap::ArgAction::SetTrue)]
        slashes_only: bool,
    },
}

pub async fn handle_slack_command(config: &AppConfig, cmd: SlackSubcommand) -> Result<()> {
    match cmd {
        SlackSubcommand::Manifest {
            write,
            name,
            description,
            slashes_only,
        } => cmd_manifest(config, write, name, description, slashes_only).await,
    }
}

async fn cmd_manifest(
    _config: &AppConfig,
    write_path: Option<String>,
    name_override: Option<String>,
    desc_override: Option<String>,
    slashes_only: bool,
) -> Result<()> {
    let app_name = name_override.unwrap_or_else(|| "Hermes".to_string());
    let app_desc = desc_override.unwrap_or_else(|| "A high-performance ReAct agent framework".to_string());

    let manifest = if slashes_only {
        serde_json::json!({
            "slash_commands": [
                {
                    "command": "/hermes",
                    "description": "Interact with Hermes",
                    "usage_hint": "[query]"
                }
            ]
        })
    } else {
        serde_json::json!({
            "name": app_name,
            "description": app_desc,
            "display_information": {
                "name": app_name,
                "description": app_desc,
                "background_color": "#1a1a2e"
            },
            "features": {
                "slash_commands": [
                    {
                        "command": "/hermes",
                        "description": "Interact with Hermes",
                        "usage_hint": "[query]"
                    }
                ]
            },
            "oauth_config": {
                "scopes": {
                    "bot": ["commands"]
                }
            }
        })
    };

    let output = serde_json::to_string_pretty(&manifest)
        .context("Failed to serialize manifest JSON")?;

    match write_path {
        Some(path) => {
            std::fs::write(&path, &output)
                .with_context(|| format!("Failed to write manifest to {}", path))?;
            println!("Manifest written to {}", path);
        }
        None => {
            println!("{}", output);
        }
    }

    Ok(())
}
