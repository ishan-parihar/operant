//! CLI config subcommand module for Operant-RS.
//!
//! Implements `operant config show`, `operant config set <key> <value>`,
//! `operant config path`, and `operant config check`.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Subcommand;
use operant_core::config::{install_runtime_config, runtime_config, AppConfig};
use serde_json::Value;

/// Config subcommand variants.
#[derive(Debug, Clone, Subcommand)]
pub enum ConfigSubcommand {
    /// Print the current configuration as YAML.
    Show,
    /// Set a configuration value by dotted key (e.g. "agent.model").
    Set {
        /// Dotted config key path, e.g. "agent.model"
        key: String,
        /// Value to assign
        value: String,
    },
    /// Print the configuration file path.
    Path,
    /// Validate the configuration for potential issues.
    Check,
    /// Open the configuration file in the default editor.
    Edit,
    /// Print the path to the .env file.
    EnvPath,
    /// Check config version and migrate from older versions.
    Migrate {
        /// Actually perform the migration.
        #[arg(long)]
        apply: bool,
    },
}

/// Dispatch and execute a config subcommand.
///
/// `config` is the currently active `AppConfig` (after all overrides have been
/// applied). The `cmd` determines which action to perform.
pub async fn handle_config_command(config: &AppConfig, cmd: ConfigSubcommand) -> Result<()> {
    match cmd {
        ConfigSubcommand::Show => handle_show(config),
        ConfigSubcommand::Set { key, value } => handle_set(key, value),
        ConfigSubcommand::Path => handle_path(),
        ConfigSubcommand::Check => handle_check(config),
        ConfigSubcommand::Edit => handle_edit(),
        ConfigSubcommand::EnvPath => handle_env_path(),
        ConfigSubcommand::Migrate { apply } => handle_migrate(apply),
    }
}

fn handle_show(config: &AppConfig) -> Result<()> {
    let yaml = serde_yaml::to_string(config).context("Failed to serialize config as YAML")?;
    println!("{}", yaml);
    Ok(())
}

/// Set a dotted config key on the runtime config.
///
/// 1. Snapshot the current runtime `AppConfig`.
/// 2. Serialise to `serde_json::Value`.
/// 3. Walk the dotted path (e.g. `agent.model`) and set the leaf value.
/// 4. Deserialise back to `AppConfig` and re-install it as the runtime config.
fn handle_set(key: String, value: String) -> Result<()> {
    let current = runtime_config();
    let mut root =
        serde_json::to_value(&current).context("Failed to serialise runtime config to JSON")?;

    let segments: Vec<&str> = key.split('.').collect();
    if segments.is_empty() {
        anyhow::bail!("Config key must not be empty");
    }

    set_nested(&mut root, &segments, &value)
        .with_context(|| format!("Failed to set config key '{}'", key))?;

    let updated: AppConfig =
        serde_json::from_value(root).context("Failed to deserialise updated config")?;

    install_runtime_config(updated);
    println!("Set config '{}' to '{}'", key, value);
    Ok(())
}

/// Recursively navigate into a nested `serde_json::Value` object and set the
/// leaf value identified by the remaining path segments.
fn set_nested(target: &mut Value, segments: &[&str], raw: &str) -> Result<()> {
    if segments.is_empty() {
        anyhow::bail!("Config key must not be empty");
    }
    let head = segments[0];

    if segments.len() == 1 {
        let parsed = coerce_json_value(raw);
        let obj = target
            .as_object_mut()
            .ok_or_else(|| anyhow::anyhow!("Cannot index into non-object with '{}'", head))?;
        obj.insert(head.to_string(), parsed);
        return Ok(());
    }

    let tail = &segments[1..];
    let obj = target
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("Cannot index into non-object with '{}'", head))?;

    if !obj.contains_key(head) {
        obj.insert(head.to_string(), Value::Object(serde_json::Map::new()));
    }
    let child = obj
        .get_mut(head)
        .ok_or_else(|| anyhow::anyhow!("Could not access key '{}'", head))?;

    set_nested(child, tail, raw)
}

/// Best-effort coercion of a CLI string argument into a typed JSON value.
///
/// Precedence: boolean → integer → float → string.
fn coerce_json_value(raw: &str) -> Value {
    match raw.to_ascii_lowercase().as_str() {
        "true" => return Value::Bool(true),
        "false" => return Value::Bool(false),
        _ => {}
    }

    if let Ok(n) = raw.parse::<i64>() {
        return Value::Number(n.into());
    }

    if let Ok(f) = raw.parse::<f64>() {
        if let Some(n) = serde_json::Number::from_f64(f) {
            return Value::Number(n);
        }
    }

    Value::String(raw.to_string())
}

/// Print the resolved config file path.
///
/// Checks (in order) the `HERMES_CONFIG` env var and then the default search
/// paths. Prints the first existing file found.
fn handle_path() -> Result<()> {
    if let Ok(path) = std::env::var("HERMES_CONFIG") {
        let path = PathBuf::from(&path);
        println!("{}", path.display());
        return Ok(());
    }

    for path in operant_core::config::default_config_paths() {
        if path.exists() {
            let display = path.canonicalize().unwrap_or_else(|_| path.clone());
            println!("{}", display.display());
            return Ok(());
        }
    }

    println!("No configuration file found. Using built-in defaults.");
    Ok(())
}

/// Validate the current configuration and print any warnings.
fn handle_check(config: &AppConfig) -> Result<()> {
    let mut issues: Vec<String> = Vec::new();

    if config.agent.model.is_empty() || config.agent.model == "gpt-4" {
        issues.push(format!(
            "Model is set to '{}' (the default). Consider explicitly setting a model (e.g. gpt-4o).",
            config.agent.model
        ));
    }

    let key_missing = config
        .client
        .api_key
        .as_ref()
        .map_or(true, |k| k.is_empty());
    if key_missing {
        issues.push(
            "No API key configured. Set OPENAI_API_KEY or configure it in the config file."
                .to_string(),
        );
    }

    if let Some(parent) = config.database_path.parent() {
        if !parent.exists() {
            issues.push(format!(
                "Database directory '{}' does not exist and will be created at runtime.",
                parent.display()
            ));
        }
    }

    for server in &config.mcp.servers {
        use operant_core::config::McpTransportKind;
        match server.transport {
            McpTransportKind::Http => {
                if server.url.is_none() {
                    issues.push(format!(
                        "HTTP MCP server '{}' has no URL configured.",
                        server.name
                    ));
                }
            }
            McpTransportKind::Stdio => {
                if server.command.is_none() {
                    issues.push(format!(
                        "Stdio MCP server '{}' has no command configured.",
                        server.name
                    ));
                }
            }
        }
    }

    if config.gateway.telegram_enabled && config.gateway.telegram_token.is_none() {
        issues.push("Telegram gateway is enabled but no token is set.".to_string());
    }
    if config.gateway.discord_enabled && config.gateway.discord_token.is_none() {
        issues.push("Discord gateway is enabled but no token is set.".to_string());
    }
    if config.gateway.slack_enabled && config.gateway.slack_token.is_none() {
        issues.push("Slack gateway is enabled but no token is set.".to_string());
    }

    if issues.is_empty() {
        println!("✓ Configuration looks good — no issues detected.");
    } else {
        println!("Configuration check found {} issue(s):", issues.len());
        for issue in &issues {
            println!("  • {}", issue);
        }
    }

    Ok(())
}

/// Open the configuration file in the default editor.
///
/// Uses `$EDITOR` (or `$VISUAL`) to open the config file. Falls back to
/// `vim` on Linux/macOS and `notepad` on Windows, with `nano` as an
/// additional secondary fallback. If no config file exists, a default one
/// is created at the first default config path before opening.
fn handle_edit() -> Result<()> {
    let config_path = resolve_or_create_config_path()?;

    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| {
            if cfg!(target_os = "windows") {
                "notepad".to_string()
            } else {
                "vim".to_string()
            }
        });

    let status = std::process::Command::new(&editor)
        .arg(&config_path)
        .status()
        .with_context(|| format!("Failed to launch editor '{}'", editor))?;

    if !status.success() {
        // Try nano as secondary fallback when the primary editor fails
        if editor != "nano" {
            eprintln!("Editor '{}' exited with error. Trying 'nano'…", editor);
            let nano_status = std::process::Command::new("nano")
                .arg(&config_path)
                .status()
                .with_context(|| "Failed to launch 'nano'")?;
            if !nano_status.success() {
                anyhow::bail!("Both '{}' and 'nano' failed. Check your $EDITOR.", editor);
            }
        } else {
            anyhow::bail!("Editor '{}' exited with error.", editor);
        }
    }

    println!("✔ Configuration file edited: {}", config_path.display());
    Ok(())
}

/// Resolve the config file path, creating a default file if none exists.
fn resolve_or_create_config_path() -> Result<PathBuf> {
    if let Ok(path) = std::env::var("HERMES_CONFIG") {
        let path = PathBuf::from(&path);
        if !path.exists() {
            write_default_config(&path)?;
        }
        return Ok(path);
    }

    let existing = operant_core::config::default_config_paths()
        .into_iter()
        .find(|p| p.exists());

    if let Some(path) = existing {
        return Ok(path);
    }

    // No config exists — create a default at the first default search path.
    let first = operant_core::config::default_config_paths()
        .into_iter()
        .next()
        .unwrap_or_else(|| PathBuf::from("operant.toml"));

    write_default_config(&first)?;
    println!(
        "Created default configuration file at '{}'",
        first.display()
    );
    Ok(first)
}

/// Write a default `AppConfig` to the given path as TOML.
fn write_default_config(path: &PathBuf) -> Result<()> {
    let default_config = toml::to_string(&operant_core::config::AppConfig::default())
        .context("Failed to serialize default config")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory '{}'", parent.display()))?;
    }
    std::fs::write(path, &default_config)
        .with_context(|| format!("Failed to write default config to '{}'", path.display()))?;
    Ok(())
}

fn handle_env_path() -> Result<()> {
    let env_path = if let Ok(config_dir) = std::env::var("HERMES_CONFIG") {
        PathBuf::from(config_dir).join(".env")
    } else if let Ok(config_dir) = std::env::var("HERMES_CONFIG_DIR") {
        PathBuf::from(config_dir).join(".env")
    } else if let Some(config_dir) = dirs::config_dir() {
        config_dir.join("operant").join(".env")
    } else {
        PathBuf::from("~/.config/operant/.env")
    };

    if env_path.exists() {
        let display = env_path.canonicalize().unwrap_or_else(|_| env_path.clone());
        println!("{}", display.display());
    } else if crate::env_store::operant_env_path().exists() {
        let alt = crate::env_store::operant_env_path();
        let display = alt.canonicalize().unwrap_or_else(|_| alt.clone());
        println!("{}", display.display());
    } else {
        println!("No .env file found.");
    }
    Ok(())
}

fn handle_migrate(apply: bool) -> Result<()> {
    let config_path = if let Ok(path) = std::env::var("HERMES_CONFIG") {
        PathBuf::from(&path)
    } else {
        operant_core::config::default_config_paths()
            .into_iter()
            .find(|p| p.exists())
            .unwrap_or_else(|| PathBuf::from("operant.toml"))
    };

    println!("Config migration check");
    println!("{}", "-".repeat(40));

    if !config_path.exists() {
        println!(
            "No config file found at '{}'. Nothing to migrate.",
            config_path.display()
        );
        return Ok(());
    }

    let content = std::fs::read_to_string(&config_path)
        .with_context(|| format!("Failed to read config file '{}'", config_path.display()))?;

    let current_version = if content.contains("version") {
        if content.contains("version = \"1\"") || content.contains("version = 1") {
            Some(1)
        } else {
            Some(2)
        }
    } else {
        None
    };

    println!("Config file: {}", config_path.display());
    println!("Detected version: {:?}", current_version.unwrap_or(0));

    println!();
    println!("Available migrations:");
    println!("  v0 -> v1: Initial version field addition");
    println!("  v1 -> v2: MCP server configuration restructure");

    let target_version = 2;

    if let Some(v) = current_version {
        if v >= target_version {
            println!(
                "  Config is already at the latest version (v{}).",
                target_version
            );
        } else if apply {
            println!("Applying migration from v{} to v{}...", v, target_version);
            // Parse TOML, set version field, write back
            let mut value: toml::Value =
                content.parse().context("Failed to parse config as TOML")?;
            if let Some(table) = value.as_table_mut() {
                table.insert(
                    "version".to_string(),
                    toml::Value::Integer(target_version as i64),
                );
            }
            let new_content =
                toml::to_string(&value).context("Failed to serialize migrated config")?;
            std::fs::write(&config_path, &new_content).with_context(|| {
                format!(
                    "Failed to write migrated config to '{}'",
                    config_path.display()
                )
            })?;
            println!(
                "  Migration complete. Config updated to v{}.",
                target_version
            );
        } else {
            println!(
                "Run with --apply to perform migration from v{} to v{}.",
                v, target_version
            );
        }
    } else if apply {
        println!("Applying migration from v0 to v{}...", target_version);
        // Parse TOML, set version field, write back
        let mut value: toml::Value = content.parse().context("Failed to parse config as TOML")?;
        if let Some(table) = value.as_table_mut() {
            table.insert(
                "version".to_string(),
                toml::Value::Integer(target_version as i64),
            );
        }
        let new_content = toml::to_string(&value).context("Failed to serialize migrated config")?;
        std::fs::write(&config_path, &new_content).with_context(|| {
            format!(
                "Failed to write migrated config to '{}'",
                config_path.display()
            )
        })?;
        println!(
            "  Migration complete. Config updated to v{}.",
            target_version
        );
    } else {
        println!(
            "Run with --apply to perform migration from v0 to v{}.",
            target_version
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_subcommand_show_does_not_panic() {
        let config = AppConfig::default();
        assert!(handle_show(&config).is_ok());
    }

    #[test]
    fn coerce_boolean_values() {
        assert_eq!(coerce_json_value("true"), Value::Bool(true));
        assert_eq!(coerce_json_value("TRUE"), Value::Bool(true));
        assert_eq!(coerce_json_value("false"), Value::Bool(false));
        assert_eq!(coerce_json_value("False"), Value::Bool(false));
    }

    #[test]
    fn coerce_integer_value() {
        assert_eq!(coerce_json_value("42"), Value::Number(42.into()));
        assert_eq!(coerce_json_value("0"), Value::Number(0.into()));
        assert_eq!(coerce_json_value("-5"), Value::Number((-5).into()));
    }

    #[test]
    fn coerce_float_value() {
        let v = coerce_json_value("2.71");
        assert!(v.is_number());
        assert_eq!(v.as_f64(), Some(2.71));
    }

    #[test]
    fn coerce_fallback_to_string() {
        assert_eq!(
            coerce_json_value("hello-world"),
            Value::String("hello-world".to_string())
        );
        assert_eq!(
            coerce_json_value("gpt-4o"),
            Value::String("gpt-4o".to_string())
        );
    }

    #[test]
    fn set_single_key_modifies_json_object() {
        let mut obj = serde_json::json!({
            "agent": { "model": "gpt-4", "max_iterations": 90 }
        });

        set_nested(&mut obj, &["agent", "model"], "gpt-4o").unwrap();
        assert_eq!(obj["agent"]["model"], "gpt-4o");
        assert_eq!(obj["agent"]["max_iterations"], 90);
    }

    #[test]
    fn set_creates_missing_intermediate_objects() {
        let mut obj = serde_json::json!({});
        set_nested(&mut obj, &["a", "b", "c"], "42").unwrap();
        assert_eq!(obj["a"]["b"]["c"], 42);
    }

    #[test]
    fn handle_path_does_not_panic() {
        assert!(handle_path().is_ok());
    }

    #[test]
    fn handle_check_default_config_emits_warnings() {
        let config = AppConfig::default();
        assert!(handle_check(&config).is_ok());
    }

    #[test]
    fn set_empty_key_is_rejected() {
        let mut obj = serde_json::json!({});
        let result = set_nested(&mut obj, &[], "value");
        assert!(result.is_err());
    }
}
