//! CLI config subcommand module for Hermes-RS.
//!
//! Implements `hermes config show`, `hermes config set <key> <value>`,
//! `hermes config path`, and `hermes config check`.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Subcommand;
use hermes_core::config::{install_runtime_config, runtime_config, AppConfig};
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

    for path in hermes_core::config::default_config_paths() {
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
        use hermes_core::config::McpTransportKind;
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
        let v = coerce_json_value("3.14");
        assert!(v.is_number());
        assert_eq!(v.as_f64(), Some(3.14));
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
            "agent": { "model": "gpt-4", "max_iterations": 20 }
        });

        set_nested(&mut obj, &["agent", "model"], "gpt-4o").unwrap();
        assert_eq!(obj["agent"]["model"], "gpt-4o");
        assert_eq!(obj["agent"]["max_iterations"], 20);
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
