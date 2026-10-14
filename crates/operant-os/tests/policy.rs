//! policy.json enforcement tests (boundary doc §5 invariants 3 and 4, phase C).
//!
//! Invariant 3 (scoped): a call the policy refuses never executes, and the
//! refusal is told to the model + recorded — here the refuse-and-tell half is
//! the loop's existing F2 semantics; these tests prove the COMPILE side: the
//! document becomes a SeatPolicy whose verdicts deny/escalate the right calls,
//! and the pure `decide` floor behaves under both postures.

use std::path::PathBuf;

use operant_core::org::seat_policy::{SeatDecision, SeatMode, decide};
use operant_os::policy::PolicyDoc;

fn doc_json(posture: &str, deny: &[&str]) -> String {
    let deny = serde_json::json!(deny);
    format!(
        r#"{{
        "version": 1, "grade": 2, "stage": "execute", "posture": "{posture}",
        "roots": ["/tmp/w"], "writes": ["hello.txt"],
        "commands": ["ls", "git"], "deny": {deny},
        "budget": {{"tokens": 8000, "cost": null}}
    }}"#
    )
}

fn write_doc(tag: &str, json: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("operant-os-policy-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("policy.json"), json).unwrap();
    dir
}

#[test]
fn observe_posture_locks_the_seat_to_the_observer_tools() {
    let dir = write_doc("obs", &doc_json("observe", &[]));
    let doc = PolicyDoc::load(&dir).unwrap();
    let policy = doc.seat_policy();
    assert_eq!(policy.mode, SeatMode::Lockdown);
    let read_ok = decide(Some(&policy), "file_read", false, false, false);
    assert!(matches!(read_ok, SeatDecision::Run(_)), "an observer reads");
    let write_denied = decide(Some(&policy), "file_write", false, true, false);
    assert!(
        matches!(write_denied, SeatDecision::Escalate(_)),
        "a mutating tool escalates (unattended: queued + denied) — never runs"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn act_posture_yolo_with_the_documents_deny_list() {
    let dir = write_doc("act", &doc_json("act", &["image_generate"]));
    let doc = PolicyDoc::load(&dir).unwrap();
    let policy = doc.seat_policy();
    assert_eq!(policy.mode, SeatMode::Yolo);
    let write_ok = decide(Some(&policy), "file_write", false, true, false);
    assert!(
        matches!(write_ok, SeatDecision::Run(_)),
        "an approved plan writes"
    );
    let denied = decide(Some(&policy), "image_generate", false, false, false);
    assert!(
        matches!(denied, SeatDecision::Deny(_)),
        "the deny list still wins under yolo"
    );
    let hardline = decide(Some(&policy), "terminal", true, true, false);
    assert!(
        matches!(hardline, SeatDecision::Deny(_)),
        "the blocklist fires before any mode"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn budget_rides_along_in_the_document() {
    let dir = write_doc("budget", &doc_json("act", &[]));
    let doc = PolicyDoc::load(&dir).unwrap();
    assert_eq!(doc.budget.tokens, Some(8000));
    assert_eq!(doc.budget.cost, None);
    assert_eq!(doc.writes, vec!["hello.txt".to_string()]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_missing_or_bad_document_fails_closed() {
    let empty =
        std::env::temp_dir().join(format!("operant-os-policy-missing-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&empty);
    std::fs::create_dir_all(&empty).unwrap();
    assert!(
        PolicyDoc::load(&empty).is_err(),
        "no document = an error, never a permissive default"
    );
    let bad = write_doc(
        "bad",
        "{\"version\": 99, \"posture\": \"observe\", \"budget\": {}}",
    );
    match PolicyDoc::load(&bad) {
        Err(operant_os::Error::PolicyVersion(99)) => {}
        other => panic!("expected PolicyVersion(99), got {other:?}"),
    }
    let weird = write_doc(
        "weird",
        "{\"version\": 1, \"posture\": \"sideways\", \"budget\": {}}",
    );
    match PolicyDoc::load(&weird) {
        Err(operant_os::Error::PolicyPosture(p)) => assert_eq!(p, "sideways"),
        other => panic!("expected PolicyPosture, got {other:?}"),
    }
    let _ = std::fs::remove_dir_all(&empty);
    let _ = std::fs::remove_dir_all(&bad);
    let _ = std::fs::remove_dir_all(&weird);
}
