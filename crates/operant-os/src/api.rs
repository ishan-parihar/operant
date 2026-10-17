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
    "api_units",    // v4 (H2/R-04): the capability layer's units, with their owner
    "api_services", // v4: the per-service contract (effect/args/output/note/health)
    "api_pool",     // v4: the pool edges a runtime indexes ("*" = org-wide surface)
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

/// A capability unit as published by `api_units` (v4): a pooled service, a surface, or a
/// department's own declared genome. `owner` names the department that keeps a surface true.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Unit {
    pub slug: String,
    pub charter: String,
    pub category: Option<String>,
    pub tier: Option<String>,
    pub owner: Option<String>,
    pub synced_at: Option<String>,
}

/// One service a unit offers, as published by `api_services` (v4). A surface declares its
/// contract as `args`/`output`; a foundation as `accepts`/`returns`. `effect` rules which
/// posture may take the route (a write route is act-stage only); `note` is the prefer-this
/// route line; `health` is the kernel's liveness probe command.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Service {
    pub unit: String,
    pub service: String,
    pub command: Option<String>,
    pub accepts: Option<String>,
    pub returns: Option<String>,
    pub effect: Option<String>,
    pub args: Option<String>,
    pub output: Option<String>,
    pub note: Option<String>,
    pub health: Option<String>,
}

/// One pool edge, as published by `api_pool` (v4): the unit, and the department that may use
/// it. `"*"` in `dept` means org-wide — every entitled brief, including department-less work.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PoolEdge {
    pub unit: String,
    pub dept: String,
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

    /// The capability layer's units (v4, H2/R-04): pooled services and surfaces, with the
    /// owner that keeps a surface true.
    pub fn units(&self) -> Result<Vec<Unit>> {
        self.rows("api_units", |r| {
            Ok(Unit {
                slug: r.get(0)?,
                charter: r.get(1)?,
                category: r.get(2)?,
                tier: r.get(3)?,
                owner: r.get(4)?,
                synced_at: r.get(5)?,
            })
        })
    }

    /// The per-service contract (v4): what each unit's services do, how to call them, and
    /// which posture may. The future deferred-tool index consumes exactly this shape.
    pub fn services(&self) -> Result<Vec<Service>> {
        self.rows("api_services", |r| {
            Ok(Service {
                unit: r.get(0)?,
                service: r.get(1)?,
                command: r.get(2)?,
                accepts: r.get(3)?,
                returns: r.get(4)?,
                effect: r.get(5)?,
                args: r.get(6)?,
                output: r.get(7)?,
                note: r.get(8)?,
                health: r.get(9)?,
            })
        })
    }

    /// The pool edges (v4): which department may use which unit; "*" is org-wide. The future
    /// deferred-tool index derives an entitlement filter from this, never from names.
    pub fn pool(&self) -> Result<Vec<PoolEdge>> {
        self.rows("api_pool", |r| {
            Ok(PoolEdge {
                unit: r.get(0)?,
                dept: r.get(1)?,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// iter-885 (capability-layer audit §2): a kernel that publishes
    /// api_version = 4 must not turn org mode off — v4 is accepted BEFORE it
    /// exists (the cross-repo order: operant knows 4 first, the kernel
    /// migrates second; a v4 kernel against a v3-only runtime would have
    /// gone dark on the live box). A version beyond the known set still
    /// refuses: stale view shapes must never be read through.
    #[test]
    fn a_v4_kernel_is_read_and_an_unknown_still_refuses() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let db_dir = home.join("execution").join("runtime");
        std::fs::create_dir_all(&db_dir).unwrap();
        let seed = |version: &str| {
            let db = db_dir.join("work.db");
            let _ = std::fs::remove_file(&db);
            let conn = rusqlite::Connection::open(&db).unwrap();
            conn.execute(
                "CREATE TABLE api_meta (key TEXT PRIMARY KEY, value TEXT)",
                [],
            )
            .unwrap();
            conn.execute("INSERT INTO api_meta VALUES ('api_version', ?)", [version])
                .unwrap();
        };

        seed("4");
        let opened = HolonApi::open(&home);
        assert!(
            opened.is_ok(),
            "a v4 kernel must keep org mode on: {opened:?}"
        );

        seed("5");
        let refused = HolonApi::open(&home);
        assert!(
            refused.is_err(),
            "an api_version this build does not know still refuses: {refused:?}"
        );
    }

    /// iter-907 (H2/R-04, consumer side): a v4 kernel publishes the capability layer — units
    /// with their owner, services with the per-service contract, and the pool edges. The
    /// client reads all three; the future deferred-tool index (svc_<unit>_<service>) consumes
    /// exactly this shape and derives entitlements from api_pool, never from names.
    #[test]
    fn a_v4_kernel_publishes_the_capability_layer() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let db_dir = home.join("execution").join("runtime");
        std::fs::create_dir_all(&db_dir).unwrap();
        let db = db_dir.join("work.db");
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute(
            "CREATE TABLE api_meta (key TEXT PRIMARY KEY, value TEXT)",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO api_meta VALUES ('api_version', '4')", [])
            .unwrap();
        conn.execute(
            "CREATE TABLE api_units (slug TEXT, charter TEXT, category TEXT, tier TEXT,
             owner TEXT, synced_at TEXT)",
            [],
        )
        .unwrap();
        conn.execute(
            "CREATE TABLE api_services (unit TEXT, service TEXT, command TEXT, accepts TEXT,
             returns TEXT, effect TEXT, args TEXT, output TEXT, note TEXT, health TEXT)",
            [],
        )
        .unwrap();
        conn.execute("CREATE TABLE api_pool (unit TEXT, dept TEXT)", [])
            .unwrap();
        conn.execute(
            "INSERT INTO api_units VALUES ('browser', 'the web surface', 'surface', 'core',
             'platform-ops', '2026-10-17T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO api_services VALUES ('browser', 'sourcehound', 'sourcehound', NULL,
             NULL, 'read', 'sourcehound <group> <query>', 'json', 'prefer for any body',
             'cloakctl doctor --json')",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO api_pool VALUES ('browser', '*')", [])
            .unwrap();

        let api = HolonApi::open(&home).unwrap();
        let units = api.units().unwrap();
        assert_eq!(units.len(), 1);
        assert_eq!(units[0].slug, "browser");
        assert_eq!(units[0].category.as_deref(), Some("surface"));
        assert_eq!(units[0].owner.as_deref(), Some("platform-ops"));

        let services = api.services().unwrap();
        assert_eq!(services.len(), 1);
        assert_eq!(services[0].effect.as_deref(), Some("read"));
        assert_eq!(
            services[0].args.as_deref(),
            Some("sourcehound <group> <query>")
        );
        assert_eq!(services[0].output.as_deref(), Some("json"));

        let pool = api.pool().unwrap();
        assert_eq!(pool.len(), 1);
        assert_eq!(pool[0].dept, "*", "the org-wide surface edge is published");
    }
}
