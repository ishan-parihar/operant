//! W3 acceptance (audit O-17, O-18, O-19): the three enforcement organs
//! behave as the plan's phase C/D acceptance names them — through the real
//! code paths (the recorder, the write fence, the budget check), not mocks
//! of their internals.

use std::path::Path;

fn holon_run_dir_with(policy: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let run = dir.path().join("run");
    std::fs::create_dir_all(&run).expect("run dir");
    std::fs::write(run.join("policy.json"), policy).expect("policy");
    (dir, run)
}

/// O-17: record_refusal appends one line; read_refusals returns it; the
/// refusal merge into result.json is exercised through prompting's writer
/// in the e2e suite (which runs the real agent) — here the file contract.
#[test]
fn a_recorded_refusal_round_trips_through_the_run_dir() {
    let (_dir, run) = holon_run_dir_with("{}");
    operant_core::org::record_refusal(&run, "file_write", "outside roots", "policy.json:roots");
    operant_core::org::record_refusal(&run, "terminal", "command not allowed", "policy.json:deny");
    let refusals = operant_core::org::read_refusals(&run);
    assert_eq!(refusals.len(), 2, "both refusals recorded: {refusals:?}");
    assert_eq!(refusals[0]["tool"], "file_write");
    assert_eq!(refusals[1]["policy_rule"], "policy.json:deny");
}

/// O-18: a write outside the policy roots is refused and recorded; a write
/// inside them (the run dir) passes; a read is never fenced; a personal
/// run (no env) is never fenced.
// SAFETY: env-var mutation is serialized across all W3 tests.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn the_write_fence_refuses_outside_and_passes_inside() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let (_dir, run) = holon_run_dir_with(
        r#"{"version":1,"stage":"execute","posture":"act","roots":["/tmp"],"writes":[],"commands":[],"deny":[],"budget":{"tokens":null,"cost":null}}"#,
    );
    // SAFETY: serialized test; the run env is the only variable.
    unsafe { std::env::set_var("HOLON_RUN_DIR", &run) };

    // inside the roots (/tmp) and inside the run dir: both pass
    assert!(operant_core::tools::file_tools::holon_write_fence("/tmp/w3-probe.txt").is_ok());
    let in_run = run.join("result.json");
    assert!(operant_core::tools::file_tools::holon_write_fence(in_run.to_str().unwrap()).is_ok());

    // outside both: refused, and the refusal is on the record
    let outside = "/etc/w3-fence-probe.txt";
    let err = operant_core::tools::file_tools::holon_write_fence(outside).expect_err("must refuse");
    assert!(err.contains("outside the run's policy roots"), "{err}");
    let refusals = operant_core::org::read_refusals(&run);
    assert!(
        refusals
            .iter()
            .any(|r| r["tool"] == "file_write" && r["policy_rule"] == "policy.json:roots"),
        "the fence recorded its refusal: {refusals:?}"
    );

    // the file was never created
    assert!(
        !Path::new(outside).exists(),
        "a refused write never happens"
    );

    unsafe { std::env::remove_var("HOLON_RUN_DIR") };

    // personal run (no env): never fenced
    assert!(operant_core::tools::file_tools::holon_write_fence("/anywhere/at-all.txt").is_ok());
}

/// O-18: an empty `roots` list means "the run directory only" — the same
/// semantics the kernel's workdir guard uses. Writes inside the run pass,
/// outside are refused.
#[test]
fn an_empty_roots_list_fences_to_the_run_dir() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let (_dir, run) = holon_run_dir_with(
        r#"{"version":1,"stage":"plan","posture":"observe","roots":[],"writes":[],"commands":[],"deny":[],"budget":{"tokens":null,"cost":null}}"#,
    );
    // SAFETY: serialized test.
    unsafe { std::env::set_var("HOLON_RUN_DIR", &run) };
    let in_run = run.join("scratch.txt");
    assert!(operant_core::tools::file_tools::holon_write_fence(in_run.to_str().unwrap()).is_ok());
    assert!(
        operant_core::tools::file_tools::holon_write_fence("/tmp/outside-the-run.txt").is_err(),
        "empty roots = the run dir is the only writable place"
    );
    unsafe { std::env::remove_var("HOLON_RUN_DIR") };
}

/// O-19: the budget cap reads from the policy pointer.
#[test]
fn the_budget_cap_reads_the_policy_pointer() {
    let (_dir, run) = holon_run_dir_with(
        r#"{"version":1,"stage":"execute","posture":"act","roots":[],"writes":[],"commands":[],"deny":[],"budget":{"tokens":5000,"cost":null}}"#,
    );
    assert_eq!(operant_core::org::holon_budget_cap(&run), Some(5000));
    let (_d2, run2) = holon_run_dir_with(
        r#"{"version":1,"stage":"execute","posture":"act","roots":[],"writes":[],"commands":[],"deny":[],"budget":{"tokens":null,"cost":null}}"#,
    );
    assert_eq!(
        operant_core::org::holon_budget_cap(&run2),
        None,
        "no cap = ungoverned"
    );
}
