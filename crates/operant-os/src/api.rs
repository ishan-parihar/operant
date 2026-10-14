//! The Holon read API client (boundary doc §4.1, consumer side of work package H2).
//!
//! One struct, one connection, read-only by construction. Holon is the only writer
//! of its databases; operant SELECTs from the published `api_*` views and that is
//! the whole surface. Views land per phase as Holon publishes them:
//!
//! | view | phase | panels |
//! |---|---|---|
//! | `api_employees`, `api_departments` | B | org roster |
//! | `api_board` | B | board panel |
//! | `api_approvals` | B | approvals panel |
//! | `api_messages` | B | DM panel |
//! | `api_triggers` | F | routines panel (event triggers extend it in H7) |

use std::path::{Path, PathBuf};

use crate::{Error, KNOWN_API_VERSIONS, Result};

/// Every table this client may touch. `read_only_view` rejects anything else, so
/// a query cannot reach under the published API even if a caller tries.
const API_VIEWS: &[&str] = &[
    "api_employees",
    "api_departments",
    "api_board",
    "api_approvals",
    "api_messages",
    "api_triggers",
];

/// A Holon employee as published by `api_employees`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Employee {
    pub id: String,
    pub name: String,
    pub title: String,
    pub department: String,
    pub manager_id: Option<String>,
    pub manager: Option<String>,
    pub status: String,
    pub driver: String,
    /// JSON array of role names (Holon stores it as a JSON string column).
    pub roles: Option<String>,
}

/// A Holon department as published by `api_departments`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Department {
    pub id: String,
    pub slug: String,
    pub name: String,
    pub mission: Option<String>,
    pub head_id: Option<String>,
    pub head: Option<String>,
    pub status: String,
    pub profile: Option<String>,
}

/// One board row: a ready or running stage, as published by `api_board`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BoardRow {
    pub id: String,
    pub title: String,
    pub stage: String,
    pub status: String,
    pub lane: Option<String>,
    pub priority: i64,
    pub risk: Option<String>,
    pub department: Option<String>,
    pub assignee: Option<String>,
    pub approver: Option<String>,
    pub grade: Option<i64>,
    pub run_id: Option<String>,
    pub lease_until: Option<String>,
    pub employee_id: Option<String>,
    pub started_at: Option<String>,
}

/// A pending approval, as published by `api_approvals`. `plan` is the plan
/// output's JSON document (steps, writes, grade, …) — the panel renders it.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Approval {
    pub id: String,
    pub title: String,
    pub department: Option<String>,
    pub grade: Option<i64>,
    pub approver: String,
    pub risk: Option<String>,
    pub plan: Option<String>,
    pub created_at: String,
}

/// An unacked message, as published by `api_messages`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Message {
    pub id: String,
    pub ts: String,
    pub sender: String,
    pub to_ref: String,
    pub kind: String,
    pub task_id: Option<String>,
    pub body: String,
}

/// A routine trigger, as published by `api_triggers` (cron today; events land in H7).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Trigger {
    pub id: String,
    pub trigger_kind: String,
    pub pattern: String,
    pub department: Option<String>,
    pub employee: Option<String>,
    pub enabled: bool,
    pub failure_streak: i64,
    pub next_run_at: Option<String>,
}

/// Read-only client over a Holon instance's `work.db`.
///
/// Opened with SQLite `mode=ro` — the connection CANNOT write, so invariant 1
/// (one writer: Holon) holds structurally. `HolonApi::open` also verifies the
/// published `api_version` and refuses one this build does not know.
#[derive(Debug)]
pub struct HolonApi {
    conn: rusqlite::Connection,
    /// $HOLON_HOME, kept for error messages and later phases (run dirs live there).
    pub home: PathBuf,
}

impl HolonApi {
    /// Open the read API for the Holon instance at `home`.
    ///
    /// Fails (never panics, never falls back to a writable handle) when the
    /// instance is missing, predates the read-API migration, or publishes an
    /// unknown version. Callers surface the error and org mode stays off.
    pub fn open(home: &Path) -> Result<Self> {
        let work_db = home.join("execution").join("runtime").join("work.db");
        if !work_db.exists() {
            return Err(Error::WorkDbMissing(work_db));
        }
        // mode=ro: a writable handle is not expressible here — invariant 1 is structural.
        let conn = rusqlite::Connection::open_with_flags(
            format!("file:{}?mode=ro", work_db.display()),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
        )?;
        let published: String = conn
            .query_row(
                "SELECT value FROM api_meta WHERE key = 'api_version'",
                [],
                |row| row.get(0),
            )
            .map_err(|source| Error::ApiNotPublished {
                path: work_db.clone(),
                source,
            })?;
        if !KNOWN_API_VERSIONS.contains(&published.as_str()) {
            return Err(Error::UnknownApiVersion {
                published,
                known: KNOWN_API_VERSIONS,
            });
        }
        Ok(Self {
            conn,
            home: home.to_path_buf(),
        })
    }

    /// Reject any table that is not a published view. Every query helper funnels
    /// through here, so the client cannot reach under the API even by refactor.
    fn read_only_view(view: &str) -> Result<()> {
        if API_VIEWS.contains(&view) {
            Ok(())
        } else {
            // Not one of the api_* views: unreachable from this crate's helpers,
            // but the check keeps future call sites honest.
            Err(Error::Sqlite(rusqlite::Error::InvalidParameterName(
                format!("'{view}' is not a published api_* view"),
            )))
        }
    }

    fn rows<T, F>(&self, view: &str, map: F) -> Result<Vec<T>>
    where
        F: Fn(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    {
        Self::read_only_view(view)?;
        let mut stmt = self.conn.prepare(&format!("SELECT * FROM {view}"))?;
        let rows = stmt.query_map([], map)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// The roster: every employee with department and manager names.
    pub fn employees(&self) -> Result<Vec<Employee>> {
        self.rows("api_employees", |r| {
            Ok(Employee {
                id: r.get(0)?,
                name: r.get(1)?,
                title: r.get(2)?,
                department: r.get(3)?,
                manager_id: r.get(4)?,
                manager: r.get(5)?,
                status: r.get(6)?,
                driver: r.get(7)?,
                roles: r.get(8)?,
            })
        })
    }

    /// Departments with their heads and genome profiles.
    pub fn departments(&self) -> Result<Vec<Department>> {
        self.rows("api_departments", |r| {
            Ok(Department {
                id: r.get(0)?,
                slug: r.get(1)?,
                name: r.get(2)?,
                mission: r.get(3)?,
                head_id: r.get(4)?,
                head: r.get(5)?,
                status: r.get(6)?,
                profile: r.get(7)?,
            })
        })
    }

    /// The board: ready and running stages.
    pub fn board(&self) -> Result<Vec<BoardRow>> {
        self.rows("api_board", |r| {
            Ok(BoardRow {
                id: r.get(0)?,
                title: r.get(1)?,
                stage: r.get(2)?,
                status: r.get(3)?,
                lane: r.get(4)?,
                priority: r.get(5)?,
                risk: r.get(6)?,
                department: r.get(7)?,
                assignee: r.get(8)?,
                approver: r.get(9)?,
                grade: r.get(10)?,
                run_id: r.get(11)?,
                lease_until: r.get(12)?,
                employee_id: r.get(13)?,
                started_at: r.get(14)?,
            })
        })
    }

    /// Pending approvals with the plan documents awaiting a decision.
    pub fn approvals(&self) -> Result<Vec<Approval>> {
        self.rows("api_approvals", |r| {
            Ok(Approval {
                id: r.get(0)?,
                title: r.get(1)?,
                department: r.get(2)?,
                grade: r.get(3)?,
                approver: r.get(4)?,
                risk: r.get(5)?,
                plan: r.get(6)?,
                created_at: r.get(7)?,
            })
        })
    }

    /// Unacked messages (the Chair's and employees' inboxes).
    pub fn messages(&self) -> Result<Vec<Message>> {
        self.rows("api_messages", |r| {
            Ok(Message {
                id: r.get(0)?,
                ts: r.get(1)?,
                sender: r.get(2)?,
                to_ref: r.get(3)?,
                kind: r.get(4)?,
                task_id: r.get(5)?,
                body: r.get(6)?,
            })
        })
    }

    /// Routine triggers (cron patterns today; event triggers land in phase F).
    pub fn triggers(&self) -> Result<Vec<Trigger>> {
        self.rows("api_triggers", |r| {
            Ok(Trigger {
                id: r.get(0)?,
                trigger_kind: r.get(1)?,
                pattern: r.get(2)?,
                department: r.get(3)?,
                employee: r.get(4)?,
                enabled: r.get::<_, i64>(5)? != 0,
                failure_streak: r.get(6)?,
                next_run_at: r.get(7)?,
            })
        })
    }
}

#[cfg(test)]
mod tests {

    /// Invariant 1 (scoped to the read API): a connection built the way `open`
    /// builds it (URI + mode=ro + SQLITE_OPEN_URI) rejects writes at the SQLite
    /// layer — mode=ro is structural, not policy.
    #[test]
    fn opened_connection_rejects_writes() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("probe.db");
        rusqlite::Connection::open(&db)
            .unwrap()
            .execute("CREATE TABLE t (x)", [])
            .unwrap();
        let conn = rusqlite::Connection::open_with_flags(
            format!("file:{}?mode=ro", db.display()),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
        )
        .expect("uri open");
        let err = conn
            .execute("CREATE TABLE probe (x)", [])
            .expect_err("mode=ro must reject writes");
        assert!(
            err.to_string().to_lowercase().contains("readonly"),
            "unexpected error shape: {err}"
        );
    }
}
