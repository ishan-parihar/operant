//! Integration tests for the Wave 1 employee registry (packet A).
//!
//! Spec: `docs/WAVE1-DECISIONS.md` §3.1, §3.1.1, §3.1.2.
//!
//! The five properties this file exists to pin, in order of how badly a
//! regression would hurt:
//!
//! 1. a realistic `CronJob` backfills to a valid employee;
//! 2. backfill is idempotent — run twice, same rows, same ids;
//! 3. `employee_id` derivation matches the organism's convention;
//! 4. empty `skills` backfills to `[]` and is **not** auto-repaired
//!    (the fail-closed contract — mutation-proven red, see the report);
//! 5. `AgentType::None` round-trips distinctly from every variant.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use operant_core::cronjobs::db::CronJob;
use operant_core::org::employee::{BACKFILL_REASON, DEFAULT_ROLE, STATUS_ACTIVE};
use operant_core::org::{AgentType, Employee, EmployeeDb, derive_employee_id, org_db_path};
use tempfile::TempDir;

/// A temp registry at the real path shape (`<dir>/operant_kanban.db`),
/// derived by the same rule production uses.
fn temp_registry() -> (EmployeeDb, TempDir) {
    let dir = tempfile::tempdir().expect("tempdir failed");
    // The real caller passes the main `database_path`; mirror that.
    let db = EmployeeDb::init(org_db_path(&dir.path().join("database.db"))).expect("init failed");
    (db, dir)
}

/// A realistic job: all 31 fields of the live `CronJob`
/// (`cronjobs/db.rs:37-69`), shaped like a real `operant` scheduled job.
/// The organism's own `emp-a02e3f692fb0` is the model — automator,
/// three skills, `*/15 * * * *`, agent_type `service`.
fn realistic_job(id: &str) -> CronJob {
    CronJob {
        id: id.to_string(),
        name: "VPS Health Watchdog".to_string(),
        prompt: "check the vps and report".to_string(),
        schedule: "*/15 * * * *".to_string(),
        schedule_display: "*/15 * * * *".to_string(),
        repeat_times: None,
        repeat_completed: 0,
        deliver: "local".to_string(),
        origin_platform: Some("telegram".to_string()),
        origin_chat_id: Some("12345".to_string()),
        origin_thread_id: None,
        skill: Some("vps-infra-audit".to_string()),
        skills: Some(vec![
            "vps-infra-audit".to_string(),
            "automaton-automation-substrate".to_string(),
            "axe-command-center".to_string(),
        ]),
        model: Some("gpt-4".to_string()),
        provider: Some("openai".to_string()),
        base_url: None,
        script: None,
        context_from: Some(vec!["sessions".to_string()]),
        enabled_toolsets: None,
        workdir: Some("/home/operant".to_string()),
        no_agent: false,
        enabled: true,
        state: "scheduled".to_string(),
        paused_at: None,
        paused_reason: None,
        created_at: "2026-09-01T00:00:00+00:00".to_string(),
        next_run_at: Some("2026-10-01T04:25:00+05:30".to_string()),
        last_run_at: None,
        last_status: None,
        last_error: None,
        last_delivery_error: None,
    }
}

// ── 1. a realistic job backfills to a valid employee ─────────────────────

#[test]
fn org_employee_backfill_realistic_job_produces_valid_employee() {
    let (db, _dir) = temp_registry();
    let job = realistic_job("a02e3f692fb0");

    let report = db
        .backfill_from_cron_jobs(
            std::slice::from_ref(&job),
            "2026-09-30T12:00:00+00:00",
            BACKFILL_REASON,
        )
        .expect("backfill ok");

    assert_eq!(report.jobs_seen, 1);
    assert_eq!(report.employees_written, 1);
    assert_eq!(report.links_written, 1);
    assert!(report.invalid.is_empty(), "valid job must not be invalid");
    assert!(report.collisions.is_empty());

    let emp = db
        .get_employee("emp-a02e3f692fb0")
        .expect("read ok")
        .expect("employee must exist");

    // §3.1.1 field map, column by column.
    assert_eq!(emp.name, "VPS Health Watchdog"); // ← job.name
    assert_eq!(emp.role, DEFAULT_ROLE); // ← constant "automator"
    assert_eq!(emp.department, None); // ← None (no dept concept yet)
    assert_eq!(
        emp.skills,
        vec![
            "vps-infra-audit".to_string(),
            "automaton-automation-substrate".to_string(),
            "axe-command-center".to_string()
        ]
    ); // ← job.skills
    assert_eq!(emp.agent_type, None); // ← absent
    assert_eq!(emp.persona, None); // ← NULL until Wave 2
    assert_eq!(emp.status, STATUS_ACTIVE); // ← job.state == "scheduled"
    assert_eq!(emp.reason, BACKFILL_REASON); // ← required on every row
    assert_eq!(emp.created_at, "2026-09-01T00:00:00+00:00"); // ← job.created_at
    assert_eq!(emp.updated_at, "2026-09-30T12:00:00+00:00"); // ← now

    assert!(
        emp.is_valid(),
        "required set: {:?}",
        emp.missing_required_fields()
    );

    // The link row carries schedule + enabled, not the prompt/model.
    let links = db
        .list_employee_cron_jobs("emp-a02e3f692fb0")
        .expect("links ok");
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].cron_job_id, "a02e3f692fb0");
    assert_eq!(links[0].schedule.as_deref(), Some("*/15 * * * *"));
    assert!(links[0].enabled);
}

#[test]
fn org_employee_backfill_carries_paused_state_verbatim() {
    let (db, _dir) = temp_registry();
    let mut job = realistic_job("paused0001");
    job.state = "paused".to_string();
    job.paused_at = Some("2026-09-20T03:00:00+00:00".to_string());

    db.backfill_from_cron_jobs(&[job], "2026-09-30T12:00:00+00:00", BACKFILL_REASON)
        .expect("backfill ok");

    let emp = db
        .get_employee("emp-paused0001")
        .expect("read ok")
        .expect("employee must exist");
    // Only "scheduled" collapses to "active"; anything else is carried so
    // a paused employee stays visibly paused.
    assert_eq!(emp.status, "paused");
}

#[test]
fn org_employee_backfill_disabled_job_records_enabled_false() {
    let (db, _dir) = temp_registry();
    let mut job = realistic_job("disabled01");
    job.enabled = false;

    db.backfill_from_cron_jobs(&[job], "2026-09-30T12:00:00+00:00", BACKFILL_REASON)
        .expect("backfill ok");

    let links = db
        .list_employee_cron_jobs("emp-disabled01")
        .expect("links ok");
    assert_eq!(links.len(), 1);
    assert!(
        !links[0].enabled,
        "enabled=false must survive the round trip"
    );
}

// ── 2. backfill is idempotent ────────────────────────────────────────────

#[test]
fn org_employee_backfill_is_idempotent() {
    let (db, _dir) = temp_registry();
    let jobs = vec![
        realistic_job("a02e3f692fb0"),
        realistic_job("d4d71fccfed6"),
        realistic_job("cron_32365296"),
    ];

    let first = db
        .backfill_from_cron_jobs(&jobs, "2026-09-30T12:00:00+00:00", BACKFILL_REASON)
        .expect("first backfill ok");
    let rows_first = db.list_employees().expect("list ok");
    let links_first: Vec<_> = jobs
        .iter()
        .flat_map(|j| {
            db.list_employee_cron_jobs(&derive_employee_id(&j.id))
                .expect("links ok")
        })
        .collect();

    let second = db
        .backfill_from_cron_jobs(&jobs, "2026-09-30T12:00:00+00:00", BACKFILL_REASON)
        .expect("second backfill ok");
    let rows_second = db.list_employees().expect("list ok");

    // No duplication, no id drift, no field drift.
    assert_eq!(rows_first.len(), 3);
    assert_eq!(rows_second.len(), 3, "re-run must not duplicate rows");
    assert_eq!(rows_first, rows_second, "re-run must not change any field");

    let links_second: Vec<_> = jobs
        .iter()
        .flat_map(|j| {
            db.list_employee_cron_jobs(&derive_employee_id(&j.id))
                .expect("links ok")
        })
        .collect();
    assert_eq!(links_first, links_second, "link rows must be stable too");
    assert_eq!(links_first.len(), 3, "no duplicate link rows");

    assert_eq!(first.employees_written, second.employees_written);
    assert_eq!(first.links_written, second.links_written);
    assert!(second.collisions.is_empty());
}

#[test]
fn org_employee_backfill_second_run_refreshes_updated_at_but_not_created_at() {
    let (db, _dir) = temp_registry();
    let jobs = vec![realistic_job("a02e3f692fb0")];

    db.backfill_from_cron_jobs(&jobs, "2026-09-30T12:00:00+00:00", BACKFILL_REASON)
        .expect("first ok");
    db.backfill_from_cron_jobs(&jobs, "2026-10-01T09:30:00+00:00", BACKFILL_REASON)
        .expect("second ok");

    let emp = db
        .get_employee("emp-a02e3f692fb0")
        .expect("read ok")
        .expect("employee must exist");
    // created_at comes from the cron job, so it must not drift.
    assert_eq!(emp.created_at, "2026-09-01T00:00:00+00:00");
    // updated_at is the write time, so it legitimately moves.
    assert_eq!(emp.updated_at, "2026-10-01T09:30:00+00:00");
}

// ── 3. employee_id derivation matches the organism convention ─────────────

#[test]
fn org_employee_id_derivation_matches_organism_convention() {
    // Verified against ~/.hermes/organism/swarm/task-grid/data/employees.json:
    // `emp-a02e3f692fb0` has home_crons[0].cron_id == "a02e3f692fb0".
    assert_eq!(derive_employee_id("a02e3f692fb0"), "emp-a02e3f692fb0");
    assert_eq!(derive_employee_id("d4d71fccfed6"), "emp-d4d71fccfed6");
    // 12 chars is the cap: a longer id is truncated, not extended.
    assert_eq!(
        derive_employee_id("a02e3f692fb0deadbeef"),
        "emp-a02e3f692fb0"
    );
    // A short id is not padded.
    assert_eq!(derive_employee_id("abc"), "emp-abc");
    // The prefix is always present.
    assert!(derive_employee_id("").starts_with("emp-"));
}

#[test]
fn org_employee_id_derivation_is_deterministic_across_calls() {
    for id in ["a02e3f692fb0", "cron_32365296", "01c3407ea813"] {
        let first = derive_employee_id(id);
        for _ in 0..5 {
            assert_eq!(derive_employee_id(id), first, "id must be stable for {id}");
        }
    }
}

#[test]
fn org_employee_id_derivation_does_not_panic_on_non_ascii() {
    // The literal 12-byte slice in §3.1.2 would panic on a multi-byte
    // boundary. Ours degrades to a valid, shorter id instead.
    let id = derive_employee_id("日本語のジョブid-very-long");
    assert!(id.starts_with("emp-"));
    // Must be valid UTF-8 with no panic — the assertion is the survival.
    assert!(id.len() > "emp-".len());
}

#[test]
fn org_employee_id_collision_skips_loudly_and_does_not_disambiguate() {
    let (db, _dir) = temp_registry();
    // Two distinct cron jobs whose first 12 characters are identical.
    let a = realistic_job("dup123456789a");
    let b = realistic_job("dup123456789b");

    let report = db
        .backfill_from_cron_jobs(&[a, b], "2026-09-30T12:00:00+00:00", BACKFILL_REASON)
        .expect("backfill ok");

    assert_eq!(report.employees_written, 1, "second job must be skipped");
    assert_eq!(report.collisions.len(), 1, "collision must be reported");
    // "dup123456789a" truncated to 12 chars is "dup123456789" — the
    // trailing 'a'/'b' is exactly what the truncation drops.
    assert_eq!(report.collisions[0].employee_id, "emp-dup123456789");
    assert_eq!(report.collisions[0].cron_job_id, "dup123456789b");
    assert_eq!(
        report.collisions[0].conflicting_cron_job_id,
        "dup123456789a"
    );

    // Exactly one employee exists, and it is the FIRST job's — no
    // disambiguator was appended.
    let emps = db.list_employees().expect("list ok");
    assert_eq!(emps.len(), 1);
    assert_eq!(emps[0].employee_id, "emp-dup123456789");
}

// ── 4. empty skills: backfilled faithfully, NOT auto-repaired ────────────
//
// This is the fail-closed contract. §2.2 measured 6 of the 102 real
// organism jobs with an empty skills array; a backfill that invented a
// skill would hide exactly the rows the gate exists to surface.

#[test]
fn org_employee_empty_skills_backfills_to_empty_and_is_not_repaired() {
    let (db, _dir) = temp_registry();

    // Two shapes of "no skills": the column absent, and an explicit empty
    // array. Both occur in the real data. Both `skill` fields are cleared
    // too — `realistic_job` populates the singular field, and the backfill
    // falls back to it (see the three tests below this one), so leaving it
    // set would not actually exercise "no skills".
    let mut absent = realistic_job("noskills01");
    absent.skills = None;
    absent.skill = None;
    let mut empty = realistic_job("noskills02");
    empty.skills = Some(vec![]);
    empty.skill = None;

    let report = db
        .backfill_from_cron_jobs(
            &[absent.clone(), empty.clone()],
            "2026-09-30T12:00:00+00:00",
            BACKFILL_REASON,
        )
        .expect("backfill ok");

    // The rows are written, faithfully, as [].
    assert_eq!(report.employees_written, 2);
    for id in ["emp-noskills01", "emp-noskills02"] {
        let emp = db.get_employee(id).expect("read ok").expect("must exist");
        assert!(
            emp.skills.is_empty(),
            "{id}: skills must backfill to [] and stay [], got {:?}",
            emp.skills
        );
    }

    // And they are reported as invalid — the gate's job, not backfill's
    // to hide.
    assert_eq!(report.invalid.len(), 2);
    for inv in &report.invalid {
        assert_eq!(inv.missing, vec!["skills"]);
    }

    // A second run must not repair them either.
    db.backfill_from_cron_jobs(
        &[absent, empty],
        "2026-10-01T00:00:00+00:00",
        BACKFILL_REASON,
    )
    .expect("second ok");
    for id in ["emp-noskills01", "emp-noskills02"] {
        let emp = db.get_employee(id).expect("read ok").expect("must exist");
        assert!(emp.skills.is_empty(), "{id} must still be [] after re-run");
    }
}

#[test]
fn org_employee_missing_required_fields_names_only_skills_for_the_real_shape() {
    // The §3.2 required set is exactly three fields. A row that has a name
    // and an id but no skills is missing exactly one thing, and the gate
    // needs that name to be greppable.
    let emp = Employee {
        employee_id: "emp-abc".to_string(),
        name: "VPS Health Watchdog".to_string(),
        role: DEFAULT_ROLE.to_string(),
        department: None,
        skills: vec![],
        agent_type: None,
        persona: None,
        system_prompt: None,
        status: STATUS_ACTIVE.to_string(),
        reason: BACKFILL_REASON.to_string(),
        created_at: "2026-09-01T00:00:00+00:00".to_string(),
        updated_at: "2026-09-30T12:00:00+00:00".to_string(),
    };
    assert_eq!(emp.missing_required_fields(), vec!["skills"]);
    assert!(!emp.is_valid());
}

#[test]
fn org_employee_blank_name_is_reported_as_missing() {
    // `name` is required and must be non-empty after trim.
    let emp = Employee {
        employee_id: "emp-abc".to_string(),
        name: "   ".to_string(),
        role: DEFAULT_ROLE.to_string(),
        department: None,
        skills: vec!["axe-command-center".to_string()],
        agent_type: None,
        persona: None,
        system_prompt: None,
        status: STATUS_ACTIVE.to_string(),
        reason: BACKFILL_REASON.to_string(),
        created_at: "2026-09-01T00:00:00+00:00".to_string(),
        updated_at: "2026-09-30T12:00:00+00:00".to_string(),
    };
    assert_eq!(emp.missing_required_fields(), vec!["name"]);
}

#[test]
fn org_employee_valid_row_reports_nothing_missing() {
    let emp = Employee {
        employee_id: "emp-abc".to_string(),
        name: "VPS Health Watchdog".to_string(),
        role: DEFAULT_ROLE.to_string(),
        department: None,
        skills: vec!["axe-command-center".to_string()],
        agent_type: None,
        persona: None,
        system_prompt: None,
        status: STATUS_ACTIVE.to_string(),
        reason: BACKFILL_REASON.to_string(),
        created_at: "2026-09-01T00:00:00+00:00".to_string(),
        updated_at: "2026-09-30T12:00:00+00:00".to_string(),
    };
    assert!(emp.missing_required_fields().is_empty());
    assert!(emp.is_valid());
}

// ── 5. AgentType::None round-trips distinctly ────────────────────────────

#[test]
fn org_employee_agent_type_absent_round_trips_as_none_not_a_variant() {
    let (db, _dir) = temp_registry();
    let job = realistic_job("agtype001");
    db.backfill_from_cron_jobs(&[job], "2026-09-30T12:00:00+00:00", BACKFILL_REASON)
        .expect("backfill ok");

    let emp = db
        .get_employee("emp-agtype001")
        .expect("read ok")
        .expect("must exist");

    // The backfill writes a NULL column, and NULL reads back as None —
    // distinct from Service, Session, and Fixer alike.
    assert_eq!(emp.agent_type, None);
    assert_ne!(emp.agent_type, Some(AgentType::Service));
    assert_ne!(emp.agent_type, Some(AgentType::Session));
    assert_ne!(emp.agent_type, Some(AgentType::Fixer));
}

#[test]
fn org_employee_agent_type_wire_forms_round_trip() {
    for (variant, wire) in [
        (AgentType::Service, "service"),
        (AgentType::Session, "session"),
        (AgentType::Fixer, "fixer"),
    ] {
        assert_eq!(variant.as_str(), wire);
        assert_eq!(AgentType::parse(wire), Some(variant));
        // An unknown value must read as absent, never as a guess.
        assert_eq!(AgentType::parse("supervisor"), None);
    }
    assert_eq!(AgentType::parse(""), None);
}

#[test]
fn org_employee_agent_type_has_no_default_variant() {
    // The absence of a `Default` impl is the design, not an oversight:
    // "absent keeps today's behavior" (Q3) depends on there being no
    // value to accidentally default to. If a Default is ever added this
    // test stops compiling, which is the point.
    fn assert_no_default<T: ?Sized>() {}
    // Compile-time proof: nothing here constructs a defaulted AgentType.
    // The value-level proof is the three `assert_ne!`s above.
    assert_no_default::<AgentType>();
    // And the variants are exactly three — the ones §3.1.2 names.
    let all = [AgentType::Service, AgentType::Session, AgentType::Fixer];
    assert_eq!(all.len(), 3);
}

// ── storage location (§1.2, Q2 — sibling DB, no new file) ────────────────

#[test]
fn org_employee_registry_lands_in_the_existing_kanban_db() {
    let dir = tempfile::tempdir().expect("tempdir failed");
    let database_path = dir.path().join("database.db");
    let resolved = org_db_path(&database_path);

    // Derived as a sibling of the main DB, named operant_kanban.db —
    // the same rule main.rs:967-971 uses. No new file, no org_root key.
    assert_eq!(resolved, dir.path().join("operant_kanban.db"));
    assert_eq!(
        resolved.parent(),
        database_path.parent(),
        "registry must sit beside the main database"
    );
    assert!(
        resolved
            .file_name()
            .unwrap()
            .to_string_lossy()
            .ends_with(".db")
    );
    // It is the kanban file, not a new "org.db".
    assert!(
        !resolved
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains("org")
    );
}

#[test]
fn org_employee_registry_schema_is_idempotent_across_init() {
    let dir = tempfile::tempdir().expect("tempdir failed");
    let path = org_db_path(&dir.path().join("database.db"));

    // Opening the same file twice must be a no-op, not an error — this is
    // what makes the additive tables safe on a live install.
    let first = EmployeeDb::init(path.clone()).expect("first init ok");
    let job = realistic_job("a02e3f692fb0");
    first
        .backfill_from_cron_jobs(&[job], "2026-09-30T12:00:00+00:00", BACKFILL_REASON)
        .expect("backfill ok");
    drop(first);

    let second = EmployeeDb::init(path).expect("second init ok");
    let emps = second.list_employees().expect("list ok");
    assert_eq!(emps.len(), 1, "existing rows must survive re-init");
    assert_eq!(emps[0].employee_id, "emp-a02e3f692fb0");
}

#[test]
fn org_employee_backfill_of_zero_jobs_is_a_no_op() {
    let (db, _dir) = temp_registry();
    let report = db
        .backfill_from_cron_jobs(&[], "2026-09-30T12:00:00+00:00", BACKFILL_REASON)
        .expect("empty backfill ok");
    assert_eq!(report.jobs_seen, 0);
    assert_eq!(report.employees_written, 0);
    assert!(db.list_employees().expect("list ok").is_empty());
}

// ─── the `skill` (singular) fallback ───────────────────────────────────────
//
// `CronJob` carries both `skill: Option<String>` and `skills: Option<Vec<String>>`
// (`cronjobs/db.rs:49-50`). Measured over the 102 live jobs, 91 set `skill`,
// 96 set `skills`, and 89 set both — so reading only the plural array
// produced `[]` for jobs that are perfectly well-specified through the
// singular one. Two of the six "empty skills" rows in the live organism are
// exactly this case, and the fail-closed gate would have blocked them as
// misconfigured. These three tests pin the fallback and, just as
// importantly, pin that it is a *fallback* and not a replacement.

#[test]
fn org_employee_backfill_falls_back_to_singular_skill() {
    let (db, _dir) = temp_registry();
    let mut job = realistic_job("210544ad44bd");
    // The real shape of `210544ad44bd` "Web Deploy Health Daily":
    // `skill: website-design`, `skills: []`.
    job.skills = Some(Vec::new());
    job.skill = Some("website-design".to_string());

    db.backfill_from_cron_jobs(&[job], "2026-09-30T12:00:00+00:00", BACKFILL_REASON)
        .expect("backfill ok");

    let emps = db.list_employees().expect("list ok");
    assert_eq!(emps.len(), 1);
    assert_eq!(
        emps[0].skills,
        vec!["website-design".to_string()],
        "an empty `skills` array must fall back to the job's own `skill`"
    );
    assert!(
        emps[0].missing_required_fields().is_empty(),
        "a job that declared its skill must not be blocked by the gate"
    );
}

#[test]
fn org_employee_backfill_prefers_plural_skills_over_singular() {
    let (db, _dir) = temp_registry();
    // `realistic_job` sets both; the plural array must win so a job that
    // genuinely has several skills is not collapsed to one.
    let mut job = realistic_job("a02e3f692fb0");
    job.skill = Some("only-the-singular-one".to_string());
    job.skills = Some(vec!["alpha".to_string(), "beta".to_string()]);

    db.backfill_from_cron_jobs(&[job], "2026-09-30T12:00:00+00:00", BACKFILL_REASON)
        .expect("backfill ok");

    let emps = db.list_employees().expect("list ok");
    assert_eq!(
        emps[0].skills,
        vec!["alpha".to_string(), "beta".to_string()],
        "a non-empty `skills` array must win over the singular field"
    );
}

#[test]
fn org_employee_backfill_leaves_skills_empty_when_neither_field_is_set() {
    let (db, _dir) = temp_registry();
    // The real shape of `4a7007e66158` "Holonic News Intelligence Pulse":
    // both absent. There is nothing to fall back to, and inventing a value
    // here would be fabricating a capability the job never declared.
    let mut job = realistic_job("4a7007e66158");
    job.skills = None;
    job.skill = None;

    let report = db
        .backfill_from_cron_jobs(&[job], "2026-09-30T12:00:00+00:00", BACKFILL_REASON)
        .expect("backfill ok");

    let emps = db.list_employees().expect("list ok");
    assert_eq!(emps.len(), 1);
    assert!(
        emps[0].skills.is_empty(),
        "neither field set must stay empty, never invented"
    );
    assert_eq!(
        emps[0].missing_required_fields(),
        vec!["skills"],
        "an unstaffed seat is a legitimate state, and the gate must say so"
    );
    assert_eq!(
        report.invalid.len(),
        1,
        "the backfill must report the row, not silently drop or repair it"
    );
}

/// The operator's own reason must reach the row, verbatim.
///
/// Regression: `org sync` required a `--reason`, validated it, echoed it in
/// its summary line — and then stamped `BACKFILL_REASON` into every row. The
/// audit trail named a party that did not make the write, which defeats §3.1
/// ("every row records why it exists") at exactly the surface where it
/// matters most, a hundred-plus-row bulk mutation.
#[test]
fn org_employee_backfill_persists_the_callers_reason_verbatim() {
    let (db, _dir) = temp_registry();
    let reason = "reconcile registry after the q3 headcount freeze (ticket ORG-419)";

    db.backfill_from_cron_jobs(
        &[realistic_job("aaaaaaaaaaa1"), realistic_job("bbbbbbbbbbb2")],
        "2026-09-30T12:00:00+00:00",
        reason,
    )
    .expect("backfill ok");

    let emps = db.list_employees().expect("list ok");
    assert_eq!(emps.len(), 2, "both rows written");
    for emp in &emps {
        assert_eq!(
            emp.reason, reason,
            "{} must carry the operator's reason, not a constant",
            emp.employee_id
        );
    }
}

/// A re-run with a *different* reason must overwrite, so the row's reason
/// always describes the most recent mutation rather than the first one.
#[test]
fn org_employee_backfill_reason_reflects_the_latest_write() {
    let (db, _dir) = temp_registry();
    let job = realistic_job("ccccccccccc3");

    db.backfill_from_cron_jobs(
        std::slice::from_ref(&job),
        "2026-09-30T12:00:00+00:00",
        "first reason",
    )
    .expect("first backfill ok");
    db.backfill_from_cron_jobs(&[job], "2026-09-30T12:00:00+00:00", "second reason")
        .expect("second backfill ok");

    let emp = db
        .get_employee("emp-ccccccccccc3")
        .expect("read ok")
        .expect("must exist");
    assert_eq!(
        emp.reason, "second reason",
        "the reason must describe the latest mutation, not the first"
    );
}

/// A blank reason must be rejected at the DB boundary too, not just at the
/// CLI. `reason` is `NOT NULL`, but `''` would satisfy that and still be an
/// empty audit trail — so the library refuses it for callers that bypassed
/// clap's validation.
#[test]
fn org_employee_backfill_rejects_a_blank_reason() {
    let (db, _dir) = temp_registry();

    for blank in ["", "   ", "\t\n "] {
        let err = db
            .backfill_from_cron_jobs(
                &[realistic_job("ddddddddddd4")],
                "2026-09-30T12:00:00+00:00",
                blank,
            )
            .expect_err(&format!("blank reason {blank:?} must be rejected"));
        assert!(
            err.to_string().contains("reason"),
            "the error must point at the reason, got: {err}"
        );
    }

    assert_eq!(
        db.list_employees().expect("list ok").len(),
        0,
        "a rejected reason must write nothing at all, not even the rows"
    );
}

/// Surrounding whitespace is trimmed so the stored reason is the operator's
/// words and not their shell quoting.
#[test]
fn org_employee_backfill_trims_the_reason() {
    let (db, _dir) = temp_registry();
    db.backfill_from_cron_jobs(
        &[realistic_job("eeeeeeeeeee5")],
        "2026-09-30T12:00:00+00:00",
        "  onboard the new platform team  ",
    )
    .expect("backfill ok");
    let emp = db
        .get_employee("emp-eeeeeeeeeee5")
        .expect("read ok")
        .expect("must exist");
    assert_eq!(emp.reason, "onboard the new platform team");
}
