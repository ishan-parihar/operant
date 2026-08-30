//! Harness-kernel seam adapter for prompt sections (plan 016 Phase 2 r2).
//!
//! Host-side half routing kernel installs into a shared [`PromptSections`]
//! slot. The builder snapshots the slot's children when assembling the system
//! prompt. Flag-off boots never construct this seam.

use std::sync::Arc;

use async_trait::async_trait;

use operant_harness::{Effect, HarnessError, Registration, Seam};

use super::prompt::{PromptSection, PromptSections};

/// Prompt-section seam: claims look like `prompt/<stable-id>`; payloads are
/// `Arc<dyn PromptSection>` (the `Arc` allows cheap clone-out from the
/// kernel payload reference and matches the hook-seam convention).
pub struct PromptSectionSeam {
    slot: Arc<PromptSections>,
}

impl PromptSectionSeam {
    pub fn new(slot: Arc<PromptSections>) -> Self {
        Self { slot }
    }

    /// The shared slot to plug into `SystemPromptBuilder::extend_from_slot`
    /// at agent construction.
    pub fn slot(&self) -> Arc<PromptSections> {
        Arc::clone(&self.slot)
    }
}

#[async_trait]
impl Seam for PromptSectionSeam {
    fn name(&self) -> &str {
        "prompt"
    }

    async fn install(&self, reg: &Registration<'_>) -> Result<Effect, HarnessError> {
        let section: Arc<dyn PromptSection> = reg
            .payload
            .and_then(|p| p.downcast_ref::<Arc<dyn PromptSection>>())
            .cloned()
            .ok_or_else(|| HarnessError::ActivationFailed {
                id: reg.provider_id.to_string(),
                message: format!(
                    "prompt seam install `{}` requires an Arc<dyn PromptSection> payload",
                    reg.key
                ),
            })?;

        // Slot owns an `Arc<dyn PromptSection>`; prompt builder snapshots
        // Arc::clone per build.
        self.slot.add(reg.key.to_string(), section).await;
        let slot = Arc::clone(&self.slot);
        let id = reg.key.to_string();
        Ok(Effect::new(format!("prompt:{id}"), move || {
            Box::pin(async move {
                slot.remove(&id).await;
            })
        }))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use anyhow::Result;
    use async_trait::async_trait;
    use std::sync::Arc;

    use operant_harness::{
        ActivateCx, Harness, KernelOptions, Provider, ProviderSource, ProviderSpec,
    };

    use super::*;
    use crate::agent::prompt::PromptContext;

    struct TagSection(&'static str);

    #[async_trait]
    impl PromptSection for TagSection {
        fn name(&self) -> &str {
            "tag"
        }
        fn build(&self, _ctx: &PromptContext<'_>) -> Result<String> {
            Ok(self.0.to_string())
        }
    }

    struct ExtProvider {
        section: Arc<dyn PromptSection>,
    }

    impl ProviderSpec for ExtProvider {
        fn id(&self) -> &str {
            "ext-prompt"
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
            cx.install_with("prompt", "ext-tag", &self.section).await?;
            Ok(())
        }
    }

    /// Kernel-mounted section appears in the slot snapshot; kernel unmount
    /// removes it. Existing flag-off callers that never call `extend_from_slot`
    /// see no change.
    #[tokio::test]
    async fn prompt_seam_add_remove_via_kernel() {
        let slot = Arc::new(PromptSections::new());
        let mut harness = Harness::new(KernelOptions { audit: false });
        harness.add_seam(Arc::new(PromptSectionSeam::new(Arc::clone(&slot))));

        let provider = ExtProvider {
            section: Arc::new(TagSection("hello-from-kernel")),
        };
        harness.mount(Arc::new(provider)).await.unwrap();
        assert_eq!(slot.len().await, 1);
        let snap = slot.snapshot().await;
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].0, "ext-tag");

        harness.unmount("ext-prompt").await.unwrap();
        assert!(slot.is_empty().await);
    }

    /// Order in the slot is the kernel install order, matching DynamicHooks
    /// semantics. Re-installing under the same id replaces, not duplicates.
    #[tokio::test]
    async fn prompt_seam_order_and_replace() {
        let slot = Arc::new(PromptSections::new());
        slot.add("dup", Arc::new(TagSection("v1"))).await;
        slot.add("dup", Arc::new(TagSection("v2"))).await;
        assert_eq!(slot.len().await, 1);
        let snap = slot.snapshot().await;
        assert_eq!(snap[0].1.build(&empty_ctx()).unwrap(), "v2");
    }

    fn empty_ctx() -> PromptContext<'static> {
        PromptContext {
            workspace_dir: std::path::Path::new("/tmp"),
            model_name: "test",
            tools: &[],
            skills: &[],
            skills_prompt_mode: operant_config::schema::SkillsPromptInjectionMode::Full,
            identity_config: None,
            dispatcher_instructions: "",
            sends_native_tool_specs: false,
            security_summary: None,
            autonomy_level: crate::security::AutonomyLevel::Supervised,
        }
    }
}
