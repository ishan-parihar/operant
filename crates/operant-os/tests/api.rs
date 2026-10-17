//! Read-API integration tests (boundary doc §5, invariants 1 and 2, phase B scope).
//!
//! The fixture builds a real Holon `work.db` by running Holon's own migrations —
//! the schema this client reads is the published one, not a hand-rolled copy. The
//! Holon checkout path comes from `HOLON_REPO`; tests skip (with a stderr note)
//! when the checkout is absent so a foreign machine does not spuriously fail.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::process::Command;

use operant_os::Error;
use operant_os::api::HolonApi;

fn holon_repo() -> PathBuf {
    PathBuf::from(std::env::var("HOLON_REPO").unwrap_or_else(|_| {
        // default: the sibling checkout this fleet runs against
        "../../../../../my-projects/holon".to_string()
    }))
}

fn repo_present() -> bool {
    holon_repo()
        .join("holon")
        .join("core")
        .join("db.py")
        .exists()
}

/// Build a scratch Holon home with Holon's migrations applied and one seeded
/// employee, exactly the way the kernel builds instances (forward-only migrations).
fn scratch_org(tag: &str) -> PathBuf {
    let repo = holon_repo();
    let home = std::env::temp_dir().join(format!("operant-os-test-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join("execution").join("runtime")).unwrap();
    let db = home.join("execution").join("runtime").join("work.db");
    let script = "import sqlite3, sys\n\
         from holon.core import db\n\
         conn = sqlite3.connect(sys.argv[1], isolation_level=None)\n\
         conn.row_factory = sqlite3.Row\n\
         db.migrate(conn, 'work')\n\
         conn.execute(\"INSERT INTO departments (id, slug, name, mission, head_id, status, created_at) \
         VALUES ('dept_x','executive','Executive',NULL,NULL,'active','2026-01-01')\")\n\
         conn.execute(\"INSERT INTO employees (id, name, title, dept_id, job_description, roles, driver, \
         max_concurrency, status, hired_at, hired_reason) VALUES ('emp_1','Ada','CEO','dept_x','d', \
         '[\\\"plan\\\"]','fake',1,'active','2026-01-01','r')\")\n";
    let out = Command::new("python3")
        .arg("-c")
        .arg(script)
        .arg(&db)
        .env("PYTHONPATH", &repo)
        .output()
        .expect("python3 runs holon migrations");
    assert!(
        out.status.success(),
        "holon migrate failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    home
}

/// Invariant 1, whole-database scope: a client built by `open` sits on a handle
/// that cannot write. The strongest available proof from outside the crate: the
/// database file's mtime and content are unchanged after every read call, and a
/// WAL/journal file never appears beside it (a writer would need one).
#[test]
fn reads_never_touch_the_database_files() {
    if !repo_present() {
        eprintln!("skipping: no holon checkout at {}", holon_repo().display());
        return;
    }
    let home = scratch_org("files");
    let db = home.join("execution").join("runtime").join("work.db");
    let before = std::fs::read(&db).unwrap();
    let api = HolonApi::open(&home).expect("read api opens on a migrated org");
    api.employees().expect("read");
    api.board().expect("read");
    api.approvals().expect("read");
    api.messages().expect("read");
    api.triggers().expect("read");
    api.departments().expect("read");
    let after = std::fs::read(&db).unwrap();
    assert_eq!(
        before, after,
        "the database file must be byte-identical after reads"
    );
    let wal = db.with_extension("db-wal");
    assert!(
        !wal.exists(),
        "no WAL sidecar may appear from a read-only handle"
    );
    let _ = std::fs::remove_dir_all(&home);
}

/// The client refuses an api_version it does not know (boundary doc §4.1): a
/// newer Holon must not be read through stale view shapes.
#[test]
fn unknown_api_version_is_refused() {
    if !repo_present() {
        eprintln!("skipping: no holon checkout at {}", holon_repo().display());
        return;
    }
    let home = scratch_org("ver");
    let db = home.join("execution").join("runtime").join("work.db");
    {
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute("UPDATE api_meta SET value='99' WHERE key='api_version'", [])
            .unwrap();
    }
    match HolonApi::open(&home) {
        Err(Error::UnknownApiVersion { published, .. }) => assert_eq!(published, "99"),
        other => panic!("expected UnknownApiVersion, got {other:?}"),
    }
    let _ = std::fs::remove_dir_all(&home);
}

/// An instance predating the read-API migration (no api_meta) is a named error.
#[test]
fn pre_api_instance_is_a_named_error() {
    if !repo_present() {
        eprintln!("skipping: no holon checkout at {}", holon_repo().display());
        return;
    }
    let home = scratch_org("pre");
    let db = home.join("execution").join("runtime").join("work.db");
    {
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch(
            "DROP VIEW api_employees; DROP VIEW api_departments; DROP VIEW api_board; \
             DROP VIEW api_approvals; DROP VIEW api_messages; DROP VIEW api_triggers; \
             DROP TABLE api_meta;",
        )
        .unwrap();
    }
    match HolonApi::open(&home) {
        Err(Error::ApiNotPublished { .. }) => {}
        other => panic!("expected ApiNotPublished, got {other:?}"),
    }
    let _ = std::fs::remove_dir_all(&home);
}

/// A home without a work.db is a named error, not a panic or a silent empty org.
#[test]
fn missing_instance_is_a_named_error() {
    let home = std::env::temp_dir().join(format!("operant-os-test-missing-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    match HolonApi::open(&home) {
        Err(Error::WorkDbMissing(p)) => assert!(p.ends_with("work.db")),
        other => panic!("expected WorkDbMissing, got {other:?}"),
    }
    let _ = std::fs::remove_dir_all(&home);
}

/// Every published view reads through the client against a real migrated org,
/// and the seeded row comes back mapped.
#[test]
fn every_view_reads_and_maps_rows() {
    if !repo_present() {
        eprintln!("skipping: no holon checkout at {}", holon_repo().display());
        return;
    }
    let home = scratch_org("views");
    let api = HolonApi::open(&home).expect("open");
    let employees = api.employees().expect("employees view reads");
    assert_eq!(employees.len(), 1, "the seeded employee is published");
    assert_eq!(employees[0].name, "Ada");
    assert_eq!(employees[0].department, "executive");
    assert_eq!(employees[0].driver, "fake");
    let departments = api.departments().expect("departments view reads");
    assert_eq!(departments.len(), 1);
    assert_eq!(departments[0].slug, "executive");
    assert!(api.board().is_ok(), "board view reads");
    assert!(api.approvals().is_ok(), "approvals view reads");
    assert!(api.messages().is_ok(), "messages view reads");
    assert!(api.triggers().is_ok(), "triggers view reads");
    let _ = std::fs::remove_dir_all(&home);
}
