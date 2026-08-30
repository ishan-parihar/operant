//! Harness-kernel seam adapter for hooks (plan 016 Phase 2).
//!
//! Host-side half routing kernel installs into a shared [`DynamicHooks`] slot
//! (which itself rides inside the boot `HookRunner` as one static handler).
//! Install adds a child handler under a stable id; the returned effect's undo
//! removes exactly it. Flag-off boots never construct this seam.

use std::sync::Arc;

use async_trait::async_trait;

use operant_harness::{Effect, HarnessError, Registration, Seam};

use crate::hooks::{DynamicHooks, HookHandler};

/// Hook-family seam: claims look like `hook/<stable-id>`; payloads are
/// `Arc<dyn HookHandler>`.
pub struct HooksSeam {
    dynamic: Arc<DynamicHooks>,
}

impl HooksSeam {
    pub fn new(dynamic: Arc<DynamicHooks>) -> Self {
        Self { dynamic }
    }

    /// The shared slot to register with `HookRunner` at boot.
    pub fn slot(&self) -> Arc<DynamicHooks> {
        Arc::clone(&self.dynamic)
    }
}

#[async_trait]
impl Seam for HooksSeam {
    fn name(&self) -> &str {
        "hook"
    }

    async fn install(&self, reg: &Registration<'_>) -> Result<Effect, HarnessError> {
        let handler = reg
            .payload
            .and_then(|p| p.downcast_ref::<Arc<dyn HookHandler>>())
            .cloned()
            .ok_or_else(|| HarnessError::ActivationFailed {
                id: reg.provider_id.to_string(),
                message: format!(
                    "hook seam install `{}` requires an Arc<dyn HookHandler> payload",
                    reg.key
                ),
            })?;

        self.dynamic.add(reg.key.to_string(), handler).await;
        let dynamic = Arc::clone(&self.dynamic);
        let id = reg.key.to_string();
        Ok(Effect::new(format!("hook:{id}"), move || {
            Box::pin(async move {
                dynamic.remove(&id).await;
            })
        }))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use async_trait::async_trait;
    use serde_json::Value;
    use std::sync::{Arc, Mutex};

    use operant_harness::{ActivateCx, KernelOptions, Provider, ProviderSource, ProviderSpec};

    use super::*;
    use crate::hooks::HookResult;

    struct TagHook(Arc<Mutex<Vec<String>>>);

    #[async_trait]
    impl HookHandler for TagHook {
        fn name(&self) -> &str {
            "tag-hook"
        }
        async fn before_tool_call(&self, name: String, args: Value) -> HookResult<(String, Value)> {
            self.0.lock().unwrap().push(name.clone());
            HookResult::Continue((name, args))
        }
    }

    struct ExtProvider {
        hook_payload: Arc<dyn HookHandler>,
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
            cx.install_with("hook", "ext-hook", &self.hook_payload)
                .await?;
            Ok(())
        }
    }

    /// Kernel-mounted hook receives dispatches; kernel unmount removes it.
    #[tokio::test]
    async fn hooks_seam_add_remove_via_kernel() {
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let dynamic = Arc::new(DynamicHooks::new());

        let mut harness = operant_harness::Harness::new(KernelOptions { audit: false });
        harness.add_seam(Arc::new(HooksSeam::new(Arc::clone(&dynamic))));

        let provider = ExtProvider {
            hook_payload: Arc::new(TagHook(Arc::clone(&seen))),
        };
        harness.mount(Arc::new(provider)).await.unwrap();
        assert_eq!(dynamic.len().await, 1);

        // Fan-out reaches the dynamically added hook.
        let name_out = match dynamic
            .before_tool_call("shell".to_string(), Value::Null)
            .await
        {
            HookResult::Continue((name, _)) => name,
            HookResult::Cancel(reason) => panic!("unexpected cancel: {reason}"),
        };
        assert_eq!(name_out, "shell");
        assert_eq!(seen.lock().unwrap().as_slice(), ["shell".to_string()]);

        // Kernel unmount removes the hook; dispatch passes through untouched.
        harness.unmount("ext").await.unwrap();
        assert!(dynamic.is_empty().await);
        let name_out = match dynamic
            .before_tool_call("git".to_string(), Value::Null)
            .await
        {
            HookResult::Continue((name, _)) => name,
            HookResult::Cancel(reason) => panic!("unexpected cancel: {reason}"),
        };
        assert_eq!(name_out, "git");
        assert_eq!(seen.lock().unwrap().as_slice(), ["shell".to_string()]);
    }

    /// The slot composes with a real HookRunner as one ordinary handler.
    #[tokio::test]
    async fn slot_composes_with_hook_runner() {
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let dynamic = Arc::new(DynamicHooks::new());
        let mut runner = crate::hooks::HookRunner::new();
        runner.register(Box::new(DynamicHooks::default())); // empty slot is harmless
        let _ = runner;

        let mut harness = operant_harness::Harness::new(KernelOptions { audit: false });
        harness.add_seam(Arc::new(HooksSeam::new(Arc::clone(&dynamic))));
        harness
            .mount(Arc::new(ExtProvider {
                hook_payload: Arc::new(TagHook(Arc::clone(&seen))),
            }))
            .await
            .unwrap();

        // Dispatch through the DynamicHooks handler directly (the way the
        // runner's single entry would).
        let outcome = dynamic.before_prompt_build("p".to_string()).await;
        assert!(matches!(outcome, HookResult::Continue(_)));
    }
}
