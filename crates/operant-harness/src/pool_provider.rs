//! G6 — pool.bundle adapter materializes a read-only `OperantTool` per
//! bundle. The kernel is generic; the actual tool impl lives in
//! `operant-core::pool_adapter::PoolBundleTool` because `OperantTool` is
//! defined in the host crate. This provider's only job is to install
//! that tool through the `tool` seam at activate time.

#[allow(unused_imports)]
use std::sync::Arc;

use async_trait::async_trait;

use crate::claim::Claim;
use crate::composition::ArchitectureRow;
use crate::provider::{Provider, ProviderSource, ProviderSpec};
use crate::{ActivateCx, HarnessError};

/// S5 — provider that materializes a `pool.family` row as a claim
/// holder. It owns the `pool/<service>` claims from `services_offered`
/// and declares `requires` from `services_consumed`, so late-binding
/// rescues dependents when the family mounts. It installs nothing via
/// a seam — its presence is the claim itself.
pub struct PoolFamilyProvider {
    id: String,
    config: serde_json::Value,
    provides_claims: Box<[Claim]>,
    requires_claims: Box<[Claim]>,
}

impl PoolFamilyProvider {
    pub fn new(row: ArchitectureRow) -> Self {
        let claims: Vec<Claim> = row
            .config
            .get("claims")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| Claim::new("pool", s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        let requires: Vec<Claim> = row
            .config
            .get("requires")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| Claim::new("pool", s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        Self {
            id: row.id,
            provides_claims: claims.into_boxed_slice(),
            requires_claims: requires.into_boxed_slice(),
            config: row.config,
        }
    }
}

impl ProviderSpec for PoolFamilyProvider {
    fn id(&self) -> &str {
        &self.id
    }
    fn source(&self) -> ProviderSource {
        let name = self.config.get("name").and_then(|v| v.as_str()).map(str::to_string);
        ProviderSource::Pool { name }
    }
    fn provides(&self) -> &[Claim] {
        &self.provides_claims
    }
    fn requires(&self) -> &[Claim] {
        &self.requires_claims
    }
}

#[async_trait]
impl Provider for PoolFamilyProvider {
    fn spec(&self) -> &dyn ProviderSpec {
        self
    }
    async fn activate(&self, _cx: &mut ActivateCx<'_>) -> Result<(), HarnessError> {
        tracing::info!(
            id = %self.id,
            provides = ?self.provides_claims,
            requires = ?self.requires_claims,
            "pool.family provider activated"
        );
        Ok(())
    }
}

/// Provider that materializes a `pool.bundle` row as a read-only tool.
///
/// The tool's typed implementation is provided by the host at registry
/// time (via the `operant_core::pool_adapter::PoolBundleTool` type).
/// This provider carries the row config and routes the install through
/// the `tool` seam.
pub struct PoolBundleProvider {
    id: String,
    config: serde_json::Value,
    /// The tool's `name()` — must match the seam install key so the
    /// `ToolRegistry` registers it under the same identifier the
    /// agent uses to invoke it.
    tool_name: String,
    /// Cached claim list (the spec trait returns `&[Claim]` so we
    /// own a small Box for the lifetime of the provider).
    provides_claim: Box<[Claim]>,
}

impl PoolBundleProvider {
    pub fn new(row: ArchitectureRow) -> Self {
        let tool_name = row.id.clone();
        let provides_claim = vec![Claim::new("tool", tool_name.clone())].into_boxed_slice();
        Self {
            id: row.id,
            config: row.config,
            tool_name,
            provides_claim,
        }
    }
}

impl ProviderSpec for PoolBundleProvider {
    fn id(&self) -> &str {
        &self.id
    }
    fn source(&self) -> ProviderSource {
        // G11 — round-trip the pool name from the row config so the
        // dump tree preserves the per-row identity.
        let name = self
            .config
            .get("name")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        ProviderSource::Pool { name }
    }
    fn provides(&self) -> &[Claim] {
        &self.provides_claim
    }
    fn requires(&self) -> &[Claim] {
        &[]
    }
}

#[async_trait]
impl Provider for PoolBundleProvider {
    fn spec(&self) -> &dyn ProviderSpec {
        self
    }
    async fn activate(&self, cx: &mut ActivateCx<'_>) -> Result<(), HarnessError> {
        // G6 — the row's config flows through `cx.config()`. We pass
        // the `Arc<dyn OperantTool>` payload via `install_with` so the
        // host's `ToolSeam` can register it without falling back to the
        // row-config path.
        let tool = crate::pool_provider::build_pool_bundle_tool_for(&self.tool_name, &self.config);
        cx.install_with("tool", &self.tool_name, &tool).await?;
        tracing::info!(
            id = %self.id,
            tool = %self.tool_name,
            "pool.bundle provider activated (typed tool payload installed)"
        );
        Ok(())
    }
}

/// Helper used by `PoolBundleProvider::activate` to build the
/// `Arc<dyn SeamToolPayload>` payload from the row config. Lives in
/// the pool_provider module to avoid a circular dep with the host's
/// `pool_adapter`.
pub fn build_pool_bundle_tool_for(
    name: &str,
    config: &serde_json::Value,
) -> std::sync::Arc<dyn crate::provider::SeamToolPayload> {
    use crate::provider::SeamToolPayload;
    use std::sync::Arc;

    #[allow(dead_code)]
    struct PoolBundlePayload {
        tool_name: String,
        path: String,
        read_only: bool,
    }

    impl SeamToolPayload for PoolBundlePayload {
        fn tool_name(&self) -> &str {
            &self.tool_name
        }
    }

    let path = config
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let read_only = config
        .get("read_only")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    Arc::new(PoolBundlePayload {
        tool_name: name.to_string(),
        path,
        read_only,
    }) as Arc<dyn SeamToolPayload>
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::provider::Seam;
    use std::sync::Mutex;

    /// Capture-seam used to assert install routing.
    struct CaptureSeam {
        installs: Mutex<Vec<(String, String)>>,
    }
    #[async_trait]
    impl Seam for CaptureSeam {
        fn name(&self) -> &str {
            "tool"
        }
        async fn install(
            &self,
            reg: &crate::Registration<'_>,
        ) -> Result<crate::Effect, HarnessError> {
            self.installs
                .lock()
                .unwrap()
                .push((reg.provider_id.to_string(), reg.key.to_string()));
            Ok(crate::Effect::noop("tool-stub"))
        }
    }

    fn bundle_row(id: &str, path: &str) -> ArchitectureRow {
        ArchitectureRow {
            id: id.to_string(),
            source: "pool".to_string(),
            disabled: false,
            config: serde_json::json!({ "path": path, "read_only": true }),
            kind: Some("pool.bundle".to_string()),
        }
    }

    #[tokio::test]
    async fn pool_bundle_provider_installs_tool_seam() {
        let provider = PoolBundleProvider::new(bundle_row("pool.research.kb", "/nonexistent"));
        let mut harness = crate::Harness::new(crate::KernelOptions { audit: false });
        let seam = Arc::new(CaptureSeam {
            installs: Mutex::new(Vec::new()),
        });
        harness.add_seam(seam.clone());
        harness
            .mount(Arc::new(provider) as Arc<dyn Provider>)
            .await
            .expect("mount");
        let installs = seam.installs.lock().unwrap();
        assert_eq!(installs.len(), 1);
        assert_eq!(installs[0].0, "pool.research.kb");
        assert_eq!(installs[0].1, "pool.research.kb");
    }
}
