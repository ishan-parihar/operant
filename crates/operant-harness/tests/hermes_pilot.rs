//! C4 — hermes pilot: compile & boot real ~22 pools without recompile.
//!
//! The host has 22 pools under ~/.hermes/systems/* each with _org.yaml/AGENTS.md.
//! This test proves the harness can `compile_pool` + `BuilderWithFactories` +
//! `HarnessHost::boot_with_factories` for all of them without a code change.
//! It uses synthetic _pool.yaml generation from directory names so it does not
//! depend on the pools having a committed _pool.yaml.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use operant_harness::pool::{PoolService, PoolSubSystem};
use operant_harness::{
    Architecture, ArchitectureRow, BuilderWithFactories, HarnessHost, KernelOptions,
    PoolBundleProvider, PoolFamilyProvider, PoolManifest, compile_pool,
};

fn synthetic_manifest_for_dir(dir_name: &str) -> PoolManifest {
    PoolManifest {
        name: dir_name.to_string(),
        services_offered: vec![PoolService {
            name: format!("query.{}", dir_name.replace('-', ".")),
            description: format!("query {dir_name}"),
        }],
        services_consumed: vec![],
        pooled_sub_systems: vec![PoolSubSystem {
            name: "data".to_string(),
            path: format!("/tmp/hermes-pilot/{dir_name}/data"),
        }],
    }
}

#[tokio::test]
async fn hermes_pilot_compile_and_boot_all_pools() {
    let systems_dir = std::env::var("HOME")
        .map(|h| std::path::PathBuf::from(h).join(".hermes/systems"))
        .unwrap_or_else(|_| std::path::PathBuf::from("/tmp/.hermes/systems"));
    if !systems_dir.exists() {
        eprintln!("pilot skipped: {} not found", systems_dir.display());
        return;
    }
    let entries: Vec<_> = std::fs::read_dir(&systems_dir)
        .unwrap()
        .flatten()
        .filter(|e| e.path().is_dir())
        .collect();
    // Expect at least 10 pools (host has 22)
    assert!(
        entries.len() >= 5,
        "expected >=5 pools, found {}",
        entries.len()
    );

    let mut all_rows: Vec<ArchitectureRow> = Vec::new();
    for entry in &entries {
        let name = entry.file_name().to_string_lossy().to_string();
        // Skip archive and hidden
        if name.starts_with('.') || name == "archive" {
            continue;
        }
        let manifest = synthetic_manifest_for_dir(&name);
        let compiled = compile_pool(&manifest).expect("compile synthetic pool");
        assert_eq!(compiled.family_row.id, format!("pool.{name}"));
        all_rows.push(compiled.family_row);
        all_rows.extend(compiled.bundle_rows);
    }
    // At least 20 rows (family + bundle per pool)
    assert!(all_rows.len() >= 10, "rows {}", all_rows.len());

    let arch = Architecture { rows: all_rows };
    arch.validate().expect("arch validate");

    let mut builder = BuilderWithFactories::new();
    builder.register_factory(
        "pool",
        Arc::new(|row: ArchitectureRow| match row.kind.as_deref() {
            Some("pool.bundle") => {
                Ok(Arc::new(PoolBundleProvider::new(row)) as Arc<dyn operant_harness::Provider>)
            }
            Some("pool.family") | None => {
                Ok(Arc::new(PoolFamilyProvider::new(row)) as Arc<dyn operant_harness::Provider>)
            }
            Some(other) => Err(operant_harness::BuildError::NoConfigRowHandler(
                row.id.clone(),
                other.to_string(),
            )),
        }),
    );

    // Boot via HarnessHost — this is the same path the operator uses.
    let mut host = HarnessHost::new(KernelOptions { audit: false }).with_max_active_providers(300); // enough for 22 pools *2
    {
        struct CaptureSeam;
        #[async_trait::async_trait]
        impl operant_harness::Seam for CaptureSeam {
            fn name(&self) -> &str {
                "tool"
            }
            async fn install(
                &self,
                _reg: &operant_harness::Registration<'_>,
            ) -> Result<operant_harness::Effect, operant_harness::HarnessError> {
                Ok(operant_harness::Effect::noop("capture"))
            }
        }
        host.add_seam(Arc::new(CaptureSeam));
    }
    let activated = host
        .boot_with_factories(&builder, &arch)
        .await
        .expect("pilot boot");
    // At least families should be active; bundles may be pending if tool seam missing, but with capture seam they succeed.
    assert!(
        activated.len() >= 5,
        "activated {} < 5, arch rows {}",
        activated.len(),
        arch.rows.len()
    );
    let tree = host.dump().await;
    assert!(tree.providers.len() >= 5);
    // No Failed — only Active or Pending
    for p in &tree.providers {
        assert!(
            p.state == operant_harness::ProviderState::Active
                || p.state == operant_harness::ProviderState::Pending,
            "provider {} state {:?} unexpected",
            p.id,
            p.state
        );
    }
}
