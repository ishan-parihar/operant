//! Wave 1 packet B — the fail-closed org identity gate on the cron tick.
//!
//! Spec: `docs/WAVE1-DECISIONS.md` §3.2.
//! Acceptance: `docs/ORGANISM-OS-UPGRADE-OUTLINE.md` §4 Wave 1 — *"a job
//! missing any of the identity fields **blocks the tick** (not a warning)."*
//!
//! ## What these tests are for
//!
//! The unit tests inside `org/identity_gate.rs` prove the gate returns the
//! right *decision*. These prove the part that actually matters and that a
//! pure function cannot: that a **real, due, script-backed cron job does not
//! dispatch** when its identity is incomplete, and that it still does when the
//! org layer is off.
//!
//! Dispatch is observed through the filesystem, not through a mock. A blocked
//! job is a `no_agent` script that would `touch` a marker file; the test
//! asserts the marker is absent after the tick. That makes the assertion
//! unfakeable by the production code — there is no seam through which the
//! scheduler could "record that it would have run" and still pass.
//!
//! ## Why a script job and not an agent job
//!
//! `no_agent` jobs dispatch to `sh -c <script>` and never touch the model
//! client, so these tests are hermetic: no API key, no network, no provider.
//! The gate runs *before* that branch, so the choice of job kind does not
//! weaken what is under test.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use operant_core::agent::OperantAgent;
use operant_core::agent::clients::openai::OpenAIModelClient;
use operant_core::client::ClientConfig;
use operant_core::client::OpenAIClient;
use operant_core::cronjobs::{CreateJobParams, CronDb, CronScheduler};
use operant_core::database::Database;
use operant_core::org::employee::{Employee, derive_employee_id};
use operant_core::org::identity_gate::{
    EmployeeLookup, GateDecision, IdentityGate, REQUIRED_IDENTITY_FIELDS,
};
use operant_core::tools::ToolRegistry;
use tempfile::TempDir;
use tokio::time::Duration;

/// A temp dir that owns the cron db, the kanban-era org db, the agent db, and
/// the marker file for one test.
struct Fixture {
    _dir: TempDir,
    db: Arc<CronDb>,
    agent: Arc<OperantAgent>,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = Arc::new(CronDb::init(dir.path().join("cron.db")).expect("CronDb::init"));
    let agent_db =
        Arc::new(Database::init(dir.path().join("agent.sqlite")).expect("Database::init"));
    let agent = Arc::new(OperantAgent::new(
        operant_core::agent::AgentConfig::default(),
        Box::new(OpenAIModelClient::new(OpenAIClient::new(
            ClientConfig::default(),
        ))),
        ToolRegistry::new(Duration::from_secs(1)),
        agent_db,
    ));
    Fixture {
        _dir: dir,
        db,
        agent,
    }
}

/// A `no_agent` job whose script touches `marker`. If the tick dispatches it,
/// the marker appears. `name` is the job's display name (the `name` required
/// identity field); `skills` is the other required field we vary.
fn script_job(
    name: &str,
    skills: Option<Vec<String>>,
    marker: &std::path::Path,
) -> CreateJobParams {
    CreateJobParams {
        name: name.to_string(),
        prompt: "irrelevant: this job never reaches the agent".to_string(),
        // Every minute past :00 — combined with the forced next_run_at below,
        // the job is due immediately.
        schedule: "* * * * *".to_string(),
        schedule_display: "every minute".to_string(),
        repeat_times: None,
        deliver: "local".to_string(),
        origin_platform: None,
        origin_chat_id: None,
        origin_thread_id: None,
        skill: None,
        skills,
        model: None,
        provider: None,
        base_url: None,
        script: Some(format!("touch {}", marker.display())),
        context_from: None,
        enabled_toolsets: None,
        workdir: None,
        no_agent: true,
    }
}

/// A complete, valid employee row for `job_id` — the "backfill succeeded" case.
fn complete_employee(job_id: &str) -> Employee {
    Employee {
        employee_id: derive_employee_id(job_id),
        name: "Nightly digest".to_string(),
        role: "automator".to_string(),
        department: None,
        skills: vec!["summarize".to_string()],
        agent_type: None,
        persona: None,
        system_prompt: None,
        status: "active".to_string(),
        reason: "test".to_string(),
        created_at: "2026-09-30T00:00:00Z".to_string(),
        updated_at: "2026-09-30T00:00:00Z".to_string(),
    }
}

/// An [`EmployeeLookup`] backed by a fixed map, standing in for packet A's
/// `EmployeeDb`. Keyed by cron job id, exactly like the trait contract.
struct MapLookup(std::collections::HashMap<String, Employee>);

impl EmployeeLookup for MapLookup {
    fn lookup_employee(&self, cron_job_id: &str) -> Option<Employee> {
        self.0.get(cron_job_id).cloned()
    }
}

fn gate_over(entries: Vec<(&str, Employee)>) -> Arc<IdentityGate> {
    let map = entries
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
    Arc::new(IdentityGate::new(Box::new(MapLookup(map))))
}

// ── The acceptance criterion: a blocked job must not dispatch ────────────

#[tokio::test]
async fn job_with_complete_employee_ticks_and_dispatches() {
    let f = fixture();
    let marker = f._dir.path().join("ran");
    let job_id =
        f.db.create_job(script_job(
            "Nightly digest",
            Some(vec!["summarize".to_string()]),
            &marker,
        ))
        .expect("create_job");
    force_due(&f.db, &job_id);
    let gate = gate_over(vec![(&job_id, complete_employee(&job_id))]);

    let sched = CronScheduler::new(f.db.clone(), f.agent.clone()).with_org_gate(gate);
    sched.tick().await.expect("tick");

    assert!(
        marker.exists(),
        "a job with a complete employee must dispatch (the gate allowed it)"
    );
}

#[tokio::test]
async fn job_with_no_employee_record_is_blocked_and_never_dispatches() {
    let f = fixture();
    let marker = f._dir.path().join("ran");
    let job_id =
        f.db.create_job(script_job(
            "Orphan job",
            Some(vec!["x".to_string()]),
            &marker,
        ))
        .expect("create_job");
    force_due(&f.db, &job_id);
    // Gate installed, but the registry has no row for this job at all.
    let gate = gate_over(vec![]);

    let sched = CronScheduler::new(f.db.clone(), f.agent.clone()).with_org_gate(gate);
    sched.tick().await.expect("tick");

    assert!(
        !marker.exists(),
        "a job with no employee record must NOT dispatch (fail closed)"
    );
    let err = last_error(&f.db, &job_id);
    assert!(
        err.contains("blocked by org gate"),
        "last_error must carry the greppable block marker, got: {err}"
    );
    assert!(
        err.contains(&job_id),
        "the error must name the offending job, got: {err}"
    );
}

#[tokio::test]
async fn job_missing_skills_is_blocked_and_never_dispatches() {
    let f = fixture();
    let marker = f._dir.path().join("ran");
    // The cron job itself has skills, but its employee row was backfilled with
    // an empty list — §3.2's measured 6-job block set, exactly.
    let job_id =
        f.db.create_job(script_job(
            "No-skill job",
            Some(vec!["x".to_string()]),
            &marker,
        ))
        .expect("create_job");
    force_due(&f.db, &job_id);
    let mut emp = complete_employee(&job_id);
    emp.skills = Vec::new();
    let gate = gate_over(vec![(&job_id, emp)]);

    let sched = CronScheduler::new(f.db.clone(), f.agent.clone()).with_org_gate(gate);
    sched.tick().await.expect("tick");

    assert!(
        !marker.exists(),
        "a job whose employee has empty skills must NOT dispatch"
    );
    let err = last_error(&f.db, &job_id);
    assert!(
        err.contains("skills"),
        "the error must name the field: {err}"
    );
}

#[tokio::test]
async fn job_missing_name_is_blocked_and_never_dispatches() {
    let f = fixture();
    let marker = f._dir.path().join("ran");
    let job_id =
        f.db.create_job(script_job(
            "Has a name",
            Some(vec!["x".to_string()]),
            &marker,
        ))
        .expect("create_job");
    force_due(&f.db, &job_id);
    let mut emp = complete_employee(&job_id);
    emp.name = "   ".to_string(); // whitespace-only: "non-empty after trim"
    let gate = gate_over(vec![(&job_id, emp)]);

    let sched = CronScheduler::new(f.db.clone(), f.agent.clone()).with_org_gate(gate);
    sched.tick().await.expect("tick");

    assert!(
        !marker.exists(),
        "a job whose employee has a blank name must NOT dispatch"
    );
    assert!(
        last_error(&f.db, &job_id).contains("name"),
        "the error must name the field"
    );
}

// ── Dark-mergeability: org layer off is byte-identical to today ──────────

#[tokio::test]
async fn with_org_layer_off_an_unbackfilled_job_still_dispatches() {
    let f = fixture();
    let marker = f._dir.path().join("ran");
    // A job with NO skills and NO employee — exactly a pre-upgrade operant
    // install. It must keep working, or the gate would be an outage on merge.
    let job_id =
        f.db.create_job(script_job("Legacy job", None, &marker))
            .expect("create_job");
    force_due(&f.db, &job_id);

    // No .with_org_gate(...) — this is the default construction.
    let sched = CronScheduler::new(f.db.clone(), f.agent.clone());
    sched.tick().await.expect("tick");

    assert!(
        marker.exists(),
        "with the org layer off, behavior must be unchanged: the job dispatches"
    );
}

#[tokio::test]
async fn with_org_layer_off_the_gate_is_never_consulted() {
    let f = fixture();
    let marker = f._dir.path().join("ran");
    let job_id =
        f.db.create_job(script_job("Legacy job", None, &marker))
            .expect("create_job");
    force_due(&f.db, &job_id);
    // A gate that would block everything, installed nowhere. If the tick path
    // reached a gate by default, this job would be stopped — it is not.
    let sched = CronScheduler::new(f.db.clone(), f.agent.clone());
    sched.tick().await.expect("tick");
    assert!(marker.exists());
    let err = last_error(&f.db, &job_id);
    assert!(
        !err.contains("blocked by org gate"),
        "org layer off must produce no gate output at all, got: {err}"
    );
}

// ── The one-case-per-field battery, at the gate (not the tick) ───────────

/// Parameterized over §3.2's exact required-field list: drop each field in
/// turn, assert the gate blocks and names that field. Driven off
/// `REQUIRED_IDENTITY_FIELDS` so the test cannot silently drift from the spec
/// list — if §3.2 grows a field, this test grows a case.
#[test]
fn every_required_field_blocks_when_dropped() {
    for field in REQUIRED_IDENTITY_FIELDS {
        let mut emp = complete_employee("cron_1234abcd");
        match field {
            "employee_id" => emp.employee_id = "emp-wrong-row".to_string(),
            "name" => emp.name = String::new(),
            "skills" => emp.skills = vec![],
            other => panic!("unhandled required field {other}"),
        }
        let gate = gate_over(vec![("cron_1234abcd", emp)]);
        let decision = gate.check("cron_1234abcd", &derive_employee_id("cron_1234abcd"));
        let GateDecision::Block(block) = decision else {
            panic!("dropping {field} must block, got {decision:?}");
        };
        let msg = block.message();
        assert!(
            msg.contains(field),
            "block for {field} must name the field in its message: {msg}"
        );
    }
}

/// The reverse control: a fully valid employee is allowed. Without this, a
/// gate that blocked *everything* would pass the battery above.
#[test]
fn complete_employee_is_allowed_by_the_gate() {
    let gate = gate_over(vec![("cron_1234abcd", complete_employee("cron_1234abcd"))]);
    let decision = gate.check("cron_1234abcd", &derive_employee_id("cron_1234abcd"));
    assert!(
        matches!(decision, GateDecision::Allow { .. }),
        "a complete employee must be allowed, got {decision:?}"
    );
}

// ── helpers ──────────────────────────────────────────────────────────────

/// Push `next_run_at` into the past so `get_due_jobs` selects the job.
fn force_due(db: &CronDb, job_id: &str) {
    db.update_job(
        job_id,
        std::collections::HashMap::from([(
            "next_run_at".to_string(),
            Some(serde_json::json!("2000-01-01T00:00:00Z")),
        )]),
    )
    .expect("force job due");
}

/// The job's persisted `last_error`, which is where a block is surfaced.
fn last_error(db: &CronDb, job_id: &str) -> String {
    db.get_job(job_id)
        .expect("get_job")
        .expect("job exists")
        .last_error
        .unwrap_or_default()
}
