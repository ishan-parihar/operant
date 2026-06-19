//! Plugin command system — discovery, registration, and dispatch.
//!
//! Provides a thread-safe [`PluginRegistry`] for plugins to register custom
//! Telegram gateway commands alongside the built-in `/` prefixed commands.
//!
//! # Architecture
//!
//! A global [`PluginRegistry`] holds all registered plugin commands, backed
//! by [`std::sync::RwLock`] for concurrent read access from the gateway's
//! command dispatch path.  Commands are function pointers
//! (`fn(&str) -> String`) — no async, no trait objects.
//!
//! # Discovery
//!
//! [`discover_plugins`] scans directories (e.g. `~/.hermes/plugins/`) for
//! `plugin.toml` / `plugin.yaml` manifest files, returning parsed metadata
//! without auto-registering.  Callers must explicitly invoke
//! [`register_plugin_command`] after discovery to make a command active.
//!
//! # Sample
//!
//! A built-in `/disk-cleanup` command is registered on construction to
//! demonstrate the system.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};
use tracing::{info, warn};

/// Handler signature for a plugin command — takes the argument string,
/// returns the response text.
pub type PluginHandler = fn(&str) -> String;

/// A registered plugin command with its handler and metadata.
#[derive(Debug, Clone)]
pub struct PluginCommand {
    /// Canonical command name (e.g. `"disk-cleanup"`).
    pub name: String,
    /// Short description shown in the bot's command menu.
    pub description: String,
    /// The handler function.
    pub handler: PluginHandler,
}

impl PluginCommand {
    /// Create a new plugin command.
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        handler: PluginHandler,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            handler,
        }
    }

    /// Invoke the handler with the given arguments and return the response.
    pub fn invoke(&self, args: &str) -> String {
        (self.handler)(args)
    }
}

/// Metadata discovered from a `plugin.toml` or `plugin.yaml` manifest.
///
/// Returned by [`discover_plugins`]; does **not** hold a handler — that
/// must be registered separately via [`register_plugin_command`].
#[derive(Debug, Clone)]
pub struct PluginManifest {
    /// Plugin name (used as the command name).
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Optional version string.
    pub version: Option<String>,
    /// Optional author string.
    pub author: Option<String>,
    /// Filesystem path to the plugin directory.
    pub path: PathBuf,
}

// ---------------------------------------------------------------------------
// Global registry
// ---------------------------------------------------------------------------

/// Thread-safe registry of plugin commands, shared via a global [`OnceLock`].
struct PluginRegistry {
    commands: RwLock<HashMap<String, PluginCommand>>,
    initialized: RwLock<bool>,
}

impl PluginRegistry {
    fn new() -> Self {
        Self {
            commands: RwLock::new(HashMap::new()),
            initialized: RwLock::new(false),
        }
    }

    fn register(&self, cmd: PluginCommand) {
        let mut map = self
            .commands
            .write()
            .expect("PluginRegistry commands lock poisoned");
        if map.contains_key(&cmd.name) {
            warn!(
                "Plugin command '/{}' is already registered; overwriting",
                cmd.name
            );
        }
        info!("Registered plugin command: /{}", cmd.name);
        map.insert(cmd.name.clone(), cmd);
    }

    fn handle(&self, name: &str, args: &str) -> Option<String> {
        let map = self
            .commands
            .read()
            .expect("PluginRegistry commands lock poisoned");
        map.get(name).map(|cmd| cmd.invoke(args))
    }

    fn list(&self) -> Vec<PluginCommand> {
        let map = self
            .commands
            .read()
            .expect("PluginRegistry commands lock poisoned");
        map.values().cloned().collect()
    }

    fn contains(&self, name: &str) -> bool {
        let map = self
            .commands
            .read()
            .expect("PluginRegistry commands lock poisoned");
        map.contains_key(name)
    }
}

static GLOBAL_REGISTRY: OnceLock<PluginRegistry> = OnceLock::new();

fn global_registry() -> &'static PluginRegistry {
    GLOBAL_REGISTRY.get_or_init(|| {
        let reg = PluginRegistry::new();

        // Register built-in sample plugin commands.
        reg.register(PluginCommand::new(
            "disk-cleanup",
            "Clean up disk space",
            disk_cleanup_handler,
        ));

        reg
    })
}

// ---------------------------------------------------------------------------
// Built-in sample handlers
// ---------------------------------------------------------------------------

/// Sample handler for `/disk-cleanup`.
fn disk_cleanup_handler(_args: &str) -> String {
    format!("🧹 Disk cleanup completed. Freed {} MB.", 96 + 47)
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Register a plugin command.
///
/// The command will appear in the gateway's command menu (via
/// [`get_plugin_commands`]) and be dispatchable through
/// [`handle_plugin_command`].
pub fn register_plugin_command(cmd: PluginCommand) {
    global_registry().register(cmd);
}

/// Dispatch a plugin command by name.
///
/// Returns `None` when no plugin command with that name is registered.
pub fn handle_plugin_command(name: &str, args: &str) -> Option<String> {
    global_registry().handle(name, args)
}

/// Return all registered plugin commands.
pub fn get_plugin_commands() -> Vec<PluginCommand> {
    global_registry().list()
}

/// Check whether a command name is registered as a plugin command.
pub fn is_plugin_command(name: &str) -> bool {
    global_registry().contains(name)
}

/// Resolve a raw text string to a plugin command name + arguments.
///
/// Expects text to start with `/`.  Extracts the first space-delimited token
/// as the command name (stripping the leading `/`), matches it
/// case-insensitively against registered plugin commands, and returns the
/// canonical name + remaining argument string.
///
/// Returns `None` when the text does not start with `/` or no plugin command
/// matches.
pub fn resolve_plugin_command(text: &str) -> Option<(String, &str)> {
    let trimmed = text.trim();

    if !trimmed.starts_with('/') {
        return None;
    }

    let after_slash = trimmed[1..].trim_start();

    let (cmd_token, args) = match after_slash.split_once(|c: char| c.is_ascii_whitespace()) {
        Some((cmd, rest)) => (cmd, rest.trim_start()),
        None => (after_slash, ""),
    };

    if cmd_token.is_empty() {
        return None;
    }

    // Match case-insensitively against registered plugin commands.
    let reg = global_registry();
    let map = reg
        .commands
        .read()
        .expect("PluginRegistry commands lock poisoned");

    // Find the canonical name by case-insensitive lookup.
    let canonical = map.keys().find(|k| k.eq_ignore_ascii_case(cmd_token))?;
    Some((canonical.clone(), args))
}

/// Scan directories for plugin manifests (`plugin.toml` or `plugin.yaml`)
/// and return parsed metadata.
///
/// This function does **not** auto-register commands — callers should
/// inspect the returned manifests and call [`register_plugin_command`] for
/// each command they want to activate.
///
/// Resilient: if a directory or manifest file cannot be read, a warning is
/// logged and iteration continues.
pub fn discover_plugins(plugin_dirs: &[PathBuf]) -> Vec<PluginManifest> {
    let mut discovered = Vec::new();
    for dir in plugin_dirs {
        if !dir.exists() {
            continue;
        }
        if !dir.is_dir() {
            warn!("Plugin path is not a directory: {}", dir.display());
            continue;
        }
        match std::fs::read_dir(dir) {
            Ok(entries) => {
                for entry in entries.flatten() {
                    let entry_path = entry.path();
                    if !entry_path.is_dir() {
                        continue;
                    }
                    // Try plugin.toml first, then plugin.yaml.
                    let manifest_path = entry_path.join("plugin.toml");
                    if !manifest_path.exists() {
                        let yaml_path = entry_path.join("plugin.yaml");
                        if !yaml_path.exists() {
                            continue;
                        }
                        if let Some(meta) = load_yaml_manifest(&yaml_path) {
                            info!(
                                "Discovered plugin: '{}' at {}",
                                meta.name,
                                yaml_path.display()
                            );
                            discovered.push(meta);
                        }
                    } else if let Some(meta) = load_toml_manifest(&manifest_path) {
                        info!(
                            "Discovered plugin: '{}' at {}",
                            meta.name,
                            manifest_path.display()
                        );
                        discovered.push(meta);
                    }
                }
            }
            Err(e) => {
                warn!("Failed to read plugin directory '{}': {}", dir.display(), e);
            }
        }
    }
    discovered
}

// ---------------------------------------------------------------------------
// Manifest parsing helpers
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Deserialize)]
struct PluginToml {
    plugin: PluginTomlMeta,
}

#[derive(Debug, serde::Deserialize)]
struct PluginTomlMeta {
    name: String,
    description: Option<String>,
    version: Option<String>,
    author: Option<String>,
}

fn load_toml_manifest(path: &Path) -> Option<PluginManifest> {
    let content = std::fs::read_to_string(path).ok()?;
    let parsed: PluginToml = toml::from_str(&content).ok()?;
    Some(PluginManifest {
        name: parsed.plugin.name,
        description: parsed.plugin.description.unwrap_or_default(),
        version: parsed.plugin.version,
        author: parsed.plugin.author,
        path: path.parent()?.to_path_buf(),
    })
}

fn load_yaml_manifest(path: &Path) -> Option<PluginManifest> {
    let content = std::fs::read_to_string(path).ok()?;
    let parsed: PluginToml = serde_yaml::from_str(&content).ok()?;
    Some(PluginManifest {
        name: parsed.plugin.name,
        description: parsed.plugin.description.unwrap_or_default(),
        version: parsed.plugin.version,
        author: parsed.plugin.author,
        path: path.parent()?.to_path_buf(),
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn test_handler(_args: &str) -> String {
        "test response".to_string()
    }

    fn another_handler(args: &str) -> String {
        format!("another: {}", args)
    }

    #[test]
    fn test_register_and_handle() {
        let cmd = PluginCommand::new("test-cmd", "A test command", test_handler);
        register_plugin_command(cmd);

        let result = handle_plugin_command("test-cmd", "hello");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "test response");
    }

    #[test]
    fn test_handle_unknown() {
        let result = handle_plugin_command("nonexistent", "");
        assert!(result.is_none());
    }

    #[test]
    fn test_get_plugin_commands_includes_disk_cleanup() {
        let cmds = get_plugin_commands();
        let disk = cmds.iter().find(|c| c.name == "disk-cleanup");
        assert!(disk.is_some(), "disk-cleanup should be pre-registered");
        assert_eq!(disk.unwrap().description, "Clean up disk space");
    }

    #[test]
    fn test_disk_cleanup_handler() {
        let result = handle_plugin_command("disk-cleanup", "");
        assert!(result.is_some());
        let resp = result.unwrap();
        assert!(resp.contains("Disk cleanup"));
        assert!(resp.contains("MB"));
    }

    #[test]
    fn test_is_plugin_command() {
        // Pre-registered
        assert!(is_plugin_command("disk-cleanup"));
        // Just registered
        register_plugin_command(PluginCommand::new("my-cmd", "My cmd", test_handler));
        assert!(is_plugin_command("my-cmd"));
        // Unknown
        assert!(!is_plugin_command("nope"));
    }

    #[test]
    fn test_resolve_plugin_command_with_slash() {
        let (name, args) = resolve_plugin_command("/disk-cleanup").unwrap();
        assert_eq!(name, "disk-cleanup");
        assert_eq!(args, "");
    }

    #[test]
    fn test_resolve_plugin_command_with_args() {
        register_plugin_command(PluginCommand::new("echo", "Echo", another_handler));
        let (name, args) = resolve_plugin_command("/echo hello world").unwrap();
        assert_eq!(name, "echo");
        assert_eq!(args, "hello world");
    }

    #[test]
    fn test_resolve_plugin_command_no_slash() {
        assert!(resolve_plugin_command("disk-cleanup").is_none());
    }

    #[test]
    fn test_resolve_plugin_command_case_insensitive() {
        let (name, _) = resolve_plugin_command("/DISK-CLEANUP").unwrap();
        assert_eq!(name, "disk-cleanup");
    }

    #[test]
    fn test_register_overwrite() {
        register_plugin_command(PluginCommand::new(
            "overwrite-test",
            "original",
            test_handler,
        ));
        register_plugin_command(PluginCommand::new("overwrite-test", "replacement", |_| {
            "replaced".to_string()
        }));
        let result = handle_plugin_command("overwrite-test", "");
        assert_eq!(result.unwrap(), "replaced");
    }

    #[test]
    fn test_discover_plugins_empty_dir() {
        let dir = std::env::temp_dir().join(format!("hermes_plugin_test_{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let manifests = discover_plugins(&[dir.clone()]);
        assert!(manifests.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_discover_plugins_skips_non_existent() {
        let dir = PathBuf::from("/tmp/hermes_plugin_nonexistent_should_not_exist_42");
        let manifests = discover_plugins(&[dir]);
        assert!(manifests.is_empty());
    }

    #[test]
    fn test_discover_plugins_toml() {
        let dir =
            std::env::temp_dir().join(format!("hermes_plugin_toml_test_{}", std::process::id()));
        let plugin_dir = dir.join("my-plugin");
        fs::create_dir_all(&plugin_dir).unwrap();
        fs::write(
            plugin_dir.join("plugin.toml"),
            r#"[plugin]
name = "my-plugin"
description = "My test plugin"
version = "1.0.0"
author = "Test Author"
"#,
        )
        .unwrap();

        let manifests = discover_plugins(&[dir.clone()]);
        assert_eq!(manifests.len(), 1);
        assert_eq!(manifests[0].name, "my-plugin");
        assert_eq!(manifests[0].description, "My test plugin");
        assert_eq!(manifests[0].version.as_deref(), Some("1.0.0"));
        assert_eq!(manifests[0].author.as_deref(), Some("Test Author"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_discover_plugins_yaml() {
        let dir =
            std::env::temp_dir().join(format!("hermes_plugin_yaml_test_{}", std::process::id()));
        let plugin_dir = dir.join("yaml-plugin");
        fs::create_dir_all(&plugin_dir).unwrap();
        fs::write(
            plugin_dir.join("plugin.yaml"),
            r#"plugin:
  name: "yaml-plugin"
  description: "YAML test"
  version: "0.1.0"
"#,
        )
        .unwrap();

        let manifests = discover_plugins(&[dir.clone()]);
        assert_eq!(manifests.len(), 1);
        assert_eq!(manifests[0].name, "yaml-plugin");
        assert_eq!(manifests[0].description, "YAML test");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_plugin_command_debug() {
        let cmd = PluginCommand::new("x", "desc", test_handler);
        let debug = format!("{:?}", cmd);
        assert!(debug.contains("x"));
        assert!(debug.contains("desc"));
    }
}
