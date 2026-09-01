//! G2 regression — ToolSeam same-name replace race.
//!
//! Before G2: `Harness::replace` staged a new provider behind id `ext` while
//! the old `ext` was still active. The old's effect-undo called
//! `unregister_tool("seam_echo")` which deleted the new tool's `Arc`.
//! The workaround was to use distinct names (`echo_v2`).
//!
//! After G2: the effect captures the installed `Arc` identity, and the undo
//! calls `unregister_tool_if` which compares `Arc::ptr_eq` before removing.
//! The new tool survives the old's unwind.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use async_trait::async_trait;
use operant_core::harness_adapters::ToolSeam;
use operant_core::schema::ToolSchema;
use operant_core::tools::{OperantTool, ToolContext, ToolRegistry};
use operant_harness::{ActivateCx, Harness, KernelOptions, Provider, ProviderSource, ProviderSpec};
use serde_json::json;

struct EchoTool {
    suffix: &'static str,
    calls: AtomicU32,
}
impl EchoTool {
    fn new(suffix: &'static str) -> Arc<Self> {
        Arc::new(Self {
            suffix,
            calls: AtomicU32::new(0),
        })
    }
}
#[async_trait]
impl OperantTool for EchoTool {
    fn name(&self) -> &str {
        "seam_echo"
    }
    fn description(&self) -> &str {
        "echoes args"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "seam_echo",
            self.description(),
            json!({"type":"object","properties":{"msg":{"type":"string"}}}),
        )
    }
    async fn execute(
        &self,
        _args: serde_json::Value,
        _context: ToolContext,
    ) -> operant_core::tools::ToolResult {
        self.calls.fetch_add(1, Ordering::SeqCst);
        operant_core::tools::ToolResult {
            tool_call_id: String::new(),
            name: self.name().to_string(),
            success: true,
            content: format!("echo{}", self.suffix),
            error: None,
        }
    }
}

struct EchoProvider {
    suffix: &'static str,
    tool: Arc<dyn OperantTool>,
}
impl ProviderSpec for EchoProvider {
    fn id(&self) -> &str {
        "ext"
    }
    fn source(&self) -> ProviderSource {
        ProviderSource::Native
    }
    fn provides(&self) -> &[operant_harness::Claim] {
        &[]
    }
    fn requires(&self) -> &[operant_harness::Claim] {
        &[]
    }
}
#[async_trait]
impl Provider for EchoProvider {
    fn spec(&self) -> &dyn ProviderSpec {
        self
    }
    async fn activate(
        &self,
        cx: &mut ActivateCx<'_>,
    ) -> Result<(), operant_harness::HarnessError> {
        cx.install_with("tool", "seam_echo", &self.tool).await?;
        Ok(())
    }
}

#[tokio::test]
async fn same_name_replace_preserves_new_tool() {
    let registry = ToolRegistry::new(std::time::Duration::from_secs(30));
    let mut harness = Harness::new(KernelOptions { audit: false });
    harness.add_seam(Arc::new(ToolSeam::new(registry.clone())));

    let first = Arc::new(EchoProvider {
        suffix: "_v1",
        tool: EchoTool::new("_v1"),
    });
    harness.mount(first).await.expect("mount v1");

    // Replace with a new provider behind the same id.
    let second = Arc::new(EchoProvider {
        suffix: "_v2",
        tool: EchoTool::new("_v2"),
    });
    harness.replace(second).await.expect("replace v2");

    // After replace, the registry should still contain seam_echo (the new
    // one) — the old's effect-undo must NOT have deleted the new Arc.
    let schemas = registry.get_schemas().await;
    assert_eq!(schemas.len(), 1, "new tool must survive old unwind");
    assert_eq!(schemas[0].name, "seam_echo");
}
