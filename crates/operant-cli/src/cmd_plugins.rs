//! CLI subcommand for managing plugins.
//!
//! Provides plugin lifecycle management:
//! - `operant plugins list` — list installed plugins
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
    }
}

/// Resolve the plugins directory: `{data_dir}/operant/plugins`
fn plugins_dir(_config: &AppConfig) -> Result<PathBuf> {
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
        .filter(|e| e.file_type().map_or(false, |t| t.is_dir()))
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
        let marker = dir.join(format!("{}.enabled", name.to_string_lossy()));
        let status = if marker.exists() {
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

    // Clean up enable marker if present
    let marker = plugins_dir(config)?.join(format!("{}.enabled", name));
    if marker.exists() {
        std::fs::remove_file(&marker).ok();
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

    let marker = dir.join(format!("{}.enabled", name));
    std::fs::write(&marker, "").with_context(|| format!("Failed to enable plugin '{}'", name))?;

    println!("Plugin '{}' has been enabled.", name);
    Ok(())
}

/// Disable a plugin by removing its `.enabled` marker file.
async fn disable_plugin(config: &AppConfig, name: &str) -> Result<()> {
    let dir = plugins_dir(config)?;
    let marker = dir.join(format!("{}.enabled", name));

    if !marker.exists() {
        println!("Plugin '{}' is not currently enabled.", name);
        return Ok(());
    }

    std::fs::remove_file(&marker)
        .with_context(|| format!("Failed to disable plugin '{}'", name))?;

    println!("Plugin '{}' has been disabled.", name);
    Ok(())
}

/// Toggle a plugin between enabled and disabled states.
async fn toggle_plugin(config: &AppConfig, name: &str) -> Result<()> {
    let dir = plugins_dir(config)?;
    let plugin_dir = dir.join(name);
    let marker = dir.join(format!("{}.enabled", name));

    if !plugin_dir.exists() {
        anyhow::bail!("Plugin '{}' is not installed.", name);
    }

    if marker.exists() {
        std::fs::remove_file(&marker)
            .with_context(|| format!("Failed to disable plugin '{}'", name))?;
        println!("Plugin '{}' has been disabled.", name);
    } else {
        std::fs::write(&marker, "")
            .with_context(|| format!("Failed to enable plugin '{}'", name))?;
        println!("Plugin '{}' has been enabled.", name);
    }

    Ok(())
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
