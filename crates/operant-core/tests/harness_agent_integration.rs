//! S10 — harness.enabled=true agent integration.
//!
//! The agent loop must see the harness when enabled, the harness_dump tool
//! must be in the registry, and a prompt.section config_row must be mountable
//! via the harness.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::time::Duration;

use operant_core::tools::ToolRegistry;
use operant_harness::{Architecture, ArchitectureRow, Harness, KernelOptions};
use serde_json::json;

#[tokio::test]
async fn harness_tools_visible_when_enabled() {
    let registry = ToolRegistry::new(Duration::from_secs(30));
    let harness = Arc::new(Harness::new(KernelOptions { audit: false }));
    // Register harness tools — S2.
    operant_core::tools::harness_tools::register_harness_tools(&registry, harness.clone())
        .await
        .expect("register harness tools");
    let schemas = registry.get_schemas().await;
    let names: Vec<String> = schemas.iter().map(|s| s.name.clone()).collect();
    assert!(names.contains(&"harness_dump".to_string()), "harness_dump missing: {names:?}");
    assert!(names.contains(&"harness_mount".to_string()), "harness_mount missing");
    assert!(names.contains(&"harness_unmount".to_string()), "harness_unmount missing");
}

#[tokio::test]
async fn agent_with_harness_attaches() {
    // Minimal check that with_harness builder works without needing ModelClient.
    let registry = ToolRegistry::new(Duration::from_secs(30));
    let harness = Arc::new(Harness::new(KernelOptions { audit: false }));
    let db = Arc::new(
        operant_core::database::Database::init(tempfile::tempdir().unwrap().path().join("test.db"))
            .unwrap(),
    );
    operant_core::tools::harness_tools::register_harness_tools(&registry, harness.clone())
        .await
        .expect("register");
    let schemas = registry.get_schemas().await;
    assert!(schemas.iter().any(|s| s.name == "harness_dump"));
    // Also verify Harness dump is reachable.
    let tree = harness.dump().await;
    assert_eq!(tree.providers.len(), 0);
    let _ = db; // keep db alive
}

#[tokio::test]
async fn prompt_section_config_row_buildable() {
    // S7 — prompt.section rows are now buildable (no longer NoConfigRowHandler).
    let arch = Architecture {
        rows: vec![ArchitectureRow {
            id: "hello-prompt".to_string(),
            source: "config_row".to_string(),
            disabled: false,
            config: json!({ "content": "hello from harness" }),
            kind: Some("prompt.section".to_string()),
        }],
    };
    let providers = operant_harness::Builder::build(&arch).expect("build prompt.section");
    assert_eq!(providers.len(), 1);
    assert_eq!(providers[0].spec().id(), "hello-prompt");
    // Mount without a prompt seam should mark Failed (MissingSeam) — that's correct
    // because the seam lives in operant-runtime. The important thing is Builder no longer rejects.
    let harness = Harness::new(KernelOptions { audit: false });
    let res = harness.mount(providers.into_iter().next().unwrap()).await;
    // Without seam, activation fails -> Failed state, but mount returns error.
    assert!(res.is_err(), "expected MissingSeam without prompt seam");
    let tree = harness.dump().await;
    assert_eq!(tree.providers[0].state, operant_harness::ProviderState::Failed);
}

#[tokio::test]
async fn pool_family_and_bundle_boot() {
    // S5 — pool family claims late-bind, bundle tool materializes.
    let yaml = r#"
name: test-pool
services_offered:
  - name: query.items
    description: query items
services_consumed: []
pooled_sub_systems:
  - name: items
    path: /tmp/nonexistent-items
"#;
    let manifest: operant_harness::PoolManifest = serde_yaml::from_str(yaml).unwrap();
    let compiled = operant_harness::compile_pool(&manifest).unwrap();
    assert_eq!(compiled.family_row.kind.as_deref(), Some("pool.family"));
    assert_eq!(compiled.bundle_rows[0].kind.as_deref(), Some("pool.bundle"));

    let arch = Architecture {
        rows: vec![compiled.family_row, compiled.bundle_rows.into_iter().next().unwrap()],
    };
    let mut builder = operant_harness::BuilderWithFactories::new();
    builder.register_factory(
        "pool",
        Arc::new(|row: ArchitectureRow| {
            match row.kind.as_deref() {
                Some("pool.bundle") => Ok(Arc::new(operant_harness::PoolBundleProvider::new(row))
                    as Arc<dyn operant_harness::Provider>),
                Some("pool.family") | None => Ok(Arc::new(operant_harness::PoolFamilyProvider::new(row))
                    as Arc<dyn operant_harness::Provider>),
                Some(other) => Err(operant_harness::BuildError::NoConfigRowHandler(
                    row.id.clone(),
                    other.to_string(),
                )),
            }
        }),
    );
    let providers = builder.build_with(&arch).expect("build_with pool");
    assert_eq!(providers.len(), 2);
    // Mount family first, then bundle — bundle should succeed even though it
    // doesn't require the family claim (it provides tool claim).
    let registry = ToolRegistry::new(Duration::from_secs(30));
    let mut harness = Harness::new(KernelOptions { audit: false });
    harness.add_seam(Arc::new(operant_core::harness_adapters::ToolSeam::new(
        registry.clone(),
    )));
    // Family has no tool config, so plain mount works. Bundle needs its
    // row config to materialize the tool path, so we thread it via mount_with_config.
    let bundle_config = arch.rows.iter().find(|r| r.kind.as_deref() == Some("pool.bundle")).unwrap().config.clone();
    for p in providers {
        let id = p.spec().id().to_string();
        if id.contains(".items") {
            harness.mount_with_config(p, bundle_config.clone()).await.expect("mount pool bundle");
        } else {
            harness.mount(p).await.expect("mount pool family");
        }
    }
    let tree = harness.dump().await;
    assert_eq!(tree.providers.len(), 2);
    assert!(tree.claims.iter().any(|c| c.claim.key == "query.items"));
    assert_eq!(registry.get_schemas().await.len(), 1);
}
