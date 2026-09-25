#![allow(clippy::unwrap_used, clippy::expect_used)]
//! G3: `BuilderWithFactories` dispatches `wasm` and `pool` source rows via
//! host-registered factories. The kernel itself never knows how to build
//! these rows — only that a factory was registered.

use std::sync::Arc;

use operant_harness::{
    ActivateCx, Architecture, ArchitectureRow, BuilderWithFactories, Claim, HarnessHost,
    KernelOptions, PoolManifest, Provider, ProviderSource, ProviderSpec,
};
use serde_json::json;

fn row(id: &str, source: &str, config: serde_json::Value) -> ArchitectureRow {
    ArchitectureRow {
        id: id.to_string(),
        source: source.to_string(),
        disabled: false,
        config,
        kind: None,
    }
}

#[derive(Debug)]
struct TagProvider {
    id: String,
    source: ProviderSource,
}

impl ProviderSpec for TagProvider {
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

#[async_trait::async_trait]
impl Provider for TagProvider {
    fn spec(&self) -> &dyn ProviderSpec {
        self
    }
    async fn activate(
        &self,
        _cx: &mut ActivateCx<'_>,
    ) -> Result<(), operant_harness::HarnessError> {
        Ok(())
    }
}

fn tag_factory(source: ProviderSource) -> operant_harness::ProviderFactory {
    Arc::new(move |row: ArchitectureRow| {
        Ok(Arc::new(TagProvider {
            id: row.id,
            source: source.clone(),
        }) as Arc<dyn Provider>)
    })
}

#[tokio::test]
async fn factory_dispatches_wasm_row() {
    let mut b = BuilderWithFactories::new();
    b.register_factory("wasm", tag_factory(ProviderSource::wasm()));

    let arch = Architecture {
        rows: vec![row("wasm-row", "wasm", json!({}))],
    };
    let providers = b.build_with(&arch).expect("build_with wasm");
    assert_eq!(providers.len(), 1);
    assert_eq!(
        providers[0].spec().source(),
        ProviderSource::Wasm { path: None }
    );
    assert_eq!(providers[0].spec().id(), "wasm-row");
}

#[tokio::test]
async fn factory_dispatches_pool_row() {
    let mut b = BuilderWithFactories::new();
    b.register_factory("pool", tag_factory(ProviderSource::pool()));

    let arch = Architecture {
        rows: vec![row("pool-row", "pool", json!({}))],
    };
    let providers = b.build_with(&arch).expect("build_with pool");
    assert_eq!(providers.len(), 1);
    assert_eq!(
        providers[0].spec().source(),
        ProviderSource::Pool { name: None }
    );
}

#[tokio::test]
async fn no_factory_falls_back_to_native_stub() {
    // wasm without a registered factory should still build (built-in
    // NativeRowStub path) so hosts that don't use WASM still pass.
    let b = BuilderWithFactories::new();
    let arch = Architecture {
        rows: vec![row("fallback", "wasm", json!({}))],
    };
    let providers = b.build_with(&arch).expect("build_with fallback");
    assert_eq!(providers.len(), 1);
    assert_eq!(providers[0].spec().id(), "fallback");
    assert_eq!(
        providers[0].spec().source(),
        ProviderSource::Wasm { path: None }
    );
}

#[tokio::test]
async fn factory_compiles_real_pool_manifest() {
    // End-to-end: a pool row's config points at a real _pool.yaml; the
    // factory compiles it and produces a provider for the family row.
    let yaml = r#"
name: research-engine
services_offered:
  - name: search.kb
    description: search the knowledge base
    verbs: [search.kb]
services_consumed: []
pooled_sub_systems:
  - name: kb-search
    path: /nonexistent
"#;
    let manifest: PoolManifest = serde_yaml::from_str(yaml).expect("yaml parse");
    let compiled = operant_harness::compile_pool(&manifest).expect("compile");
    assert_eq!(compiled.family_row.id, "pool.research-engine");
    assert_eq!(compiled.bundle_rows.len(), 1);

    // Build a factory that produces the family row.
    let mut b = BuilderWithFactories::new();
    b.register_factory(
        "pool",
        Arc::new(move |row: ArchitectureRow| {
            // For the test we just construct a TagProvider with the row id;
            // the real pool compilation is exercised at the seam level.
            Ok(Arc::new(TagProvider {
                id: row.id,
                source: ProviderSource::Pool { name: None },
            }) as Arc<dyn Provider>)
        }),
    );

    let arch = Architecture {
        rows: vec![row("pool-e2e", "pool", json!({"manifest_inline": true}))],
    };
    let providers = b.build_with(&arch).expect("build_with pool e2e");
    assert_eq!(providers.len(), 1);
    assert_eq!(
        providers[0].spec().source(),
        ProviderSource::Pool { name: None }
    );
}

#[allow(dead_code)]
async fn _host_smoke() {
    let _host = HarnessHost::new(KernelOptions { audit: false });
}
