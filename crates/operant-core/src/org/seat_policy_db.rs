//! The `seat_policies` sqlite store — wave-1 slice A of
//! `docs/NEXT-IMPLEMENTATION-OUTLINE.md` §11, answering alignment Q1 with the
//! recommended default (a sqlite table).
//!
//! Mirrors [`crate::org::authority::GrantDb`]: DDL as a `pub const` schema
//! string, idempotent [`SeatPolicyDb::init`], `Arc<Mutex<Connection>>` with
//! poisoning treated as a recoverable error, and the same table-not-a-file
//! choice (`seat_policies` is a table inside a database the kanban family
//! owns — see the authority module docs for why a second file would repeat
//! `BUGS.md` R5-1's split-brain).
//!
//! ## Mode parsing is loud, never defaulting
//!
//! `mode` is stored as [`SeatMode::as_str`] and parsed back with
//! [`SeatMode: FromStr`](std::str::FromStr). A row carrying a spelling this
//! build does not know is an **error on read**, not a silent fallback: a
//! typo'd `lockdown` degrading to `standard` would silently re-open tools a
//! seat was shut out of, which is the exact widening the genome exists to
//! make impossible.

use crate::error::Error;
use crate::org::notice::rfc3339;
use crate::org::seat_policy::{SeatMode, SeatPolicy, SeatPolicySource};
use rusqlite::{Connection, OptionalExtension, Row, params};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Arc, Mutex, MutexGuard};

/// The `seat_policies` store.
///
/// One row per employee seat. `allow`/`deny` are JSON arrays of glob
/// patterns (serde_json), so a policy round-trips without a join table.
pub struct SeatPolicyDb {
    conn: Arc<Mutex<Connection>>,
}

impl SeatPolicyDb {
    /// DDL, applied declaratively and idempotently.
    ///
    /// Same "never `crate::migrations::migrate`, never `PRAGMA user_version`"
    /// discipline as [`crate::org::authority::GrantDb::GRANTS_SCHEMA`] — the
    /// counter belongs to the kanban family that owns the file.
    pub const SEAT_POLICIES_SCHEMA: &str = r#"
        CREATE TABLE IF NOT EXISTS seat_policies (
            employee_id TEXT PRIMARY KEY,
            mode        TEXT NOT NULL,   -- SeatMode::as_str(); unknown spelling = read error
            allow       TEXT NOT NULL,   -- JSON array of glob patterns
            deny        TEXT NOT NULL,   -- JSON array of glob patterns
            delegation  TEXT,            -- DelegationPosture::as_str(); NULL = no posture = ungoverned
            updated_at  TEXT NOT NULL    -- RFC3339, fixed millisecond precision
        );
    "#;

    /// The additive column pre-697 files lack. Applied via the org layer's
    /// `ensure_column` probe (never `PRAGMA user_version` — same discipline
    /// as the schema above and `decisions_db`'s ALTER family).
    const DELEGATION_COLUMN_DEF: &str = "TEXT";

    /// Open (or create) the seat_policies table in a sqlite file.
    ///
    /// Tests point this at a temp path; production points it at the shared
    /// kanban-family file via [`SeatPolicyDb::from_shared_connection`].
    pub fn init(path: PathBuf) -> Result<Self, Error> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)
                .map_err(|e| Error::Agent(format!("seat policies: create db dir: {e}")))?;
        }
        let conn = Connection::open(&path)
            .map_err(|e| Error::Agent(format!("seat policies: open {}: {e}", path.display())))?;
        Self::from_connection(conn)
    }

    /// Attach the store to a connection somebody else already owns.
    pub fn from_connection(conn: Connection) -> Result<Self, Error> {
        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        db.ensure_schema()?;
        Ok(db)
    }

    /// Borrow an already-open `Arc<Mutex<Connection>>` so seat policies share
    /// that handle instead of opening a second writer.
    pub fn from_shared_connection(conn: Arc<Mutex<Connection>>) -> Result<Self, Error> {
        let db = Self { conn };
        db.ensure_schema()?;
        Ok(db)
    }

    /// The underlying connection.
    pub fn conn(&self) -> &Arc<Mutex<Connection>> {
        &self.conn
    }

    /// Lock the connection, turning poisoning into a recoverable error rather
    /// than a panic (same as `authority.rs` and `kanban/db.rs`).
    fn lock_conn(&self) -> Result<MutexGuard<'_, Connection>, Error> {
        self.conn
            .lock()
            .map_err(|_| Error::Agent("seat policies db mutex poisoned".to_string()))
    }

    fn ensure_schema(&self) -> Result<(), Error> {
        let conn = self.lock_conn()?;
        conn.execute_batch(Self::SEAT_POLICIES_SCHEMA)
            .map_err(|e| Error::Agent(format!("seat policies: schema: {e}")))?;
        // Pre-697 files: add `delegation` idempotently (additive-safe probe —
        // the org layer's own rule, see the schema note in authority.rs).
        crate::org::schema::ensure_column(
            &conn,
            "seat_policies",
            "delegation",
            Self::DELEGATION_COLUMN_DEF,
        )
        .map_err(|e| Error::Agent(format!("seat policies: delegation column: {e}")))
        .map(|_| ())
    }

    /// Write one seat's policy, replacing any previous row for that seat
    /// (`employee_id` is the primary key, so this is a true upsert).
    pub fn upsert(&self, employee_id: &str, policy: &SeatPolicy) -> Result<(), Error> {
        let allow = serde_json::to_string(&policy.allow)
            .map_err(|e| Error::Agent(format!("seat policies: serialize allow: {e}")))?;
        let deny = serde_json::to_string(&policy.deny)
            .map_err(|e| Error::Agent(format!("seat policies: serialize deny: {e}")))?;
        let conn = self.lock_conn()?;
        conn.execute(
            "INSERT INTO seat_policies (employee_id, mode, allow, deny, delegation, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(employee_id) DO UPDATE SET
                 mode = excluded.mode,
                 allow = excluded.allow,
                 deny  = excluded.deny,
                 delegation = excluded.delegation,
                 updated_at = excluded.updated_at",
            params![
                employee_id,
                policy.mode.as_str(),
                allow,
                deny,
                policy.delegation.map(|p| p.as_str()),
                rfc3339(chrono::Utc::now()),
            ],
        )
        .map_err(|e| Error::Agent(format!("seat policies: upsert {employee_id}: {e}")))?;
        Ok(())
    }

    /// Read one seat's policy. `None` = no row = ungoverned seat.
    ///
    /// Errors (does not default) when a row's `mode` or `allow`/`deny` JSON is
    /// corrupt — see the module docs for why a silent fallback is forbidden.
    pub fn get(&self, employee_id: &str) -> Result<Option<SeatPolicy>, Error> {
        let conn = self.lock_conn()?;
        let raw = conn
            .query_row(
                "SELECT mode, allow, deny, delegation FROM seat_policies WHERE employee_id = ?1",
                params![employee_id],
                Self::row_to_raw,
            )
            .optional()
            .map_err(|e| Error::Agent(format!("seat policies: get {employee_id}: {e}")))?;
        raw.map(|(mode, allow, deny, delegation)| {
            columns_to_policy(employee_id, mode, allow, deny, delegation)
        })
        .transpose()
    }

    /// Delete one seat's policy row. `false` = there was no row to remove
    /// (removing an ungoverned seat is not an error — it is already mainline).
    pub fn remove(&self, employee_id: &str) -> Result<bool, Error> {
        let conn = self.lock_conn()?;
        let n = conn
            .execute(
                "DELETE FROM seat_policies WHERE employee_id = ?1",
                params![employee_id],
            )
            .map_err(|e| Error::Agent(format!("seat policies: remove {employee_id}: {e}")))?;
        Ok(n > 0)
    }

    /// Every stored policy, with its `employee_id`.
    pub fn list(&self) -> Result<Vec<(String, SeatPolicy)>, Error> {
        let conn = self.lock_conn()?;
        let mut stmt = conn
            .prepare("SELECT employee_id, mode, allow, deny, delegation FROM seat_policies")
            .map_err(|e| Error::Agent(format!("seat policies: list: {e}")))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            })
            .map_err(|e| Error::Agent(format!("seat policies: list: {e}")))?;
        let mut out = Vec::new();
        for row in rows {
            let (employee_id, mode, allow, deny, delegation) =
                row.map_err(|e| Error::Agent(format!("seat policies: list row: {e}")))?;
            out.push((
                employee_id.clone(),
                columns_to_policy(&employee_id, mode, allow, deny, delegation)?,
            ));
        }
        Ok(out)
    }

    /// Read one result row's raw columns. Stays in `rusqlite` error space
    /// because that is the mapper closure's contract; validation that needs
    /// the crate's loud [`Error`] happens in [`columns_to_policy`].
    fn row_to_raw(row: &Row<'_>) -> rusqlite::Result<(String, String, String, Option<String>)> {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
    }
}

/// Validate one row's `mode` + `allow`/`deny` JSON into a [`SeatPolicy`].
///
/// A `mode` spelling this build does not know is an error naming the seat
/// and the raw string, never a silent default (a `lockdown` typo degrading
/// to `standard` would silently re-open tools a seat was shut out of).
fn columns_to_policy(
    employee_id: &str,
    mode: String,
    allow: String,
    deny: String,
    delegation: Option<String>,
) -> Result<SeatPolicy, Error> {
    let mode = SeatMode::from_str(&mode).map_err(|e| {
        Error::Agent(format!(
            "seat policies: {employee_id}: stored policy is unreadable: {e}"
        ))
    })?;
    let allow: Vec<String> = serde_json::from_str(&allow)
        .map_err(|e| Error::Agent(format!("seat policies: {employee_id}: allow column: {e}")))?;
    let deny: Vec<String> = serde_json::from_str(&deny)
        .map_err(|e| Error::Agent(format!("seat policies: {employee_id}: deny column: {e}")))?;
    let delegation = delegation
        .map(|raw| {
            crate::org::authority::DelegationPosture::from_str(&raw).map_err(|e| {
                Error::Agent(format!(
                    "seat policies: {employee_id}: stored policy is unreadable: {e}"
                ))
            })
        })
        .transpose()?;
    Ok(SeatPolicy {
        mode,
        allow,
        deny,
        delegation,
    })
}

impl SeatPolicySource for SeatPolicyDb {
    fn policy_for(&self, employee_id: &str) -> Option<SeatPolicy> {
        // A corrupt row must not silently widen the seat to "no policy" —
        // log it loudly. The trait has no error channel by design (the run
        // path must not await and must not panic); the loud path is `get`.
        match self.get(employee_id) {
            Ok(policy) => policy,
            Err(e) => {
                tracing::error!(
                    "seat policy read failed for {employee_id}; treating as ungoverned: {e}"
                );
                None
            }
        }
    }

    fn delegation_posture_for(
        &self,
        employee_id: &str,
    ) -> Option<crate::org::authority::DelegationPosture> {
        // Same loud-on-corrupt contract as `policy_for`: an unreadable row
        // is treated as ungoverned HERE (never a widening — `None` is
        // Independent, today's behaviour) with `get` as the loud path.
        match self.get(employee_id) {
            Ok(policy) => policy.and_then(|p| p.delegation),
            Err(e) => {
                tracing::error!(
                    "seat policy read failed for {employee_id}; delegation treated as ungoverned: {e}"
                );
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn mem_db() -> SeatPolicyDb {
        SeatPolicyDb::from_connection(Connection::open_in_memory().expect("in-memory"))
            .expect("open")
    }

    fn policy(mode: SeatMode, allow: &[&str], deny: &[&str]) -> SeatPolicy {
        SeatPolicy {
            mode,
            allow: allow.iter().map(|s| s.to_string()).collect(),
            deny: deny.iter().map(|s| s.to_string()).collect(),
        delegation: None,
        }
    }

    #[test]
    fn upsert_then_get_round_trips() {
        let db = mem_db();
        let p = policy(
            SeatMode::Scoped,
            &["content.*", "memory_search"],
            &["shell"],
        );
        db.upsert("emp-1", &p).expect("upsert");
        assert_eq!(db.get("emp-1").expect("get"), Some(p));
    }

    #[test]
    fn missing_employee_id_is_none() {
        let db = mem_db();
        assert_eq!(db.get("emp-404").expect("get"), None);
    }

    #[test]
    fn second_upsert_replaces_the_first() {
        let db = mem_db();
        db.upsert("emp-1", &policy(SeatMode::Standard, &[], &[]))
            .expect("first upsert");
        let replacement = policy(
            SeatMode::Lockdown,
            &["memory_search"],
            &["shell", "jcode.*"],
        );
        db.upsert("emp-1", &replacement).expect("second upsert");
        assert_eq!(db.get("emp-1").expect("get"), Some(replacement));
        assert_eq!(db.list().expect("list").len(), 1, "upsert, not append");
    }

    #[test]
    fn empty_allow_and_deny_round_trip() {
        let db = mem_db();
        let p = policy(SeatMode::Standard, &[], &[]);
        db.upsert("emp-1", &p).expect("upsert");
        assert_eq!(db.get("emp-1").expect("get"), Some(p));
    }

    #[test]
    fn a_bad_mode_string_errors_on_read_not_defaults() {
        let db = mem_db();
        db.upsert("emp-1", &policy(SeatMode::Lockdown, &[], &[]))
            .expect("upsert");
        // Corrupt the row the way a hand edit or an older build would.
        let conn = db.conn();
        let conn = conn.lock().expect("lock");
        conn.execute(
            "UPDATE seat_policies SET mode = 'lockdwn' WHERE employee_id = 'emp-1'",
            [],
        )
        .expect("corrupt");
        drop(conn);

        let err = db.get("emp-1").expect_err("unreadable mode must be loud");
        assert!(
            err.to_string().contains("lockdwn"),
            "error must name the raw string: {err}"
        );
        // And the storage seam reports the same refusal rather than
        // silently widening the seat.
        assert_eq!(db.policy_for("emp-1"), None);
    }

    #[test]
    fn remove_deletes_and_reports_absence() {
        let db = mem_db();
        db.upsert("emp-1", &policy(SeatMode::Yolo, &[], &[]))
            .expect("upsert");
        assert!(db.remove("emp-1").expect("remove"));
        assert_eq!(db.get("emp-1").expect("get"), None);
        assert!(!db.remove("emp-1").expect("remove again"));
    }

    #[test]
    fn list_returns_every_seat_with_its_id() {
        let db = mem_db();
        let p1 = policy(SeatMode::Scoped, &["content.*"], &[]);
        let p2 = policy(SeatMode::Lockdown, &[], &["shell"]);
        db.upsert("emp-1", &p1).expect("upsert 1");
        db.upsert("emp-2", &p2).expect("upsert 2");
        let mut listed = db.list().expect("list");
        listed.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            listed,
            vec![("emp-1".to_string(), p1), ("emp-2".to_string(), p2)]
        );
    }

    #[test]
    fn init_creates_the_parent_directory_idempotently() {
        let dir = std::env::temp_dir().join(format!(
            "operant_seat_policies_{}_{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let path = dir.join("nested").join("operant_kanban.db");
        let first = SeatPolicyDb::init(path.clone()).expect("first init");
        first
            .upsert("emp-1", &policy(SeatMode::Standard, &[], &[]))
            .expect("upsert");
        assert!(Path::new(&path).exists(), "the file must be created");
        let second = SeatPolicyDb::init(path).expect("second init");
        assert_eq!(
            second.get("emp-1").expect("get"),
            Some(policy(SeatMode::Standard, &[], &[])),
            "CREATE TABLE IF NOT EXISTS must not disturb existing rows"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── delegation posture column (P0 delegation governance) ──────────

    #[test]
    fn delegation_posture_round_trips_and_defaults_to_none() {
        let db = mem_db();
        let mut with_posture = policy(SeatMode::Standard, &[], &[]);
        with_posture.delegation = Some(crate::org::authority::DelegationPosture::Bounded);
        db.upsert("emp-1", &with_posture).expect("upsert");
        assert_eq!(
            db.get("emp-1").expect("get").and_then(|p| p.delegation),
            Some(crate::org::authority::DelegationPosture::Bounded)
        );
        // A row without a posture (the pre-697 shape) reads as ungoverned.
        db.upsert("emp-2", &policy(SeatMode::Standard, &[], &[]))
            .expect("upsert");
        assert_eq!(
            db.get("emp-2").expect("get").and_then(|p| p.delegation),
            None
        );
        // The trait override reads the same surface.
        assert_eq!(
            db.delegation_posture_for("emp-1"),
            Some(crate::org::authority::DelegationPosture::Bounded)
        );
        assert_eq!(db.delegation_posture_for("emp-2"), None);
    }

    #[test]
    fn a_bad_delegation_spelling_errors_on_read_not_defaults() {
        let db = mem_db();
        db.upsert("emp-1", &policy(SeatMode::Standard, &[], &[]))
            .expect("upsert");
        db.lock_conn()
            .expect("conn")
            .execute(
                "UPDATE seat_policies SET delegation = 'sometimes' WHERE employee_id = 'emp-1'",
                [],
            )
            .expect("seed bad spelling");
        let err = db.get("emp-1").expect_err("bad spelling must error");
        assert!(err.to_string().contains("unknown delegation posture"));
        // And the trait surface degrades loudly to ungoverned, never widens.
        assert_eq!(db.delegation_posture_for("emp-1"), None);
    }

    #[test]
    fn pre_697_file_gains_the_delegation_column_idempotently() {
        // The old five-column shape, hand-built: opening through the store
        // must ALTER it in place without disturbing the row.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("old-seat-policies.sqlite");
        {
            let conn = Connection::open(&path).expect("open");
            conn.execute_batch(
                "CREATE TABLE seat_policies (
                    employee_id TEXT PRIMARY KEY,
                    mode TEXT NOT NULL, allow TEXT NOT NULL,
                    deny TEXT NOT NULL, updated_at TEXT NOT NULL);
                INSERT INTO seat_policies VALUES ('emp-1', 'standard', '[]', '[]', '2020-01-01');",
            )
            .expect("old schema");
        }
        let db = SeatPolicyDb::init(path).expect("migrated open");
        assert_eq!(
            db.get("emp-1").expect("get"),
            Some(policy(SeatMode::Standard, &[], &[])),
            "the migration must preserve the pre-697 row"
        );
        // Idempotent: a second open must not re-ALTER (ensure_column no-ops).
        // A write through the migrated surface proves the column is live.
        let mut with_posture = policy(SeatMode::Standard, &[], &[]);
        with_posture.delegation = Some(crate::org::authority::DelegationPosture::Forbidden);
        db.upsert("emp-1", &with_posture).expect("upsert post-migration");
        assert_eq!(
            db.get("emp-1").expect("get").and_then(|p| p.delegation),
            Some(crate::org::authority::DelegationPosture::Forbidden)
        );
    }
}
