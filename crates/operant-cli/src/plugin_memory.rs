//! WASM memory-plugin bridge.
//!
//! Consumes `operant-plugins`' `PluginCapability::Memory`: a WASM plugin
//! declaring the `memory` capability can back the core `MemoryProvider`
//! trait, exactly like hermes-agent's `plugins/memory/<name>` directory
//! plugins (mem0, honcho, hindsight, ...) plug into `MemoryManager`.
//!
//! ## WASM ABI
//!
//! A memory plugin exports the same hook surface as the core
//! `MemoryProvider` trait, over JSON strings:
//!
//! | Export           | Input                         | Output (JSON)                         |
//! |------------------|-------------------------------|---------------------------------------|
//! | `prefetch`       | `{"query": str}`              | `{"text": str}`                       |
//! | `sync_turn`      | `{"user": str, "assistant": str}` | `{"ok": true}`                    |
//! | `tool_schemas`   | `""`                          | `{"schemas": [ ... ]}`                |
//! | `handle_tool_call` | `{"name": str, "args": {...}}` | `{"output": str}`                  |
//!
//! Missing exports degrade gracefully (empty prefetch, no-op sync) so a
//! partial plugin never breaks the agent loop. Selected via
//! `memory.provider = "plugin:<name>"`; when multiple memory plugins are
//! installed only the named one activates (hermes `MemoryManager` parity —
//! one external provider at a time).

use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

use operant_core::memory_provider::MemoryProvider;

/// WASM-backed memory provider. Wraps a memory-capable plugin's exports.
pub struct PluginMemoryProvider {
    name: String,
    wasm_path: PathBuf,
    permissions: Vec<operant_plugins::PluginPermission>,
    /// Cached reachability — a plugin that fails to load or lacks a
    /// `prefetch` export reports `is_available() == false`.
    available: bool,
}

impl PluginMemoryProvider {
    /// Build a provider from a discovered memory plugin. The availability
    /// probe runs on a blocking thread so startup never stalls the async
    /// executor; a load failure yields an unavailable provider (never a
    /// hard error).
    pub async fn from_plugin(
        manifest_name: &str,
        wasm_path: PathBuf,
        permissions: Vec<operant_plugins::PluginPermission>,
    ) -> Self {
        let probe_path = wasm_path.clone();
        let probe_perms = permissions.clone();
        let available = tokio::task::spawn_blocking(move || {
            matches!(probe_plugin(&probe_path, &probe_perms), Ok(true))
        })
        .await
        .unwrap_or(false);
        Self {
            name: format!("plugin:{}", manifest_name),
            wasm_path,
            permissions,
            available,
        }
    }

    /// Call a plugin export with a JSON input string inside spawn_blocking
    /// (Extism `Plugin` is `!Send`, so it must be created per-call).
    async fn call_export(&self, export: &str, input: Value) -> Option<Value> {
        let wasm_path = self.wasm_path.clone();
        let permissions = self.permissions.clone();
        let export = export.to_string();
        let input = input.to_string();
        let result = tokio::task::spawn_blocking(move || {
            let mut plugin =
                operant_plugins::runtime::create_plugin(&wasm_path, &permissions).ok()?;
            let out: String = plugin.call::<&str, String>(&export, &input).ok()?;
            serde_json::from_str(&out).ok()
        })
        .await;
        match result {
            Ok(Some(v)) => Some(v),
            _ => None,
        }
    }
}

/// Probe whether the plugin exposes the `prefetch` export (the minimum
/// memory surface). Returns `Ok(true)` only when both the plugin loads and
/// the export exists.
fn probe_plugin(
    wasm_path: &std::path::Path,
    permissions: &[operant_plugins::PluginPermission],
) -> std::result::Result<bool, anyhow::Error> {
    let mut plugin = operant_plugins::runtime::create_plugin(wasm_path, permissions)?;
    // `tool_metadata` is exported by every well-formed plugin (also used by
    // WasmTool). A memory plugin additionally exports `prefetch`.
    let has_prefetch = plugin
        .call::<&str, String>("prefetch", r#"{"query":""}"#)
        .is_ok();
    Ok(has_prefetch)
}

#[async_trait]
impl MemoryProvider for PluginMemoryProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn is_available(&self) -> bool {
        self.available
    }

    async fn initialize(&self, _session_id: &str) -> operant_core::error::Result<()> {
        // Nothing to warm up — the WASM is loaded per-call.
        Ok(())
    }

    fn system_prompt_block(&self) -> String {
        format!(
            "Plugin memory active ({}). Recall with prefetch; save with sync_turn.",
            self.name
        )
    }

    async fn prefetch(&self, query: &str) -> String {
        let Some(value) = self
            .call_export("prefetch", serde_json::json!({ "query": query }))
            .await
        else {
            return String::new();
        };
        value
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    }

    async fn sync_turn(&self, user: &str, assistant: &str) -> operant_core::error::Result<()> {
        let _ = self
            .call_export(
                "sync_turn",
                serde_json::json!({ "user": user, "assistant": assistant }),
            )
            .await;
        Ok(())
    }

    fn tool_schemas(&self) -> Vec<Value> {
        // The plugin may expose its own tool schemas; surfaced as-is. The
        // blocking probe runs once at construction for availability, so we
        // only return schemas when the plugin is actually usable.
        if !self.available {
            return Vec::new();
        }
        // Best-effort synchronous probe of the tool_schemas export. This
        // runs per-call; the blocking create is cheap for a cold read.
        let wasm_path = self.wasm_path.clone();
        let permissions = self.permissions.clone();
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut plugin =
                operant_plugins::runtime::create_plugin(&wasm_path, &permissions).ok()?;
            let out: String = plugin.call::<&str, String>("tool_schemas", "").ok()?;
            let value: Value = serde_json::from_str(&out).ok()?;
            value.get("schemas").and_then(Value::as_array).cloned()
        })) {
            Ok(Some(schemas)) => schemas,
            _ => Vec::new(),
        }
    }

    async fn handle_tool_call(&self, name: &str, args: Value) -> String {
        let Some(value) = self
            .call_export(
                "handle_tool_call",
                serde_json::json!({ "name": name, "args": args }),
            )
            .await
        else {
            return serde_json::json!({ "error": format!("plugin {name} could not handle tool call") })
                .to_string();
        };
        value
            .get("output")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    }

    async fn shutdown(&self) {}
}

/// Discover the named memory plugin (`memory.provider = "plugin:<name>"`)
/// and return a provider for it. Returns `None` when no memory plugin
/// matches or the plugin system is disabled.
pub async fn build_plugin_memory_provider(
    provider_name: &str,
    config: &operant_core::config::AppConfig,
) -> Option<Arc<dyn MemoryProvider>> {
    let plugin_dirs = &config.plugins.plugin_dirs;
    if plugin_dirs.is_empty() {
        return None;
    }
    let plugin_name = provider_name.strip_prefix("plugin:")?;
    if plugin_name.is_empty() {
        return None;
    }

    let plugins_dir = &plugin_dirs[0];
    let parent = plugins_dir.parent()?;
    // Default (disabled) signature verification — the CLI-level
    // PluginSettings has no security section; runtime skill loading
    // enforces [plugins.security] for the schema config.
    let host = operant_plugins::host::PluginHost::new(parent).ok()?;

    for (manifest, wasm_path) in host.memory_plugin_details() {
        if manifest.name == plugin_name {
            let provider = PluginMemoryProvider::from_plugin(
                &manifest.name,
                wasm_path.to_path_buf(),
                manifest.permissions.clone(),
            )
            .await;
            tracing::info!(
                plugin = %manifest.name,
                available = provider.is_available(),
                "Activating WASM memory plugin"
            );
            return Some(Arc::new(provider));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn build_plugin_memory_provider_without_plugin_dir_returns_none() {
        // No configured plugin dirs → no plugin provider, regardless of name.
        let mut config = operant_core::config::AppConfig::default();
        config.plugins.plugin_dirs.clear();
        assert!(
            build_plugin_memory_provider("plugin:anything", &config)
                .await
                .is_none(),
            "empty plugin_dirs must yield None"
        );
    }

    #[test]
    fn provider_name_parsing() {
        // plugin:foo → strip prefix
        assert!(matches!(
            "plugin:my-mem".strip_prefix("plugin:"),
            Some("my-mem")
        ));
        assert_eq!("builtin".strip_prefix("plugin:"), None);
    }

    #[tokio::test]
    async fn from_plugin_without_wasm_file_is_unavailable() {
        // A plugin whose WASM file doesn't exist must yield an unavailable
        // provider — never a panic.
        let provider = PluginMemoryProvider::from_plugin(
            "ghost",
            PathBuf::from("/nonexistent/plugin.wasm"),
            vec![],
        )
        .await;
        assert!(!provider.is_available());
        assert_eq!(provider.name(), "plugin:ghost");
    }
}
