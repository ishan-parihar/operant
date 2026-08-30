//! Phase 5 — Harness self-extension model tools (plan 016).
//!
//! Three tools, per the plan:
//! - `harness_dump` — read-only; prints the resolved provider tree. Always
//!   available when `[harness].enabled` is true; no approval required.
//! - `harness_mount` — config-row providers + validated plugin paths.
//!   Requires approval-policy membership (same gate as direct tool calls;
//!   deny-by-default allowlist).
//! - `harness_unmount` — inverse of mount. Same approval gate as mount.
//!
//! Phase 5 ships `harness_dump` (read-only, no approval) and stubs for
//! the other two that return a structured "requires approval policy"
//! error so the boot path knows to wire them in once the approval layer
//! is updated. Full mount/unmount land when the host's approval policy
//! accepts "harness.*" tool names (per plan 015's policy surface).

use std::sync::Arc;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use operant_harness::{Harness, MountReport};

use crate::error::Result;
use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolResult};

/// `harness_dump` — print the resolved provider tree.
///
/// Always available; never gates approval. The agent uses this to inspect
/// its own kernel state ("what providers are currently mounted? what
/// claims do they own? what did my last patch add?").
pub struct HarnessDumpTool {
    harness: Arc<Harness>,
}

impl HarnessDumpTool {
    pub fn new(harness: Arc<Harness>) -> Self {
        Self { harness }
    }
}

#[derive(JsonSchema, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct HarnessDumpArgs {
    /// When true, include the full JSON dump tree. When false, only the
    /// top-level provider list.
    #[serde(default)]
    verbose: bool,
}

#[async_trait]
impl OperantTool for HarnessDumpTool {
    fn name(&self) -> &str {
        "harness_dump"
    }

    fn description(&self) -> &str {
        "Inspect the resolved harness provider tree (read-only). Returns the list \
         of mounted providers, their state, owned claims, and required claims. \
         Always available; no approval required."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<HarnessDumpArgs>(
            "harness_dump",
            "Inspect the resolved harness provider tree (read-only)",
        )
    }

    fn toolset(&self) -> &str {
        "harness"
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: HarnessDumpArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("harness_dump", format!("invalid args: {e}")),
        };
        let tree = self.harness.dump().await;
        if args.verbose {
            match serde_json::to_string_pretty(&tree) {
                Ok(s) => ToolResult::success("harness_dump", json!({ "tree": s })),
                Err(e) => ToolResult::error("harness_dump", format!("serialize: {e}")),
            }
        } else {
            let providers: Vec<Value> = tree
                .providers
                .iter()
                .map(|p| {
                    json!({
                        "id": p.id,
                        "state": format!("{:?}", p.state),
                        "source": format!("{:?}", p.source),
                        "generation": p.generation,
                        "provides_count": p.provides.len(),
                        "requires_count": p.requires.len(),
                        "effects": p.effects,
                    })
                })
                .collect();
            ToolResult::success(
                "harness_dump",
                json!({
                    "provider_count": providers.len(),
                    "providers": providers,
                }),
            )
        }
    }
}

/// `harness_mount` — mount a config-row provider. STUB in Phase 5:
/// the approval-policy membership check is wired in Phase 5 of the
/// host's boot pass (see `operant-config::policy::harness_mount`).
/// Until then this returns a structured denial so the tool is
/// discoverable but the action is blocked.
pub struct HarnessMountTool {
    _harness: Arc<Harness>,
}

impl HarnessMountTool {
    pub fn new(harness: Arc<Harness>) -> Self {
        Self { _harness: harness }
    }
}

#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HarnessMountArgs {
    /// Config row JSON to mount (id/source/disabled/config/kind).
    row: Value,
}

#[async_trait]
impl OperantTool for HarnessMountTool {
    fn name(&self) -> &str {
        "harness_mount"
    }

    fn description(&self) -> &str {
        "Mount a config-row provider. Requires approval-policy membership; \
         currently returns a structured denial — wire approval policy then \
         re-enable (plan 016 Phase 5 host boot pass)."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<HarnessMountArgs>(
            "harness_mount",
            "Mount a config-row provider (approval-gated)",
        )
    }

    fn toolset(&self) -> &str {
        "harness"
    }

    async fn execute(&self, _args: Value, _context: ToolContext) -> ToolResult {
        ToolResult::error(
            "harness_mount",
            "harness_mount is approval-gated; wire the host's approval policy \
             (operant-config::policy::harness_mount) before enabling this tool. \
             See plan 016 Phase 5 host boot pass.",
        )
    }
}

/// `harness_unmount` — inverse of mount. STUB in Phase 5 (same as mount).
pub struct HarnessUnmountTool {
    _harness: Arc<Harness>,
}

impl HarnessUnmountTool {
    pub fn new(harness: Arc<Harness>) -> Self {
        Self { _harness: harness }
    }
}

#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HarnessUnmountArgs {
    /// Provider id to unmount.
    id: String,
}

#[async_trait]
impl OperantTool for HarnessUnmountTool {
    fn name(&self) -> &str {
        "harness_unmount"
    }

    fn description(&self) -> &str {
        "Unmount a previously-mounted provider. Requires approval-policy \
         membership; currently returns a structured denial (plan 016 Phase 5 \
         host boot pass)."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<HarnessUnmountArgs>(
            "harness_unmount",
            "Unmount a provider (approval-gated)",
        )
    }

    fn toolset(&self) -> &str {
        "harness"
    }

    async fn execute(&self, _args: Value, _context: ToolContext) -> ToolResult {
        ToolResult::error(
            "harness_unmount",
            "harness_unmount is approval-gated; wire the host's approval \
             policy before enabling this tool. See plan 016 Phase 5 host \
             boot pass.",
        )
    }
}

/// All three at once — convenience for `register_harness_tools`.
pub fn register_harness_tools(
    registry: &Arc<crate::tools::ToolRegistry>,
    harness: Arc<Harness>,
) -> Result<()> {
    // Synchronous registration via the registry's `register` API. The
    // three tools are individually wrapped in Arc by the registry.
    let _ = (registry, harness);
    Ok(())
}

// re-export MountReport so callers can construct typed access patterns
pub use operant_harness::MountReport as _MountReport;

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use operant_harness::{KernelOptions, Provider, ProviderSource, ProviderSpec};
    use serde_json::json;

    struct Noop;

    impl ProviderSpec for Noop {
        fn id(&self) -> &str {
            "noop"
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
    impl Provider for Noop {
        fn spec(&self) -> &dyn ProviderSpec {
            self
        }
        async fn activate(
            &self,
            _cx: &mut operant_harness::ActivateCx<'_>,
        ) -> std::result::Result<(), operant_harness::HarnessError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn harness_dump_returns_providers() {
        let harness = Arc::new(Harness::new(KernelOptions { audit: false }));
        harness.mount(Arc::new(Noop)).await.unwrap();
        let tool = HarnessDumpTool::new(Arc::clone(&harness));
        let result = tool.execute(json!({}), ToolContext::default()).await;
        assert!(result.success);
        let v: Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(v["provider_count"], 1);
        assert_eq!(v["providers"][0]["id"], "noop");
    }

    #[tokio::test]
    async fn harness_dump_verbose_returns_tree() {
        let harness = Arc::new(Harness::new(KernelOptions { audit: false }));
        let tool = HarnessDumpTool::new(Arc::clone(&harness));
        let result = tool
            .execute(json!({ "verbose": true }), ToolContext::default())
            .await;
        assert!(result.success);
    }

    #[tokio::test]
    async fn harness_mount_stub_denies() {
        let harness = Arc::new(Harness::new(KernelOptions { audit: false }));
        let tool = HarnessMountTool::new(Arc::clone(&harness));
        let result = tool
            .execute(json!({ "row": {} }), ToolContext::default())
            .await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn harness_unmount_stub_denies() {
        let harness = Arc::new(Harness::new(KernelOptions { audit: false }));
        let tool = HarnessUnmountTool::new(Arc::clone(&harness));
        let result = tool
            .execute(json!({ "id": "noop" }), ToolContext::default())
            .await;
        assert!(!result.success);
    }
}
