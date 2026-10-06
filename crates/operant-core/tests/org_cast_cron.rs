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

    // Every mappable seat persisted exactly its manifest schedule.
    for seat in CAST {
        match cast_cron_job_for(seat) {
            Some((job_id, schedule)) => {
                let job = s
                    .cron
                    .get_job(&job_id)
                    .expect("get_job query")
                    .expect("cast job");
                assert_eq!(job.schedule, schedule, "seat {}", seat.id);
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
        "INSERT OR IGNORE leaves the existing row untouched"
    );
}

#[test]
fn cadence_table_covers_the_manifest() {
    assert_eq!(cadence_to_schedule("daily").expect("daily"), "0 0 9 * * *");
    assert_eq!(
        cadence_to_schedule("hourly").expect("hourly"),
        "0 */2 * * * *"
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
