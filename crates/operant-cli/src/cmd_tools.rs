//! CLI subcommand for managing tools (`operant tools`).
//!
//! # Usage
//!
//! - `operant tools list`           — list all available tools
//! - `operant tools enable <name>`   — enable a tool
//! - `operant tools disable <name>`  — disable a tool
//!
//! Tool enable/disable state is persisted to a JSON file (`tool_state.json`)
//! in the Operant config directory, independent of the main TOML config.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Subcommand;
use operant_core::client::{ClientConfig, OpenAIClient};
use operant_core::config::AppConfig;
use operant_core::database::Database;
use operant_core::mcp::McpManager;

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
        ToolsSubcommand::Enable { names } => handle_enable(config, names),
        ToolsSubcommand::Disable { names } => handle_disable(config, names),
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
        None,
    )
    .await?;

    // Hermes-plugin parity: surface the active memory provider's own tools
    // (memory_smart_search / memory_save for agentmemory) so `operant tools
    // list` reflects the exact agent-visible toolset (the run path registers
    // these in build_agent_core; here we mirror that for the listing).
    if config.memory.enabled
        && config.memory.provider != "builtin"
        && config.memory.provider != "disabled"
    {
        let (_memory_manager, memory_provider) = crate::load_repo_memory_manager().await?;
        if let Some(provider) = memory_provider {
            // The provider is background-initialized by load_repo_memory_manager;
            // synchronously warm it so is_available() is true and the tools
            // actually appear in the listing (matches the run path's surface).
            provider.ensure_server().await;
            crate::memory_provider_tools::register_provider_tools(&registry, provider).await;
        }
    }

    let tools = registry.get_schemas().await;

    if tools.is_empty() {
        println!("No tools registered.");
        return Ok(());
    }

    let count = tools.len();
    let disabled = load_disabled_tools(config);
    println!("Available tools ({}):", count);
    println!();

    for tool in &tools {
        let status = if disabled.contains(&tool.name) {
            " [disabled]"
        } else {
            ""
        };
        println!("  {:<30} {}{}", tool.name, tool.description, status);
    }

    println!();
    println!(
        "Use `operant tools enable <name>` or `operant tools disable <name>` to manage tools."
    );

    Ok(())
}

/// Enable one or more tools.
fn handle_enable(config: &AppConfig, names: Vec<String>) -> Result<()> {
    if names.is_empty() {
        println!("No tool names provided. Usage: operant tools enable <name> [<name>...]");
        return Ok(());
    }

    let mut disabled = load_disabled_tools(config);
    for name in &names {
        let was_disabled = disabled.remove(name);
        if was_disabled {
            println!("Tool '{}' enabled.", name);
        } else {
            println!("Tool '{}' was already enabled.", name);
        }
    }

    save_disabled_tools(config, &disabled)?;
    Ok(())
}

/// Disable one or more tools.
fn handle_disable(config: &AppConfig, names: Vec<String>) -> Result<()> {
    if names.is_empty() {
        println!("No tool names provided. Usage: operant tools disable <name> [<name>...]");
        return Ok(());
    }

    let mut disabled = load_disabled_tools(config);
    for name in &names {
        if disabled.contains(name) {
            println!("Tool '{}' was already disabled.", name);
        } else {
            disabled.insert(name.clone());
            println!("Tool '{}' disabled.", name);
        }
    }

    save_disabled_tools(config, &disabled)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tool state persistence (JSON file in Operant config directory)
// ---------------------------------------------------------------------------

/// Path to the tool state file.
fn tool_state_path(config: &AppConfig) -> PathBuf {
    // Use the database_path parent as the config directory (~/.operant/)
    let dir = config
        .database_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    dir.join("tool_state.json")
}

/// Load the set of disabled tool names from the state file.
fn load_disabled_tools(config: &AppConfig) -> std::collections::HashSet<String> {
    let path = tool_state_path(config);
    if !path.exists() {
        return std::collections::HashSet::new();
    }
    match std::fs::read_to_string(&path) {
        Ok(content) => {
            let state: serde_json::Result<ToolState> = serde_json::from_str(&content);
            match state {
                Ok(s) => s.disabled.into_iter().collect(),
                Err(_) => std::collections::HashSet::new(),
            }
        }
        Err(_) => std::collections::HashSet::new(),
    }
}

/// Save the set of disabled tool names to the state file.
fn save_disabled_tools(
    config: &AppConfig,
    disabled: &std::collections::HashSet<String>,
) -> Result<()> {
    let path = tool_state_path(config);
    let mut list: Vec<String> = disabled.iter().cloned().collect();
    list.sort();
    let state = ToolState { disabled: list };
    let content =
        serde_json::to_string_pretty(&state).context("Failed to serialise tool state as JSON")?;
    std::fs::write(&path, &content)
        .with_context(|| format!("Failed to write tool state to '{}'", path.display()))?;
    println!("  Tool state saved to {}", path.display());
    Ok(())
}

/// JSON schema for the tool state file.
#[derive(serde::Serialize, serde::Deserialize)]
struct ToolState {
    disabled: Vec<String>,
}
