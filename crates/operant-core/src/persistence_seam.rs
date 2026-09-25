//! 018-U1 — Persistence seam: Rust `prompt`/`subagent` claims delegate to the
//! Python sidecar file store (kernel-sidecar/harness.py) via NDJSON.
//!
//! The Rust `operant-harness` owns the composition (tool/pool/wasm/hook)
//! but `prompt`/`subagent` are file-persistent and already live in the
//! sidecar (`~/.local/share/operant/kernel/harness/{global,sessions/...}`).
//! This seam makes the two harnesses complementary instead of duplicate:
//! Rust is the single Harness, Python is its persistence seam.
//!
//! When the kernel is off (`global_runtime()==None`), `install` returns
//! `MissingSeam` → PENDING, so lessons wait until the sidecar is available
//! instead of being lost. This is the lazy deletion: no data loss on fallback.

use async_trait::async_trait;
use serde_json::Value;

use operant_harness::{Effect, HarnessError, Registration, Seam};

/// Delegates `prompt` and `subagent` claims to the sidecar file store.
/// One instance handles both kinds; `name()` returns the kind it was
/// constructed for (the harness mounts two instances, one per kind).
pub struct PersistenceSeam {
    kind: &'static str,
}

impl PersistenceSeam {
    pub fn prompt() -> Self {
        Self { kind: "prompt" }
    }
    pub fn subagent() -> Self {
        Self { kind: "subagent" }
    }
}

#[async_trait]
impl Seam for PersistenceSeam {
    fn name(&self) -> &str {
        self.kind
    }

    async fn install(&self, reg: &Registration<'_>) -> Result<Effect, HarnessError> {
        let rt = crate::tools::kernel::global_runtime()
            .cloned()
            .ok_or_else(|| HarnessError::MissingSeam(self.kind.to_string()))?;

        if !rt.settings().enabled {
            return Err(HarnessError::MissingSeam(self.kind.to_string()));
        }

        // Registration key is the entry title; config carries {content, scope, session_key?}.
        // For prompt/subagent the composition row's `config` is {title, content, scope?}.
        let title = reg.key.to_string();
        let content = reg
            .config
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let scope = reg
            .config
            .get("scope")
            .and_then(Value::as_str)
            .unwrap_or("local")
            .to_string();
        let session_key = reg
            .config
            .get("session_key")
            .and_then(Value::as_str)
            .map(|s| s.to_string());

        // Foreign async call must not hold the harness write lock — but
        // `install` is called with the lock held. The sidecar request is
        // bounded (sidecar timeout) and the harness lock is async, so this
        // is safe as long as we don't call back into Harness.
        let mut params = serde_json::json!({
            "kind": self.kind,
            "title": title,
            "content": content,
            "scope": scope,
        });
        if let Some(sk) = session_key {
            params["session_key"] = Value::String(sk);
        }

        let result = rt
            .request("harness_upsert", params)
            .await
            .map_err(|e| HarnessError::ActivationFailed {
                id: reg.provider_id.to_string(),
                message: format!("persistence seam upsert failed: {e}"),
            })?;

        let entry_id = result
            .get("id")
            .or_else(|| result.get("entry_id"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();

        // LIFO undo: delete the file entry.
        let rt_clone = rt.clone();
        let kind = self.kind.to_string();
        let scope_clone = scope.clone();
        Ok(Effect::new(format!("{}:{title}", self.kind), move || {
            let rt = rt_clone.clone();
            let kind = kind.clone();
            let entry_id = entry_id.clone();
            let scope = scope_clone.clone();
            Box::pin(async move {
                let _ = rt
                    .request(
                        "harness_delete",
                        serde_json::json!({
                            "kind": kind,
                            "id": entry_id,
                            "scope": scope,
                        }),
                    )
                    .await;
            })
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use operant_harness::{ActivateCx, KernelOptions, Provider, ProviderSource, ProviderSpec};

    struct PromptProvider;

    impl ProviderSpec for PromptProvider {
        fn id(&self) -> &str { "test-prompt" }
        fn source(&self) -> ProviderSource { ProviderSource::ConfigRow }
        fn provides(&self) -> &[operant_harness::Claim] { &[] }
        fn requires(&self) -> &[operant_harness::Claim] { &[] }
    }

    #[async_trait]
    impl Provider for PromptProvider {
        fn spec(&self) -> &dyn ProviderSpec { self }
        async fn activate(&self, cx: &mut ActivateCx<'_>) -> Result<(), HarnessError> {
            cx.install("prompt", "test-title").await?;
            Ok(())
        }
    }

    #[tokio::test]
    async fn persistence_seam_pends_when_kernel_off() {
        // No global_runtime installed → MissingSeam → PENDING.
        // This is the fallback path that keeps 016 dark-safe.
        let mut harness = operant_harness::Harness::new(KernelOptions { audit: false });
        harness.add_seam(Arc::new(PersistenceSeam::prompt()));
        harness.add_seam(Arc::new(PersistenceSeam::subagent()));
        let err = harness
            .mount(Arc::new(PromptProvider) as Arc<dyn Provider>)
            .await
            .unwrap_err();
        assert!(matches!(err, HarnessError::MissingSeam(_)));
        // The entry stays Pending, not Failed — will activate when seam appears.
        assert_eq!(
            harness.state_of("test-prompt").await,
            Some(operant_harness::ProviderState::Pending)
        );
    }
}
