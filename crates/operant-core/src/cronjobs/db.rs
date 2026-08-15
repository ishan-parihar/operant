use crate::error::Error;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Report from rewriting cron job skill references.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CronRewriteReport {
    /// Total cron jobs scanned.
    pub jobs_scanned: usize,
    /// Cron jobs that had skill refs changed.
    pub jobs_updated: usize,
    /// Individual skill-to-umbrella mappings applied.
    pub mappings: Vec<CronRewriteMapping>,
    /// Individual pruned skill drops.
    pub drops: Vec<CronRewriteDrop>,
}

/// A single skill-to-umbrella mapping applied to a cron job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronRewriteMapping {
    pub job_id: String,
    pub old_skill: String,
    pub new_skill: String,
}

/// A single pruned skill drop from a cron job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronRewriteDrop {
    pub job_id: String,
    pub dropped_skill: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronJob {
    pub id: String,
    pub name: String,
    pub prompt: String,
    pub schedule: String,
    pub schedule_display: String,
    pub repeat_times: Option<i32>,
    pub repeat_completed: i32,
    pub deliver: String,
    pub origin_platform: Option<String>,
    pub origin_chat_id: Option<String>,
    pub origin_thread_id: Option<String>,
    pub skill: Option<String>,
    pub skills: Option<Vec<String>>,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub base_url: Option<String>,
    pub script: Option<String>,
    pub context_from: Option<Vec<String>>,
    pub enabled_toolsets: Option<Vec<String>>,
    pub workdir: Option<String>,
    pub no_agent: bool,
    pub enabled: bool,
    pub state: String,
    pub paused_at: Option<String>,
    pub paused_reason: Option<String>,
    pub created_at: String,
    pub next_run_at: Option<String>,
    pub last_run_at: Option<String>,
    pub last_status: Option<String>,
    pub last_error: Option<String>,
    pub last_delivery_error: Option<String>,
}

/// Parameters for creating a new cron job.
pub struct CreateJobParams {
    pub name: String,
    pub prompt: String,
    pub schedule: String,
    pub schedule_display: String,
    pub repeat_times: Option<i32>,
    pub deliver: String,
    pub origin_platform: Option<String>,
    pub origin_chat_id: Option<String>,
    pub origin_thread_id: Option<String>,
    pub skill: Option<String>,
    pub skills: Option<Vec<String>>,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub base_url: Option<String>,
    pub script: Option<String>,
    pub context_from: Option<Vec<String>>,
    pub enabled_toolsets: Option<Vec<String>>,
    pub workdir: Option<String>,
    pub no_agent: bool,
}

pub struct CronDb {
    conn: Arc<Mutex<Connection>>,
}

impl CronDb {
    /// Lock the SQLite connection, converting mutex poisoning into a
    /// recoverable error instead of panicking (same pattern as database.rs).
    fn lock_conn(&self) -> Result<std::sync::MutexGuard<'_, Connection>, Error> {
        self.conn
            .lock()
            .map_err(|_| Error::Agent("cron db mutex poisoned".to_string()))
    }

    pub fn init(path: PathBuf) -> Result<Self, Error> {
        let conn = Connection::open(path)
            .map_err(|e| Error::Agent(format!("Failed to open cron database: {}", e)))?;

        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
        };

        db.setup_schema()?;
        Ok(db)
    }

    fn setup_schema(&self) -> Result<(), Error> {
        let conn = self.lock_conn()?;

        let schema = r#"
            CREATE TABLE IF NOT EXISTS cron_jobs (
                id                   TEXT PRIMARY KEY,
                name                 TEXT NOT NULL,
                prompt               TEXT NOT NULL,
                schedule             TEXT NOT NULL,
                schedule_display     TEXT NOT NULL,
                repeat_times         INTEGER,
                repeat_completed     INTEGER NOT NULL DEFAULT 0,
                deliver              TEXT NOT NULL DEFAULT 'local',
                origin_platform      TEXT,
                origin_chat_id       TEXT,
                origin_thread_id     TEXT,
                skill                TEXT,
                skills               TEXT,
                model                TEXT,
                provider             TEXT,
                base_url             TEXT,
                script               TEXT,
                context_from         TEXT,
                enabled_toolsets     TEXT,
                workdir              TEXT,
                no_agent             BOOLEAN NOT NULL DEFAULT 0,
                enabled              BOOLEAN NOT NULL DEFAULT 1,
                state                TEXT NOT NULL DEFAULT 'scheduled',
                paused_at            TEXT,
                paused_reason        TEXT,
                created_at           TEXT NOT NULL,
                next_run_at          TEXT,
                last_run_at          TEXT,
                last_status          TEXT,
                last_error           TEXT,
                last_delivery_error  TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_cron_next_run ON cron_jobs(next_run_at);
            CREATE INDEX IF NOT EXISTS idx_cron_enabled ON cron_jobs(enabled);
        "#;

        conn.execute_batch(schema)
            .map_err(|e| Error::Agent(format!("Failed to initialize cron schema: {}", e)))?;

        Ok(())
    }

    pub fn create_job(&self, p: CreateJobParams) -> Result<String, Error> {
        let conn = self.lock_conn()?;
        let id = format!(
            "cron_{}",
            uuid::Uuid::new_v4().to_string()[..8].replace('-', "")
        );
        let created_at = chrono::Utc::now().to_rfc3339();

        let skills_json = p.skills.and_then(|s| serde_json::to_string(&s).ok());
        let context_json = p.context_from.and_then(|c| serde_json::to_string(&c).ok());
        let toolsets_json = p
            .enabled_toolsets
            .and_then(|t| serde_json::to_string(&t).ok());

        conn.execute(
            "INSERT INTO cron_jobs (
                id, name, prompt, schedule, schedule_display, repeat_times, repeat_completed,
                deliver, origin_platform, origin_chat_id, origin_thread_id, skill, skills,
                model, provider, base_url, script, context_from, enabled_toolsets, workdir,
                no_agent, enabled, state, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, 1, 'scheduled', ?21)",
            params![
                id, p.name, p.prompt, p.schedule, p.schedule_display, p.repeat_times, p.deliver,
                p.origin_platform, p.origin_chat_id, p.origin_thread_id, p.skill, skills_json,
                p.model, p.provider, p.base_url, p.script, context_json, toolsets_json, p.workdir,
                p.no_agent, created_at
            ],
        ).map_err(|e| Error::Agent(format!("Failed to create cron job: {}", e)))?;

        // Initial next_run_at — without it `get_due_jobs` (`next_run_at <= now`)
        // would never match and a freshly-created job would never fire.
        // (Latent bug surfaced by the hermes-parity audit: CLI/blueprint/
        // suggestions-created jobs had NULL next_run_at forever.)
        if let Some(next) = crate::cronjobs::schedule::next_run_from_schedule(&p.schedule) {
            conn.execute(
                "UPDATE cron_jobs SET next_run_at = ?1 WHERE id = ?2",
                params![next, id],
            )
            .map_err(|e| Error::Agent(format!("Failed to set initial next_run_at: {}", e)))?;
        }

        Ok(id)
    }

    /// Self-heal pass for jobs created before schedule normalization existed:
    /// normalizes stored schedules (5-field / "every Nh" → 6-field) and
    /// backfills any missing `next_run_at`. Returns the number of jobs healed.
    /// Called on scheduler start and `operant cron tick`.
    pub fn repair_schedules(&self) -> Result<usize, Error> {
        let conn = self.lock_conn()?;
        let mut stmt = conn
            .prepare("SELECT id, schedule, next_run_at FROM cron_jobs WHERE enabled = 1")
            .map_err(|e| Error::Agent(format!("Failed to prepare repair_schedules: {}", e)))?;
        let rows: Vec<(String, String, Option<String>)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .and_then(|mapped| mapped.collect::<rusqlite::Result<Vec<_>>>())
            .map_err(|e| Error::Agent(format!("Failed to read cron jobs: {}", e)))?;
        drop(stmt);

        let mut healed = 0;
        for (id, schedule, next_run_at) in rows {
            let normalized = crate::cronjobs::schedule::normalize_schedule(&schedule).ok();
            let schedule_changed = normalized.as_deref() != Some(schedule.as_str());
            let next = normalized
                .as_deref()
                .and_then(crate::cronjobs::schedule::next_run_from_schedule);
            if !schedule_changed && !(next_run_at.is_none() && next.is_some()) {
                continue;
            }

            let mut set_clauses: Vec<String> = Vec::new();
            let mut args: Vec<&dyn rusqlite::ToSql> = Vec::new();
            if let Some(n) = &normalized {
                set_clauses.push(format!("schedule = ?{}", args.len() + 1));
                args.push(n);
            }
            if let Some(n) = &next {
                set_clauses.push(format!("next_run_at = ?{}", args.len() + 1));
                args.push(n);
            }
            if set_clauses.is_empty() {
                continue;
            }
            let sql = format!(
                "UPDATE cron_jobs SET {} WHERE id = ?{}",
                set_clauses.join(", "),
                args.len() + 1
            );
            args.push(&id);
            conn.execute(&sql, rusqlite::params_from_iter(args))
                .map_err(|e| Error::Agent(format!("Failed to repair cron job {}: {}", id, e)))?;
            healed += 1;
        }
        Ok(healed)
    }

    pub fn list_jobs(&self, include_disabled: bool) -> Result<Vec<CronJob>, Error> {
        let conn = self.lock_conn()?;
        let query = if include_disabled {
            "SELECT * FROM cron_jobs"
        } else {
            "SELECT * FROM cron_jobs WHERE enabled = 1"
        };

        let mut stmt = conn
            .prepare(query)
            .map_err(|e| Error::Agent(format!("Failed to prepare list_jobs: {}", e)))?;

        let rows = stmt
            .query_map([], |row| {
                let skills_raw: Option<String> = row.get("skills")?;
                let skills = skills_raw.and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok());
                let context_raw: Option<String> = row.get("context_from")?;
                let context_from =
                    context_raw.and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok());
                let toolsets_raw: Option<String> = row.get("enabled_toolsets")?;
                let toolsets =
                    toolsets_raw.and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok());

                Ok(CronJob {
                    id: row.get("id")?,
                    name: row.get("name")?,
                    prompt: row.get("prompt")?,
                    schedule: row.get("schedule")?,
                    schedule_display: row.get("schedule_display")?,
                    repeat_times: row.get("repeat_times")?,
                    repeat_completed: row.get("repeat_completed")?,
                    deliver: row.get("deliver")?,
                    origin_platform: row.get("origin_platform")?,
                    origin_chat_id: row.get("origin_chat_id")?,
                    origin_thread_id: row.get("origin_thread_id")?,
                    skill: row.get("skill")?,
                    skills,
                    model: row.get("model")?,
                    provider: row.get("provider")?,
                    base_url: row.get("base_url")?,
                    script: row.get("script")?,
                    context_from,
                    enabled_toolsets: toolsets,
                    workdir: row.get("workdir")?,
                    no_agent: row.get("no_agent")?,
                    enabled: row.get("enabled")?,
                    state: row.get("state")?,
                    paused_at: row.get("paused_at")?,
                    paused_reason: row.get("paused_reason")?,
                    created_at: row.get("created_at")?,
                    next_run_at: row.get("next_run_at")?,
                    last_run_at: row.get("last_run_at")?,
                    last_status: row.get("last_status")?,
                    last_error: row.get("last_error")?,
                    last_delivery_error: row.get("last_delivery_error")?,
                })
            })
            .map_err(|e| Error::Agent(format!("Error listing cron jobs: {}", e)))?;

        let mut jobs = Vec::new();
        for row in rows {
            jobs.push(row.map_err(|e| Error::Agent(format!("Row error: {}", e)))?);
        }
        Ok(jobs)
    }

    pub fn get_job(&self, id: &str) -> Result<Option<CronJob>, Error> {
        let conn = self.lock_conn()?;
        let mut stmt = conn
            .prepare("SELECT * FROM cron_jobs WHERE id = ?1")
            .map_err(|e| Error::Agent(format!("Failed to prepare get_job: {}", e)))?;

        let job = stmt
            .query_row(params![id], |row| {
                let skills_raw: Option<String> = row.get("skills")?;
                let skills = skills_raw.and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok());
                let context_raw: Option<String> = row.get("context_from")?;
                let context_from =
                    context_raw.and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok());
                let toolsets_raw: Option<String> = row.get("enabled_toolsets")?;
                let toolsets =
                    toolsets_raw.and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok());

                Ok(CronJob {
                    id: row.get("id")?,
                    name: row.get("name")?,
                    prompt: row.get("prompt")?,
                    schedule: row.get("schedule")?,
                    schedule_display: row.get("schedule_display")?,
                    repeat_times: row.get("repeat_times")?,
                    repeat_completed: row.get("repeat_completed")?,
                    deliver: row.get("deliver")?,
                    origin_platform: row.get("origin_platform")?,
                    origin_chat_id: row.get("origin_chat_id")?,
                    origin_thread_id: row.get("origin_thread_id")?,
                    skill: row.get("skill")?,
                    skills,
                    model: row.get("model")?,
                    provider: row.get("provider")?,
                    base_url: row.get("base_url")?,
                    script: row.get("script")?,
                    context_from,
                    enabled_toolsets: toolsets,
                    workdir: row.get("workdir")?,
                    no_agent: row.get("no_agent")?,
                    enabled: row.get("enabled")?,
                    state: row.get("state")?,
                    paused_at: row.get("paused_at")?,
                    paused_reason: row.get("paused_reason")?,
                    created_at: row.get("created_at")?,
                    next_run_at: row.get("next_run_at")?,
                    last_run_at: row.get("last_run_at")?,
                    last_status: row.get("last_status")?,
                    last_error: row.get("last_error")?,
                    last_delivery_error: row.get("last_delivery_error")?,
                })
            })
            .optional()
            .map_err(|e| Error::Agent(format!("Error fetching cron job: {}", e)))?;

        Ok(job)
    }

    pub fn update_job(
        &self,
        id: &str,
        updates: HashMap<String, Option<serde_json::Value>>,
    ) -> Result<Option<CronJob>, Error> {
        let conn = self.lock_conn()?;

        if updates.is_empty() {
            return self.get_job(id);
        }

        let mut set_clauses = Vec::new();
        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        for (i, (key, value)) in updates.iter().enumerate() {
            set_clauses.push(format!("{} = ?{}", key, i + 1));
            match value {
                Some(val) => {
                    // Convert serde_json::Value to rusqlite::Value so booleans
                    // and numbers are stored with correct SQLite types (not TEXT).
                    // Convert serde_json::Value to rusqlite::Value so booleans
                    // and numbers are stored with correct SQLite types (not TEXT).
                    let sqlite_val = match val {
                        serde_json::Value::Bool(b) => {
                            rusqlite::types::Value::Integer(if *b { 1 } else { 0 })
                        }
                        serde_json::Value::Number(n) => {
                            if let Some(i) = n.as_i64() {
                                rusqlite::types::Value::Integer(i)
                            } else if let Some(f) = n.as_f64() {
                                rusqlite::types::Value::Real(f)
                            } else {
                                rusqlite::types::Value::Text(n.to_string())
                            }
                        }
                        serde_json::Value::String(s) => rusqlite::types::Value::Text(s.clone()),
                        serde_json::Value::Null => rusqlite::types::Value::Null,
                        other => rusqlite::types::Value::Text(other.to_string()),
                    };
                    params_vec.push(Box::new(sqlite_val));
                }
                None => params_vec.push(Box::new(rusqlite::types::Value::Null)),
            }
        }

        let sql = format!(
            "UPDATE cron_jobs SET {} WHERE id = ?{}",
            set_clauses.join(", "),
            params_vec.len() + 1
        );

        let mut final_params = params_vec;
        final_params.push(Box::new(id.to_string()));

        conn.execute(&sql, rusqlite::params_from_iter(final_params))
            .map_err(|e| Error::Agent(format!("Failed to update cron job: {}", e)))?;

        let mut stmt = conn
            .prepare("SELECT * FROM cron_jobs WHERE id = ?1")
            .map_err(|e| Error::Agent(format!("Failed to prepare get_job: {}", e)))?;

        let job = stmt
            .query_row(params![id], |row| {
                let skills_raw: Option<String> = row.get("skills")?;
                let skills = skills_raw.and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok());
                let context_raw: Option<String> = row.get("context_from")?;
                let context_from =
                    context_raw.and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok());
                let toolsets_raw: Option<String> = row.get("enabled_toolsets")?;
                let toolsets =
                    toolsets_raw.and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok());

                Ok(CronJob {
                    id: row.get("id")?,
                    name: row.get("name")?,
                    prompt: row.get("prompt")?,
                    schedule: row.get("schedule")?,
                    schedule_display: row.get("schedule_display")?,
                    repeat_times: row.get("repeat_times")?,
                    repeat_completed: row.get("repeat_completed")?,
                    deliver: row.get("deliver")?,
                    origin_platform: row.get("origin_platform")?,
                    origin_chat_id: row.get("origin_chat_id")?,
                    origin_thread_id: row.get("origin_thread_id")?,
                    skill: row.get("skill")?,
                    skills,
                    model: row.get("model")?,
                    provider: row.get("provider")?,
                    base_url: row.get("base_url")?,
                    script: row.get("script")?,
                    context_from,
                    enabled_toolsets: toolsets,
                    workdir: row.get("workdir")?,
                    no_agent: row.get("no_agent")?,
                    enabled: row.get("enabled")?,
                    state: row.get("state")?,
                    paused_at: row.get("paused_at")?,
                    paused_reason: row.get("paused_reason")?,
                    created_at: row.get("created_at")?,
                    next_run_at: row.get("next_run_at")?,
                    last_run_at: row.get("last_run_at")?,
                    last_status: row.get("last_status")?,
                    last_error: row.get("last_error")?,
                    last_delivery_error: row.get("last_delivery_error")?,
                })
            })
            .optional()
            .map_err(|e| Error::Agent(format!("Error fetching updated cron job: {}", e)))?;

        Ok(job)
    }

    pub fn mark_job_run(
        &self,
        id: &str,
        success: bool,
        error: Option<String>,
        delivery_error: Option<String>,
        next_run_at: Option<String>,
    ) -> Result<(), Error> {
        let conn = self.lock_conn()?;

        conn.execute(
            "UPDATE cron_jobs SET
                last_run_at = ?1,
                last_status = ?2,
                last_error = ?3,
                last_delivery_error = ?4,
                next_run_at = ?5,
                repeat_completed = repeat_completed + 1
             WHERE id = ?6",
            params![
                chrono::Utc::now().to_rfc3339(),
                if success { "ok" } else { "error" },
                error,
                delivery_error,
                next_run_at,
                id
            ],
        )
        .map_err(|e| Error::Agent(format!("Failed to mark cron job run: {}", e)))?;

        Ok(())
    }

    pub fn set_next_run(&self, id: &str, next_run_at: Option<String>) -> Result<(), Error> {
        let conn = self.lock_conn()?;
        conn.execute(
            "UPDATE cron_jobs SET next_run_at = ?1 WHERE id = ?2",
            params![next_run_at, id],
        )
        .map_err(|e| Error::Agent(format!("Failed to set next run for cron job: {}", e)))?;
        Ok(())
    }

    pub fn delete_job(&self, id: &str) -> Result<bool, Error> {
        let conn = self.lock_conn()?;
        let affected = conn
            .execute("DELETE FROM cron_jobs WHERE id = ?1", params![id])
            .map_err(|e| Error::Agent(format!("Failed to delete cron job: {}", e)))?;
        Ok(affected > 0)
    }

    /// Return the set of skill names referenced by any cron job (including disabled).
    ///
    /// Used by the curator to protect cron-dependent skills from inactivity
    /// archival and to identify which jobs need rewriting after consolidation.
    pub fn referenced_skill_names(&self) -> Result<std::collections::HashSet<String>, Error> {
        let conn = self.lock_conn()?;
        let mut stmt = conn
            .prepare("SELECT skill, skills FROM cron_jobs")
            .map_err(|e| {
                Error::Agent(format!("Failed to prepare referenced_skill_names: {}", e))
            })?;

        let mut refs = std::collections::HashSet::new();
        let rows = stmt
            .query_map([], |row| {
                let skill: Option<String> = row.get(0)?;
                let skills_raw: Option<String> = row.get(1)?;
                Ok((skill, skills_raw))
            })
            .map_err(|e| Error::Agent(format!("Error querying cron skill refs: {}", e)))?;

        for row in rows {
            let (skill, skills_raw) = row.map_err(|e| Error::Agent(format!("Row error: {}", e)))?;
            if let Some(s) = skill {
                let trimmed = s.trim().to_string();
                if !trimmed.is_empty() {
                    refs.insert(trimmed);
                }
            }
            if let Some(raw) = skills_raw
                && let Ok(list) = serde_json::from_str::<Vec<String>>(&raw)
            {
                for s in list {
                    let trimmed = s.trim().to_string();
                    if !trimmed.is_empty() {
                        refs.insert(trimmed);
                    }
                }
            }
        }
        Ok(refs)
    }

    /// Rewrite cron job skill references after a curator consolidation pass.
    ///
    /// For each job:
    /// - Skills in `consolidated` mapping are replaced with their umbrella target.
    /// - Skills in `pruned` list are dropped entirely.
    /// - Deduplication: if the umbrella is already in the list, the old ref is
    ///   removed without adding a duplicate.
    ///
    /// Returns a report describing what was rewritten.
    pub fn rewrite_skill_refs(
        &self,
        consolidated: &HashMap<String, String>,
        pruned: &[String],
    ) -> Result<CronRewriteReport, Error> {
        // NOTE: Mutex is held for the entire rewrite loop so all job references
        // are rewritten atomically (no partial rewrites on failure). This is fine
        // for CLI (short-lived) but could be relaxed in daemon mode if contention
        // becomes an issue — e.g. by batching into per-job transactions.
        let conn = self.lock_conn()?;
        let mut report = CronRewriteReport::default();

        // Load all jobs
        let mut stmt = conn
            .prepare("SELECT id, skill, skills FROM cron_jobs")
            .map_err(|e| Error::Agent(format!("Failed to prepare rewrite query: {}", e)))?;

        let job_rows: Vec<(String, Option<String>, Option<String>)> = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })
            .map_err(|e| Error::Agent(format!("Error querying jobs for rewrite: {}", e)))?
            .filter_map(|r| r.ok())
            .collect();

        report.jobs_scanned = job_rows.len();

        for (job_id, skill_field, skills_field) in &job_rows {
            // Parse the skills list
            let mut skills: Vec<String> = skills_field
                .as_ref()
                .and_then(|raw| serde_json::from_str::<Vec<String>>(raw).ok())
                .unwrap_or_default();

            // Also include the legacy single skill field
            if let Some(s) = skill_field {
                let trimmed = s.trim().to_string();
                if !trimmed.is_empty() && !skills.contains(&trimmed) {
                    skills.push(trimmed);
                }
            }

            if skills.is_empty() {
                continue;
            }

            let original = skills.clone();
            let mut changed = false;
            let mut new_skills = Vec::new();

            for skill_name in &skills {
                if let Some(umbrella) = consolidated.get(skill_name) {
                    // Replace with umbrella, avoiding duplicates
                    if !new_skills.contains(umbrella) {
                        new_skills.push(umbrella.clone());
                        report.mappings.push(CronRewriteMapping {
                            job_id: job_id.clone(),
                            old_skill: skill_name.clone(),
                            new_skill: umbrella.clone(),
                        });
                    }
                    changed = true;
                } else if pruned.contains(skill_name) {
                    // Drop pruned skill entirely
                    report.drops.push(CronRewriteDrop {
                        job_id: job_id.clone(),
                        dropped_skill: skill_name.clone(),
                    });
                    changed = true;
                } else {
                    // Keep as-is, but skip if already present (dedup)
                    if !new_skills.contains(skill_name) {
                        new_skills.push(skill_name.clone());
                    }
                }
            }

            if !changed {
                continue;
            }

            report.jobs_updated += 1;

            // Serialize and update
            // Filter empty strings from stale production entries before serializing.
            let new_skills: Vec<String> =
                new_skills.into_iter().filter(|s| !s.is_empty()).collect();
            let new_skills_json = serde_json::to_string(&new_skills)
                .map_err(|e| Error::Agent(format!("Failed to serialize skills: {}", e)))?;
            let new_primary = new_skills.first().cloned();

            conn.execute(
                "UPDATE cron_jobs SET skills = ?1, skill = ?2 WHERE id = ?3",
                params![new_skills_json, new_primary, job_id],
            )
            .map_err(|e| Error::Agent(format!("Failed to update cron job {}: {}", job_id, e)))?;

            tracing::info!(
                job_id = %job_id,
                before = ?original,
                after = ?new_skills,
                "Cron job skill refs rewritten"
            );
        }

        Ok(report)
    }

    pub fn get_due_jobs(&self) -> Result<Vec<CronJob>, Error> {
        let conn = self.lock_conn()?;
        let now = chrono::Utc::now().to_rfc3339();

        let mut stmt = conn
            .prepare("SELECT * FROM cron_jobs WHERE enabled = 1 AND next_run_at <= ?1")
            .map_err(|e| Error::Agent(format!("Failed to prepare get_due_jobs: {}", e)))?;

        let rows = stmt
            .query_map(params![now], |row| {
                let skills_raw: Option<String> = row.get("skills")?;
                let skills = skills_raw.and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok());
                let context_raw: Option<String> = row.get("context_from")?;
                let context_from =
                    context_raw.and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok());
                let toolsets_raw: Option<String> = row.get("enabled_toolsets")?;
                let toolsets =
                    toolsets_raw.and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok());

                Ok(CronJob {
                    id: row.get("id")?,
                    name: row.get("name")?,
                    prompt: row.get("prompt")?,
                    schedule: row.get("schedule")?,
                    schedule_display: row.get("schedule_display")?,
                    repeat_times: row.get("repeat_times")?,
                    repeat_completed: row.get("repeat_completed")?,
                    deliver: row.get("deliver")?,
                    origin_platform: row.get("origin_platform")?,
                    origin_chat_id: row.get("origin_chat_id")?,
                    origin_thread_id: row.get("origin_thread_id")?,
                    skill: row.get("skill")?,
                    skills,
                    model: row.get("model")?,
                    provider: row.get("provider")?,
                    base_url: row.get("base_url")?,
                    script: row.get("script")?,
                    context_from,
                    enabled_toolsets: toolsets,
                    workdir: row.get("workdir")?,
                    no_agent: row.get("no_agent")?,
                    enabled: row.get("enabled")?,
                    state: row.get("state")?,
                    paused_at: row.get("paused_at")?,
                    paused_reason: row.get("paused_reason")?,
                    created_at: row.get("created_at")?,
                    next_run_at: row.get("next_run_at")?,
                    last_run_at: row.get("last_run_at")?,
                    last_status: row.get("last_status")?,
                    last_error: row.get("last_error")?,
                    last_delivery_error: row.get("last_delivery_error")?,
                })
            })
            .map_err(|e| Error::Agent(format!("Error fetching due cron jobs: {}", e)))?;

        let mut jobs = Vec::new();
        for row in rows {
            jobs.push(row.map_err(|e| Error::Agent(format!("Row error: {}", e)))?);
        }
        Ok(jobs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn test_db() -> (CronDb, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cron.db");
        let db = CronDb::init(path).unwrap();
        // Return the TempDir so it stays alive for the whole test — dropping
        // it deletes the directory, leaving SQLite with a readonly handle.
        (db, dir)
    }

    fn create_repeat_job(db: &CronDb, repeat_times: Option<i32>) -> String {
        db.create_job(CreateJobParams {
            name: "test job".to_string(),
            prompt: "do a thing".to_string(),
            schedule: "* * * * * *".to_string(),
            schedule_display: "every second".to_string(),
            repeat_times,
            deliver: "local".to_string(),
            origin_platform: None,
            origin_chat_id: None,
            origin_thread_id: None,
            skill: None,
            skills: None,
            model: None,
            provider: None,
            base_url: None,
            script: None,
            context_from: None,
            enabled_toolsets: None,
            workdir: None,
            no_agent: true,
        })
        .unwrap()
    }

    #[test]
    fn mark_job_run_increments_repeat_completed() {
        let (db, _dir) = test_db();
        let id = create_repeat_job(&db, Some(3));
        db.mark_job_run(
            &id,
            true,
            None,
            None,
            Some("2030-01-01T00:00:00Z".to_string()),
        )
        .unwrap();
        let job = db.get_job(&id).unwrap().unwrap();
        assert_eq!(job.repeat_completed, 1);
        assert_eq!(job.last_status.as_deref(), Some("ok"));
    }

    #[test]
    fn update_job_can_express_terminal_completion_shape() {
        // The scheduler marks a finished finite-repeat job by disabling it,
        // setting state="completed" and clearing next_run_at (hermes parity).
        let (db, _dir) = test_db();
        let id = create_repeat_job(&db, Some(2));
        db.update_job(
            &id,
            HashMap::from([
                ("enabled".to_string(), Some(serde_json::json!(false))),
                ("state".to_string(), Some(serde_json::json!("completed"))),
                ("next_run_at".to_string(), None),
            ]),
        )
        .unwrap();
        let job = db.get_job(&id).unwrap().unwrap();
        assert!(!job.enabled);
        assert_eq!(job.state, "completed");
        assert_eq!(job.next_run_at, None);
        // A disabled/completed job is no longer due.
        assert!(db.get_due_jobs().unwrap().is_empty());
    }

    #[test]
    fn scheduler_final_run_records_status_and_bumps_completed_then_disables() {
        // Locks the reviewer-caught regression: the terminal run must record
        // its outcome (last_status, repeat_completed) BEFORE the completion
        // override is applied — the final run's status must not be lost and
        // the counter must reach `times`, not `times - 1`.
        let (db, _dir) = test_db();
        let id = create_repeat_job(&db, Some(2));

        // First run: not exhausted, counter 0 -> 1.
        db.mark_job_run(
            &id,
            true,
            None,
            None,
            Some("2030-01-01T00:00:00Z".to_string()),
        )
        .unwrap();
        let job = db.get_job(&id).unwrap().unwrap();
        assert_eq!(job.repeat_completed, 1);
        assert_eq!(job.last_status.as_deref(), Some("ok"));

        // Final run: mark_job_run records status + bumps counter to 2, then
        // the scheduler applies the terminal override on top.
        db.mark_job_run(
            &id,
            true,
            None,
            None,
            Some("2030-01-02T00:00:00Z".to_string()),
        )
        .unwrap();
        db.update_job(
            &id,
            HashMap::from([
                ("enabled".to_string(), Some(serde_json::json!(false))),
                ("state".to_string(), Some(serde_json::json!("completed"))),
                ("next_run_at".to_string(), None),
            ]),
        )
        .unwrap();

        let job = db.get_job(&id).unwrap().unwrap();
        assert_eq!(job.repeat_completed, 2);
        assert_eq!(job.last_status.as_deref(), Some("ok"));
        assert!(!job.enabled);
        assert_eq!(job.state, "completed");
        assert_eq!(job.next_run_at, None);
    }
}
