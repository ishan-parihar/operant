//! The enforcement chain, deterministically (phase C's regression battery).
//!
//! smoke-verified live: the mount fires on a real Holon claim; the historical
//! bug these tests pin is the smoke2 failure — the authority mounted but the
//! consult keyed on a seat that never matched (session UUID vs the exported
//! employee), so the policy silently governed nothing.

use std::sync::Arc;

use operant_core::org::seat_authority::SeatAuthority;
use operant_core::org::seat_policy::{SeatDecision, SeatPolicySource};

use operant_os::policy::PolicyDoc;

fn observe_authority(home: &std::path::Path) -> Arc<SeatAuthority> {
    let dir = home.join("run");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("policy.json"),
        r#"{"version":1,"grade":null,"stage":"shape","posture":"observe",
            "roots":[],"writes":[],"commands":[],"deny":[],
            "budget":{"tokens":null,"cost":null}}"#,
    )
    .unwrap();
    let policy = PolicyDoc::load(&dir).unwrap();
    Arc::new(SeatAuthority::new(Arc::new(TestSource { policy })))
}

struct TestSource {
    policy: PolicyDoc,
}

impl SeatPolicySource for TestSource {
    fn policy_for(&self, employee_id: &str) -> Option<operant_core::org::seat_policy::SeatPolicy> {
        (employee_id == "emp_probe").then(|| self.policy.seat_policy())
    }
}

#[test]
fn the_consult_matches_the_exported_employee_and_refuses_mutations() {
    let home = std::env::temp_dir().join(format!("os-enf-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let authority = observe_authority(&home);

    // the consult that must ESCALATE (the loop denies it with no approver)
    let verdict = authority.consult("emp_probe", "file_write", true);
    assert!(
        matches!(verdict, Some(SeatDecision::Escalate(_))),
        "an observe seat escalates a mutating tool, got {verdict:?}"
    );
    // the smoke2 regression: a seat the policy does not name is ungoverned
    let stranger = authority.consult("some-session-uuid", "file_write", true);
    assert_eq!(
        stranger, None,
        "a non-matching seat is ungoverned — the source must key on the employee the kernel exported"
    );
    // an observer's reads run
    let read = authority.consult("emp_probe", "file_read", false);
    assert!(matches!(read, Some(SeatDecision::Run(_))));
    let _ = std::fs::remove_dir_all(&home);
}
