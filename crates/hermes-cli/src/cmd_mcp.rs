//! CLI subcommand for managing MCP servers.
//!
//! Provides `hermes mcp list`, `hermes mcp add`, `hermes mcp remove`,
//! `hermes mcp test`, and `hermes mcp serve` subcommands.

use anyhow::{Context, Result};
use clap::Subcommand;
use hermes_core::config::{AppConfig, McpTransportKind};
use hermes_core::mcp::McpManager;

/// Manage MCP (Model Context Protocol) servers
#[derive(Debug, Clone, Subcommand)]
pub enum McpSubcommand {
    /// List configured MCP servers from the config file
    List,
    /// Add a new MCP server (tests the connection, then prints config instructions)
    Add {
        /// Name for the MCP server
        name: String,
        /// URL for HTTP MCP servers (required for HTTP transport)
        #[arg(long)]
        url: Option<String>,
        /// Command for stdio MCP servers (required for stdio transport)
        #[arg(long)]
        command: Option<String>,
        /// Arguments for stdio MCP server command (can be repeated)
        #[arg(long, num_args = 0..)]
        args: Vec<String>,
        /// Environment variables for stdio MCP server, e.g. --env KEY=VALUE (can be repeated)
        #[arg(long, num_args = 0..)]
        env: Vec<String>,
    },
    /// Remove an MCP server from the config file
    Remove {
        /// Name of the MCP server to remove
        name: String,
    },
    /// Test connection to a configured MCP server
    Test {
        /// Name of the MCP server to test
        name: String,
    },
    /// Run Hermes as an MCP server (stub)
    Serve,
}

/// Handle an MCP subcommand.
///
/// `config` provides the loaded configuration (read-only for add/remove —
/// those subcommands print file-editing instructions since config is file-based).
/// `mcp_manager` is used to test live connections.
pub async fn handle_mcp_command(
    config: &AppConfig,
    mcp_manager: &McpManager,
    cmd: McpSubcommand,
) -> Result<()> {
    match cmd {
        McpSubcommand::List => handle_list(config),
        McpSubcommand::Add {
            name,
            url,
            command,
            args,
            env,
        } => handle_add(config, mcp_manager, name, url, command, args, env).await,
        McpSubcommand::Remove { name } => handle_remove(name),
        McpSubcommand::Test { name } => handle_test(config, mcp_manager, name).await,
        McpSubcommand::Serve => handle_serve(),
    }
}

fn handle_list(config: &AppConfig) -> Result<()> {
    if config.mcp.servers.is_empty() {
        println!("No MCP servers configured.");
        println!();
        println!("  Add servers to your config file (e.g. hermes.toml):");
        println!();
        println!("  [mcp]");
        println!("  autoload = true");
        println!();
        println!("  [[mcp.servers]]");
        println!("  name = \"example\"");
        println!("  transport = \"http\"");
        println!("  url = \"http://localhost:8080/mcp\"");
        println!();
        return Ok(());
    }

    println!(
        "{:<20} {:<10} {:<8} {:<40}",
        "NAME", "TRANSPORT", "ENABLED", "URL / COMMAND"
    );
    println!("{:-<78}", "");

    for server in &config.mcp.servers {
        let transport = match server.transport {
            McpTransportKind::Http => "http",
            McpTransportKind::Stdio => "stdio",
        };
        let enabled = if server.enabled { "yes" } else { "no" };
        let endpoint = match server.transport {
            McpTransportKind::Http => server.url.as_deref().unwrap_or("-"),
            McpTransportKind::Stdio => server.command.as_deref().unwrap_or("-"),
        };

        println!("{:<20} {:<10} {:<8} {:<40}", server.name, transport, enabled, endpoint);
    }

    println!();
    println!("Autoload: {}", if config.mcp.autoload { "enabled" } else { "disabled" });
    println!(
        "Total servers: {}",
        config.mcp.servers.len()
    );

    Ok(())
}

async fn handle_add(
    config: &AppConfig,
    mcp_manager: &McpManager,
    name: String,
    url: Option<String>,
    command: Option<String>,
    args: Vec<String>,
    env: Vec<String>,
) -> Result<()> {
    if config.mcp.servers.iter().any(|s| s.name == name) {
        anyhow::bail!(
            "An MCP server named '{}' is already configured. Use `hermes mcp remove {}` first, or edit the config file directly.",
            name,
            name
        );
    }

    match (url.as_ref(), command.as_ref()) {
        (Some(_), Some(_)) => {
            anyhow::bail!("Provide either --url (for HTTP transport) or --command (for stdio transport), not both.");
        }
        (None, None) => {
            anyhow::bail!("Provide either --url <URL> (for HTTP transport) or --command <CMD> (for stdio transport).");
        }
        (Some(url_val), None) => {
            println!("Testing connection to HTTP MCP server '{}' at {} ...", name, url_val);

            mcp_manager
                .add_server(&name, url_val.clone(), None)
                .await
                .context(format!(
                    "Failed to connect to MCP server at {}",
                    url_val
                ))?;

            if let Some(transport) = mcp_manager.get(&name).await {
                let tools = transport.get_tools().await;
                println!(
                    "✓ Successfully connected to '{}' ({} tool(s) available)",
                    name,
                    tools.len()
                );
            }

            mcp_manager.remove_server(&name).await?;

            println!();
            println!("To add this server permanently, add the following to your config file:");
            println!();
            println!("  [[mcp.servers]]");
            println!("  name = \"{}\"", name);
            println!("  transport = \"http\"");
            println!("  url = \"{}\"", url_val);
            println!("  enabled = true");
        }
        (None, Some(cmd)) => {
            println!("Testing connection to stdio MCP server '{}': {}", name, cmd);

            let env_map: std::collections::HashMap<String, String> = env
                .iter()
                .map(|e| {
                    let mut parts = e.splitn(2, '=');
                    let key = parts.next().unwrap_or("").to_string();
                    let value = parts.next().unwrap_or("").to_string();
                    (key, value)
                })
                .collect();

            mcp_manager
                .add_stdio_server(&name, cmd.clone(), args.clone(), env_map.clone())
                .await
                .context(format!(
                    "Failed to connect to stdio MCP server '{}'",
                    cmd
                ))?;

            if let Some(transport) = mcp_manager.get(&name).await {
                let tools = transport.get_tools().await;
                println!(
                    "✓ Successfully connected to '{}' ({} tool(s) available)",
                    name,
                    tools.len()
                );
            }

            mcp_manager.remove_server(&name).await?;

            println!();
            println!("To add this server permanently, add the following to your config file:");
            println!();
            println!("  [[mcp.servers]]");
            println!("  name = \"{}\"", name);
            println!("  transport = \"stdio\"");
            println!("  command = \"{}\"", cmd);
            if !args.is_empty() {
                println!("  args = {:?}", args);
            }
            if !env_map.is_empty() {
                println!("  env = {{");
                for (k, v) in &env_map {
                    println!("    \"{}\" = \"{}\"", k, v);
                }
                println!("  }}");
            }
            println!("  enabled = true");
        }
    }

    Ok(())
}

fn handle_remove(name: String) -> Result<()> {
    println!(
        "To remove the MCP server '{}', delete or comment out its [[mcp.servers]] entry in your config file.",
        name
    );
    println!();
    println!("Look for a section like:");
    println!();
    println!("  [[mcp.servers]]");
    println!("  name = \"{}\"", name);
    println!("  ...");
    println!();
    println!("And remove it (or set `enabled = false` to disable without deleting).");

    Ok(())
}

async fn handle_test(
    config: &AppConfig,
    mcp_manager: &McpManager,
    name: String,
) -> Result<()> {
    let server = config
        .mcp
        .servers
        .iter()
        .find(|s| s.name == name)
        .with_context(|| format!("No MCP server named '{}' found in config", name))?;

    println!("Testing MCP server '{}' ...", name);
    println!("  Transport:  {:?}", server.transport);
    println!("  Enabled:    {}", if server.enabled { "yes" } else { "no" });

    match server.transport {
        McpTransportKind::Http => {
            let url = server
                .url
                .as_deref()
                .context("HTTP MCP server is missing a URL in config")?;
            println!("  URL:        {}", url);

            mcp_manager
                .add_server(&name, url.to_string(), server.auth_token.clone())
                .await
                .context(format!("Failed to connect to MCP server '{}'", name))?;
        }
        McpTransportKind::Stdio => {
            let command = server
                .command
                .as_deref()
                .context("Stdio MCP server is missing a command in config")?;
            println!("  Command:    {}", command);
            if !server.args.is_empty() {
                println!("  Args:       {:?}", server.args);
            }
            if !server.env.is_empty() {
                println!("  Env vars:   {} entries", server.env.len());
            }

            mcp_manager
                .add_stdio_server(
                    &name,
                    command.to_string(),
                    server.args.clone(),
                    server.env.clone(),
                )
                .await
                .context(format!("Failed to connect to MCP server '{}'", name))?;
        }
    }

    if let Some(transport) = mcp_manager.get(&name).await {
        let tools = transport.get_tools().await;
        if tools.is_empty() {
            println!();
            println!("✓ Connected, but no tools advertised.");
        } else {
            println!();
            println!("✓ Connected! Available tools:");
            for tool in &tools {
                let def = tool.definition();
                println!("  - {}: {}", def.name, def.description);
            }
        }
    }

    mcp_manager.remove_server(&name).await?;
    println!();
    println!("Connection closed.");

    Ok(())
}

fn handle_serve() -> Result<()> {
    println!("MCP server mode not yet implemented — use `hermes mcp serve` from the Python version");
    Ok(())
}
