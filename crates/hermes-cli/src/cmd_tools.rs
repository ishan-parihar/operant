//! CLI subcommand for managing tools (`hermes tools`).
//!
//! # Usage
//!
//! - `hermes tools list`           — list all available tools
//! - `hermes tools enable <name>`   — enable a tool (placeholder)
//! - `hermes tools disable <name>`  — disable a tool (placeholder)

use std::sync::Arc;

use anyhow::Result;
use clap::Subcommand;
use hermes_core::client::{ClientConfig, OpenAIClient};
use hermes_core::config::AppConfig;
use hermes_core::database::Database;
use hermes_core::mcp::McpManager;

/// Subcommands for tool management.
#[derive(Debug, Clone, Subcommand)]
pub enum ToolsSubcommand {
    /// List all available tools
    List {
        /// Filter by platform (linux, macos, windows)
        #[arg(long)]
        platform: Option<String>,
    },
    /// Enable one or more tools by name
    Enable {
        /// Tool name(s) to enable
        names: Vec<String>,
    },
    /// Disable one or more tools by name
    Disable {
        /// Tool name(s) to disable
        names: Vec<String>,
    },
}

/// Dispatch and execute a tools subcommand.
pub async fn handle_tools_command(config: &AppConfig, cmd: ToolsSubcommand) -> Result<()> {
    match cmd {
        ToolsSubcommand::List { platform } => handle_list(config, platform).await,
        ToolsSubcommand::Enable { names } => handle_enable(names),
        ToolsSubcommand::Disable { names } => handle_disable(names),
    }
}

/// List all registered tools in a table format.
async fn handle_list(config: &AppConfig, _platform: Option<String>) -> Result<()> {
    let mcp_manager = McpManager::new();
    let client = OpenAIClient::new(ClientConfig::from(&config.client));
    let database = Arc::new(Database::init(config.database_path.clone())?);
    let registry = crate::build_registry(
        config,
        &mcp_manager,
        &client,
        &config.agent.model,
        database,
    )
    .await?;
    let tools = registry.get_schemas().await;

    if tools.is_empty() {
        println!("No tools registered.");
        return Ok(());
    }

    let count = tools.len();
    println!("Available tools ({}):", count);
    println!();

    for tool in &tools {
        println!("  {:<30} {}", tool.name, tool.description);
    }

    println!();
    println!("Use `hermes tools enable <name>` or `hermes tools disable <name>` to manage tools.");

    Ok(())
}

/// Placeholder: enable one or more tools.
fn handle_enable(names: Vec<String>) -> Result<()> {
    for name in &names {
        println!("Tool '{}' enabled.", name);
    }
    if names.is_empty() {
        println!("No tool names provided. Usage: hermes tools enable <name> [<name>...]");
    }
    Ok(())
}

/// Placeholder: disable one or more tools.
fn handle_disable(names: Vec<String>) -> Result<()> {
    for name in &names {
        println!("Tool '{}' disabled.", name);
    }
    if names.is_empty() {
        println!("No tool names provided. Usage: hermes tools disable <name> [<name>...]");
    }
    Ok(())
}
