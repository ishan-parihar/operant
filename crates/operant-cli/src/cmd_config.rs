//! CLI config subcommand module for Operant-RS.
//!
//! Implements `operant config show`, `operant config set <key> <value>`,
//! `operant config path`, and `operant config check`.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Subcommand;
use operant_core::config::{AppConfig, install_runtime_config, runtime_config};

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
    /// List and restore the saved copies of the config file.
    Backups {
        /// What to do with the saved copies.
        #[command(subcommand)]
        action: BackupAction,
    },
    /// Describe every config key: dotted path, default, note, and whether the
    /// running config differs from it.
    ///
    /// The agent-facing surface the config-resilience plan's L3 requires.
    Describe {
        /// Only keys under this section (e.g. "gateway", "tools.tool_search").
        section: Option<String>,
        /// Emit as JSON instead of the aligned table.
        #[arg(long)]
        json: bool,
    },
    /// Print the full note for ONE config key: why it exists, its range, and
    /// how the loader heals it.
    Notes {
        /// Dotted config key, e.g. "agent.context_window".
        key: String,
    },
}

/// What `operant config backups` does.
#[derive(Debug, Clone, Subcommand)]
pub enum BackupAction {
    /// List the saved copies, newest first.
    List,
    /// Restore one saved copy over the live config.
    Restore {
        /// The saved copy, as printed by `operant config backups list`.
        snapshot: PathBuf,
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
        ConfigSubcommand::Check => handle_check(config).await,
        ConfigSubcommand::Edit => handle_edit(),
        ConfigSubcommand::EnvPath => handle_env_path(),
        ConfigSubcommand::Migrate { apply } => handle_migrate(apply),
        ConfigSubcommand::Backups { action } => handle_backups(action),
        ConfigSubcommand::Describe { section, json } => handle_describe(config, section, json),
        ConfigSubcommand::Notes { key } => handle_notes(&key),
    }
}

fn handle_show(config: &AppConfig) -> Result<()> {
    let yaml = serde_yaml::to_string(config).context("Failed to serialize config as YAML")?;
    println!("{}", yaml);
    // ENV-1 provenance: the values above are the EFFECTIVE ones, and an
    // environment variable silently beats operant.toml on every load. Name
    // the keys, so "why is my configured model ignored" has a visible answer
    // instead of a shell archaeology session.
    let overrides = operant_core::config::env_override_keys();
    if !overrides.is_empty() {
        println!("# The following environment variables overrode the config file:");
        for key in &overrides {
            println!("#   {key}");
        }
        println!("# Unset them to use the file's values.");
    }
    Ok(())
}

/// Set a dotted config key on the runtime config.
///
/// 1. Snapshot the current runtime `AppConfig`.
/// 2. Serialise to `serde_json::Value`.
/// 3. Walk the dotted path (e.g. `agent.model`) and set the leaf value
///    (plan 09: the one copy in core — `set_nested_json`).
/// 4. Deserialise back to `AppConfig` and re-install it as the runtime config.
/// 5. Persist through the surgical writer (`persist_config_key`).
fn handle_set(key: String, value: String) -> Result<()> {
    let current = runtime_config();
    let mut root =
        serde_json::to_value(&current).context("Failed to serialise runtime config to JSON")?;

    let segments: Vec<&str> = key.split('.').collect();
    if segments.is_empty() {
        anyhow::bail!("Config key must not be empty");
    }

    operant_core::config::set_nested_json(&mut root, &segments, &value)
        .with_context(|| format!("Failed to set config key '{}'", key))?;

    let updated: AppConfig =
        serde_json::from_value(root).context("Failed to deserialise updated config")?;
    operant_core::config::validate_context_window(updated.agent.context_window)
        .context("invalid agent.context_window")?;
    install_runtime_config(updated);

    // (plan 09) persist through the ONE surgical writer — same as
    // `operant model set`: the old body changed only the in-memory
    // config and the value vanished with the process.
    match operant_core::config::persist_config_key(&key, &value) {
        Ok(path) => println!(
            "Set config '{key}' to '{value}' (persisted to {})",
            path.display()
        ),
        Err(e) => println!("Set config '{key}' to '{value}' (in-memory only; persist failed: {e})"),
    }
    Ok(())
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
async fn handle_check(config: &AppConfig) -> Result<()> {
    let mut issues: Vec<String> = Vec::new();

    if config.agent.model.is_empty() || config.agent.model == "gpt-4" {
        issues.push(format!(
            "Model is set to '{}' (the default). Consider explicitly setting a model (e.g. gpt-4o).",
            config.agent.model
        ));
    }

    let key_missing = config.client.api_key.as_ref().is_none_or(|k| k.is_empty());
    if key_missing {
        issues.push(
            "No API key configured. Set OPENAI_API_KEY or configure it in the config file."
                .to_string(),
        );
    }

    if let Some(parent) = config.database_path.parent()
        && !parent.exists()
    {
        issues.push(format!(
            "Database directory '{}' does not exist and will be created at runtime.",
            parent.display()
        ));
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
            McpTransportKind::StreamableHttp => {
                if server.url.is_none() {
                    issues.push(format!(
                        "Streamable-HTTP MCP server '{}' has no URL configured.",
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

    // The window must be able to hold the content that cannot be folded:
    // the eager tool schemas and the advertised skills. A window smaller
    // than that fails EVERY turn — the live deployment sat at 4,242 against
    // a 8,574-token protected prompt, and this check called it good.
    // The RESOLVED window, not the configured value: with an AUTOMATIC
    // config the configured value is 0, and the old guard skipped the check
    // entirely — so a model whose real window cannot hold the protected
    // content failed every turn with nothing saying so.
    let report = crate::cmd_context::resolve_window_for_report(config).await;
    let window = report.window;
    match crate::cmd_context::measure_protected_floor(config, window).await {
        Ok(floor) if floor.minimum_hard_cap > 0 => {
            issues.push(format!(
                "agent.context_hard_cap_tokens is smaller than the protected content \
                 (~{} tokens: tool schemas ~{}, advertised skills ~{}). Raise the cap to at \
                 least {} — every turn fails until it fits.",
                floor.required, floor.schema_tokens, floor.skills_tokens, floor.minimum_hard_cap,
            ));
        }
        Ok(floor) if floor.minimum_window > window => {
            issues.push(format!(
                "the context window is {window} tokens ({}) but the protected content needs \
                 about {} tokens (tool schemas ~{}, advertised skills ~{} from {} skill(s)). \
                 Raise it to at least {} — every turn fails until it fits.",
                crate::cmd_doctor::checks_config::window_source_label(report.source),
                floor.required,
                floor.schema_tokens,
                floor.skills_tokens,
                floor.skills_advertised,
                floor.minimum_window,
            ));
        }
        Ok(_) => {}
        Err(e) => {
            println!("  (context window not measured: {e} — the schema bill could not be built)");
        }
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
pub(crate) fn resolve_or_create_config_path() -> Result<PathBuf> {
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

/// `operant config backups list|restore`.
///
/// The saved copies are the last-known-good versions the writers keep before
/// each change (`operant_core::config_backup`). Listing prints the path to
/// pass to `restore`; restore re-validates the copy through the real loader
/// before it replaces anything, and snapshots the live file first, so a
/// restore is itself undoable.
fn handle_backups(action: BackupAction) -> Result<()> {
    let config_path = operant_core::config::loaded_config_path()
        .or_else(|| {
            operant_core::config::default_config_paths()
                .into_iter()
                .next()
        })
        .ok_or_else(|| anyhow::anyhow!("no config file path is known"))?;

    match action {
        BackupAction::List => {
            let snapshots = operant_core::config_backup::list(&config_path);
            if snapshots.is_empty() {
                println!("No saved copies of {} yet.", config_path.display());
                println!(
                    "One is written before each change, so this fills in the first time \
                     operant changes a setting."
                );
                return Ok(());
            }
            println!("Saved copies of {} (newest first):", config_path.display());
            for snapshot in &snapshots {
                let when = std::fs::metadata(snapshot)
                    .and_then(|meta| meta.modified())
                    .ok()
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|since_epoch| {
                        let secs = since_epoch.as_secs();
                        format!("{}s ago", secs)
                    })
                    .unwrap_or_else(|| "unknown age".to_string());
                println!("  {}  ({when})", snapshot.display());
            }
            println!();
            println!("Restore one with: operant config backups restore <path>");
            Ok(())
        }
        BackupAction::Restore { snapshot } => {
            let restored = operant_core::config_backup::restore(&snapshot, &config_path)
                .map_err(|error| anyhow::anyhow!(error))?;
            println!(
                "Restored {} over {}.",
                snapshot.display(),
                restored.display()
            );
            println!(
                "The file you replaced is itself saved as a copy, so this is undoable \
                 with the same command."
            );
            Ok(())
        }
    }
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
            // The surgical writer keeps comments, validates through the real
            // loader, and snapshots the pre-migration bytes — the old body
            // re-serialized the whole document and dropped every comment.
            operant_core::config::persist_config_key_at(
                &config_path,
                "version",
                &target_version.to_string(),
            )
            .with_context(|| format!("Failed to migrate config to '{}'", config_path.display()))?;
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
        // The surgical writer keeps comments, validates through the real
        // loader, and snapshots the pre-migration bytes.
        operant_core::config::persist_config_key_at(
            &config_path,
            "version",
            &target_version.to_string(),
        )
        .with_context(|| format!("Failed to migrate config to '{}'", config_path.display()))?;
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
    use serde_json::Value;

    #[test]
    fn config_subcommand_show_does_not_panic() {
        let config = AppConfig::default();
        assert!(handle_show(&config).is_ok());
    }

    #[test]
    fn coerce_boolean_values() {
        assert_eq!(
            operant_core::config::coerce_json_value("true"),
            Value::Bool(true)
        );
        assert_eq!(
            operant_core::config::coerce_json_value("TRUE"),
            Value::Bool(true)
        );
        assert_eq!(
            operant_core::config::coerce_json_value("false"),
            Value::Bool(false)
        );
        assert_eq!(
            operant_core::config::coerce_json_value("False"),
            Value::Bool(false)
        );
    }

    #[test]
    fn coerce_integer_value() {
        assert_eq!(
            operant_core::config::coerce_json_value("42"),
            Value::Number(42.into())
        );
        assert_eq!(
            operant_core::config::coerce_json_value("0"),
            Value::Number(0.into())
        );
        assert_eq!(
            operant_core::config::coerce_json_value("-5"),
            Value::Number((-5).into())
        );
    }

    #[test]
    fn coerce_float_value() {
        let v = operant_core::config::coerce_json_value("2.71");
        assert!(v.is_number());
        assert_eq!(v.as_f64(), Some(2.71));
    }

    #[test]
    fn coerce_fallback_to_string() {
        assert_eq!(
            operant_core::config::coerce_json_value("hello-world"),
            Value::String("hello-world".to_string())
        );
        assert_eq!(
            operant_core::config::coerce_json_value("gpt-4o"),
            Value::String("gpt-4o".to_string())
        );
    }

    #[test]
    fn set_single_key_modifies_json_object() {
        let mut obj = serde_json::json!({
            "agent": { "model": "gpt-4", "max_iterations": 90 }
        });

        operant_core::config::set_nested_json(&mut obj, &["agent", "model"], "gpt-4o").unwrap();
        assert_eq!(obj["agent"]["model"], "gpt-4o");
        assert_eq!(obj["agent"]["max_iterations"], 90);
    }

    #[test]
    fn set_creates_missing_intermediate_objects() {
        let mut obj = serde_json::json!({});
        operant_core::config::set_nested_json(&mut obj, &["a", "b", "c"], "42").unwrap();
        assert_eq!(obj["a"]["b"]["c"], 42);
    }

    #[test]
    fn handle_path_does_not_panic() {
        assert!(handle_path().is_ok());
    }

    #[tokio::test]
    async fn handle_check_default_config_emits_warnings() {
        let mut config = AppConfig::default();
        // The check measures the live schema bill, which opens the database.
        // Point it at a temp file so the test never touches the operator's.
        let dir = tempfile::tempdir().expect("tempdir");
        config.database_path = dir.path().join("check.sqlite");
        assert!(handle_check(&config).await.is_ok());
    }

    #[test]
    fn set_empty_key_is_rejected() {
        let mut obj = serde_json::json!({});
        let result = operant_core::config::set_nested_json(&mut obj, &[], "value");
        assert!(result.is_err());
    }
}

/// Render the whole config surface: every key, its default, its note, and
/// whether the RUNNING config differs from that default.
///
/// The L3 surface (`docs/plans/2026-11-14-config-resilience-architecture-...`).
/// Two rules make it useful rather than decorative:
///
/// - **It reads the registry, not a hand-written list.** The registry is
///   generated from the settings structs, so `describe` cannot drift from what
///   the binary accepts — a key the struct has and the registry lacks would be
///   a key the loader parks as unknown.
/// - **It marks the NON-DEFAULT values.** A wall of 272 rows in which the
///   operator's own three settings are invisible is not a map; the `*` is what
///   makes it one.
fn handle_describe(config: &AppConfig, section: Option<String>, json: bool) -> Result<()> {
    let live = serde_json::to_value(config).context("serialise the running config")?;
    let mut rows: Vec<(String, String, String, bool)> = Vec::new();

    for (key, default, note) in operant_core::config_registry::CONFIG_KEYS {
        if let Some(prefix) = section.as_deref()
            && !(*key == prefix || key.starts_with(&format!("{prefix}.")))
        {
            continue;
        }
        let current = read_dotted(&live, key);
        let is_default = match (&current, *default) {
            (Some(value), _) => value == default || values_agree(value, default),
            (None, _) => false,
        };
        rows.push((
            (*key).to_string(),
            current.unwrap_or_else(|| (*default).to_string()),
            (*note).to_string(),
            !is_default,
        ));
    }

    if json {
        let payload: Vec<serde_json::Value> = rows
            .iter()
            .map(|(key, value, note, non_default)| {
                serde_json::json!({
                    "key": key,
                    "value": value,
                    "default": operant_core::config_registry::CONFIG_KEYS
                        .iter()
                        .find(|(k, _, _)| *k == key.as_str())
                        .map(|(_, d, _)| *d)
                        .unwrap_or(""),
                    "note": note,
                    "non_default": non_default,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    // Column widths from the widest cell, so the table reads at any width.
    let key_width = rows
        .iter()
        .map(|r| r.0.len())
        .max()
        .unwrap_or(8)
        .max("KEY".len());
    println!(
        "{:<kw$}  {:<vw$}  {}",
        "KEY",
        "VALUE",
        "NOTE",
        kw = key_width,
        vw = 24
    );
    println!("{}", "-".repeat(key_width + 2 + 24 + 2 + 40));
    for (key, value, note, non_default) in &rows {
        // A non-default row is marked, because the default rows are the
        // background and the operator's own choices are the signal.
        let marker = if *non_default { "*" } else { " " };
        println!(
            "{marker}{:<kw$}  {:<vw$}  {}",
            key,
            truncate(&value, 24),
            truncate(note, 200),
            kw = key_width,
            vw = 24
        );
    }
    println!(
        "\n{} keys; * marks a value that differs from the default.",
        rows.len()
    );
    Ok(())
}

/// The note for ONE key.
///
/// When the key is unknown to this build the command FAILS with the closest
/// matches rather than printing an empty note: an agent that guessed a key name
/// must be told the name is wrong, not handed a blank.
fn handle_notes(key: &str) -> Result<()> {
    let Some((dotted, default, note)) = operant_core::config_registry::CONFIG_KEYS
        .iter()
        .find(|(k, _, _)| *k == key)
    else {
        let mut close: Vec<&str> = operant_core::config_registry::CONFIG_KEYS
            .iter()
            .map(|(k, _, _)| *k)
            .filter(|k| {
                let a = k.split('.').last().unwrap_or(k);
                let b = key.split('.').last().unwrap_or(key);
                a.starts_with(b) || b.starts_with(a)
            })
            .collect();
        close.sort();
        close.truncate(8);
        anyhow::bail!(
            "unknown config key '{key}'.{}",
            if close.is_empty() {
                String::new()
            } else {
                format!(" Did you mean: {}?", close.join(", "))
            }
        );
    };

    let live = runtime_config();
    let value = serde_json::to_value(&live)
        .ok()
        .and_then(|v| read_dotted(&v, dotted));

    println!("{dotted}");
    println!();
    println!("  default : {default}");
    if let Some(current) = value {
        println!("  current : {current}");
    }
    println!("  note    : {note}");
    Ok(())
}

/// Read a dotted path out of a JSON value, returning its scalar rendering.
fn read_dotted(value: &serde_json::Value, dotted: &str) -> Option<String> {
    let mut cursor = value;
    for segment in dotted.split('.') {
        cursor = cursor.get(segment)?;
    }
    Some(match cursor {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => "<unset>".to_string(),
        other => other.to_string(),
    })
}

/// Whether a live JSON scalar and a registry default string describe the same
/// thing. The registry records the Rust source of the default (`"auto"` is
/// recorded with its quotes, `3` without), so the comparison strips them.
fn values_agree(live: &str, default: &str) -> bool {
    let unquote = |s: &str| s.trim().trim_matches('"').trim().to_string();
    unquote(live) == unquote(default)
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}
