//! WASM plugin tool bridge.
//!
//! Connects the `operant-plugins` WASM host to the core `ToolRegistry`:
//! plugin manifests declaring the `tool` capability are discovered,
//! their `WasmTool`s are adapted to the core `OperantTool` trait, and
//! registered so the agent can call them exactly like built-in tools.
//!
//! This is the missing half of the plugin architecture: `operant-plugins`
//! already shipped a `PluginHost` (discovery, manifests, signature
//! verification) and `WasmTool`/`runtime` bridges, but nothing wired them
//! into the live agent. Feature-gated behind `plugins-wasm` (default-off;
//! `ci-all` enables it).

use anyhow::Context as _;
use operant_core::config::AppConfig;
use operant_core::schema::ToolSchema;
use operant_core::tools::{OperantTool, ToolContext, ToolRegistry};
use operant_plugins::PluginTool as _;
use serde_json::Value;

/// Adapter that presents a WASM-backed plugin tool through the core
/// `OperantTool` trait. `WasmTool` implements `operant_api::tool::Tool`;
/// this wrapper translates its schema/result shapes into the core types.
pub struct PluginToolAdapter {
    inner: operant_plugins::wasm_tool::WasmTool,
    toolset: String,
}

impl PluginToolAdapter {
    /// Wrap a plugin `WasmTool`, tagging it with the plugin's name so the
    /// registry can attribute it (and users can disable the whole plugin
    /// via `disabled_toolsets`).
    pub fn new(inner: operant_plugins::wasm_tool::WasmTool, plugin_name: &str) -> Self {
        Self {
            inner,
            toolset: format!("plugin:{}", plugin_name),
        }
    }
}

#[async_trait::async_trait]
impl OperantTool for PluginToolAdapter {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            self.inner.name(),
            self.inner.description(),
            self.inner.parameters_schema(),
        )
    }

    fn toolset(&self) -> &str {
        &self.toolset
    }

    fn is_available(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> operant_core::tools::ToolResult {
        match self.inner.execute(args).await {
            Ok(result) => {
                if result.success {
                    operant_core::tools::ToolResult::success_with_name(
                        self.inner.name(),
                        self.inner.name(),
                        result.output,
                    )
                } else {
                    operant_core::tools::ToolResult::error_with_name(
                        self.inner.name(),
                        self.inner.name(),
                        result
                            .error
                            .unwrap_or_else(|| "plugin execution failed".into()),
                    )
                }
            }
            Err(e) => operant_core::tools::ToolResult::error_with_name(
                self.inner.name(),
                self.inner.name(),
                format!("plugin execution error: {e:#}"),
            ),
        }
    }
}

/// Discover plugins and register every `tool`-capable plugin into the
/// registry. Best-effort: a broken plugin logs a warning and is skipped —
/// it must never fail the whole agent startup.
pub async fn register_plugin_tools(
    registry: &ToolRegistry,
    config: &AppConfig,
) -> anyhow::Result<()> {
    let plugin_dirs = &config.plugins.plugin_dirs;
    if plugin_dirs.is_empty() {
        return Ok(());
    }

    // The PluginHost discovers `<workspace>/plugins/*`; for a configured
    // plugin dir like `~/.operant/plugins` the workspace is its parent.
    let plugins_dir = &plugin_dirs[0];
    let parent = plugins_dir
        .parent()
        .context("plugins dir has no parent — cannot build plugin host")?;

    // The CLI-level PluginSettings carries only `plugin_dirs`; signature
    // policy lives in the runtime schema config ([plugins.security]), which
    // the runtime skill loader enforces. Default (disabled) verification is
    // consistent with the config default and keeps the CLI bridge simple.
    let host = match operant_plugins::host::PluginHost::new(parent) {
        Ok(host) => host,
        Err(e) => {
            tracing::warn!(error = %e, "plugin host failed to initialize; skipping plugin tools");
            return Ok(());
        }
    };

    for (manifest, wasm_path) in host.tool_plugin_details() {
        let tool = operant_plugins::wasm_tool::WasmTool::from_wasm(
            wasm_path.to_path_buf(),
            manifest.permissions.clone(),
            manifest.name.clone(),
            manifest.description.clone().unwrap_or_default(),
        );
        let tool_name = tool.name().to_string();
        let plugin_name = manifest.name.clone();
        match registry
            .register(PluginToolAdapter::new(tool, &plugin_name))
            .await
        {
            Ok(()) => {
                tracing::info!(
                    plugin = %plugin_name,
                    tool = %tool_name,
                    "Registered WASM plugin tool"
                );
            }
            Err(e) => {
                tracing::warn!(
                    plugin = %manifest.name,
                    error = %e,
                    "Failed to register plugin tool (non-fatal)"
                );
            }
        }
    }

    Ok(())
}

/// List tool-capable plugins (used by `operant plugins list` to surface
/// what will be loaded). Returns plugin names + tool count.
pub fn list_plugin_tools(config: &AppConfig) -> Vec<(String, usize)> {
    let plugin_dirs = &config.plugins.plugin_dirs;
    if plugin_dirs.is_empty() {
        return Vec::new();
    }
    let parent = match plugin_dirs[0].parent() {
        Some(p) => p.to_path_buf(),
        None => return Vec::new(),
    };
    let Ok(host) = operant_plugins::host::PluginHost::new(&parent) else {
        return Vec::new();
    };
    host.tool_plugin_details()
        .iter()
        .map(|(m, _)| (m.name.clone(), 1))
        .collect()
}
