//! Integration tests for the Wave 2-3 org layer landing in the shared kanban file.
//!
//! The risk these cover is not any single store's logic — their own inline
//! tests do that. The risk is **cohabitation**: five new stores opened the
//! same `operant_kanban.db` that the kanban family claims at
//! `PRAGMA user_version = 1`, and `PRAGMA user_version` is file-wide. If any
//! new store claimed a version, or if opening one perturbed the others, the
//! failure mode is kanban's documented "refusing to downgrade" hard-fail
//! (see `migrations.rs`, added after R39-7) — an outage in the *existing*
//! product, caused by new code that no unit test would catch.
//!
//! So the tests below assert the shared-file contract directly:
//!
//! 1. All five stores open the same file without interfering.
//! 2. The version counter is untouched by any of them.
//! 3. Data written by one store is readable by another.
//! 4. Re-opening an existing file is idempotent and additive.
//!
//! Modelled on `crates/operant-core/tests/org_notice_board.rs`, and the temp
//! file is named exactly `operant_kanban.db` so a test that passed against a
//! differently-named file cannot pass here.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use operant_core::org::authority::{AuthorityScope, GrantDb};
use operant_core::org::decisions_db::DecisionsDb;
use operant_core::org::department_db::{Department, DepartmentDb};
use operant_core::org::dm_thread::{DEFAULT_TURN_BUDGET, DmThreadDb};
use operant_core::org::employee::{AgentType, Employee};
use operant_core::org::employee_db::EmployeeDb;
use operant_core::org::hierarchy_edges::HierarchyEdgesDb;
use operant_core::org::notice_db::NoticeBoard;
use operant_core::org::pending_requests::PendingRequestDb;
use operant_core::org::resolver::{identity_for, inbox_query_for, resolve_recipients};
use operant_core::org::schema::ensure_column;
use operant_core::org::seat_authority::SeatApprover;
use rusqlite::Connection;
use std::path::PathBuf;
use tempfile::TempDir;

/// The shared sibling DB every org store opens.
fn shared_db() -> (PathBuf, TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path: PathBuf = dir.path().join("operant_kanban.db");
    (path, dir)
}

fn user_version(path: &PathBuf) -> i64 {
    let conn = Connection::open(path).expect("open");
    conn.pragma_query_value(None, "user_version", |r| r.get(0))
        .expect("read user_version")
}

/// A minimal employee row. `Employee`'s constructor surface is set by
/// Wave 1; build it through its public fields only.
fn employee(id: &str, dept: Option<&str>) -> Employee {
    Employee {
        employee_id: id.to_string(),
        name: format!("Employee {id}"),
        role: "automator".to_string(),
        department: dept.map(|d| d.to_string()),
        skills: vec![],
        agent_type: Some(AgentType::Session),
        persona: None,
        system_prompt: None,
        status: "active".to_string(),
        reason: "integration test".to_string(),
        created_at: "2026-10-01T00:00:00Z".to_string(),
        updated_at: "2026-10-01T00:00:00Z".to_string(),
    }
}

#[test]
fn five_org_stores_coexist_in_the_shared_kanban_file() {
    let (path, _dir) = shared_db();

    // Every store opens the SAME path. If any of them created its own file
    // instead, this test would still pass — so also assert the file count
    // afterwards.
    let _board = NoticeBoard::init(path.clone()).expect("notice board");
    let _departments = DepartmentDb::init(path.clone()).expect("department db");
    let _grants = GrantDb::init(path.clone()).expect("grant db");
    let _threads = DmThreadDb::init(path.clone()).expect("dm thread db");

    let entries: Vec<_> = std::fs::read_dir(path.parent().expect("parent"))
        .expect("read dir")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        entries.len(),
        1,
        "expected exactly one DB file, found {entries:?} — a store opened its own file"
    );
}

#[test]
fn no_org_store_moves_the_user_version_counter() {
    let (path, _dir) = shared_db();
    assert_eq!(user_version(&path), 0, "fresh file starts at 0");

    let _board = NoticeBoard::init(path.clone()).expect("notice board");
    let _departments = DepartmentDb::init(path.clone()).expect("department db");
    let _grants = GrantDb::init(path.clone()).expect("grant db");
    let _threads = DmThreadDb::init(path.clone()).expect("dm thread db");
    assert_eq!(
        user_version(&path),
        0,
        "an org store claimed a migration family on the kanban file"
    );

    // The decisions store lives in its OWN file precisely so it cannot do this.
    let decisions_path = path.parent().expect("parent").join("operant_decisions.db");
    let _decisions = DecisionsDb::init(decisions_path.clone()).expect("decisions db");
    assert_eq!(
        user_version(&decisions_path),
        0,
        "a fresh decisions file must stay unmigrated"
    );
}

#[test]
fn the_kanban_family_still_migrates_after_the_org_tables_land() {
    use operant_core::kanban::db::KanbanDb;

    let (path, _dir) = shared_db();
    // Org stores open first and create their tables.
    let _board = NoticeBoard::init(path.clone()).expect("notice board");
    let _departments = DepartmentDb::init(path.clone()).expect("department db");

    // Now kanban migrates the same file. If any org store had moved the
    // counter, kanban's single-entry family would see version > 1 and refuse.
    KanbanDb::init(path.clone()).expect("kanban must still migrate");
    assert_eq!(
        user_version(&path),
        1,
        "kanban must own version 1 and no more"
    );
}

#[test]
fn data_written_by_one_store_is_visible_to_another() {
    let (path, _dir) = shared_db();

    let departments = DepartmentDb::init(path.clone()).expect("department db");
    let dept = Department {
        dept_key: "infra".to_string(),
        display_name: "Platform Infrastructure".to_string(),
        mandate: Some("Keep the platform up.".to_string()),
        protocols: vec!["write a worklog before finishing".to_string()],
        rules: vec!["do not write outside your own tree".to_string()],
        required_capabilities: vec![],
        head_employee_id: None,
        target_headcount: Some(4),
        reason: "integration test".to_string(),
        created_at: "2026-10-01T00:00:00Z".to_string(),
        updated_at: "2026-10-01T00:00:00Z".to_string(),
    };
    departments.upsert(&dept).expect("upsert department");

    // Re-open from scratch: the row must still be there. This proves the org
    // tables are durable in the shared file, not per-connection state.
    drop(departments);
    let reopened = DepartmentDb::init(path.clone()).expect("reopen");
    let read_back = reopened.get("infra").expect("get").expect("row must exist");
    assert_eq!(read_back.display_name, "Platform Infrastructure");
    assert_eq!(read_back.rules.len(), 1);
}

#[test]
fn schema_helper_adds_a_column_to_the_shared_employees_table() {
    use operant_core::org::employee_db::EmployeeDb;

    let (path, _dir) = shared_db();
    let employees = EmployeeDb::init(path.clone()).expect("employee db");
    drop(employees);

    {
        let conn = Connection::open(&path).expect("open");
        let report = ensure_column(&conn, "employees", "reports_to", "TEXT").expect("ensure");
        assert_eq!(report.added, vec!["reports_to".to_string()]);
    }

    // Re-running must be a no-op, not a duplicate-column error.
    {
        let conn = Connection::open(&path).expect("open");
        let report = ensure_column(&conn, "employees", "reports_to", "TEXT").expect("ensure again");
        assert!(report.is_noop(), "second run must be a no-op");
    }

    // And the existing kanban/organ notice paths still work on the file.
    let _board = NoticeBoard::init(path).expect("board still opens");
}

#[test]
fn dm_thread_budget_is_shared_and_enforced_end_to_end() {
    let (path, _dir) = shared_db();
    let threads = DmThreadDb::init(path).expect("dm thread db");

    let thread = threads
        .open("emp-a", "emp-b", DEFAULT_TURN_BUDGET)
        .expect("open thread");
    assert_eq!(thread.turns_used, 0);
    assert_eq!(threads.remaining(&thread.thread_id).expect("remaining"), 3);

    // Spend the whole budget; the third spend exhausts rather than erroring.
    threads.spend_turn(&thread.thread_id).expect("spend 1");
    threads.spend_turn(&thread.thread_id).expect("spend 2");
    let last = threads
        .spend_turn(&thread.thread_id)
        .expect("spend 3 exhausts");
    assert_eq!(last.remaining(), 0);

    // A fourth turn must be REJECTED, not silently accepted.
    assert!(
        threads.spend_turn(&thread.thread_id).is_err(),
        "the loop guard must refuse a turn past the budget"
    );

    let final_state = threads.get(&thread.thread_id).expect("get").expect("row");
    assert!(
        matches!(
            final_state.state,
            operant_core::org::dm_thread::ThreadState::Exhausted
        ),
        "an exhausted thread must be visibly exhausted, got {:?}",
        final_state.state
    );
}

#[test]
fn dept_notice_reaches_members_but_not_outsiders_end_to_end() {
    let (path, _dir) = shared_db();
    let board = NoticeBoard::init(path).expect("board");

    let roster = vec![
        employee("emp-a1", Some("infra")),
        employee("emp-a2", Some("infra")),
        employee("emp-b1", Some("content")),
    ];

    let post = operant_core::org::notice::PostNotice {
        sender: "emp-head".to_string(),
        from_dept: Some("infra".to_string()),
        recipients: vec![operant_core::org::notice::Recipient::Dept(
            "infra".to_string(),
        )],
        subject: Some("infra sync".to_string()),
        body: "standup notes".to_string(),
        correlation_id: None,
        ack_required: false,
        ttl: None,
        ttl_expires_at: None,
        tags: vec![],
        thread_id: None,
        pinned: false,
        reason: "integration test".to_string(),
        metadata: None,
    };
    board.post(&post).expect("post to dept");

    // The resolver's audience matches the department, and only it.
    let audience = resolve_recipients(
        &operant_core::org::notice::Recipient::Dept("infra".into()),
        &roster,
    );
    assert_eq!(audience.len(), 2, "both infra members, no outsiders");

    for id in &audience {
        let row = roster
            .iter()
            .find(|e| &e.employee_id == id)
            .expect("in roster");
        let identity = identity_for(row, &[], &[]);
        let inbox = board
            .query_inbox(&inbox_query_for(&identity, None, None))
            .expect("inbox");
        assert_eq!(inbox.len(), 1, "{id} must receive the dept notice");
    }

    let outsider = roster
        .iter()
        .find(|e| e.employee_id == "emp-b1")
        .expect("row");
    let outsider_identity = identity_for(outsider, &[], &[]);
    let outsider_inbox = board
        .query_inbox(&inbox_query_for(&outsider_identity, None, None))
        .expect("outsider inbox");
    assert!(
        outsider_inbox.is_empty(),
        "a dept notice must not leak to another department"
    );
}

#[test]
fn unstaffed_employees_remain_discoverable() {
    let roster = vec![
        employee("emp-a1", Some("infra")),
        employee("emp-a2", None),
        employee("emp-a3", None),
    ];
    let findings = operant_core::org::resolver::unstaffed_employees(&roster);
    assert_eq!(
        findings.len(),
        2,
        "employees with no department must stay visible, not be dropped"
    );
    let ids: Vec<_> = findings.iter().map(|f| f.employee_id.as_str()).collect();
    assert!(ids.contains(&"emp-a2") && ids.contains(&"emp-a3"));
}

#[test]
fn authority_lattice_and_grant_db_coexist() {
    let (path, _dir) = shared_db();
    let grants = std::sync::Arc::new(GrantDb::init(path.clone()).expect("grant db"));

    // The lattice: a wider scope contains a narrower one, and Self (Own)
    // contains nothing. §2.1 — hierarchy suffices inside a department.
    use operant_core::org::authority::AuthorityScope as S;
    assert!(S::Org.contains(S::Department));
    assert!(S::Department.contains(S::Peers));
    assert!(!S::Own.contains(S::Peers));
    // §2.1: only Org crosses a department boundary on its own.
    assert!(S::Org.is_cross_department());
    assert!(!S::Department.is_cross_department());

    // F1 (ORGANISM-ARCHITECTURE §6): `GrantDb::insert` is `pub(crate)`, so
    // from OUTSIDE the crate a grant enters the ledger only through the
    // SeatApprover. The store's blank-reason rule still bites — it is
    // asserted here at that seam, the only pub write path — and the
    // four stores the approver reads must coexist in this same file,
    // which is this suite's actual subject.
    let conn = std::sync::Arc::new(std::sync::Mutex::new(
        Connection::open(&path).expect("open shared file"),
    ));
    let requests = std::sync::Arc::new(
        PendingRequestDb::from_shared_connection(std::sync::Arc::clone(&conn)).expect("requests"),
    );
    let employees = std::sync::Arc::new(
        EmployeeDb::from_shared_connection(std::sync::Arc::clone(&conn)).expect("employees"),
    );
    let edges = std::sync::Arc::new(
        HierarchyEdgesDb::from_connection(Connection::open(&path).expect("edges conn"))
            .expect("edges"),
    );
    // Documented seeding (the same shape the gateway's P3 tests use —
    // `EmployeeDb` has no direct insert API): a one-edge hierarchy, so the
    // head's approver-of-record is the ceo, an org lead.
    for id in ["emp-ceo", "emp-hod"] {
        conn.lock()
            .expect("conn")
            .execute(
                "INSERT OR REPLACE INTO employees (
                     employee_id, name, role, department, skills, agent_type,
                     persona, status, reason, created_at, updated_at
                 ) VALUES (?1, ?1, 'tester', 'content', '[\"probe\"]', NULL, NULL,
                           'active', 'integration seed', '2026-10-01T00:00:00Z',
                           '2026-10-01T00:00:00Z')",
                rusqlite::params![id],
            )
            .expect("seed employee");
    }
    edges
        .upsert_edge("emp-hod", "emp-ceo", "integration seed")
        .expect("edge");

    let approver = SeatApprover::new(
        std::sync::Arc::clone(&grants),
        requests,
        employees,
        edges,
        7,
    );
    let refused = approver.grant_direct("emp-hod", "content.tooling", None, "   ");
    let refusal = refused.expect_err("a whitespace-only reason must be refused");
    assert!(
        refusal.contains("reason"),
        "the refusal must name the reason rule: {refusal}"
    );

    // And the real mint through the same seam lands in the ledger the
    // store reads back — approver-shaped: the org lead's standing grant.
    let minted = approver
        .grant_direct(
            "emp-hod",
            "content.tooling",
            None,
            "integration: cohabitation probe",
        )
        .expect("the mint must succeed");
    let rows = grants.list_for_grantee("emp-hod").expect("ledger");
    let grant = rows
        .iter()
        .find(|g| g.grant_id == minted)
        .expect("the minted grant stands in the ledger");
    assert_eq!(grant.scope, AuthorityScope::Org);
    assert_eq!(grant.target_dept, None);
    assert_eq!(grant.expires_at, None, "an org lead mints standing");
}
