//! Harness-kernel seam adapter for tools (plan 016 Phase 2).
//!
//! Host-side half routing kernel installs into operant's real
//! [`ToolRegistry`]: install registers a boxed tool, the returned effect's
//! undo unregisters exactly it. Existing registration paths are untouched —
//! this seam is only mounted into a `Harness` when `[harness].enabled`.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use operant_harness::{Effect, HarnessError, Registration, Seam};

use crate::pool_adapter;
use crate::tools::{OperantTool, ToolRegistry};

/// Optional factory for `pool.bundle` rows. When set, the seam uses it
/// to materialize a typed `OperantTool` from the row's config; when
/// unset, a `pool.bundle` row that calls `install` without a payload
/// still registers a `PoolBundleTool` built from the key + an empty
/// config (G6 default).
pub type PoolToolFactory = Arc<dyn Fn(&str, &Value) -> Option<Arc<dyn OperantTool>> + Send + Sync>;

/// Tool-family seam: claims look like `tool/<name>`; payloads are
/// `Arc<dyn OperantTool>`.
pub struct ToolSeam {
    registry: ToolRegistry,
    pool_tool_factory: Option<PoolToolFactory>,
}

impl ToolSeam {
    pub fn new(registry: ToolRegistry) -> Self {
        Self {
            registry,
            pool_tool_factory: None,
        }
    }

    /// G6 — install a custom factory for `pool.bundle` rows. The
    /// factory takes `(tool_name, row_config)` and returns a typed
    /// `OperantTool` or `None` to fall through to the default
    /// `PoolBundleTool`.
    pub fn with_pool_factory(mut self, factory: PoolToolFactory) -> Self {
        self.pool_tool_factory = Some(factory);
        self
    }
}

#[async_trait]
impl Seam for ToolSeam {
    fn name(&self) -> &str {
        "tool"
    }

    async fn install(&self, reg: &Registration<'_>) -> Result<Effect, HarnessError> {
        // Three install paths:
        // (a) explicit `Arc<dyn OperantTool>` payload — preferred for
        //     in-process providers that already have the tool.
        // (b) G6 `Arc<dyn SeamToolPayload>` payload — the provider
        //     knows the tool name; the seam materializes the host-
        //     side `OperantTool` from the payload + the pool adapter.
        // (c) G6 row-config fallback for `pool.*` rows whose provider
        //     didn't ship a payload (the row's `cx.config()` is read).
        let tool: Arc<dyn OperantTool> = if let Some(p) = reg
            .payload
            .and_then(|p| p.downcast_ref::<Arc<dyn OperantTool>>())
            .cloned()
        {
            p
        } else if let Some(payload) = reg
            .payload
            .and_then(|p| p.downcast_ref::<Arc<dyn operant_harness::provider::SeamToolPayload>>())
            .cloned()
        {
            // G6 — materialization from typed payload.
            let tool_name = payload.tool_name().to_string();
            let config = reg.config.clone();
            let tool = if let Some(f) = &self.pool_tool_factory {
                f(&tool_name, &config)
            } else {
                pool_adapter::build_pool_bundle_tool(&tool_name, &config)
            };
            tool.ok_or_else(|| HarnessError::ActivationFailed {
                id: reg.provider_id.to_string(),
                message: format!(
                    "SeamToolPayload install `{}` could not materialize a tool (no path in config?)",
                    reg.key
                ),
            })?
        } else if reg.provider_id.starts_with("pool.") {
            // G6 — row-config fallback (when the provider used
            // `install` instead of `install_with`).
            let config = reg.config.clone();
            let tool = if let Some(f) = &self.pool_tool_factory {
                f(reg.key, &config)
            } else {
                pool_adapter::build_pool_bundle_tool(reg.key, &config)
            };
            tool.ok_or_else(|| HarnessError::ActivationFailed {
                id: reg.provider_id.to_string(),
                message: format!(
                    "pool.bundle install `{}` could not materialize a tool (no path in config?)",
                    reg.key
                ),
            })?
        } else {
            return Err(HarnessError::ActivationFailed {
                id: reg.provider_id.to_string(),
                message: format!(
                    "tool seam install `{}` requires an Arc<dyn OperantTool> payload",
                    reg.key
                ),
            });
        };
        self.registry
            .register_dyn(tool.clone())
            .await
            .map_err(|e| HarnessError::ActivationFailed {
                id: reg.provider_id.to_string(),
                message: e.to_string(),
            })?;

        // G2 — capture the Arc identity for the effect-undo. Comparing
        // Arc::ptr_eq at unwind time means staging `echo` (new Arc) then
        // unwinding old `echo` (old Arc) is a no-op for the new value.
        let registry = self.registry.clone();
        let name = reg.key.to_string();
        let installed_arc = tool;
        Ok(Effect::new(format!("tool:{name}"), move || {
            Box::pin(async move {
                registry.unregister_tool_if(&name, &installed_arc).await;
            })
        }))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use std::sync::atomic::{AtomicU32, Ordering};

    use operant_harness::{ActivateCx, KernelOptions, Provider, ProviderSource, ProviderSpec};
    use serde_json::json;

    use super::*;
    use crate::schema::ToolSchema;
    use crate::tools::ToolContext;

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
            "echoes args; installed via harness seam"
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
        ) -> crate::tools::ToolResult {
            self.calls.fetch_add(1, Ordering::SeqCst);
            crate::tools::ToolResult {
                tool_call_id: String::new(),
                name: self.name().to_string(),
                success: true,
                content: format!("echo{}", self.suffix),
                error: None,
            }
        }
    }

    struct ExtProvider {
        tool_payload: Arc<dyn OperantTool>,
    }

    impl ProviderSpec for ExtProvider {
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
    impl Provider for ExtProvider {
        fn spec(&self) -> &dyn ProviderSpec {
            self
        }
        async fn activate(
            &self,
            cx: &mut ActivateCx<'_>,
        ) -> Result<(), operant_harness::HarnessError> {
            cx.install_with("tool", self.tool_payload.name(), &self.tool_payload)
                .await?;
            Ok(())
        }
    }

    /// Equivalence: a seam-installed tool behaves identically to a directly
    /// registered one — same schema surface, same execution path — and its
    /// uninstall removes exactly it while direct registrations survive.
    #[tokio::test]
    async fn tool_seam_equivalence_and_unwind() {
        let direct_registry = ToolRegistry::new(std::time::Duration::from_secs(30));
        direct_registry
            .register_dyn(EchoTool::new("_direct"))
            .await
            .unwrap();

        let harness_registry = ToolRegistry::new(std::time::Duration::from_secs(30));
        let mut harness = operant_harness::Harness::new(KernelOptions { audit: false });
        harness.add_seam(Arc::new(ToolSeam::new(harness_registry.clone())));

        let provider = ExtProvider {
            tool_payload: EchoTool::new("_seam"),
        };
        harness.mount(Arc::new(provider)).await.unwrap();

        // Schema surfaces match (same name/description shape).
        let schemas = harness_registry.get_schemas().await;
        assert_eq!(schemas.len(), 1);
        assert_eq!(schemas[0].name, "seam_echo");
        let direct_schemas = direct_registry.get_schemas().await;
        assert_eq!(schemas[0].description, direct_schemas[0].description);

        // Unmount unwinds: tool disappears from the seam registry only.
        harness.unmount("ext").await.unwrap();
        let after = harness_registry.get_schemas().await;
        assert!(after.is_empty());
        assert_eq!(direct_registry.get_schemas().await.len(), 1);
    }

    /// Payload type mismatch is contained as a structured activation failure.
    #[tokio::test]
    async fn tool_seam_rejects_wrong_payload() {
        let registry = ToolRegistry::new(std::time::Duration::from_secs(30));
        let mut harness = operant_harness::Harness::new(KernelOptions { audit: false });
        harness.add_seam(Arc::new(ToolSeam::new(registry.clone())));

        struct BadPayload;
        struct P;
        impl ProviderSpec for P {
            fn id(&self) -> &str {
                "p"
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
        impl Provider for P {
            fn spec(&self) -> &dyn ProviderSpec {
                self
            }
            async fn activate(
                &self,
                cx: &mut ActivateCx<'_>,
            ) -> Result<(), operant_harness::HarnessError> {
                cx.install_with("tool", "nope", &BadPayload).await?;
                Ok(())
            }
        }

        let err = harness.mount(Arc::new(P)).await.unwrap_err();
        assert!(matches!(
            err,
            operant_harness::HarnessError::ActivationFailed { .. }
        ));
        assert_eq!(
            harness.state_of("p").await,
            Some(operant_harness::ProviderState::Failed)
        );
        assert!(registry.get_schemas().await.is_empty());
    }
}
