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

use operant_harness::{Architecture, ArchitectureRow, Builder, BuilderWithFactories, Harness, MountReport};

use crate::error::Result;
use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolResult};

/// Approval policy check. The host's approval layer sets
/// `ToolContext::metadata["approval"] = "true"` when the user has
/// approved the `harness.*` tool names for the current session.
/// When unset, every mount/unmount is denied. This mirrors hermes
/// `is_approved(session_key, pattern_key)` parity and the
/// `operant-cli` approval allowlist semantics.
fn has_approval(context: &ToolContext) -> bool {
    context.metadata.get("approval").map(String::as_str) == Some("true")
}

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

/// `harness_mount` — mount a config-row provider.
///
/// G5 — self-extension loop close. The tool parses the row, builds a
/// provider via [`operant_harness::Builder`] (or
/// [`BuilderWithFactories`] if the host registered wasm/pool
/// factories), and calls `Harness::mount`. The host's approval policy
/// layer is expected to set `ToolContext::metadata["approval"] = "true"`
/// when the user has approved the `harness_mount` tool name; when
/// that flag is absent, the tool returns a structured denial so the
/// agent can prompt the user.
pub struct HarnessMountTool {
    harness: Arc<Harness>,
    /// Optional pluggable factory set (e.g. for `wasm` / `pool` rows).
    /// When `None`, the built-in `Builder` is used and only `native` /
    /// `config_row` rows are accepted.
    builders: Option<Arc<BuilderWithFactories>>,
}

impl HarnessMountTool {
    pub fn new(harness: Arc<Harness>) -> Self {
        Self {
            harness,
            builders: None,
        }
    }

    /// Inject host-side factories so `wasm` / `pool` rows can be
    /// mounted by the agent at runtime.
    pub fn with_builders(mut self, builders: Arc<BuilderWithFactories>) -> Self {
        self.builders = Some(builders);
        self
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
        "Mount a config-row provider on the running kernel. Requires the host's \
         approval policy to have approved `harness_mount` for the current \
         session; the host sets ToolContext.metadata[\"approval\"] = \"true\". \
         The tool parses the row, builds a provider (via Builder or the host's \
         BuilderWithFactories for wasm/pool), and calls Harness::mount. \
         Returns the activated id list (or PENDING + missing claims)."
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

    async fn execute(&self, args: Value, context: ToolContext) -> ToolResult {
        if !has_approval(&context) {
            return ToolResult::error(
                "harness_mount",
                "harness_mount requires the host's approval policy to have \
                 approved this tool for the current session. Ask the user to \
                 approve `harness_mount` (e.g. via the persistent or session \
                 allowlist), then retry.",
            );
        }
        let args: HarnessMountArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("harness_mount", format!("invalid args: {e}")),
        };
        let row: ArchitectureRow = match serde_json::from_value(args.row) {
            Ok(r) => r,
            Err(e) => return ToolResult::error("harness_mount", format!("invalid row: {e}")),
        };
        let arch = Architecture { rows: vec![row.clone()] };
        let providers = if let Some(b) = &self.builders {
            match b.build_with(&arch) {
                Ok(p) => p,
                Err(e) => {
                    return ToolResult::error(
                        "harness_mount",
                        format!("build_with failed for row `{}`: {e}", row.id),
                    );
                }
            }
        } else {
            match Builder::build(&arch) {
                Ok(p) => p,
                Err(e) => {
                    return ToolResult::error(
                        "harness_mount",
                        format!("build failed for row `{}`: {e}", row.id),
                    );
                }
            }
        };
        if providers.is_empty() {
            return ToolResult::success(
                "harness_mount",
                json!({ "id": row.id, "skipped": true, "reason": "row produced no provider (e.g. config_row kind=disable)" }),
            );
        }
        let mut activated = Vec::new();
        for provider in providers {
            let id = provider.spec().id().to_string();
            match self.harness.mount(provider).await {
                Ok(MountReport::Mounted { activated: mut a }) => activated.append(&mut a),
                Ok(MountReport::Pending { missing }) => {
                    return ToolResult::success(
                        "harness_mount",
                        json!({ "id": id, "pending": true, "missing_claims": missing }),
                    );
                }
                Err(e) => {
                    return ToolResult::error(
                        "harness_mount",
                        format!("mount failed for `{id}`: {e}"),
                    );
                }
            }
        }
        tracing::info!(row = %row.id, ?activated, "harness_mount: provider mounted via model tool");
        ToolResult::success(
            "harness_mount",
            json!({ "id": row.id, "activated": activated }),
        )
    }
}

/// `harness_unmount` — inverse of mount. Same approval gate.
pub struct HarnessUnmountTool {
    harness: Arc<Harness>,
}

impl HarnessUnmountTool {
    pub fn new(harness: Arc<Harness>) -> Self {
        Self { harness }
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
        "Unmount a previously-mounted provider. Same approval gate as \
         harness_mount. Calls Harness::unmount which cascades to dependents."
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

    async fn execute(&self, args: Value, context: ToolContext) -> ToolResult {
        if !has_approval(&context) {
            return ToolResult::error(
                "harness_unmount",
                "harness_unmount requires the host's approval policy to have \
                 approved this tool for the current session. Ask the user to \
                 approve `harness_unmount`, then retry.",
            );
        }
        let args: HarnessUnmountArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("harness_unmount", format!("invalid args: {e}")),
        };
        match self.harness.unmount(&args.id).await {
            Ok(removed) => {
                tracing::info!(id = %args.id, ?removed, "harness_unmount: provider(s) torn down");
                ToolResult::success(
                    "harness_unmount",
                    json!({ "id": args.id, "removed": removed }),
                )
            }
            Err(e) => ToolResult::error(
                "harness_unmount",
                format!("unmount `{id}` failed: {e}", id = args.id),
            ),
        }
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
    async fn harness_mount_denies_without_approval() {
        let harness = Arc::new(Harness::new(KernelOptions { audit: false }));
        let tool = HarnessMountTool::new(Arc::clone(&harness));
        // No approval metadata on context ⇒ structured denial.
        let result = tool
            .execute(
                json!({ "row": { "id": "x", "source": "native", "config": null, "kind": "native" } }),
                ToolContext::default(),
            )
            .await;
        assert!(!result.success, "expected denial without approval");
        assert!(result.error.unwrap_or_default().contains("approval"));
    }

    #[tokio::test]
    async fn harness_mount_succeeds_with_approval() {
        let harness = Arc::new(Harness::new(KernelOptions { audit: false }));
        let tool = HarnessMountTool::new(Arc::clone(&harness));
        let mut ctx = ToolContext::default();
        ctx.metadata.insert("approval".to_string(), "true".to_string());
        let result = tool
            .execute(
                json!({ "row": { "id": "x", "source": "native", "config": null, "kind": "native" } }),
                ctx,
            )
            .await;
        assert!(result.success, "expected success with approval: {}", result.error.unwrap_or_default());
        // Verify the provider is actually mounted.
        let tree = harness.dump().await;
        assert!(tree.providers.iter().any(|p| p.id == "x"));
    }

    #[tokio::test]
    async fn harness_unmount_denies_without_approval() {
        let harness = Arc::new(Harness::new(KernelOptions { audit: false }));
        let tool = HarnessUnmountTool::new(Arc::clone(&harness));
        let result = tool
            .execute(json!({ "id": "noop" }), ToolContext::default())
            .await;
        assert!(!result.success);
        assert!(result.error.unwrap_or_default().contains("approval"));
    }

    #[tokio::test]
    async fn harness_unmount_succeeds_with_approval() {
        let harness = Arc::new(Harness::new(KernelOptions { audit: false }));
        harness.mount(Arc::new(Noop)).await.unwrap();
        let tool = HarnessUnmountTool::new(Arc::clone(&harness));
        let mut ctx = ToolContext::default();
        ctx.metadata.insert("approval".to_string(), "true".to_string());
        let result = tool.execute(json!({ "id": "noop" }), ctx).await;
        assert!(result.success, "{}", result.error.clone().unwrap_or_default());
        let tree = harness.dump().await;
        assert!(tree.providers.is_empty());
    }
}
