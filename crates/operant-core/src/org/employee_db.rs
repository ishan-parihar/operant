//! Employee registry store: sqlite DDL, reads, and the deterministic
//! backfill from the live `CronJob`.
//!
//! Spec: `docs/WAVE1-DECISIONS.md` §3.1 (DDL + types), §3.1.1 (backfill
//! field map), §3.1.2 (`employee_id` derivation and collisions).
//!
//! ## Storage location (§1.2, Q2)
//!
//! The tables live in the **existing** kanban sqlite file, reached via
//! [`org_db_path`]. No new database file, no JSONL — the "no second store"
//! rule (`OUTLINE.md:277-280`).
//!
//! ## Backfill contract
//!
//! Deterministic and idempotent. Running it twice produces the same rows
//! and the same ids:
//!
//! - `employee_id` is *derived* from the cron job id, never generated
//!   (§3.1.2), so it is stable across runs.
//! - writes are `INSERT ... ON CONFLICT(employee_id) DO UPDATE` on the
//!   mutable fields and `INSERT ... ON CONFLICT(employee_id, cron_job_id)
//!   DO UPDATE` on the link row, so a re-run converges instead of
//!   duplicating.
//! - `created_at` is taken from the cron job, not from "now", so a re-run
//!   does not drift it. `updated_at` *is* now, and is the one field a
//!   re-run legitimately refreshes — a second backfill genuinely did
//!   re-verify the row.
//!
//! ## What backfill refuses to do
//!
//! **It does not repair `skills`.** A job with `skills: None` or
//! `skills: Some(vec![])` backfills to `[]`, faithfully, and stays `[]`.
//! §2.2 measured 6 of the 102 real organism jobs in exactly that state.
//! Silently inventing a skill would hide the very rows the fail-closed
//! gate exists to surface. [`Employee::missing_required_fields`] is the
//! predicate that makes them visible; packet B wires it to the tick.

use rusqlite::{Connection, OptionalExtension, params};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

// `CronJob` is not re-exported from `cronjobs::mod`; the live struct is
// `cronjobs::db::CronJob` (`db.rs:37-69`, 31 fields). The other
// `CronJob` in the workspace — `operant-runtime/src/cron/types.rs:143`,
// 20 fields — is never spawned by the CLI and is deliberately NOT the
// backfill source (WAVE1-DECISIONS §3.1.1).
use crate::cronjobs::db::CronJob;
use crate::error::Error;

use super::employee::{
    AgentType, CRON_STATE_SCHEDULED, DEFAULT_ROLE, Employee, EmployeeCronJob, STATUS_ACTIVE,
    derive_employee_id,
};

/// The kanban sqlite file that holds the registry, derived from the main
/// app database path by the same sibling-DB rule `main.rs:967-971` and
/// `cmd_cron.rs:82-92` use.
///
/// The `parent()` fallback mirrors `cmd_cron.rs:86-89` exactly. Note this
/// is a *path* derivation, not a `org_root` config key — §1.2 rejected a
/// second precedence rule, and `$HERMES_HOME` already provides the
/// override via `operant_home()`.
pub fn org_db_path(database_path: &Path) -> PathBuf {
    database_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("operant_kanban.db")
}

/// What one backfill run did. Every count is derivable from the registry
/// itself; the report exists so the caller can log the *diff* and so a
/// collision (§3.1.2) is impossible to miss in a report-only run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BackfillReport {
    /// Cron jobs offered to the backfill.
    pub jobs_seen: usize,
    /// Employee rows written (inserted or updated).
    pub employees_written: usize,
    /// `employee_cron_jobs` link rows written.
    pub links_written: usize,
    /// Jobs that backfilled to a row failing the Wave 1 required set.
    /// Expected non-zero against a real DB: ~6 of 102 (§2.2). These are
    /// the rows the gate must surface, and backfill does **not** hide
    /// them.
    pub invalid: Vec<InvalidEmployee>,
    /// Jobs skipped because their derived `employee_id` collided with a
    /// different cron job's (§3.1.2: fail loudly, skip, never append a
    /// disambiguator).
    pub collisions: Vec<EmployeeIdCollision>,
}

/// A backfilled row that does not satisfy the Wave 1 required set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidEmployee {
    pub cron_job_id: String,
    pub employee_id: String,
    /// Names of the missing required fields, in §3.2 order.
    pub missing: Vec<&'static str>,
}

/// Two distinct cron jobs derived the same `employee_id` from the
/// 12-character truncation. Per §3.1.2 the second job is skipped and
/// reported — determinism beats cleverness, a collision is a bug to see.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmployeeIdCollision {
    pub cron_job_id: String,
    pub employee_id: String,
    /// The cron job that already owns this id.
    pub conflicting_cron_job_id: String,
}

pub struct EmployeeDb {
    conn: Arc<Mutex<Connection>>,
}

impl EmployeeDb {
    /// Open (or create) the registry at an **exact file path**. The DDL is
    /// idempotent, so this is safe to call against a live kanban DB and on
    /// every boot.
    ///
    /// The name says what it does because the org layer has two different path
    /// contracts and they were previously both called `init`: this one takes
    /// the literal file, [`Self::for_app`] takes the *main* app database and
    /// derives the sibling. Calling the literal form with the main path opened
    /// the wrong file with no error, which is how the daemon and the `org` CLI
    /// ended up writing two different employee registries. Prefer [`Self::for_app`]
    /// unless you genuinely hold the sibling path.
    pub fn open_at(path: PathBuf) -> Result<Self, Error> {
        let conn = Connection::open(&path)
            .map_err(|e| Error::Agent(format!("Failed to open org database: {}", e)))?;

        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
        };

        db.setup_schema()?;
        Ok(db)
    }

    /// Open the registry from the *main* app database path, deriving the
    /// kanban sibling through the one canonical helper.
    ///
    /// This is the form every production caller should use: it cannot disagree
    /// with the `org` CLI or the doctor, because all three go through
    /// [`org_db_path`]. Mirrors [`super::decisions_db::DecisionsDb::for_app`].
    pub fn for_app(database_path: &Path) -> Result<Self, Error> {
        Self::open_at(org_db_path(database_path))
    }

    /// Open the registry on a connection somebody else already owns.
    ///
    /// The registry is a table inside the kanban database, not a file of its
    /// own. A caller that already holds the shared handle — the same shape
    /// `kanban/db.rs` hands out — should pass it here rather than opening a
    /// second writer on the same file.
    pub fn from_shared_connection(conn: Arc<Mutex<Connection>>) -> Result<Self, Error> {
        let db = Self { conn };
        db.setup_schema()?;
        Ok(db)
    }

    /// Lock the SQLite connection, converting mutex poisoning into a
    /// recoverable error instead of panicking (same pattern as
    /// `kanban/db.rs:141` and `cronjobs/db.rs:101`).
    fn lock_conn(&self) -> Result<std::sync::MutexGuard<'_, Connection>, Error> {
        self.conn
            .lock()
            .map_err(|_| Error::Agent("org db mutex poisoned".to_string()))
    }

    /// §3.1 DDL, verbatim, plus the three indexes.
    ///
    /// Declared as a **separate migration family** from `kanban`'s, and
    /// it does *not* go through `crate::migrations::migrate`. That runner
    /// stamps `PRAGMA user_version`, which is **file-wide** in SQLite —
    /// `migrations.rs:31-47` documents the invariant: "every subsystem
    /// that declares its own `MIGRATIONS` array MUST point at its own
    /// dedicated DB file." The kanban file is already claimed by the
    /// kanban family at v1; a second family on the same file would
    /// hard-fail as soon as either side bumps the shared version, which
    /// is exactly the R39-7 class of bug the doc warns about.
    ///
    /// So the org tables are created with plain `CREATE TABLE IF NOT
    /// EXISTS` / `CREATE INDEX IF NOT EXISTS`, applied in one
    /// `execute_batch`. That is already idempotent at the SQL level, so
    /// it needs no version tracking — and it leaves the kanban family's
    /// `user_version` untouched, so neither subsystem can downgrade the
    /// other.
    const SCHEMA: &str = r#"
        CREATE TABLE IF NOT EXISTS employees (
            employee_id      TEXT PRIMARY KEY,          -- 'emp-' || 12 hex; see §3.1.1
            name             TEXT NOT NULL,             -- human label, from cron job name
            role             TEXT NOT NULL DEFAULT 'automator',
            department       TEXT,                      -- NULL until Wave 3 registry
            skills           TEXT NOT NULL DEFAULT '[]',-- JSON array of leaf names
            agent_type       TEXT,                      -- NULL|session|service|fixer  (§3.1.2)
            persona          TEXT,                      -- JSON Persona, NULL until Wave 2
            system_prompt    TEXT,                      -- the charter (ORGANISM-ARCHITECTURE §2); NULL on cron backfill
            status           TEXT NOT NULL DEFAULT 'active',
            reason           TEXT NOT NULL,             -- the --reason of the creating write
            created_at       TEXT NOT NULL,             -- RFC3339
            updated_at       TEXT NOT NULL              -- RFC3339
        );

        CREATE TABLE IF NOT EXISTS employee_cron_jobs (
            employee_id  TEXT NOT NULL,
            cron_job_id  TEXT NOT NULL,
            schedule     TEXT,          -- human display string, e.g. '*/15 * * * *'
            enabled      INTEGER NOT NULL DEFAULT 1,
            PRIMARY KEY (employee_id, cron_job_id),
            FOREIGN KEY (employee_id) REFERENCES employees(employee_id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_employees_dept     ON employees(department);
        CREATE INDEX IF NOT EXISTS idx_employees_status   ON employees(status);
        CREATE INDEX IF NOT EXISTS idx_employee_cron_job  ON employee_cron_jobs(cron_job_id);
    "#;

    fn setup_schema(&self) -> Result<(), Error> {
        let conn = self.lock_conn()?;
        conn.execute_batch(Self::SCHEMA)
            .map_err(|e| Error::Agent(format!("org schema failed: {}", e)))?;
        // Additive reconciliation for an `employees` table written before
        // the charter column existed (ORGANISM-ARCHITECTURE §2). `ensure_column`
        // is a no-op when the column is present — the normal case — and the
        // org layer never touches `PRAGMA user_version` (see `schema.rs`).
        crate::org::schema::ensure_columns(&conn, "employees", &[("system_prompt", "TEXT")])
            .map_err(|e| Error::Agent(format!("org schema failed: {e}")))?;
        Ok(())
    }

    /// Map one `CronJob` to its `Employee` and link row, per the §3.1.1
    /// field map. Pure — no I/O — so the map is unit-testable on its own.
    ///
    /// The three fields §3.1.1 names as absent from the live struct
    /// (`source`, `allowed_tools`, `uses_memory`) are deliberately not
    /// read. `skills` *is* read, from `db.rs:50`.
    /// `reason` is the caller-supplied justification, stored verbatim on every
    /// row this call writes. It must be the *operator's* words when an
    /// operator is driving (the CLI's mandatory `--reason`), so the registry
    /// names the party actually accountable for the write. Passing
    /// [`BACKFILL_REASON`] here is the honest choice only for library callers
    /// with no operator in the loop.
    pub fn employee_from_cron_job(
        job: &CronJob,
        now: &str,
        reason: &str,
    ) -> (Employee, EmployeeCronJob) {
        let employee_id = derive_employee_id(&job.id);

        // `CronJob` carries BOTH `skill` (singular) and `skills` (plural) —
        // `db.rs:49-50`. Measured over the 102 live jobs, 91 set `skill` and
        // 96 set `skills`, and 89 set both. The plural array is the newer
        // field; reading only it silently produced `[]` for jobs that are
        // perfectly well-specified through the singular one. Two of the six
        // "empty skills" rows in the live DB are exactly this case (e.g.
        // `210544ad44bd` Web Deploy Health Daily carries
        // `skill: website-design`), and the fail-closed gate would have blocked
        // them as misconfigured.
        //
        // The fallback is the job's own declared skill, not an invention:
        // if both are absent the row stays empty and the gate blocks it,
        // which is the correct outcome for a job that genuinely has not
        // said what it can do.
        let skills = match job.skills.clone() {
            Some(s) if !s.is_empty() => s,
            _ => job.skill.clone().into_iter().collect(),
        };

        // §3.1.1: `state == "scheduled"` → "active", else carry verbatim
        // so a paused job stays visibly paused in the registry.
        let status = if job.state == CRON_STATE_SCHEDULED {
            STATUS_ACTIVE.to_string()
        } else {
            job.state.clone()
        };

        let employee = Employee {
            employee_id: employee_id.clone(),
            name: job.name.clone(),
            role: DEFAULT_ROLE.to_string(),
            // Operant has no department concept yet. NULL is honest.
            department: None,
            skills,
            // Absent, and stays absent. See AgentType's doc comment.
            agent_type: None,
            // NULL until Wave 2 ships the persona contract.
            persona: None,
            // A cron-derived row carries no charter: the cast seed is the
            // only charter source (ORGANISM-ARCHITECTURE §2), and the cron
            // job owns its prompt elsewhere.
            system_prompt: None,
            status,
            reason: reason.to_string(),
            // From the cron job, not from `now` — a re-run must not drift it.
            created_at: job.created_at.clone(),
            updated_at: now.to_string(),
        };

        let link = EmployeeCronJob {
            employee_id,
            cron_job_id: job.id.clone(),
            schedule: Some(job.schedule.clone()),
            enabled: job.enabled,
        };

        (employee, link)
    }

    /// Backfill the registry from cron jobs. See the module docs for the
    /// idempotence contract.
    ///
    /// `reason` is stored verbatim on every row this call writes — the
    /// operator's own justification, not a constant. A blank reason is
    /// rejected here as well as at the CLI: §3.1 makes the reason mandatory
    /// on the row itself, so a caller that bypassed argument validation must
    /// not be able to write a row with an empty audit trail.
    ///
    /// `now` is the RFC3339 timestamp stamped on `updated_at`; it is a
    /// parameter so the caller (and the tests) control it.
    pub fn backfill_from_cron_jobs(
        &self,
        jobs: &[CronJob],
        now: &str,
        reason: &str,
    ) -> Result<BackfillReport, Error> {
        let reason = reason.trim();
        if reason.is_empty() {
            return Err(Error::Agent(
                "backfill requires a non-empty reason: every employee row records \
                 why it exists (§3.1), and an empty audit trail is not one"
                    .to_string(),
            ));
        }
        let conn = self.lock_conn()?;
        let mut report = BackfillReport {
            jobs_seen: jobs.len(),
            ..BackfillReport::default()
        };

        // employee_id -> the cron job that owns it. Collision detection
        // (§3.1.2) is against ids *seen in this run* plus ids already in
        // the table, so it works both on a fresh registry and a re-run.
        let mut owner: HashMap<String, String> = HashMap::new();
        {
            let mut stmt = conn
                .prepare("SELECT employee_id, cron_job_id FROM employee_cron_jobs")
                .map_err(|e| Error::Agent(format!("Failed to prepare collision scan: {}", e)))?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(|e| Error::Agent(format!("Failed to read existing links: {}", e)))?;
            for row in rows {
                let (employee_id, cron_job_id) =
                    row.map_err(|e| Error::Agent(format!("Failed to read existing link: {}", e)))?;
                owner.insert(employee_id, cron_job_id);
            }
        }

        for job in jobs {
            let (employee, link) = Self::employee_from_cron_job(job, now, reason);

            // §3.1.2: a truncation collision means two different jobs map
            // to one identity. Skip this job, loudly. Do not append a
            // disambiguator.
            if let Some(existing_job) = owner.get(&employee.employee_id)
                && existing_job != &link.cron_job_id
            {
                report.collisions.push(EmployeeIdCollision {
                    cron_job_id: link.cron_job_id.clone(),
                    employee_id: employee.employee_id.clone(),
                    conflicting_cron_job_id: existing_job.clone(),
                });
                continue;
            }

            let skills_json = serde_json::to_string(&employee.skills)
                .map_err(|e| Error::Agent(format!("Failed to encode skills: {}", e)))?;
            let agent_type_json = employee.agent_type.map(|a| a.as_str().to_string());
            let persona_json = employee
                .persona
                .as_ref()
                .map(serde_json::to_string)
                .transpose()
                .map_err(|e| Error::Agent(format!("Failed to encode persona: {}", e)))?;

            conn.execute(
                "INSERT INTO employees (
                    employee_id, name, role, department, skills, agent_type,
                    persona, status, reason, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT(employee_id) DO UPDATE SET
                    name        = excluded.name,
                    role        = excluded.role,
                    department  = excluded.department,
                    skills      = excluded.skills,
                    agent_type  = excluded.agent_type,
                    persona     = excluded.persona,
                    status      = excluded.status,
                    reason      = excluded.reason,
                    created_at  = excluded.created_at,
                    updated_at  = excluded.updated_at",
                params![
                    employee.employee_id,
                    employee.name,
                    employee.role,
                    employee.department,
                    skills_json,
                    agent_type_json,
                    persona_json,
                    employee.status,
                    employee.reason,
                    employee.created_at,
                    employee.updated_at,
                ],
            )
            .map_err(|e| Error::Agent(format!("Failed to backfill employee: {}", e)))?;

            conn.execute(
                "INSERT INTO employee_cron_jobs (employee_id, cron_job_id, schedule, enabled)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(employee_id, cron_job_id) DO UPDATE SET
                    schedule = excluded.schedule,
                    enabled  = excluded.enabled",
                params![
                    link.employee_id,
                    link.cron_job_id,
                    link.schedule,
                    i64::from(link.enabled)
                ],
            )
            .map_err(|e| Error::Agent(format!("Failed to backfill employee link: {}", e)))?;

            owner.insert(link.employee_id.clone(), link.cron_job_id.clone());
            report.employees_written += 1;
            report.links_written += 1;

            // Record the fail-closed rows rather than repairing them.
            let missing = employee.missing_required_fields();
            if !missing.is_empty() {
                report.invalid.push(InvalidEmployee {
                    cron_job_id: link.cron_job_id,
                    employee_id: link.employee_id,
                    missing,
                });
            }
        }

        Ok(report)
    }

    /// Insert one manifest-declared employee row, ignoring the write when
    /// the `employee_id` already exists.
    ///
    /// This is the cast seeder's write path (ORGANISM-ARCHITECTURE §1).
    /// Unlike [`Self::backfill_from_cron_jobs`], which derives its rows from
    /// live cron jobs, a cast seat arrives whole from the declared manifest.
    /// The OR IGNORE is the idempotence contract: a re-seed must leave an
    /// operator-edited row exactly as the operator left it, never converge it
    /// back to the manifest. Returns `true` when this call inserted a row.
    pub fn insert_ignore(&self, employee: &Employee) -> Result<bool, Error> {
        let skills_json = serde_json::to_string(&employee.skills)
            .map_err(|e| Error::Agent(format!("Failed to encode skills: {}", e)))?;
        let agent_type_json = employee.agent_type.map(|a| a.as_str().to_string());
        let persona_json = employee
            .persona
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| Error::Agent(format!("Failed to encode persona: {}", e)))?;
        let conn = self.lock_conn()?;
        let written = conn
            .execute(
                "INSERT OR IGNORE INTO employees (
                    employee_id, name, role, department, skills, agent_type,
                    persona, system_prompt, status, reason, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    employee.employee_id,
                    employee.name,
                    employee.role,
                    employee.department,
                    skills_json,
                    agent_type_json,
                    persona_json,
                    employee.system_prompt,
                    employee.status,
                    employee.reason,
                    employee.created_at,
                    employee.updated_at,
                ],
            )
            .map_err(|e| Error::Agent(format!("Failed to insert employee: {}", e)))?;
        Ok(written > 0)
    }

    /// Read one employee, or `None` if the id is not registered.
    pub fn get_employee(&self, employee_id: &str) -> Result<Option<Employee>, Error> {
        let conn = self.lock_conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT employee_id, name, role, department, skills, agent_type,
                        persona, system_prompt, status, reason, created_at, updated_at
                 FROM employees WHERE employee_id = ?1",
            )
            .map_err(|e| Error::Agent(format!("Failed to prepare get_employee: {}", e)))?;

        let employee = stmt
            .query_row(params![employee_id], map_employee_row)
            .optional()
            .map_err(|e| Error::Agent(format!("Failed to read employee: {}", e)))?;
        Ok(employee)
    }

    /// Every employee, ordered by id so a caller's output is stable.
    pub fn list_employees(&self) -> Result<Vec<Employee>, Error> {
        let conn = self.lock_conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT employee_id, name, role, department, skills, agent_type,
                        persona, system_prompt, status, reason, created_at, updated_at
                 FROM employees ORDER BY employee_id",
            )
            .map_err(|e| Error::Agent(format!("Failed to prepare list_employees: {}", e)))?;

        let rows = stmt
            .query_map([], map_employee_row)
            .map_err(|e| Error::Agent(format!("Failed to list employees: {}", e)))?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| Error::Agent(format!("Failed to read employee: {}", e)))?);
        }
        Ok(out)
    }

    /// The cron links for one employee.
    pub fn list_employee_cron_jobs(
        &self,
        employee_id: &str,
    ) -> Result<Vec<EmployeeCronJob>, Error> {
        let conn = self.lock_conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT employee_id, cron_job_id, schedule, enabled
                 FROM employee_cron_jobs WHERE employee_id = ?1
                 ORDER BY cron_job_id",
            )
            .map_err(|e| {
                Error::Agent(format!("Failed to prepare list_employee_cron_jobs: {}", e))
            })?;

        let rows = stmt
            .query_map(params![employee_id], |row| {
                Ok(EmployeeCronJob {
                    employee_id: row.get(0)?,
                    cron_job_id: row.get(1)?,
                    schedule: row.get(2)?,
                    enabled: row.get::<_, i64>(3)? != 0,
                })
            })
            .map_err(|e| Error::Agent(format!("Failed to list employee cron jobs: {}", e)))?;

        let mut out = Vec::new();
        for row in rows {
            out.push(
                row.map_err(|e| Error::Agent(format!("Failed to read employee cron job: {}", e)))?,
            );
        }
        Ok(out)
    }
}

/// Map an `employees` row. Named columns (not positional) — the
/// positional-mapper hazard §1.3 Finding 3 warns about does not apply
/// when the column names are in the SELECT.
fn map_employee_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Employee> {
    let skills_raw: String = row.get("skills")?;
    let skills = serde_json::from_str::<Vec<String>>(&skills_raw).unwrap_or_default();
    let agent_type_raw: Option<String> = row.get("agent_type")?;
    let persona_raw: Option<String> = row.get("persona")?;
    let persona = persona_raw
        .as_deref()
        .and_then(|p| serde_json::from_str::<serde_json::Value>(p).ok());

    Ok(Employee {
        employee_id: row.get("employee_id")?,
        name: row.get("name")?,
        role: row.get("role")?,
        department: row.get("department")?,
        skills,
        // NULL → None. A NULL column is the round-trip proof that
        // "absent" is preserved distinctly from every variant.
        agent_type: agent_type_raw.as_deref().and_then(AgentType::parse),
        persona,
        system_prompt: row.get("system_prompt")?,
        status: row.get("status")?,
        reason: row.get("reason")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}
