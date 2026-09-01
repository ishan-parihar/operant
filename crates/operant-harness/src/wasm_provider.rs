//! C2 — minimal WASM provider for hot-swap.
//!
//! The real Extism host lives in `operant-plugins` (feature `plugins-wasm`);
//! this provider is the kernel-side handle that the `BuilderWithFactories`
//! factory returns for `source = "wasm"` rows. Its `activate` is a no-op
//! (the host's Extism instantiation happens outside the kernel), but its
//! identity (`id` + `Wasm { path }`) and generation bump are what the swap
//! protocol exercises. A future `plugins-wasm` build can replace this factory
//! with one that actually calls `operant_plugins::runtime::create_plugin`.

#[allow(unused_imports)]
use std::sync::Arc;

use async_trait::async_trait;

use crate::claim::Claim;
use crate::composition::ArchitectureRow;
use crate::provider::{Provider, ProviderSource, ProviderSpec};
use crate::{ActivateCx, HarnessError};

/// Minimal WASM provider — kernel side of a hot-swappable Extism module.
pub struct WasmProvider {
    id: String,
    path: Option<String>,
    provides_claims: Box<[Claim]>,
}

impl WasmProvider {
    pub fn new(row: ArchitectureRow) -> Self {
        let path = row
            .config
            .get("path")
            .and_then(|v| v.as_str())
            .or_else(|| row.config.get("manifest").and_then(|v| v.as_str()))
            .map(|s| s.to_string());
        // WASM providers can optionally declare claims via config.claims
        let provides_claims: Box<[Claim]> = row
            .config
            .get("claims")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| {
                        // Split "tool/foo" or "hook/bar" into seam/key
                        if let Some((seam, key)) = s.split_once('/') {
                            Claim::new(seam, key.to_string())
                        } else {
                            Claim::new("tool", s.to_string())
                        }
                    })
                    .collect::<Vec<_>>()
                    .into_boxed_slice()
            })
            .unwrap_or_else(|| Box::new([]));
        Self {
            id: row.id,
            path,
            provides_claims,
        }
    }

    pub fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }
}

impl ProviderSpec for WasmProvider {
    fn id(&self) -> &str {
        &self.id
    }
    fn source(&self) -> ProviderSource {
        ProviderSource::Wasm {
            path: self.path.clone(),
        }
    }
    fn provides(&self) -> &[Claim] {
        &self.provides_claims
    }
    fn requires(&self) -> &[Claim] {
        &[]
    }
}

#[async_trait]
impl Provider for WasmProvider {
    fn spec(&self) -> &dyn ProviderSpec {
        self
    }
    async fn activate(&self, _cx: &mut ActivateCx<'_>) -> Result<(), HarnessError> {
        // Kernel-side activate is no-op; the host's Extism instantiation
        // (operant_plugins::runtime::create_plugin) happens in the factory
        // before this provider is constructed. This keeps the kernel pure.
        tracing::info!(id = %self.id, path = ?self.path, "wasm provider activated (kernel side)");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::composition::ArchitectureRow;
    use crate::Harness;
    use crate::KernelOptions;

    #[tokio::test]
    async fn wasm_provider_mount_and_replace_bumps_generation() {
        let row = ArchitectureRow {
            id: "wasm.test".to_string(),
            source: "wasm".to_string(),
            disabled: false,
            config: serde_json::json!({ "path": "/tmp/test.wasm" }),
            kind: None,
        };
        let p1 = Arc::new(WasmProvider::new(row.clone()));
        let h = Harness::new(KernelOptions { audit: false });
        h.mount(p1).await.expect("mount wasm");
        assert_eq!(h.generation_of("wasm.test").await, Some(1));
        // Replace with new path (simulating rebuilt wasm)
        let row2 = ArchitectureRow {
            id: "wasm.test".to_string(),
            source: "wasm".to_string(),
            disabled: false,
            config: serde_json::json!({ "path": "/tmp/test_v2.wasm" }),
            kind: None,
        };
        let p2 = Arc::new(WasmProvider::new(row2));
        h.replace(p2).await.expect("replace wasm");
        assert_eq!(h.generation_of("wasm.test").await, Some(2));
        let tree = h.dump().await;
        assert_eq!(tree.providers.len(), 1);
        assert_eq!(tree.providers[0].generation, 2);
    }
}
