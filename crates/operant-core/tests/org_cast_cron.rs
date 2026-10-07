//! Cold-start cast cron seeding — the §1 self-operationalization wiring.
//!
//! These tests execute the real `CronDb::seed_cast_jobs` INSERT against a
//! real tempdir CronDb, not the seed report alone: the INSERT is the only
//! place a placeholder/param count mismatch or a shifted column binding
//! can surface, and the gateway wiring is fail-open warn — a broken seed
//! would otherwise ship silently as a warn line while the cast jobs never
//! land.

use std::sync::Arc;

use operant_core::cronjobs::CronDb;
use operant_core::org::authority::GrantDb;
use operant_core::org::cast::{CAST, cadence_to_schedule, cast_cron_job_for, seed_cast};
use operant_core::org::employee_db::EmployeeDb;
use operant_core::org::hierarchy_edges::HierarchyEdgesDb;
use operant_core::org::pending_requests::PendingRequestDb;

/// One seeded fixture: the org stores over one temp file plus the cron
/// store over another, mirroring the production layout where
/// `operant_kanban.db` and `operant_cron.db` are sibling files, not one db.
struct Seeded {
    _org_dir: tempfile::TempDir,
    _cron_dir: tempfile::TempDir,
    cron: CronDb,
    employees: Arc<EmployeeDb>,
}

fn seeded() -> Seeded {
    let org_dir = tempfile::tempdir().expect("org tempdir");
    let cron_dir = tempfile::tempdir().expect("cron tempdir");
    let org_db = org_dir.path().join("operant_kanban.db");

    // Production order: the cast lands in the registry first, then the
    // cron seeder links jobs to the already-present seats.
    let employees = Arc::new(EmployeeDb::open_at(org_db.clone()).expect("employee db"));
    let edges = Arc::new(HierarchyEdgesDb::init(&org_db).expect("edges db"));
    let grants = Arc::new(GrantDb::init(org_db.clone()).expect("grant db"));
    let requests = Arc::new(PendingRequestDb::init(org_db.clone()).expect("request db"));
    seed_cast(&employees, &edges, &grants, &requests, 7).expect("cast seed");

    let cron = CronDb::init(cron_dir.path().join("operant_cron.db")).expect("cron db");
    Seeded {
        _org_dir: org_dir,
        _cron_dir: cron_dir,
        cron,
        employees,
    }
}

#[test]
fn seed_writes_eight_jobs_with_every_column_bound() {
    let s = seeded();
    let report = s
        .cron
        .seed_cast_jobs(CAST, Some(&s.employees))
        .expect("seed");
    assert_eq!(report.cast_seats, 9, "all nine seats inspected");
    assert_eq!(report.jobs_written, 8, "dp-the-program is the only skip");

    // Column-level pins: these catch a placeholder/param mismatch (the
    // seed call itself would error) and a shifted binding (a column would
    // read as its neighbor's value).
    let premiere = s
        .cron
        .get_job("cron_cast_premiere")
        .expect("get_job query")
        .expect("premiere job seeded");
    assert_eq!(premiere.schedule, "0 0 9 * * *");
    assert_eq!(premiere.name, "daily directive + aspirations review");
    assert!(
        premiere.prompt.contains("premiere"),
        "the job's prompt is the seat's charter"
    );
    assert!(
        premiere.prompt.contains("## Seat memory"),
        "iter-666: the seeded prompt must carry the seat-memory postscript"
    );
    assert_eq!(premiere.deliver, "local");
    assert!(!premiere.no_agent);
    assert!(premiere.enabled);
    assert_eq!(premiere.state, "scheduled");
    assert!(
        premiere.next_run_at.is_some(),
        "a seeded job must be due to fire"
    );

    // dp-the-program (continuous cadence) takes no scheduled job.
    assert!(
        s.cron
            .get_job("cron_cast_dp-the-program")
            .expect("get_job query")
            .is_none()
    );

    // Every mappable seat persisted exactly its manifest schedule and
    // cadence display.
    for seat in CAST {
        match cast_cron_job_for(seat) {
            Some((job_id, schedule)) => {
                let job = s
                    .cron
                    .get_job(&job_id)
                    .expect("get_job query")
                    .expect("cast job");
                assert_eq!(job.schedule, schedule, "seat {}", seat.id);
                assert_eq!(
                    job.schedule_display, seat.cron.cadence,
                    "seat {} display is the cadence label",
                    seat.id
                );
            }
            None => {
                assert_eq!(
                    seat.id, "dp-the-program",
                    "only the continuous seat is unmapped"
                );
            }
        }
    }

    let jobs = s.cron.list_jobs(true).expect("list_jobs");
    assert_eq!(jobs.len(), 8, "a fresh db holds exactly the cast jobs");
}

#[test]
fn seed_links_each_job_to_its_employee() {
    let s = seeded();
    s.cron
        .seed_cast_jobs(CAST, Some(&s.employees))
        .expect("seed");
    let links = s
        .employees
        .list_employee_cron_jobs("premiere")
        .expect("list links");
    let link = links
        .iter()
        .find(|l| l.cron_job_id == "cron_cast_premiere")
        .expect("premiere link row");
    assert!(link.enabled);
    assert_eq!(
        link.schedule.as_deref(),
        Some("daily"),
        "link carries the cadence display"
    );

    // The crew-chief link pins the hourly mapping — the seat whose live row
    // was found carrying a foreign daily value (2026-10-07 incident).
    let crew = s
        .employees
        .list_employee_cron_jobs("crew-chief")
        .expect("list crew-chief links");
    assert!(
        crew.iter().any(|l| l.cron_job_id == "cron_cast_crew-chief"),
        "crew-chief linked to its job"
    );
}

#[test]
fn reseed_is_idempotent() {
    let s = seeded();
    s.cron
        .seed_cast_jobs(CAST, Some(&s.employees))
        .expect("first seed");
    let first = s
        .cron
        .get_job("cron_cast_premiere")
        .expect("get_job query")
        .expect("job after first seed");

    s.cron
        .seed_cast_jobs(CAST, Some(&s.employees))
        .expect("reseed");
    let jobs = s.cron.list_jobs(true).expect("list_jobs");
    assert_eq!(jobs.len(), 8, "deterministic ids: a reseed adds no rows");

    let second = s
        .cron
        .get_job("cron_cast_premiere")
        .expect("get_job query")
        .expect("job after reseed");
    assert_eq!(
        first.created_at, second.created_at,
        "the heal upsert never touches created_at"
    );
}

/// The `cron_cast_*` namespace is seed-owned: a stale row (2026-10-07 live
/// incident — dispatcher seeded on a 2-minute schedule, crew-chief and
/// identity-warden on foreign cadences by a concurrent implementation) is
/// healed to the manifest mapping on the next boot. A paused job stays
/// paused, and a scheduler-set next_run_at survives a no-change reseed.
#[test]
fn seed_heals_a_stale_cast_schedule_without_resurrecting_a_paused_job() {
    use std::collections::HashMap;

    let s = seeded();
    s.cron.seed_cast_jobs(CAST, None).expect("first seed");

    // Corrupt dispatcher the way the incident row arrived: the 2-minute
    // schedule, a stale display, and the operator's pause (enabled=0).
    let mut stale = HashMap::new();
    stale.insert(
        "schedule".to_string(),
        Some(serde_json::json!("0 */2 * * * *")),
    );
    stale.insert(
        "schedule_display".to_string(),
        Some(serde_json::json!("stale")),
    );
    stale.insert("enabled".to_string(), Some(serde_json::json!(false)));
    s.cron
        .update_job("cron_cast_dispatcher", stale)
        .expect("corrupt dispatcher row")
        .expect("row present after update");

    // Healing seed: schedule + display take the manifest value, next_run
    // re-arms, and the operator's pause survives.
    s.cron.seed_cast_jobs(CAST, None).expect("healing seed");
    let healed = s
        .cron
        .get_job("cron_cast_dispatcher")
        .expect("get_job query")
        .expect("healed row");
    assert_eq!(
        healed.schedule, "0 0 * * * *",
        "healed to the manifest mapping"
    );
    assert_eq!(healed.schedule_display, "hourly");
    assert!(
        healed.next_run_at.is_some(),
        "a changed schedule re-arms next_run"
    );
    assert!(
        !healed.enabled,
        "a paused job is not resurrected by a reseed"
    );

    // A no-change reseed must not stomp a scheduler-set next_run_at.
    let pinned_next = "2030-01-01T00:00:00+00:00";
    s.cron
        .set_next_run("cron_cast_dispatcher", Some(pinned_next.to_string()))
        .expect("pin next_run");
    s.cron.seed_cast_jobs(CAST, None).expect("no-change seed");
    let untouched = s
        .cron
        .get_job("cron_cast_dispatcher")
        .expect("get_job query")
        .expect("row");
    assert_eq!(
        untouched.next_run_at.as_deref(),
        Some(pinned_next),
        "an unchanged schedule keeps the stored next_run"
    );
}

#[test]
fn cadence_table_covers_the_manifest() {
    assert_eq!(cadence_to_schedule("daily").expect("daily"), "0 0 9 * * *");
    assert_eq!(
        cadence_to_schedule("hourly").expect("hourly"),
        "0 0 * * * *",
        "hourly is top-of-hour — */2 in the minutes field fires every 2 MINUTES"
    );
    assert_eq!(
        cadence_to_schedule("weekly").expect("weekly"),
        "0 0 9 * * 1"
    );
    assert!(cadence_to_schedule("continuous (existing)").is_err());
    assert!(
        cadence_to_schedule("biweekly").is_err(),
        "an unknown cadence fails loudly, not silently"
    );
}
