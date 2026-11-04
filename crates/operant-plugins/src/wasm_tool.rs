//! Bridge between WASM plugins and the Tool trait.

use crate::PluginPermission;
use crate::runtime;
use anyhow::Context as _;
use async_trait::async_trait;
use operant_api::tool::{Tool, ToolResult};
use serde_json::Value;
use std::path::PathBuf;

/// A tool backed by a WASM plugin function.
pub struct WasmTool {
    name: String,
    description: String,
    parameters_schema: Value,
    /// Resident instance pool for the module — one compile per process,
    /// reused across calls (iter-1725; the per-call instantiate this
    /// replaces is red-team audit F1).
    pool: runtime::PluginPool,
}

impl WasmTool {
    pub fn new(
        name: String,
        description: String,
        parameters_schema: Value,
        wasm_path: PathBuf,
        permissions: Vec<PluginPermission>,
    ) -> Self {
        Self {
            name,
            description,
            parameters_schema,
            pool: runtime::PluginPool::new(&wasm_path, &permissions),
        }
    }

    /// Create a WasmTool by loading metadata from the plugin's `tool_metadata` export.
    /// Falls back to manifest-supplied values if the export is missing.
    pub fn from_wasm(
        wasm_path: PathBuf,
        permissions: Vec<PluginPermission>,
        fallback_name: String,
        fallback_description: String,
    ) -> Self {
        // Try to load metadata from the WASM module itself.
        let (name, description, schema) = match runtime::create_plugin(&wasm_path, &permissions) {
            Ok(mut plugin) => match runtime::call_tool_metadata(&mut plugin) {
                Ok(meta) => (meta.name, meta.description, meta.parameters_schema),
                Err(e) => {
                    tracing::debug!(
                        "plugin at {} has no tool_metadata export ({e}), using fallback",
                        wasm_path.display()
                    );
                    (
                        fallback_name.clone(),
                        fallback_description.clone(),
                        default_schema(),
                    )
                }
            },
            Err(e) => {
                tracing::warn!(
                    "failed to load WASM plugin at {} for metadata: {e}",
                    wasm_path.display()
                );
                (
                    fallback_name.clone(),
                    fallback_description.clone(),
                    default_schema(),
                )
            }
        };

        Self {
            name,
            description,
            parameters_schema: schema,
            pool: runtime::PluginPool::new(&wasm_path, &permissions),
        }
    }
}

/// The JSON Schema returned when a plugin lacks a `tool_metadata` export or fails
/// to load at discovery time. Single source of truth so the fallback shape stays
/// consistent across code paths.
fn default_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "input": {
                "type": "string",
                "description": "Input for the plugin"
            }
        },
        "required": ["input"]
    })
}

#[async_trait]
impl Tool for WasmTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters_schema(&self) -> Value {
        self.parameters_schema.clone()
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let pool = self.pool.clone();
        let args_json = serde_json::to_vec(&args)?;

        // The pool checkout + extism call are blocking; they run on the
        // blocking pool (same boundary the per-call path used).
        let checked_out = tokio::task::spawn_blocking(move || pool.execute(&args_json))
            .await
            .context("plugin execution task failed")?;
        match checked_out? {
            Some(result) => Ok(result),
            None => Err(anyhow::anyhow!(
                "plugin instance pool busy — no free instance within the wait ceiling"
            )),
        }
    }
}
