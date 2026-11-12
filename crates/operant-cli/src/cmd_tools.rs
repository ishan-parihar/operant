//! CLI subcommand for managing tools (`operant tools`).
//!
//! # Usage
//!
//! - `operant tools list`           — list all available tools
//! - `operant tools enable <name>`   — enable a tool
//! - `operant tools disable <name>`  — disable a tool
//!
//! Tool enable/disable state is persisted to a JSON file (`tool_state.json`)
//! in the Operant config directory, independent of the main TOML config — and
//! since iter-1124 the RUNTIME reads that file, so `operant tools disable`
//! actually removes the tool from the agent's toolset.

use std::sync::Arc;

use anyhow::Result;
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
    /// Defer one or more tools behind the `tool_search` bridge: their schemas
    /// stop riding every request, and the tool stays reachable through
    /// search → describe → call.
    Defer {
        /// Tool name(s) to defer
        names: Vec<String>,
    },
    /// Return one or more deferred tools to the per-request toolset.
    Eager {
        /// Tool name(s) to make eager again
        names: Vec<String>,
    },
}

/// Dispatch and execute a tools subcommand.
pub async fn handle_tools_command(config: &AppConfig, cmd: ToolsSubcommand) -> Result<()> {
    match cmd {
        ToolsSubcommand::List { platform } => handle_list(config, platform).await,
        ToolsSubcommand::Enable { names } => handle_enable(config, names),
        ToolsSubcommand::Disable { names } => handle_disable(config, names),
        ToolsSubcommand::Defer { names } => handle_defer(config, names),
        ToolsSubcommand::Eager { names } => handle_eager(config, names),
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

    // The FULL registry, not the filtered view: a disabled tool must stay
    // visible (with its status) — hiding it would leave the operator
    // unable to see what they turned off.
    let tools = registry.get_schemas_all().await;

    if tools.is_empty() {
        println!("No tools registered.");
        return Ok(());
    }

    // The effective disabled set = the TOML list UNION the CLI state file.
    let disabled = operant_core::tools::tool_state::disabled_tool_names(config);
    let enabled = tools.iter().filter(|t| !disabled.contains(&t.name)).count();
    println!("Tools ({} registered, {} enabled):", tools.len(), enabled);
    println!();

    let deferred = operant_core::tools::tool_state::load_deferred_tools(config);
    for tool in &tools {
        let status = if disabled.contains(&tool.name) {
            " [disabled]"
        } else if deferred.contains(&tool.name) {
            " [deferred: reachable via tool_search]"
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

    let mut disabled = operant_core::tools::tool_state::load_disabled_tools(config);
    for name in &names {
        let was_disabled = disabled.remove(name);
        if was_disabled {
            println!("Tool '{}' enabled.", name);
        } else {
            println!("Tool '{}' was already enabled.", name);
        }
        if config.tools.disabled_tools.iter().any(|t| t == name) {
            println!(
                "  note: '{}' is still disabled by [tools] disabled_tools in the config file.",
                name
            );
        }
    }

    operant_core::tools::tool_state::save_disabled_tools(config, &disabled)
        .map_err(anyhow::Error::msg)?;
    Ok(())
}

/// Defer tools behind the `tool_search` bridge.
///
/// A deferred tool is NOT disabled: it stays registered, stays callable, and
/// the bridge still reaches it — search for it, describe it, call it. What
/// stops is its schema riding every request, which is where the per-request
/// bill comes from (MEASURED 2026-11-12: 96 tools, 18 732 tokens per
/// request).
///
/// The offending state is refused loudly rather than silently ignored: a
/// DISABLED tool cannot be deferred (there is nothing to reach), and a tool
/// that is already deferred is reported as a no-op rather than written
/// twice.
fn handle_defer(config: &AppConfig, names: Vec<String>) -> Result<()> {
    if names.is_empty() {
        println!("No tool names provided. Usage: operant tools defer <name> [<name>...]");
        return Ok(());
    }

    let disabled = operant_core::tools::tool_state::load_disabled_tools(config);
    let mut deferred = operant_core::tools::tool_state::load_deferred_tools(config);
    for name in &names {
        if disabled.contains(name) {
            println!(
                "Tool '{}' is disabled — deferring a disabled tool would make it unreachable. \
                 Enable it first: operant tools enable {}",
                name, name
            );
            continue;
        }
        if deferred.contains(name) {
            println!("Tool '{}' was already deferred.", name);
            continue;
        }
        deferred.insert(name.clone());
        println!("Tool '{}' deferred — its schema no longer rides every request.", name);
    }

    operant_core::tools::tool_state::save_deferred_tools(config, &deferred)
        .map_err(anyhow::Error::msg)?;
    println!("Run `operant context budget` to measure the saving.");
    // WHEN it takes effect, stated here: a long-running host (the gateway)
    // builds its registry at start-up, so ITS toolset does not change until
    // it restarts. A lever whose effect is invisible at the place it was
    // applied looks like a lever that does not work — the same failure shape
    // as the CLI's disable that was a no-op before iter-1124.
    println!(
        "Takes effect for processes that START after this — a running gateway keeps its \
         toolset until it restarts (`operant service restart operant-gateway`)."
    );
    Ok(())
}

/// Return deferred tools to the per-request toolset.
fn handle_eager(config: &AppConfig, names: Vec<String>) -> Result<()> {
    if names.is_empty() {
        println!("No tool names provided. Usage: operant tools eager <name> [<name>...]");
        return Ok(());
    }

    let mut deferred = operant_core::tools::tool_state::load_deferred_tools(config);
    let mut changed = false;
    for name in &names {
        if deferred.remove(name) {
            changed = true;
            println!("Tool '{}' is eager again — its schema rides every request.", name);
        } else {
            println!("Tool '{}' was not deferred.", name);
        }
    }
    if changed {
        operant_core::tools::tool_state::save_deferred_tools(config, &deferred)
            .map_err(anyhow::Error::msg)?;
    }
    Ok(())
}

/// Disable one or more tools.
fn handle_disable(config: &AppConfig, names: Vec<String>) -> Result<()> {
    if names.is_empty() {
        println!("No tool names provided. Usage: operant tools disable <name> [<name>...]");
        return Ok(());
    }

    let mut disabled = operant_core::tools::tool_state::load_disabled_tools(config);
    for name in &names {
        if disabled.contains(name) {
            println!("Tool '{}' was already disabled.", name);
        } else {
            disabled.insert(name.clone());
            println!("Tool '{}' disabled.", name);
        }
    }

    operant_core::tools::tool_state::save_disabled_tools(config, &disabled)
        .map_err(anyhow::Error::msg)?;
    Ok(())
}
