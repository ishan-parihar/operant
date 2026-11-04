//! CLI subcommand for managing plugins.
//!
//! Provides plugin lifecycle management:
//! - `operant plugins list` — list installed plugins
//! - `operant plugins new <name> --capability <c>` — scaffold a plugin project (creator mode)
//! - `operant plugins validate <path>` — validate a plugin directory's manifest
//! - `operant plugins install <identifier>` — install a plugin from git URL or path
//! - `operant plugins remove <name>` — remove/uninstall a plugin
//! - `operant plugins enable <name>` — enable a plugin
//! - `operant plugins disable <name>` — disable a plugin

use anyhow::{Context, Result};
use clap::Subcommand;
use operant_core::config::AppConfig;
use std::path::PathBuf;

/// Manage installed plugins.
#[derive(Debug, Clone, Subcommand)]
pub enum PluginsSubcommand {
    /// List all installed plugins
    List,
    /// Install a plugin from a git URL or local path
    Install {
        /// Git URL or local path to the plugin
        identifier: String,
        /// Force reinstallation if already installed
        #[arg(long)]
        force: bool,
        /// Enable the plugin after installation
        #[arg(long)]
        enable: bool,
    },
    /// Remove/uninstall a plugin
    Remove {
        /// Name of the plugin to remove
        name: String,
    },
    /// Enable a plugin
    Enable {
        /// Name of the plugin to enable
        name: String,
    },
    /// Disable a plugin
    Disable {
        /// Name of the plugin to disable
        name: String,
    },
    /// Toggle a plugin on/off
    Toggle {
        /// Plugin name to toggle
        name: String,
    },
    /// Scaffold a new plugin project in the current directory (creator mode)
    New {
        /// Plugin name (also the directory name created here)
        name: String,
        /// Capability to scaffold: tool | observer | memory | skill
        #[arg(long)]
        capability: String,
    },
    /// Validate a plugin directory's manifest (the installer's own validator)
    Validate {
        /// Path to the plugin directory
        path: String,
    },
    /// Per-plugin status: discovery view of this process (gap G9)
    Status,
}

pub async fn handle_plugins_command(config: &AppConfig, cmd: PluginsSubcommand) -> Result<()> {
    match cmd {
        PluginsSubcommand::List => list_plugins(config).await,
        PluginsSubcommand::Install {
            identifier,
            force,
            enable,
        } => install_plugin(config, &identifier, force, enable).await,
        PluginsSubcommand::Remove { name } => remove_plugin(config, &name).await,
        PluginsSubcommand::Enable { name } => enable_plugin(config, &name).await,
        PluginsSubcommand::Disable { name } => disable_plugin(config, &name).await,
        PluginsSubcommand::Toggle { name } => toggle_plugin(config, &name).await,
        PluginsSubcommand::New { name, capability } => new_plugin(&name, &capability),
        PluginsSubcommand::Validate { path } => validate_plugin_cmd(&path),
        PluginsSubcommand::Status => status_plugins(config).await,
    }
}

/// Resolve the plugins directory: `{data_dir}/operant/plugins`
pub fn plugins_dir(_config: &AppConfig) -> Result<PathBuf> {
    let dir = dirs::data_dir()
        .context("Failed to determine system data directory")?
        .join("operant")
        .join("plugins");
    Ok(dir)
}

/// List installed plugins with size and enabled/disabled status.
async fn list_plugins(config: &AppConfig) -> Result<()> {
    let dir = plugins_dir(config)?;

    if !dir.exists() {
        println!("No plugins installed.");
        return Ok(());
    }

    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .with_context(|| format!("Failed to read plugins directory '{}'", dir.display()))?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .collect();

    entries.sort_by_key(|e| e.file_name());

    if entries.is_empty() {
        println!("No plugins installed.");
        return Ok(());
    }

    println!("Installed plugins ({}):", entries.len());
    println!();

    for entry in &entries {
        let name = entry.file_name();
        let size = dir_size(&entry.path()).unwrap_or(0);
        let status = if config.plugins.is_enabled(&name.to_string_lossy()) {
            "enabled"
        } else {
            "disabled"
        };
        println!(
            "  {:<24} {:>8}K  {}",
            name.to_string_lossy(),
            size / 1024,
            status
        );
    }

    // Surface WASM plugin tools (feature `plugins-wasm`) so users can see
    // which plugins will contribute tools to the agent.
    #[cfg(feature = "plugins-wasm")]
    {
        let plugin_tools = crate::plugin_tools::list_plugin_tools(config);
        if !plugin_tools.is_empty() {
            println!();
            println!("WASM plugin tools ({}):", plugin_tools.len());
            for (name, count) in &plugin_tools {
                println!("  {:<24} {} tool(s)", name, count);
            }
        }
    }

    Ok(())
}

/// Install a plugin by cloning its git repository and validating the manifest.
async fn install_plugin(
    config: &AppConfig,
    identifier: &str,
    force: bool,
    enable: bool,
) -> Result<()> {
    let dir = plugins_dir(config)?;
    let name = crate::plugins_install::install_plugin(identifier, &dir, force).await?;
    println!("Plugin '{}' installed successfully.", name);
    if enable {
        let marker = dir.join(format!("{}.enabled", name));
        std::fs::write(&marker, "")
            .with_context(|| format!("Failed to enable plugin '{}'", name))?;
        println!("Plugin '{}' enabled.", name);
    }
    Ok(())
}

/// Remove an installed plugin directory and its enable marker.
async fn remove_plugin(config: &AppConfig, name: &str) -> Result<()> {
    let dir = plugins_dir(config)?.join(name);

    if !dir.exists() {
        anyhow::bail!("Plugin '{}' is not installed.", name);
    }

    std::fs::remove_dir_all(&dir)
        .with_context(|| format!("Failed to remove plugin '{}' at '{}'", name, dir.display()))?;

    // A removed plugin leaves no disable entry behind, so a reinstall starts enabled.
    if !config.plugins.is_enabled(name) {
        operant_core::config::set_plugin_disabled(name, false).map_err(anyhow::Error::msg)?;
    }

    println!("Plugin '{}' has been removed.", name);
    Ok(())
}

/// Enable a plugin by creating a `.enabled` marker file.
async fn enable_plugin(config: &AppConfig, name: &str) -> Result<()> {
    let dir = plugins_dir(config)?;
    let plugin_dir = dir.join(name);

    if !plugin_dir.exists() {
        anyhow::bail!("Plugin '{}' is not installed.", name);
    }

    operant_core::config::set_plugin_disabled(name, false).map_err(anyhow::Error::msg)?;

    println!(
        "Plugin '{}' has been enabled. It loads on the next start.",
        name
    );
    Ok(())
}

/// Disable a plugin by removing its `.enabled` marker file.
async fn disable_plugin(config: &AppConfig, name: &str) -> Result<()> {
    if !config.plugins.is_enabled(name) {
        println!("Plugin '{}' is not currently enabled.", name);
        return Ok(());
    }

    operant_core::config::set_plugin_disabled(name, true).map_err(anyhow::Error::msg)?;

    println!(
        "Plugin '{}' has been disabled. It stays out on the next start.",
        name
    );
    Ok(())
}

/// Toggle a plugin between enabled and disabled states.
async fn toggle_plugin(config: &AppConfig, name: &str) -> Result<()> {
    let plugin_dir = plugins_dir(config)?.join(name);

    if !plugin_dir.exists() {
        anyhow::bail!("Plugin '{}' is not installed.", name);
    }

    if config.plugins.is_enabled(name) {
        disable_plugin(config, name).await
    } else {
        enable_plugin(config, name).await
    }
}

/// Scaffold a plugin project (creator mode, gap G7): write the directory,
/// print the edit → build → install loop.
fn new_plugin(name: &str, capability: &str) -> Result<()> {
    let dest = std::env::current_dir()?.join(name);
    let dir = crate::plugins_install::scaffold_plugin(name, capability, &dest)?;
    println!("Scaffolded {capability} plugin at {}", dir.display());
    println!("Next steps (the export contract is in README.md):");
    if capability != "skill" {
        println!("  1. edit src/lib.rs");
        println!("  2. rustup target add wasm32-wasip1");
        println!("  3. cargo build --release --target wasm32-wasip1");
        println!("  4. cp target/wasm32-wasip1/release/{name}.wasm .");
    } else {
        println!("  1. edit skills/{name}/SKILL.md");
    }
    println!("  ?. operant plugins validate {}", dir.display());
    println!("  ?. operant plugins install file://{}", dir.display());
    Ok(())
}

/// Validate a plugin directory with the installer's own manifest validator.
fn validate_plugin_cmd(path: &str) -> Result<()> {
    let dir = PathBuf::from(path);
    crate::plugins_install::validate_plugin(&dir)?;
    println!("OK: {} has a valid manifest.toml.", dir.display());
    Ok(())
}

/// Per-plugin status (gap G9): what THIS process can honestly know —
/// discovery (manifest, capabilities, signature verdict, enabled state,
/// wasm module present + the tool name its `tool_metadata` export would
/// register). The running gateway's live registry state (registered tools,
/// active interceptors) lives in that process — say so instead of faking it.
async fn status_plugins(config: &AppConfig) -> Result<()> {
    #[cfg(feature = "plugins-wasm")]
    {
        let plugin_dirs = &config.plugins.plugin_dirs;
        if plugin_dirs.is_empty() {
            println!("No plugin dirs configured ([plugins] plugin_dirs).");
            return Ok(());
        }
        let parent = plugin_dirs[0]
            .parent()
            .context("plugins dir has no parent — cannot build plugin host")?;
        let signature_mode =
            operant_plugins::host::PluginHost::parse_signature_mode(&config.plugins.signature_mode);
        let host = operant_plugins::host::PluginHost::with_security(
            parent,
            signature_mode,
            config.plugins.trusted_publisher_keys.clone(),
        )?;
        let infos = host.list_plugins();
        if infos.is_empty() {
            println!("No plugins discovered in {}.", plugin_dirs[0].display());
            return Ok(());
        }
        println!(
            "Plugin status ({}) — discovery view (signature mode: {}):",
            infos.len(),
            config.plugins.signature_mode
        );
        println!();
        for info in &infos {
            let enabled = if config.plugins.is_enabled(&info.name) {
                "enabled"
            } else {
                "disabled"
            };
            let caps: Vec<String> = info
                .capabilities
                .iter()
                .map(|c| {
                    serde_json::to_value(c)
                        .ok()
                        .and_then(|v| v.as_str().map(str::to_string))
                        .unwrap_or_else(|| format!("{c:?}"))
                })
                .collect();
            let wasm = info
                .wasm_path
                .as_ref()
                .map(|p| {
                    if p.exists() {
                        format!(
                            "{}K",
                            std::fs::metadata(p).map(|m| m.len()).unwrap_or(0) / 1024
                        )
                    } else {
                        "MISSING".to_string()
                    }
                })
                .unwrap_or_else(|| "none (skill-only)".to_string());
            println!(
                "  {:<22} v{:<8} caps=[{}] {} wasm={}",
                info.name,
                info.version,
                caps.join(","),
                enabled,
                wasm
            );
            // The tool name a tool-capable module would register comes from
            // its `tool_metadata` export — one throwaway probe instance, the
            // same cost discovery already pays at boot.
            #[cfg(feature = "plugins-wasm")]
            use operant_plugins::PluginTool as _;
            #[cfg(feature = "plugins-wasm")]
            if info.loaded
                && info
                    .capabilities
                    .contains(&operant_plugins::PluginCapability::Tool)
                && let Some(p) = &info.wasm_path
            {
                let tool = operant_plugins::wasm_tool::WasmTool::from_wasm(
                    p.clone(),
                    Vec::new(),
                    info.name.clone(),
                    String::new(),
                );
                println!(
                    "      tool: {} (registers at boot; live re-registration under the watcher)",
                    tool.name()
                );
            }
        }
        println!();
        println!(
            "Live registry state (registered tools, active interceptors) is owned by the running gateway process — check its logs or the dashboard."
        );
        return Ok(());
    }
    #[cfg(not(feature = "plugins-wasm"))]
    {
        println!("plugins-wasm feature is not enabled in this build.");
        Ok(())
    }
}

/// Recursively compute the total size of a directory in bytes.
fn dir_size(path: &std::path::Path) -> std::io::Result<u64> {
    let mut total = 0u64;
    if path.is_dir() {
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            let ty = entry.file_type()?;
            if ty.is_dir() {
                total += dir_size(&entry.path())?;
            } else if ty.is_file() {
                total += entry.metadata()?.len();
            }
        }
    }
    Ok(total)
}
