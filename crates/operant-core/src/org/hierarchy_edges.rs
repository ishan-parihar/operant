//! The reporting-edge store: sqlite DDL, the `upsert`/`remove` writes, and
//! the row → [`HierarchyEntry`] adapter that feeds [`Hierarchy::new`].
//!
//! `docs/NEXT-IMPLEMENTATION-OUTLINE.md` §11 wave-1 slice B. The query
//! surface over a *set* of edges already lives in [`super::hierarchy`], pure
//! and unit-testable; this module is only where the edges are kept between
//! runs, so it adds no query the pure module does not already answer.
//!
//! ## Why a separate table (alignment Q6)
//!
//! §11 resolves Q6 with the recommended default: edges live in their own
//! `hierarchy_edges` table, not on the `employees` row, for the reason
//! `hierarchy.rs`'s module docs already give — `Employee` is constructed
//! from struct literals in eight files, so a new field breaks every one of
//! them, while a separate table changes nothing that exists. An answer that
//! later prefers a column changes this adapter, not the architecture.
//!
//! ## Top-of-org is absence, not a sentinel
//!
//! A seat with **no row** here is a top (or not yet assigned —
//! [`HierarchyEntry`]'s docs explain why those are the same fact). There is
//! no `REPORTS_TO_NONE` row to store: `remove_edge` is how a seat becomes
//! a top. Inventing a sentinel manager id would put [`super::hierarchy`]'s
//! `REPORTS_TO_NONE` convention in two places that can drift.
//!
//! ## Schema style
//!
//! Like the employee registry and the worklog, the DDL is a plain
//! `CREATE TABLE IF NOT EXISTS` applied in one `execute_batch` — idempotent
//! at the SQL level, no version tracking, leaving the kanban family's
//! `user_version` untouched (see `employee_db.rs` for the invariant).
//!
//! `created_at` is stamped on every upsert, including a re-upsert that only
//! changes the reason: this table has no `updated_at` column, and a
//! re-recorded edge genuinely is a new operator act — the timestamp names
//! the act, per the §3.1 rule that every `employees`/edge row records who
//! made it and when.

use rusqlite::{Connection, params};
use std::path::Path;
use std::sync::{Arc, Mutex};

use super::hierarchy::HierarchyEntry;
use super::notice::rfc3339;
use crate::error::Error;

const SCHEMA: &str = r#"
    CREATE TABLE IF NOT EXISTS hierarchy_edges (
        employee_id    TEXT PRIMARY KEY,  -- emp-<hex>; one row = one reporting line
        manager_id    TEXT NOT NULL,      -- the employee_id they report to
        reason        TEXT NOT NULL,      -- operator's one-line justification
        created_at    TEXT NOT NULL       -- RFC3339, stamped on every upsert
    );
"#;

/// The `hierarchy_edges` store. One instance, shared behind an
/// `Arc<Mutex<Connection>>` — same shape as `EmployeeDb` and `WorklogDb`.
pub struct HierarchyEdgesDb {
    conn: Arc<Mutex<Connection>>,
}

impl HierarchyEdgesDb {
    /// Open (or create) the edge table in the org database file.
    ///
    /// `database_path` is the sqlite file itself (the kanban sibling —
    /// `employee_db::org_db_path` derives it). The DDL is idempotent, so
    /// this is safe on every boot.
    pub fn init(database_path: &Path) -> Result<Self, Error> {
        let conn = Connection::open(database_path)
            .map_err(|e| Error::Agent(format!("Failed to open hierarchy edges database: {e}")))?;
        Self::from_connection(conn)
    }

    /// Open the store on a connection the caller already owns — the form a
    /// caller holding the live kanban connection uses, so there is one file
    /// and one writer, not two.
    pub fn from_connection(conn: Connection) -> Result<Self, Error> {
        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        db.setup_schema()?;
        Ok(db)
    }

    /// Lock the SQLite connection, converting mutex poisoning into a
    /// recoverable error instead of panicking (same pattern as
    /// `employee_db.rs`).
    fn lock_conn(&self) -> Result<std::sync::MutexGuard<'_, Connection>, Error> {
        self.conn
            .lock()
            .map_err(|_| Error::Agent("hierarchy edges db mutex poisoned".to_string()))
    }

    fn setup_schema(&self) -> Result<(), Error> {
        let conn = self.lock_conn()?;
        conn.execute_batch(SCHEMA)
            .map_err(|e| Error::Agent(format!("hierarchy edges schema failed: {e}")))
    }

    /// Record (or re-record) one reporting line: `employee_id` reports to
    /// `manager_id`.
    ///
    /// Idempotent per employee: re-upserting the same seat overwrites the
    /// manager and reason instead of growing a second edge. `manager_id`
    /// is stored verbatim — whether it names a known employee is
    /// [`super::hierarchy::Hierarchy::findings`]'s question to report, not
    /// this store's to refuse: a typo'd edge must stay visible, not be
    /// silently dropped by the write path.
    pub fn upsert_edge(
        &self,
        employee_id: &str,
        manager_id: &str,
        reason: &str,
    ) -> Result<(), Error> {
        let conn = self.lock_conn()?;
        conn.execute(
            "INSERT INTO hierarchy_edges (employee_id, manager_id, reason, created_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(employee_id) DO UPDATE SET
                 manager_id = excluded.manager_id,
                 reason     = excluded.reason,
                 created_at = excluded.created_at",
            params![employee_id, manager_id, reason, rfc3339(chrono::Utc::now())],
        )
        .map_err(|e| Error::Agent(format!("hierarchy edge upsert failed: {e}")))?;
        Ok(())
    }

    /// Delete `employee_id`'s reporting line, making the seat a top by
    /// absence. Returns whether a row was actually removed.
    pub fn remove_edge(&self, employee_id: &str) -> Result<bool, Error> {
        let conn = self.lock_conn()?;
        let removed = conn
            .execute(
                "DELETE FROM hierarchy_edges WHERE employee_id = ?1",
                params![employee_id],
            )
            .map_err(|e| Error::Agent(format!("hierarchy edge remove failed: {e}")))?;
        Ok(removed > 0)
    }

    /// Every edge, as [`HierarchyEntry`] rows ready for
    /// [`super::hierarchy::Hierarchy::new`]. Ordered by `employee_id` so
    /// two reads of the same table feed identical entry slices.
    ///
    /// This is the whole adapter: one row, one `HierarchyEntry::new`. Top
    /// seats never appear — absence is the top, and a missing employee on
    /// the `employees` side is reported by the hierarchy's findings, not
    /// repaired here.
    pub fn entries(&self) -> Result<Vec<HierarchyEntry>, Error> {
        let conn = self.lock_conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT employee_id, manager_id, reason
                 FROM hierarchy_edges
                 ORDER BY employee_id",
            )
            .map_err(|e| Error::Agent(format!("hierarchy edges select failed: {e}")))?;
        let rows = stmt
            .query_map(params![], |row| {
                Ok(HierarchyEntry::new(
                    &row.get::<_, String>(0)?,
                    &row.get::<_, String>(1)?,
                    &row.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| Error::Agent(format!("hierarchy edges query failed: {e}")))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| Error::Agent(format!("hierarchy edge read failed: {e}")))
    }

    /// How many edges are recorded — the wave-2 escalations test's cheap
    /// way to assert a seeded org matches its fixture count.
    pub fn edge_count(&self) -> Result<usize, Error> {
        let conn = self.lock_conn()?;
        conn.query_row("SELECT COUNT(*) FROM hierarchy_edges", params![], |row| {
            row.get::<_, i64>(0)
        })
        .map(|n| n as usize)
        .map_err(|e| Error::Agent(format!("hierarchy edge count failed: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::org::employee::{Employee, STATUS_ACTIVE};
    use crate::org::hierarchy::{AUTHORITY_UNRESOLVED, DANGLING_MANAGER, Hierarchy};

    fn temp_db() -> (tempfile::TempDir, HierarchyEdgesDb) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = HierarchyEdgesDb::init(&dir.path().join("operant_kanban.db"))
            .expect("hierarchy edges init");
        (dir, db)
    }

    /// Mirrors the fixture in `hierarchy.rs`'s tests: an employee row with
    /// sensible defaults, so a test states only what it is about.
    fn employee(id: &str) -> Employee {
        Employee {
            employee_id: id.to_string(),
            name: format!("Employee {id}"),
            role: "automator".to_string(),
            department: Some("Platform Infrastructure".to_string()),
            skills: vec!["ops".to_string()],
            agent_type: None,
            persona: None,
            system_prompt: None,
            status: STATUS_ACTIVE.to_string(),
            reason: "test fixture".to_string(),
            created_at: "2026-10-01T00:00:00Z".to_string(),
            updated_at: "2026-10-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn stored_edges_feed_hierarchy_manager_of() {
        let (_dir, db) = temp_db();
        db.upsert_edge("emp-child", "emp-manager", "hired under platform lead")
            .expect("upsert child");
        assert_eq!(db.edge_count().expect("count"), 1);

        let entries = db.entries().expect("entries");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].employee_id, "emp-child");
        assert_eq!(entries[0].manager(), Some("emp-manager"));
        assert_eq!(entries[0].reason, "hired under platform lead");

        let employees = vec![employee("emp-manager"), employee("emp-child")];
        let hierarchy = Hierarchy::new(&employees, &entries);
        assert_eq!(hierarchy.manager_of("emp-child"), Some("emp-manager"));
    }

    #[test]
    fn two_level_chain_routes_and_absent_top_has_no_manager() {
        let (_dir, db) = temp_db();
        db.upsert_edge("emp-intern", "emp-hod", "graduate rotation")
            .expect("upsert intern");
        db.upsert_edge("emp-hod", "emp-ceo", "promoted to head of department")
            .expect("upsert hod");
        // The CEO gets no row: a top is an absent edge, not a sentinel.

        let employees = vec![
            employee("emp-ceo"),
            employee("emp-hod"),
            employee("emp-intern"),
        ];
        let entries = db.entries().expect("entries");
        let hierarchy = Hierarchy::new(&employees, &entries);

        assert_eq!(hierarchy.manager_of("emp-intern"), Some("emp-hod"));
        assert_eq!(hierarchy.manager_of("emp-hod"), Some("emp-ceo"));
        assert_eq!(hierarchy.manager_of("emp-ceo"), None);
    }

    #[test]
    fn dangling_manager_edge_is_reported_not_hidden() {
        let (_dir, db) = temp_db();
        db.upsert_edge("emp-child", "emp-ghost", "typo in manager id")
            .expect("upsert");

        let employees = vec![employee("emp-child")]; // emp-ghost: not an employee
        let entries = db.entries().expect("entries");
        let hierarchy = Hierarchy::new(&employees, &entries);

        let dangling: Vec<_> = hierarchy
            .findings()
            .into_iter()
            .filter(|f| f.employee_id == "emp-child")
            .collect();
        assert!(
            dangling.iter().any(|f| f.kind == DANGLING_MANAGER),
            "expected a {DANGLING_MANAGER} finding for emp-child, got {dangling:?}"
        );
    }

    #[test]
    fn cycle_edges_are_reported_not_repaired() {
        let (_dir, db) = temp_db();
        db.upsert_edge("emp-a", "emp-b", "loop one")
            .expect("upsert a");
        db.upsert_edge("emp-b", "emp-a", "loop two")
            .expect("upsert b");

        let employees = vec![employee("emp-a"), employee("emp-b")];
        let entries = db.entries().expect("entries");
        let hierarchy = Hierarchy::new(&employees, &entries);

        // The store must not have dropped or rewritten either edge.
        assert_eq!(db.edge_count().expect("count"), 2);
        let findings = hierarchy.findings();
        for id in ["emp-a", "emp-b"] {
            assert!(
                findings
                    .iter()
                    .any(|f| f.employee_id == id && f.kind == AUTHORITY_UNRESOLVED),
                "expected an {AUTHORITY_UNRESOLVED} finding for {id}, got {findings:?}"
            );
        }
    }

    #[test]
    fn upsert_overwrites_and_remove_reports_presence() {
        let (_dir, db) = temp_db();
        db.upsert_edge("emp-seat", "emp-first", "initial assignment")
            .expect("first upsert");
        db.upsert_edge("emp-seat", "emp-second", "reassigned")
            .expect("second upsert");
        // One seat, one edge — a re-upsert replaces, never duplicates.
        assert_eq!(db.edge_count().expect("count"), 1);
        assert_eq!(
            db.entries().expect("entries")[0].manager(),
            Some("emp-second")
        );

        assert!(db.remove_edge("emp-seat").expect("remove present"));
        assert!(!db.remove_edge("emp-seat").expect("remove absent"));
        assert_eq!(db.edge_count().expect("count"), 0);
        assert!(db.entries().expect("entries").is_empty());
    }

    #[test]
    fn init_is_idempotent_on_an_existing_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("operant_kanban.db");
        let db = HierarchyEdgesDb::init(&path).expect("first init");
        db.upsert_edge("emp-a", "emp-b", "before reopen")
            .expect("upsert");
        drop(db);

        let reopened = HierarchyEdgesDb::init(&path).expect("second init");
        assert_eq!(reopened.edge_count().expect("count"), 1);
    }
}
