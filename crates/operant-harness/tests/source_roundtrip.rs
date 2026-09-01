//! G11 — `ProviderSource` payload round-trip.
//!
//! A `Wasm` row's `path` and a `Pool` row's `name` must survive the
//! Builder → Provider → Harness → dump() round-trip. Before G11 the
//! `ProviderSource` enum was unit-only and the per-row metadata was
//! lost on `dump()`.

use std::sync::Arc;

use operant_harness::{
    ActivateCx, Architecture, ArchitectureRow, BuilderWithFactories, Claim, Harness, KernelOptions,
    Provider, ProviderSource, ProviderSpec,
};
use serde_json::json;

struct PathProvider {
    id: String,
    source: ProviderSource,
}

impl ProviderSpec for PathProvider {
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
impl Provider for PathProvider {
    fn spec(&self) -> &dyn ProviderSpec {
        self
    }
    async fn activate(&self, _cx: &mut ActivateCx<'_>) -> Result<(), operant_harness::HarnessError> {
        Ok(())
    }
}

#[tokio::test]
async fn wasm_path_roundtrips_through_dump() {
    let path = "/usr/local/share/plugins/kb-search.wasm".to_string();
    let mut b = BuilderWithFactories::new();
    b.register_factory(
        "wasm",
        Arc::new(move |row: ArchitectureRow| {
            let source = ProviderSource::Wasm {
                path: Some(path.clone()),
            };
            Ok(Arc::new(PathProvider {
                id: row.id,
                source,
            }) as Arc<dyn Provider>)
        }),
    );

    let arch = Architecture {
        rows: vec![ArchitectureRow {
            id: "kb-search".to_string(),
            source: "wasm".to_string(),
            disabled: false,
            config: json!({}),
            kind: None,
        }],
    };
    let providers = b.build_with(&arch).expect("build_with");
    let harness = Harness::new(KernelOptions { audit: false });
    harness.mount(providers.into_iter().next().unwrap()).await.unwrap();

    let tree = harness.dump().await;
    let row = &tree.providers[0];
    assert_eq!(
        row.source,
        ProviderSource::Wasm {
            path: Some("/usr/local/share/plugins/kb-search.wasm".to_string())
        }
    );
}

#[tokio::test]
async fn pool_name_roundtrips_through_dump() {
    let mut b = BuilderWithFactories::new();
    b.register_factory(
        "pool",
        Arc::new(move |row: ArchitectureRow| {
            let name = row
                .config
                .get("name")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            Ok(Arc::new(PathProvider {
                id: row.id,
                source: ProviderSource::Pool { name },
            }) as Arc<dyn Provider>)
        }),
    );

    let arch = Architecture {
        rows: vec![ArchitectureRow {
            id: "pool.research".to_string(),
            source: "pool".to_string(),
            disabled: false,
            config: json!({ "name": "research-engine" }),
            kind: None,
        }],
    };
    let providers = b.build_with(&arch).expect("build_with");
    let harness = Harness::new(KernelOptions { audit: false });
    harness.mount(providers.into_iter().next().unwrap()).await.unwrap();

    let tree = harness.dump().await;
    let row = &tree.providers[0];
    assert_eq!(
        row.source,
        ProviderSource::Pool {
            name: Some("research-engine".to_string())
        }
    );
}
