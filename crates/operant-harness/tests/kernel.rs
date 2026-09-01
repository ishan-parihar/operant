//! Harness kernel lifecycle suite (plan 016 Phase 1).
//!
//! Covers: LIFO effect unwind, late binding (PENDING rescue), pre-activation
//! claim-conflict rejection, partial-failure containment, transactional
//! replace (failure keeps old / success flips atomically + generation bump),
//! transitive cascade unmount, concurrency serialization, dump shape, and
//! not-found errors.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};

use operant_harness::{
    ActivateCx, Claim, DumpTree, Effect, Harness, HarnessError, KernelOptions, MountReport,
    Provider, ProviderSource, Registration, Seam,
};

// ── test infrastructure ────────────────────────────────────────────────

#[derive(Clone, Default)]
struct Log(Arc<Mutex<Vec<String>>>);

impl Log {
    fn push(&self, entry: impl Into<String>) {
        self.0.lock().unwrap().push(entry.into());
    }
    fn snapshot(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

/// Records installs/uninstalls; can be scripted to fail specific keys.
struct TestSeam {
    name: String,
    log: Log,
    fail_keys: Vec<String>,
}

impl TestSeam {
    fn new(name: &str, log: Log) -> Self {
        Self {
            name: name.to_string(),
            log,
            fail_keys: Vec::new(),
        }
    }
}

#[async_trait::async_trait]
impl Seam for TestSeam {
    fn name(&self) -> &str {
        &self.name
    }

    async fn install(&self, reg: &Registration<'_>) -> Result<Effect, HarnessError> {
        if self.fail_keys.iter().any(|k| k == reg.key) {
            return Err(HarnessError::MissingSeam(format!(
                "scripted-failure:{}/{}",
                self.name, reg.key
            )));
        }
        let label = format!("{}:{}:{}", reg.provider_id, self.name, reg.key);
        let undo_label = format!("undo:{label}");
        let log = self.log.clone();
        self.log.push(format!("install:{label}"));
        Ok(Effect::new(label, move || {
            let log = log.clone();
            Box::pin(async move {
                log.push(&undo_label);
            })
        }))
    }
}

struct FakeProvider {
    id: String,
    source: ProviderSource,
    provides: Vec<Claim>,
    requires: Vec<Claim>,
    /// (seam, key) pairs installed sequentially on activate.
    installs: Vec<(String, String)>,
    /// Fail AFTER this many successful installs (None = never fail).
    fail_after_installs: Option<usize>,
}

impl FakeProvider {
    fn new(id: &str, provides: Vec<Claim>) -> Self {
        Self {
            id: id.to_string(),
            source: ProviderSource::Native,
            provides,
            requires: Vec::new(),
            installs: Vec::new(),
            fail_after_installs: None,
        }
    }
    fn requires(mut self, r: Vec<Claim>) -> Self {
        self.requires = r;
        self
    }
    fn installs(mut self, pairs: &[(&str, &str)]) -> Self {
        self.installs = pairs
            .iter()
            .map(|(s, k)| (s.to_string(), k.to_string()))
            .collect();
        self
    }
    fn failing_after(mut self, n: usize) -> Self {
        self.fail_after_installs = Some(n);
        self
    }
    fn arc(self) -> Arc<dyn Provider> {
        Arc::new(self)
    }
}

impl ProviderSpec for FakeProvider {
    fn id(&self) -> &str {
        &self.id
    }
    fn source(&self) -> ProviderSource {
        self.source.clone()
    }
    fn provides(&self) -> &[Claim] {
        &self.provides
    }
    fn requires(&self) -> &[Claim] {
        &self.requires
    }
}
use operant_harness::ProviderSpec;

#[async_trait::async_trait]
impl Provider for FakeProvider {
    fn spec(&self) -> &dyn ProviderSpec {
        self
    }

    async fn activate(&self, cx: &mut ActivateCx<'_>) -> Result<(), HarnessError> {
        for (idx, (seam, key)) in self.installs.iter().enumerate() {
            if self.fail_after_installs == Some(idx) {
                return Err(HarnessError::ActivationFailed {
                    id: self.id.clone(),
                    message: "scripted mid-activate failure".to_string(),
                });
            }
            cx.install(seam, key).await?;
        }
        // A trailing failure after ALL installs still exercises staged rollback.
        // (Only meaningful when at least one install ran; an empty-install
        // provider has nothing staged to roll back.)
        if !self.installs.is_empty()
            && self
                .fail_after_installs
                .is_some_and(|limit| limit >= self.installs.len())
        {
            return Err(HarnessError::ActivationFailed {
                id: self.id.clone(),
                message: "scripted trailing failure".to_string(),
            });
        }
        Ok(())
    }
}

fn harness_with(log: &Log, seams: &[&str]) -> Harness {
    let mut h = Harness::new(KernelOptions { audit: false });
    for name in seams {
        h.add_seam(Arc::new(TestSeam::new(name, log.clone())));
    }
    h
}

async fn dump_ids(h: &Harness) -> Vec<String> {
    h.dump().await.providers.into_iter().map(|p| p.id).collect()
}

// ── T1: mount + LIFO unwind ────────────────────────────────────────────

#[tokio::test]
async fn t1_mount_active_then_lifo_unwind() {
    let log = Log::default();
    let h = harness_with(&log, &["tool"]);

    let p = FakeProvider::new("p1", vec![Claim::tool("a"), Claim::tool("b")])
        .installs(&[("tool", "a"), ("tool", "b")])
        .arc();
    let report = h.mount(p).await.unwrap();
    let MountReport::Mounted { activated } = report else {
        panic!("expected mounted");
    };
    assert_eq!(activated, vec!["p1"]);
    assert_eq!(
        h.state_of("p1").await,
        Some(operant_harness::ProviderState::Active)
    );
    assert_eq!(
        h.claim_owner(&Claim::tool("a")).await.as_deref(),
        Some("p1")
    );

    h.unmount("p1").await.unwrap();
    assert_eq!(h.state_of("p1").await, None);
    assert_eq!(
        log.snapshot(),
        vec![
            "install:p1:tool:a",
            "install:p1:tool:b",
            "undo:p1:tool:b",
            "undo:p1:tool:a",
        ]
    );
}

// ── T2: late binding ───────────────────────────────────────────────────

#[tokio::test]
async fn t2_pending_rescued_when_dependency_mounts() {
    let log = Log::default();
    let h = harness_with(&log, &["tool"]);

    let dependent = FakeProvider::new("dep", vec![Claim::tool("dep_cap")])
        .requires(vec![Claim::new("hook", "h1")])
        .installs(&[("tool", "dep_tool")])
        .arc();

    let report = h.mount(dependent).await.unwrap();
    let MountReport::Pending { missing } = report else {
        panic!("expected pending");
    };
    assert_eq!(missing, vec![Claim::new("hook", "h1")]);
    assert_eq!(
        h.state_of("dep").await,
        Some(operant_harness::ProviderState::Pending)
    );

    // Dependency arrives → dependent activates automatically.
    let provider_hook = FakeProvider::new("hookprov", vec![Claim::new("hook", "h1")]).arc();
    let report = h.mount(provider_hook).await.unwrap();
    let MountReport::Mounted { activated } = report else {
        panic!("expected mounted");
    };
    assert!(activated.contains(&"hookprov".to_string()));
    assert!(activated.contains(&"dep".to_string()));
    assert_eq!(
        h.state_of("dep").await,
        Some(operant_harness::ProviderState::Active)
    );
    assert!(
        log.snapshot()
            .contains(&"install:dep:tool:dep_tool".to_string())
    );
}

// ── T3: claim conflicts rejected BEFORE activation ─────────────────────

#[tokio::test]
async fn t3_claim_conflict_rejected_without_side_effects() {
    let log = Log::default();
    let h = harness_with(&log, &["tool"]);

    h.mount(
        FakeProvider::new("p1", vec![Claim::tool("x")])
            .installs(&[("tool", "x")])
            .arc(),
    )
    .await
    .unwrap();
    let err = h
        .mount(
            FakeProvider::new("p2", vec![Claim::tool("x")])
                .installs(&[("tool", "y")])
                .arc(),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, HarnessError::ClaimConflict { ref owner, .. } if owner == "p1"));
    // p2 never activated: no install side effects from it, not in tree.
    assert!(!log.snapshot().iter().any(|l| l.starts_with("install:p2")));
    assert!(!dump_ids(&h).await.contains(&"p2".to_string()));
}

// ── T4: partial activation failure contained ───────────────────────────

#[tokio::test]
async fn t4_partial_failure_unwinds_collected_effects() {
    let log = Log::default();
    let mut h = harness_with(&log, &["tool"]);
    // Script the tool seam to reject key "b".
    h.add_seam(Arc::new(TestSeam {
        name: "tool".to_string(),
        log: log.clone(),
        fail_keys: vec!["b".to_string()],
    }));

    let err = h
        .mount(
            FakeProvider::new("bad", vec![Claim::tool("never")])
                .installs(&[("tool", "a"), ("tool", "b"), ("tool", "c")])
                .arc(),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, HarnessError::ActivationFailed { ref id, .. } if id == "bad"));
    assert_eq!(
        log.snapshot(),
        vec!["install:bad:tool:a", "undo:bad:tool:a"]
    );
    assert_eq!(
        h.state_of("bad").await,
        Some(operant_harness::ProviderState::Failed)
    );
    assert_eq!(h.claim_owner(&Claim::tool("never")).await, None);
    assert_eq!(h.claim_owner(&Claim::tool("a")).await, None);
}

// ── T5: transactional replace — failure keeps old serving ──────────────

#[tokio::test]
async fn t5_replace_failure_leaves_old_intact() {
    let log = Log::default();
    let h = harness_with(&log, &["tool"]);

    let v1 = FakeProvider::new("svc", vec![Claim::tool("x")]).installs(&[("tool", "x")]);
    h.mount(v1.arc()).await.unwrap();
    assert_eq!(h.generation_of("svc").await, Some(1));

    // Successor installs a fresh capability then fails trailing.
    let v2 = FakeProvider::new("svc", vec![Claim::tool("x")])
        .installs(&[("tool", "x_new")])
        .failing_after(1);
    let err = h.replace(v2.arc()).await.unwrap_err();
    assert!(matches!(err, HarnessError::ActivationFailed { .. }));

    // Old instance fully serving: claim held, generation unchanged.
    assert_eq!(
        h.state_of("svc").await,
        Some(operant_harness::ProviderState::Active)
    );
    assert_eq!(h.generation_of("svc").await, Some(1));
    assert_eq!(
        h.claim_owner(&Claim::tool("x")).await.as_deref(),
        Some("svc")
    );
    // Staged effect was rolled back.
    assert!(log.snapshot().contains(&"undo:svc:tool:x_new".to_string()));
}

// ── T6: transactional replace — success flips atomically ──────────────

#[tokio::test]
async fn t6_replace_success_swaps_and_bumps_generation() {
    let log = Log::default();
    let h = harness_with(&log, &["tool"]);

    h.mount(
        FakeProvider::new("svc", vec![Claim::tool("x")])
            .installs(&[("tool", "x")])
            .arc(),
    )
    .await
    .unwrap();

    let v2 = FakeProvider::new("svc", vec![Claim::tool("x2")]).installs(&[("tool", "x2")]);
    h.replace(v2.arc()).await.unwrap();

    assert_eq!(h.generation_of("svc").await, Some(2));
    assert_eq!(h.claim_owner(&Claim::tool("x")).await, None);
    assert_eq!(
        h.claim_owner(&Claim::tool("x2")).await.as_deref(),
        Some("svc")
    );
    // Order proof: successor staged FIRST, old unwound at commit.
    let events = log.snapshot();
    let staged = events
        .iter()
        .position(|e| e == "install:svc:tool:x2")
        .unwrap();
    let old_undo = events.iter().position(|e| e == "undo:svc:tool:x").unwrap();
    assert!(staged < old_undo);
}

// ── T7: transitive cascade unmount ─────────────────────────────────────

#[tokio::test]
async fn t7_cascade_unmount_dependents_first() {
    let log = Log::default();
    let h = harness_with(&log, &["tool"]);

    h.mount(FakeProvider::new("base", vec![Claim::new("hook", "hb")]).arc())
        .await
        .unwrap();
    h.mount(
        FakeProvider::new("mid", vec![Claim::tool("mid_cap")])
            .requires(vec![Claim::new("hook", "hb")])
            .installs(&[("tool", "m")])
            .arc(),
    )
    .await
    .unwrap();
    h.mount(
        FakeProvider::new("leaf", vec![Claim::tool("leaf_cap")])
            .requires(vec![Claim::tool("mid_cap")])
            .installs(&[("tool", "l")])
            .arc(),
    )
    .await
    .unwrap();

    let unloaded = h.unmount("base").await.unwrap();
    assert_eq!(unloaded, vec!["leaf", "mid", "base"]); // dependents first
    assert!(dump_ids(&h).await.is_empty());
    let events = log.snapshot();
    let undo_m = events.iter().position(|e| e == "undo:mid:tool:m").unwrap();
    let undo_base_none = events.iter().filter(|e| e.starts_with("undo:base")).count();
    assert!(undo_m < events.len());
    assert_eq!(undo_base_none, 0); // base installed nothing
}

// ── T8: concurrent mounts serialize cleanly ────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn t8_concurrent_mounts_all_land() {
    let log = Log::default();
    let h = Arc::new(harness_with(&log, &["tool"]));

    let mut handles = Vec::new();
    for i in 0..30 {
        let h = Arc::clone(&h);
        handles.push(tokio::spawn(async move {
            let p = FakeProvider::new(&format!("c{i}"), vec![Claim::tool(format!("t{i}"))])
                .installs(&[("tool", &format!("t{i}"))])
                .arc();
            h.mount(p).await.unwrap();
            assert!(matches!(
                h.state_of(&format!("c{i}")).await,
                Some(operant_harness::ProviderState::Active)
            ));
        }));
    }
    for handle in handles {
        handle.await.unwrap();
    }
    let tree = h.dump().await;
    assert_eq!(tree.providers.len(), 30);
    assert_eq!(tree.claims.len(), 30);
}

// ── T9: dump shape ────────────────────────────────────────────────────────

#[tokio::test]
async fn t9_dump_is_sorted_and_serializable() {
    let log = Log::default();
    let h = harness_with(&log, &["tool", "hook"]);
    h.mount(FakeProvider::new("zeta", vec![Claim::tool("z")]).arc())
        .await
        .unwrap();
    h.mount(FakeProvider::new("alpha", vec![Claim::tool("a")]).arc())
        .await
        .unwrap();

    let tree: DumpTree = h.dump().await;
    let ids: Vec<&str> = tree.providers.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, vec!["zeta", "alpha"]); // insertion seq order, not alphabetical
    let keys: Vec<&str> = tree.claims.iter().map(|c| c.claim.key.as_str()).collect();
    assert_eq!(keys, vec!["a", "z"]); // claims sorted by key
    let json = tree.to_json();
    assert!(json.contains("\"state\": \"active\""));
}

// ── T10: not-found errors ──────────────────────────────────────────────

#[tokio::test]
async fn t10_unknown_ids_error_cleanly() {
    let log = Log::default();
    let h = harness_with(&log, &["tool"]);
    assert!(matches!(
        h.unmount("ghost").await.unwrap_err(),
        HarnessError::NotFound(_)
    ));
    assert!(matches!(
        h.replace(FakeProvider::new("ghost", vec![Claim::tool("g")]).arc())
            .await
            .unwrap_err(),
        HarnessError::NotFound(_)
    ));
}

// ── T11: multiple pendings rescued in fair order ───────────────────────

#[tokio::test]
async fn t11_fair_pending_rescue() {
    let log = Log::default();
    let h = harness_with(&log, &["tool"]);

    let x = FakeProvider::new("x", vec![Claim::tool("xc")])
        .requires(vec![Claim::new("hook", "h1")])
        .arc();
    let y = FakeProvider::new("y", vec![Claim::tool("yc")])
        .requires(vec![Claim::new("hook", "h2")])
        .arc();
    h.mount(x).await.unwrap();
    h.mount(y).await.unwrap();

    // Only h1 arrives → exactly x activates; y stays parked.
    h.mount(FakeProvider::new("h1prov", vec![Claim::new("hook", "h1")]).arc())
        .await
        .unwrap();
    assert_eq!(
        h.state_of("x").await,
        Some(operant_harness::ProviderState::Active)
    );
    assert_eq!(
        h.state_of("y").await,
        Some(operant_harness::ProviderState::Pending)
    );

    // h2 arrives → y activates via late binding.
    h.mount(FakeProvider::new("h2prov", vec![Claim::new("hook", "h2")]).arc())
        .await
        .unwrap();
    assert_eq!(
        h.state_of("y").await,
        Some(operant_harness::ProviderState::Active)
    );
}

// ── T12: missing seam surfaces as activation failure ───────────────────

#[tokio::test]
async fn t12_missing_seam_contained() {
    let log = Log::default();
    let h = harness_with(&log, &[]); // no seams registered
    let err = h
        .mount(
            FakeProvider::new("lonely", vec![Claim::tool("t")])
                .installs(&[("tool", "t")])
                .arc(),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, HarnessError::ActivationFailed { .. }));
    assert_eq!(
        h.state_of("lonely").await,
        Some(operant_harness::ProviderState::Failed)
    );
    assert!(log.snapshot().is_empty());
}

// ── T13: replace drops a claim ⇒ dependents cascade AFTER commit ──────

#[tokio::test]
async fn t13_replace_cascades_dependents_that_lost_claims() {
    let log = Log::default();
    let h = harness_with(&log, &["tool"]);

    // svc owns hook/h AND tool/x; dependent requires BOTH.
    h.mount(FakeProvider::new("svc", vec![Claim::new("hook", "h"), Claim::tool("x")]).arc())
        .await
        .unwrap();
    h.mount(
        FakeProvider::new("dependent", vec![Claim::tool("d")])
            .requires(vec![Claim::new("hook", "h"), Claim::tool("x")])
            .arc(),
    )
    .await
    .unwrap();

    // Successor keeps hook/h but DROPS tool/x → dependent orphans post-commit.
    let v2 = FakeProvider::new("svc", vec![Claim::new("hook", "h")]).arc();
    h.replace(v2).await.unwrap();

    // Successor live at generation 2 holding its remaining claim.
    assert_eq!(
        h.state_of("svc").await,
        Some(operant_harness::ProviderState::Active)
    );
    assert_eq!(h.generation_of("svc").await, Some(2));
    assert_eq!(
        h.claim_owner(&Claim::new("hook", "h")).await.as_deref(),
        Some("svc")
    );
    assert_eq!(h.claim_owner(&Claim::tool("x")).await, None);
    // Orphaned dependent cascaded away after commit.
    assert_eq!(h.state_of("dependent").await, None);
}
