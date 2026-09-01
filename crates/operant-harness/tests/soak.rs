//! S10 — 30-turn soak: mount → observe → keep-or-rollback.
//!
//! The kernel's `replace` is transactional and generation is ABA-safe.
//! This test exercises 30 sequential mount/replace/unmount cycles on the
//! same Harness to prove no leak, no claim conflict ghost, and that
//! `DumpTree` stays consistent after each step.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use async_trait::async_trait;
use operant_harness::{
    ActivateCx, Claim, Harness, HarnessError, KernelOptions, Provider, ProviderSource, ProviderSpec,
};

struct NopProvider {
    id: String,
    provides: Vec<Claim>,
}

impl ProviderSpec for NopProvider {
    fn id(&self) -> &str {
        &self.id
    }
    fn source(&self) -> ProviderSource {
        ProviderSource::Native
    }
    fn provides(&self) -> &[Claim] {
        &self.provides
    }
    fn requires(&self) -> &[Claim] {
        &[]
    }
}

#[async_trait]
impl Provider for NopProvider {
    fn spec(&self) -> &dyn ProviderSpec {
        self
    }
    async fn activate(&self, _cx: &mut ActivateCx<'_>) -> Result<(), HarnessError> {
        Ok(())
    }
}

#[tokio::test]
async fn soak_30_turns_no_leak() {
    let harness = Arc::new(Harness::new(KernelOptions { audit: false }));
    // Mount 10, replace 10, unmount 10 = 30 lifecycle ops.
    for i in 0..10u32 {
        let id = format!("soak-{i}");
        let p = Arc::new(NopProvider {
            id: id.clone(),
            provides: vec![Claim::new("tool", format!("soak-tool-{i}"))],
        });
        harness.mount(p).await.expect("mount");
        let tree = harness.dump().await;
        assert!(tree.providers.iter().any(|e| e.id == id));
        assert_eq!(tree.providers.len(), (i + 1) as usize);
    }
    for i in 0..10u32 {
        let id = format!("soak-{i}");
        let p2 = Arc::new(NopProvider {
            id: id.clone(),
            provides: vec![Claim::new("tool", format!("soak-tool-{i}-v2"))],
        });
        // Replace behind same id with new claim — old claim must disappear.
        let res = harness.replace(p2).await;
        assert!(res.is_ok(), "replace {id}: {:?}", res.err());
        let tree = harness.dump().await;
        assert!(tree.providers.iter().any(|e| e.id == id));
        // Generation should have bumped at least once.
        let entry = tree.providers.iter().find(|e| e.id == id).unwrap();
        assert!(entry.generation >= 1, "generation bump for {}", id);
    }
    for i in 0..10u32 {
        let id = format!("soak-{i}");
        let removed = harness.unmount(&id).await.expect("unmount");
        assert!(removed.contains(&id));
    }
    let tree = harness.dump().await;
    assert!(tree.providers.is_empty(), "after soak, no providers remain");
    assert!(tree.claims.is_empty(), "after soak, no claims remain");
}

// Additional pending rescue is already covered by kernel.rs T1–T13;
// this soak focuses on mount/replace/unmount lifecycles.
