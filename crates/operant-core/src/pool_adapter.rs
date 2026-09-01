//! G6 — host-side `PoolBundleTool` for read-only pool.bundle providers.
//!
//! A `pool.bundle` row's activation routes through the `tool` seam. The
//! host's `ToolSeam` (in `harness_adapters.rs`) looks for an
//! `Arc<dyn OperantTool>` payload matching the tool name. This module
//! provides the typed implementation: a `PoolBundleTool` whose execute
//! returns a structured read-only response derived from the row config.
//!
//! Read-only is enforced by the verb policy in `compile_pool` (only
//! read-only verbs are accepted at compile time). At execution time the
//! tool always succeeds with a payload describing the bundle — the
//! `write_approval` gate in the agent loop is the secondary defense.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolResult};

/// A read-only tool that materializes a `pool.bundle` row.
pub struct PoolBundleTool {
    /// Tool name (matches the seam key — the `id()` of the row).
    name: String,
    /// Path to the underlying bundle (e.g. file path, KB URI).
    path: String,
    /// Whether the bundle is read-only (always true for v1).
    read_only: bool,
}

impl PoolBundleTool {
    pub fn new(name: String, config: &Value) -> Self {
        Self {
            name,
            path: config
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            read_only: config
                .get("read_only")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
        }
    }
}

#[async_trait]
impl OperantTool for PoolBundleTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        "Read-only pool.bundle adapter — returns the bundle's path and \
         read-only marker. Hosts can swap in a richer implementation that \
         actually reads from `config.path` (file, KB, ...)."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            &self.name,
            self.description(),
            serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false,
            }),
        )
    }

    fn toolset(&self) -> &str {
        "pool"
    }

    async fn execute(&self, _args: Value, _context: ToolContext) -> ToolResult {
        ToolResult::success(
            &self.name,
            serde_json::json!({
                "kind": "pool.bundle",
                "name": self.name,
                "path": self.path,
                "read_only": self.read_only,
                "hint": "host can replace this tool with a real adapter that reads from `path`",
            }),
        )
    }
}

/// Build a `PoolBundleTool` for the given row config. Returns None if
/// the config is missing required fields.
pub fn build_pool_bundle_tool(name: &str, config: &Value) -> Option<Arc<dyn OperantTool>> {
    if config.get("path").and_then(|v| v.as_str()).is_none() {
        return None;
    }
    Some(Arc::new(PoolBundleTool::new(name.to_string(), config)))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn pool_bundle_tool_reports_read_only() {
        let tool = PoolBundleTool::new(
            "pool.research.kb".to_string(),
            &json!({ "path": "/tmp/kb", "read_only": true }),
        );
        let result = tool.execute(json!({}), ToolContext::default()).await;
        assert!(result.success);
        let v: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(v["kind"], "pool.bundle");
        assert_eq!(v["read_only"], true);
        assert_eq!(v["path"], "/tmp/kb");
    }

    #[test]
    fn build_pool_bundle_tool_requires_path() {
        assert!(build_pool_bundle_tool("x", &json!({})).is_none());
        assert!(build_pool_bundle_tool("x", &json!({ "path": "/y" })).is_some());
    }
}
