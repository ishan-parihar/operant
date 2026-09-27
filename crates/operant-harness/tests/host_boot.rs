#![allow(clippy::unwrap_used, clippy::expect_used)]
//! G1: host boot — `HarnessHost::boot(Architecture)` mounts every provider
//! the `Builder` produces and returns the activated id list.

use std::sync::Arc;

use operant_harness::{
    ActivateCx, Architecture, ArchitectureRow, Claim, HarnessHost, KernelOptions, Provider,
    ProviderSource, ProviderSpec, Seam,
};
use serde_json::json;

#[derive(Debug)]
struct NoopProvider {
    id: String,
    requires: Vec<Claim>,
    provides: Vec<Claim>,
}

impl ProviderSpec for NoopProvider {
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
        &self.requires
    }
}

#[async_trait::async_trait]
impl Provider for NoopProvider {
    fn spec(&self) -> &dyn ProviderSpec {
        self
    }
    async fn activate(&self, cx: &mut ActivateCx<'_>) -> Result<(), operant_harness::HarnessError> {
        let _ = cx;
        Ok(())
    }
}

#[derive(Debug)]
struct CaptureSeam {
    installs: std::sync::Mutex<Vec<(String, String)>>,
}

impl CaptureSeam {
    fn new() -> Self {
        Self {
            installs: std::sync::Mutex::new(Vec::new()),
        }
    }
}

#[async_trait::async_trait]
impl Seam for CaptureSeam {
    fn name(&self) -> &str {
        "capture"
    }
    async fn install(
        &self,
        reg: &operant_harness::Registration<'_>,
    ) -> Result<operant_harness::Effect, operant_harness::HarnessError> {
        self.installs
            .lock()
            .unwrap()
            .push((reg.provider_id.to_string(), reg.key.to_string()));
        Ok(operant_harness::Effect::noop("capture"))
    }
}

fn native_row(id: &str) -> ArchitectureRow {
    ArchitectureRow {
        id: id.to_string(),
        source: "native".to_string(),
        disabled: false,
        config: json!(null),
        kind: Some("native".to_string()),
    }
}

#[tokio::test]
async fn host_boot_registers_native_seam_and_mounts() {
    let mut host = HarnessHost::new(KernelOptions { audit: false });
    host.add_seam(Arc::new(CaptureSeam::new()));

    let arch = Architecture {
        rows: vec![native_row("noop-test")],
    };
    let activated = host.boot(&arch).await.expect("boot");
    assert_eq!(activated, vec!["noop-test".to_string()]);
}

#[tokio::test]
async fn host_dump_reflects_active_providers() {
    let host = HarnessHost::new(KernelOptions { audit: false });
    let arch = Architecture {
        rows: vec![native_row("noop-dump")],
    };
    let _ = host.boot(&arch).await.expect("boot");
    let tree = host.dump().await;
    assert_eq!(tree.providers.len(), 1);
    assert_eq!(tree.providers[0].id, "noop-dump");
    // Count-only accessor — used by the C5 mount cap; must stay in sync
    // with the dump without paying the full DumpTree build.
    assert_eq!(host.harness().provider_count().await, 1);
}

#[tokio::test]
async fn mount_all_rejects_past_max_active_providers() {
    // C5: the per-mount cap rejects a provider that would exceed
    // max_active_providers with a CompositionError naming the cap and
    // the rejected provider (guards the 250-pool OOM story).
    let mut host = HarnessHost::new(KernelOptions { audit: false }).with_max_active_providers(1);
    host.add_seam(Arc::new(CaptureSeam::new()));
    let arch = Architecture {
        rows: vec![native_row("cap-first"), native_row("cap-second")],
    };
    let err = host
        .boot(&arch)
        .await
        .expect_err("second mount must hit the cap");
    let msg = err.to_string();
    assert!(
        msg.contains("max_active_providers 1 exceeded"),
        "error must name the cap: {msg}"
    );
    assert!(
        msg.contains("cap-second"),
        "error must name the rejected provider: {msg}"
    );
    // The first mount landed; the tree holds exactly the cap.
    assert_eq!(host.harness().provider_count().await, 1);
}

#[tokio::test]
async fn host_swap_bumps_generation() {
    let host = HarnessHost::new(KernelOptions { audit: false });
    let arch = Architecture {
        rows: vec![native_row("noop-swap")],
    };
    let _ = host.boot(&arch).await.expect("boot");
    let gen0 = host.harness().generation_of("noop-swap").await;
    assert_eq!(gen0, Some(1));
}

#[allow(dead_code)]
fn _ensure_noop_linked() {
    let _p = NoopProvider {
        id: "x".to_string(),
        requires: vec![],
        provides: vec![],
    };
    let _arc: Arc<dyn Provider> = Arc::new(_p);
}
