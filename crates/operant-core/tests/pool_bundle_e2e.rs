//! G6 end-to-end — `PoolBundleProvider` → `ToolSeam` fallback → `ToolRegistry`.
//!
//! Compiles a `_pool.yaml`, builds the rows, registers them through the
//! `BuilderWithFactories::register_factory("pool", ...)` path, and
//! verifies the resulting `pool.bundle` provider is materialized as a
//! typed `OperantTool` that the agent can invoke.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use operant_core::harness_adapters::ToolSeam;
use operant_core::pool_adapter;
use operant_core::tools::{OperantTool, ToolContext, ToolRegistry};
use operant_harness::{
    BuilderWithFactories, Architecture, ArchitectureRow, Harness, PoolBundleProvider, PoolManifest,
};
use serde_json::json;

fn manifest_yaml() -> &'static str {
    r#"
name: research-engine
services_offered:
  - name: search.kb
    description: search the knowledge base
  - name: query.kb
    description: query the knowledge base
services_consumed: []
pooled_sub_systems:
  - name: kb
    path: /tmp/nonexistent
"#
}

fn row_from_compiled(row: ArchitectureRow) -> ArchitectureRow {
    row
}

#[tokio::test]
async fn pool_bundle_registers_as_readonly_tool() {
    let manifest: PoolManifest = serde_yaml::from_str(manifest_yaml()).unwrap();
    let compiled = operant_harness::compile_pool(&manifest).unwrap();
    assert_eq!(compiled.bundle_rows.len(), 1);
    let bundle_row = compiled.bundle_rows.into_iter().next().unwrap();

    // Build a BuilderWithFactories whose pool factory produces a
    // PoolBundleProvider for `pool.bundle` rows.
    let mut b = BuilderWithFactories::new();
    b.register_factory(
        "pool",
        Arc::new(move |row: ArchitectureRow| {
            Ok(Arc::new(PoolBundleProvider::new(row)) as Arc<dyn operant_harness::Provider>)
        }),
    );

    let arch = Architecture {
        rows: vec![row_from_compiled(bundle_row.clone())],
    };
    let providers = b.build_with(&arch).expect("build_with");
    assert_eq!(providers.len(), 1);
    assert_eq!(providers[0].spec().source(), operant_harness::ProviderSource::Pool);

    // Mount through a Harness wired with the ToolSeam, threading the
    // row's config so the seam can read the path field.
    let registry = ToolRegistry::new(std::time::Duration::from_secs(30));
    let mut harness = Harness::new(operant_harness::KernelOptions { audit: false });
    harness.add_seam(Arc::new(ToolSeam::new(registry.clone())));

    let provider = providers.into_iter().next().unwrap();
    harness
        .mount_with_config(provider, bundle_row.config.clone())
        .await
        .expect("mount");

    // The tool should now be in the registry.
    let schemas = registry.get_schemas().await;
    assert_eq!(schemas.len(), 1, "expected one tool registered");
    assert_eq!(schemas[0].name, bundle_row.id);

    // And it should be invokable.
    let tool = registry.get(&bundle_row.id).await.expect("tool present");
    let result = tool.execute(json!({}), ToolContext::default()).await;
    assert!(result.success, "tool should be callable: {}", result.error.unwrap_or_default());
    let v: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(v["kind"], "pool.bundle");
    assert_eq!(v["read_only"], true);
}

#[tokio::test]
async fn pool_bundle_unmount_removes_tool() {
    let manifest: PoolManifest = serde_yaml::from_str(manifest_yaml()).unwrap();
    let compiled = operant_harness::compile_pool(&manifest).unwrap();
    let bundle_row = compiled.bundle_rows.into_iter().next().unwrap();

    let mut b = BuilderWithFactories::new();
    b.register_factory(
        "pool",
        Arc::new(move |row: ArchitectureRow| {
            Ok(Arc::new(PoolBundleProvider::new(row)) as Arc<dyn operant_harness::Provider>)
        }),
    );
    let arch = Architecture {
        rows: vec![row_from_compiled(bundle_row.clone())],
    };
    let providers = b.build_with(&arch).expect("build_with");

    let registry = ToolRegistry::new(std::time::Duration::from_secs(30));
    let mut harness = Harness::new(operant_harness::KernelOptions { audit: false });
    harness.add_seam(Arc::new(ToolSeam::new(registry.clone())));

    let provider = providers.into_iter().next().unwrap();
    let id = provider.spec().id().to_string();
    harness
        .mount_with_config(provider, bundle_row.config.clone())
        .await
        .expect("mount");
    assert!(!registry.get_schemas().await.is_empty());

    harness.unmount(&id).await.expect("unmount");
    let after = registry.get_schemas().await;
    assert!(after.is_empty(), "tool should be removed after unmount");
}

#[test]
fn pool_adapter_smoke() {
    // Sanity: the pool_adapter helper builds a tool from a config that
    // has a path field.
    let config = json!({ "path": "/tmp/kb", "read_only": true });
    let tool = pool_adapter::build_pool_bundle_tool("pool.x.kb", &config).expect("build");
    assert_eq!(tool.name(), "pool.x.kb");
}
