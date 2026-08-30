//! Harness-kernel seam adapter for tools (plan 016 Phase 2).
//!
//! Host-side half routing kernel installs into operant's real
//! [`ToolRegistry`]: install registers a boxed tool, the returned effect's
//! undo unregisters exactly it. Existing registration paths are untouched —
//! this seam is only mounted into a `Harness` when `[harness].enabled`.

use std::sync::Arc;

use async_trait::async_trait;

use operant_harness::{Effect, HarnessError, Registration, Seam};

use crate::tools::{OperantTool, ToolRegistry};

/// Tool-family seam: claims look like `tool/<name>`; payloads are
/// `Arc<dyn OperantTool>`.
pub struct ToolSeam {
    registry: ToolRegistry,
}

impl ToolSeam {
    pub fn new(registry: ToolRegistry) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Seam for ToolSeam {
    fn name(&self) -> &str {
        "tool"
    }

    async fn install(&self, reg: &Registration<'_>) -> Result<Effect, HarnessError> {
        let tool = reg
            .payload
            .and_then(|p| p.downcast_ref::<Arc<dyn OperantTool>>())
            .cloned()
            .ok_or_else(|| HarnessError::ActivationFailed {
                id: reg.provider_id.to_string(),
                message: format!(
                    "tool seam install `{}` requires an Arc<dyn OperantTool> payload",
                    reg.key
                ),
            })?;
        self.registry
            .register_dyn(tool)
            .await
            .map_err(|e| HarnessError::ActivationFailed {
                id: reg.provider_id.to_string(),
                message: e.to_string(),
            })?;

        let registry = self.registry.clone();
        let name = reg.key.to_string();
        Ok(Effect::new(format!("tool:{name}"), move || {
            Box::pin(async move {
                registry.unregister_tool(&name).await;
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
