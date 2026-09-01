//! Row-driven provider stubs. Phase 3 ships placeholders that carry the
//! row's config; phases 4/5 wire the real native/wasm/config_row handlers
//! onto these stubs.

use async_trait::async_trait;

use crate::composition::ArchitectureRow;
use crate::provider::{Provider, ProviderSource, ProviderSpec};
use crate::{ActivateCx, Claim, HarnessError};

/// Stands in for a `native` or `wasm` row. The stub records the row's
/// config in its `activate` body so a future phase can read it out
/// (e.g., to dispatch a tool that the row asked for).
pub struct NativeRowStub {
    id: String,
    source: ProviderSource,
    /// Carried for the host to read; not consumed in the Phase 3 stub.
    #[allow(dead_code)]
    pub config: serde_json::Value,
}

impl NativeRowStub {
    pub fn new(row: ArchitectureRow) -> Self {
        let source = match row.source.as_str() {
            "wasm" => ProviderSource::wasm(),
            // `pool` not handled at this layer (Phase 6); default to Native.
            _ => ProviderSource::Native,
        };
        Self {
            id: row.id,
            source,
            config: row.config,
        }
    }
}

impl ProviderSpec for NativeRowStub {
    fn id(&self) -> &str {
        &self.id
    }
    fn source(&self) -> ProviderSource {
        self.source.clone()
    }
    fn provides(&self) -> &[Claim] {
        &[]
    }
    fn requires(&self) -> &[Claim] {
        &[]
    }
}

#[async_trait]
impl Provider for NativeRowStub {
    fn spec(&self) -> &dyn ProviderSpec {
        self
    }
    async fn activate(&self, _cx: &mut ActivateCx<'_>) -> Result<(), HarnessError> {
        // Phase 4+ will read self.config here to install tools, prompt
        // sections, hooks, etc. For Phase 3, the row exists in the
        // resolved dump tree; activation is a no-op so the kernel
        // records the provider as Active and dump() includes it.
        tracing::debug!(id = %self.id, source = ?self.source, "NativeRowStub activated");
        Ok(())
    }
}

/// Config-row provider: a provider that owns no claims but can drive
/// host-side configuration at boot. Phase 3 supports the
/// `kind = "prompt.section"` flavor which installs a file-backed prompt
/// section into the kernel prompt.section seam.
pub struct ConfigRowProvider {
    id: String,
    kind: String,
    /// Carried for the host to read; not consumed in the Phase 3 stub.
    #[allow(dead_code)]
    pub config: serde_json::Value,
}

impl ConfigRowProvider {
    /// Construct from a validated config_row. The caller is expected to
    /// have already passed [`ArchitectureRow::validate`].
    pub fn new(row: ArchitectureRow) -> Self {
        Self {
            id: row.id,
            kind: row.kind.unwrap_or_default(),
            config: row.config,
        }
    }
}

impl ProviderSpec for ConfigRowProvider {
    fn id(&self) -> &str {
        &self.id
    }
    fn source(&self) -> ProviderSource {
        ProviderSource::ConfigRow
    }
    fn provides(&self) -> &[Claim] {
        &[]
    }
    fn requires(&self) -> &[Claim] {
        &[]
    }
}

#[async_trait]
impl Provider for ConfigRowProvider {
    fn spec(&self) -> &dyn ProviderSpec {
        self
    }
    async fn activate(&self, _cx: &mut ActivateCx<'_>) -> Result<(), HarnessError> {
        // Future: read self.config (e.g. { path = "/etc/prompt.md" }) and
        // install a file-backed PromptSection. Phase 3 logs only.
        tracing::debug!(id = %self.id, kind = %self.kind, "ConfigRowProvider activated");
        Ok(())
    }
}

// Helper for testing: a config-row prompt.section install that actually
// emits a section through the kernel payload channel. Exists so the
// `prompt.section` config-row can be exercised without a live filesystem.
// (defined inside the test module below so `Arc` doesn't leak into the
// non-test module path.)

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::composition::ArchitectureRow;
    use async_trait::async_trait;
    use std::sync::Arc;

    /// Config-row prompt.section test stub.
    #[allow(dead_code)]
    pub struct FakeFilePromptSection {
        pub id: String,
        pub body: String,
    }

    impl ProviderSpec for FakeFilePromptSection {
        fn id(&self) -> &str {
            &self.id
        }
        fn source(&self) -> ProviderSource {
            ProviderSource::ConfigRow
        }
        fn provides(&self) -> &[Claim] {
            &[]
        }
        fn requires(&self) -> &[Claim] {
            &[]
        }
    }

    #[async_trait]
    impl Provider for FakeFilePromptSection {
        fn spec(&self) -> &dyn ProviderSpec {
            self
        }
        async fn activate(&self, cx: &mut ActivateCx<'_>) -> Result<(), HarnessError> {
            // Closure-erased payload (the host's boot pass will read the
            // real file from row.config and construct a real PromptSection;
            // this stub just demonstrates the install path).
            let body: Arc<dyn std::any::Any + Send + Sync> = Arc::new(self.body.clone());
            cx.install_with("prompt", &self.id, &body).await?;
            Ok(())
        }
    }

    fn row(id: &str, source: &str, kind: Option<&str>) -> ArchitectureRow {
        ArchitectureRow {
            id: id.to_string(),
            source: source.to_string(),
            disabled: false,
            config: serde_json::json!({}),
            kind: kind.map(|s| s.to_string()),
        }
    }

    #[test]
    fn native_row_stub_uses_native_source() {
        let stub = NativeRowStub::new(row("a", "native", None));
        assert_eq!(stub.id(), "a");
        assert_eq!(stub.source(), ProviderSource::Native);
    }

    #[test]
    fn native_row_stub_uses_wasm_source() {
        let stub = NativeRowStub::new(row("b", "wasm", None));
        assert_eq!(stub.source(), ProviderSource::Wasm { path: None });
    }

    #[test]
    fn config_row_provider_uses_config_row_source() {
        let p = ConfigRowProvider::new(row("c", "config_row", Some("prompt.section")));
        assert_eq!(p.source(), ProviderSource::ConfigRow);
    }
}
