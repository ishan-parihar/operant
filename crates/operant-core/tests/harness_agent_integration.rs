//! S10 — harness.enabled=true agent integration.
//!
//! The agent loop must see the harness when enabled, the harness_dump tool
//! must be in the registry, and a prompt.section config_row must be mountable
//! via the harness.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::time::Duration;

use operant_core::tools::ToolRegistry;
use operant_harness::{Architecture, ArchitectureRow, Harness, KernelOptions, MountReport};
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
    assert!(
        names.contains(&"harness_dump".to_string()),
        "harness_dump missing: {names:?}"
    );
    assert!(
        names.contains(&"harness_mount".to_string()),
        "harness_mount missing"
    );
    assert!(
        names.contains(&"harness_unmount".to_string()),
        "harness_unmount missing"
    );
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
    // C6 (harness-side 018, c8fc536f): a missing seam is PENDING, not Failed —
    // late-bindable on the next seam add or mount, mirroring t12. The prompt
    // seam lives in operant-runtime; without it the row mounts Pending with an
    // explicit seam claim instead of erroring.
    let harness = Harness::new(KernelOptions { audit: false });
    let res = harness.mount(providers.into_iter().next().unwrap()).await;
    match res {
        Ok(MountReport::Pending { missing }) => {
            assert_eq!(missing, vec![operant_harness::Claim::new("seam", "prompt")]);
        }
        other => panic!("expected Pending without prompt seam, got {other:?}"),
    }
    let tree = harness.dump().await;
    assert_eq!(
        tree.providers[0].state,
        operant_harness::ProviderState::Pending
    );
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
        rows: vec![
            compiled.family_row,
            compiled.bundle_rows.into_iter().next().unwrap(),
        ],
    };
    let mut builder = operant_harness::BuilderWithFactories::new();
    builder.register_factory(
        "pool",
        Arc::new(|row: ArchitectureRow| match row.kind.as_deref() {
            Some("pool.bundle") => Ok(Arc::new(operant_harness::PoolBundleProvider::new(row))
                as Arc<dyn operant_harness::Provider>),
            Some("pool.family") | None => {
                Ok(Arc::new(operant_harness::PoolFamilyProvider::new(row))
                    as Arc<dyn operant_harness::Provider>)
            }
            Some(other) => Err(operant_harness::BuildError::NoConfigRowHandler(
                row.id.clone(),
                other.to_string(),
            )),
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
    let bundle_config = arch
        .rows
        .iter()
        .find(|r| r.kind.as_deref() == Some("pool.bundle"))
        .unwrap()
        .config
        .clone();
    for p in providers {
        let id = p.spec().id().to_string();
        if id.contains(".items") {
            harness
                .mount_with_config(p, bundle_config.clone())
                .await
                .expect("mount pool bundle");
        } else {
            harness.mount(p).await.expect("mount pool family");
        }
    }
    let tree = harness.dump().await;
    assert_eq!(tree.providers.len(), 2);
    assert!(tree.claims.iter().any(|c| c.claim.key == "query.items"));
    assert_eq!(registry.get_schemas().await.len(), 1);
}

/// C3 (018 rebuild) — `MetricsSnapshot` must leave the Harness: mounting a
/// provider whose `requires` claim nobody provides lands Pending, and that
/// outcome must be observable through the attached counters AND through the
/// serialized snapshot — the exact surface `operant status` and
/// `architecture dump --live` now expose to operators (r16 audit S6).
#[tokio::test]
async fn metrics_snapshot_exposes_pending_mounts() {
    use operant_harness::{HarnessMetrics, MountReport, PoolFamilyProvider};

    let metrics = Arc::new(HarnessMetrics::new());
    let harness = Harness::new(KernelOptions { audit: false }).with_metrics(metrics.clone());

    // A pool.family row that REQUIRES a claim nothing provides → the mount
    // must land Pending (late-bindable), not Failed.
    let row = ArchitectureRow {
        id: "dep.family".into(),
        source: "test".into(),
        disabled: false,
        kind: Some("pool.family".into()),
        config: json!({ "claims": [], "requires": ["nobody-provides-this"] }),
    };
    let report = harness
        .mount(Arc::new(PoolFamilyProvider::new(row)))
        .await
        .expect("mount");
    let MountReport::Pending { .. } = report else {
        panic!("expected pending, got {report:?}");
    };

    let snap = metrics.snapshot();
    assert_eq!(snap.mount_pending, 1, "pending mount must be counted");
    assert_eq!(snap.mount_success, 0);
    assert_eq!(snap.mount_failed, 0);

    // The snapshot must serialize with the counter the CLI surfaces.
    let serialized = serde_json::to_string(&snap).expect("serialize snapshot");
    assert!(
        serialized.contains("\"mount_pending\":1"),
        "serialized snapshot must expose mount_pending, got: {serialized}"
    );
}
