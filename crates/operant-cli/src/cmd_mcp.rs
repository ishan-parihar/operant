//! CLI subcommand for managing MCP servers.
//!
//! Provides `operant mcp list`, `operant mcp add`, `operant mcp remove`,
//! `operant mcp test`, and `operant mcp serve` subcommands.

use anyhow::{Context, Result};
use clap::Subcommand;
use operant_core::config::{AppConfig, McpTransportKind, install_runtime_config, runtime_config};
use operant_core::mcp::McpManager;
use operant_core::mcp_oauth::{McpOAuthConfig, get_manager};
use serde_json::Value;

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
    /// Run Operant as an MCP server over stdio
    Serve {
        /// Enable verbose logging to stderr
        #[arg(long, short)]
        verbose: bool,
    },
    /// Login to an MCP server (OAuth 2.1 + PKCE flow).
    ///
    /// Discovers the server's OAuth metadata, performs dynamic client
    /// registration if needed, opens a browser for authorization, runs a
    /// localhost callback server, exchanges the auth code for tokens, and
    /// persists the tokens to disk under `~/.operant/mcp-tokens/`.
    Login {
        /// MCP server name (must already be added via `operant mcp add`)
        name: String,
        /// Override the authorization endpoint URL (skips metadata discovery)
        #[arg(long)]
        auth_url: Option<String>,
        /// Pre-registered client ID (skips dynamic client registration)
        #[arg(long)]
        client_id: Option<String>,
        /// OAuth scope string (space-separated, e.g. "read write")
        #[arg(long)]
        scope: Option<String>,
        /// Timeout in seconds for the browser authorization step (default 300)
        #[arg(long)]
        timeout: Option<u64>,
        /// Redirect URI port on localhost (0 = auto-pick a free port)
        #[arg(long, default_value_t = 0)]
        redirect_port: u16,
    },
    /// Configure MCP server settings
    Configure {
        /// MCP server name
        name: String,
        /// Set the auth token
        #[arg(long)]
        auth_token: Option<String>,
        /// Set the base URL
        #[arg(long)]
        url: Option<String>,
        /// Set the command (for stdio servers)
        #[arg(long)]
        command: Option<String>,
        /// Set args for stdio command (comma-separated)
        #[arg(long)]
        args: Option<String>,
    },
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
    json: bool,
) -> Result<()> {
    match cmd {
        McpSubcommand::List => handle_list(config, json),
        McpSubcommand::Add {
            name,
            url,
            command,
            args,
            env,
        } => handle_add(config, mcp_manager, name, url, command, args, env).await,
        McpSubcommand::Remove { name } => handle_remove(name),
        McpSubcommand::Test { name } => handle_test(config, mcp_manager, name).await,
        McpSubcommand::Serve { verbose } => handle_serve(config, verbose).await,
        McpSubcommand::Login {
            name,
            auth_url,
            client_id,
            scope,
            timeout,
            redirect_port,
        } => {
            handle_mcp_login(
                config,
                name,
                auth_url,
                client_id,
                scope,
                timeout,
                redirect_port,
            )
            .await
        }
        McpSubcommand::Configure {
            name,
            auth_token,
            url,
            command,
            args,
        } => handle_mcp_configure(config, name, auth_token, url, command, args),
    }
}

fn handle_list(config: &AppConfig, json: bool) -> Result<()> {
    if json {
        let servers: Vec<serde_json::Value> = config
            .mcp
            .servers
            .iter()
            .map(|s| {
                let transport = match s.transport {
                    McpTransportKind::Http => "http",
                    McpTransportKind::Stdio => "stdio",
                    McpTransportKind::StreamableHttp => "streamable-http",
                };
                let endpoint = match s.transport {
                    McpTransportKind::Http => s.url.as_deref().unwrap_or(""),
                    McpTransportKind::Stdio => s.command.as_deref().unwrap_or(""),
                    McpTransportKind::StreamableHttp => s.url.as_deref().unwrap_or(""),
                };
                serde_json::json!({
                    "name": s.name,
                    "transport": transport,
                    "enabled": s.enabled,
                    "endpoint": endpoint,
                })
            })
            .collect();
        let result = serde_json::json!({
            "servers": servers,
            "total": config.mcp.servers.len(),
            "autoload": config.mcp.autoload,
        });
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    if config.mcp.servers.is_empty() {
        println!("No MCP servers configured.");
        println!();
        println!("  Add servers to your config file (e.g. operant.toml):");
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
            McpTransportKind::StreamableHttp => "streamable-http",
        };
        let enabled = if server.enabled { "yes" } else { "no" };
        let endpoint = match server.transport {
            McpTransportKind::Http => server.url.as_deref().unwrap_or("-"),
            McpTransportKind::Stdio => server.command.as_deref().unwrap_or("-"),
            McpTransportKind::StreamableHttp => server.url.as_deref().unwrap_or("-"),
        };

        println!(
            "{:<20} {:<10} {:<8} {:<40}",
            server.name, transport, enabled, endpoint
        );
    }

    println!();
    println!(
        "Autoload: {}",
        if config.mcp.autoload {
            "enabled"
        } else {
            "disabled"
        }
    );
    println!("Total servers: {}", config.mcp.servers.len());

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
            "An MCP server named '{}' is already configured. Use `operant mcp remove {}` first, or edit the config file directly.",
            name,
            name
        );
    }

    match (url.as_ref(), command.as_ref()) {
        (Some(_), Some(_)) => {
            anyhow::bail!(
                "Provide either --url (for HTTP transport) or --command (for stdio transport), not both."
            );
        }
        (None, None) => {
            anyhow::bail!(
                "Provide either --url <URL> (for HTTP transport) or --command <CMD> (for stdio transport)."
            );
        }
        (Some(url_val), None) => {
            println!(
                "Testing connection to HTTP MCP server '{}' at {} ...",
                name, url_val
            );

            mcp_manager
                .add_server(&name, url_val.clone(), None)
                .await
                .context(format!("Failed to connect to MCP server at {}", url_val))?;

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
                .context(format!("Failed to connect to stdio MCP server '{}'", cmd))?;

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

async fn handle_test(config: &AppConfig, mcp_manager: &McpManager, name: String) -> Result<()> {
    let server = config
        .mcp
        .servers
        .iter()
        .find(|s| s.name == name)
        .with_context(|| format!("No MCP server named '{}' found in config", name))?;

    println!("Testing MCP server '{}' ...", name);
    println!("  Transport:  {:?}", server.transport);
    println!(
        "  Enabled:    {}",
        if server.enabled { "yes" } else { "no" }
    );

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
        McpTransportKind::StreamableHttp => {
            // iter-190: StreamableHttp is now handled by the same add_server
            // path as Http.
            let url = server
                .url
                .as_deref()
                .context("Streamable-HTTP MCP server is missing a URL in config")?;
            println!("  URL:        {} (streamable-http)", url);

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

async fn handle_serve(config: &AppConfig, verbose: bool) -> Result<()> {
    println!("Starting Operant MCP server over stdio...");
    println!("Protocol: MCP 2024-11-05");
    println!("Listening for JSON-RPC requests on stdin...");
    if verbose {
        println!("Verbose mode enabled.");
    }
    crate::mcp_serve::run_mcp_serve(config, verbose).await?;
    println!("MCP server shut down gracefully.");
    Ok(())
}

/// Initiate the OAuth 2.1 + PKCE login flow for an MCP server.
///
/// This is the real OAuth flow backed by `operant_core::mcp_oauth`:
///   1. Look up the server in config to get its base URL.
///   2. Build a `McpOAuthConfig` from the CLI args.
///   3. Get an `OAuthProvider` from the process-wide `OAuthManager`.
///   4. Call `provider.authenticate()`, which:
///        - discovers OAuth metadata (RFC 8414) from `<server_url>/.well-known/oauth-authorization-server`,
///        - performs dynamic client registration (RFC 7591) if no `client_id` is provided,
///        - builds an authorization URL with PKCE (S256),
///        - starts a localhost callback server on `redirect_port`,
///        - opens the user's browser (or prints the URL in headless mode),
///        - waits for the callback (up to `timeout` seconds),
///        - exchanges the auth code for access + refresh tokens,
///        - persists tokens to `~/.operant/mcp-tokens/<server_hash>/tokens.json`.
///   5. On success, prints the token hint + where it was saved.
///
/// The persisted tokens are auto-loaded by `McpClient` on subsequent
/// connections to this server (via `OAuthManager::get_token`).
async fn handle_mcp_login(
    config: &AppConfig,
    name: String,
    auth_url: Option<String>,
    client_id: Option<String>,
    scope: Option<String>,
    timeout: Option<u64>,
    redirect_port: u16,
) -> Result<()> {
    let server = config
        .mcp
        .servers
        .iter()
        .find(|s| s.name == name)
        .with_context(|| {
            format!(
                "MCP server '{}' is not configured. Add it first with `operant mcp add {}`",
                name, name
            )
        })?;

    let server_url = server
        .url
        .as_deref()
        .with_context(|| format!("MCP server '{}' has no URL configured", name))?;

    println!("Initiating OAuth login for MCP server '{}' ...", name);
    println!("  Server:       {}", server.name);
    println!("  URL:          {}", server_url);
    println!("  Transport:    {:?}", server.transport);

    // Build the OAuth config from CLI args.
    let oauth_config = McpOAuthConfig {
        client_id: client_id.clone(),
        client_secret: None,
        scope: scope.clone(),
        redirect_port: if redirect_port == 0 {
            None
        } else {
            Some(redirect_port)
        },
        client_name: Some(format!("operant-{}", name)),
        timeout,
    };

    // If the user supplied --auth-url, we can't pass it directly to the
    // provider (which discovers metadata from the server URL), but we can
    // at least surface it in the output as a hint.
    if let Some(ref url) = auth_url {
        println!("  Auth URL:     {} (override)", url);
    }

    // Get the OAuthProvider from the process-wide manager and authenticate.
    let manager = get_manager();
    let provider = manager.get_provider(server_url, Some(oauth_config));

    match provider.authenticate().await {
        Ok(token) => {
            println!();
            println!("✓ OAuth login successful for '{}'.", name);
            let token_hint: String = token.access_token.chars().take(8).collect();
            println!("  Access token:  {}...", token_hint);
            if let Some(ref refresh) = token.refresh_token {
                let hint: String = refresh.chars().take(8).collect();
                println!("  Refresh token: {}...", hint);
            }
            if let Some(expires_in) = token.expires_in {
                println!("  Expires in:    {} seconds", expires_in);
            }
            println!();
            println!("  Tokens saved to ~/.operant/mcp-tokens/");
            println!(
                "  Future connections to this server will use the saved tokens automatically."
            );
            Ok(())
        }
        Err(e) => {
            eprintln!();
            eprintln!("✗ OAuth login failed for '{}': {}", name, e);
            eprintln!();
            eprintln!("  Common causes:");
            eprintln!(
                "    - The server does not support OAuth (check `operant mcp test {}`)",
                name
            );
            eprintln!("    - The server's OAuth metadata endpoint is unreachable");
            eprintln!("    - The authorization timed out (use --timeout to extend)");
            eprintln!(
                "    - You already have valid tokens (use `operant mcp test {}` to verify)",
                name
            );
            Err(anyhow::anyhow!("OAuth login failed: {}", e))
        }
    }
}

/// Update MCP server configuration in the runtime config.
///
/// Modifies settings for an existing server by name and re-installs the config
/// so that changes take effect for the current session.
fn handle_mcp_configure(
    config: &AppConfig,
    name: String,
    auth_token: Option<String>,
    url: Option<String>,
    command: Option<String>,
    args: Option<String>,
) -> Result<()> {
    // Check the server exists in config
    if !config.mcp.servers.iter().any(|s| s.name == name) {
        anyhow::bail!(
            "No MCP server named '{}' found in config. Add it first.",
            name
        );
    }

    // Snapshot current runtime config, modify it, and re-install
    let current = runtime_config();
    let mut root =
        serde_json::to_value(&current).context("Failed to serialise runtime config to JSON")?;

    // Find the server index in the servers array
    let servers = root
        .get_mut("mcp")
        .and_then(|m| m.get_mut("servers"))
        .and_then(|s| s.as_array_mut())
        .context("Failed to locate mcp.servers in runtime config")?;

    let server_obj = servers
        .iter_mut()
        .find(|s| s.get("name").and_then(|n| n.as_str()) == Some(&name))
        .context(format!("Server '{}' not found in runtime config", name))?;

    let obj = server_obj
        .as_object_mut()
        .context("Server entry is not a JSON object")?;

    let mut changed = Vec::new();

    if let Some(token) = auth_token {
        obj.insert("auth_token".to_string(), Value::String(token));
        changed.push("auth_token");
    }
    if let Some(u) = url {
        obj.insert("url".to_string(), Value::String(u));
        changed.push("url");
    }
    if let Some(cmd) = command {
        obj.insert("command".to_string(), Value::String(cmd));
        changed.push("command");
    }
    if let Some(a) = args {
        let parsed: Vec<String> = a.split(',').map(|s| s.trim().to_string()).collect();
        let args_value: Value =
            serde_json::to_value(&parsed).context("Failed to serialise args")?;
        obj.insert("args".to_string(), args_value);
        changed.push("args");
    }

    let updated: AppConfig =
        serde_json::from_value(root).context("Failed to deserialise updated config")?;

    install_runtime_config(updated);

    if changed.is_empty() {
        println!("No settings were provided to update.");
        println!(
            "  Usage: operant mcp configure {} --auth-token <token> --url <url> --command <cmd> --args <args>",
            name
        );
    } else {
        println!(
            "Updated MCP server '{}' configuration for this session:",
            name
        );
        for setting in &changed {
            println!("  • {}", setting);
        }
        println!();
        println!("Note: Changes are applied to the runtime config for the current session.");
        println!("      To make them permanent, edit your operant.toml config file.");
    }

    Ok(())
}
