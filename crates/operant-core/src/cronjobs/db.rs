use crate::error::Error;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

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
        let conn = self.conn.lock().unwrap();

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
        let conn = self.conn.lock().unwrap();
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

        Ok(id)
    }

    pub fn list_jobs(&self, include_disabled: bool) -> Result<Vec<CronJob>, Error> {
        let conn = self.conn.lock().unwrap();
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
        let conn = self.conn.lock().unwrap();
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
        updates: std::collections::HashMap<String, Option<serde_json::Value>>,
    ) -> Result<Option<CronJob>, Error> {
        let conn = self.conn.lock().unwrap();

        if updates.is_empty() {
            return self.get_job(id);
        }

        let mut set_clauses = Vec::new();
        let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        for (i, (key, value)) in updates.iter().enumerate() {
            set_clauses.push(format!("{} = ?{}", key, i + 1));
            match value {
                Some(val) => {
                    let val_str = val.to_string();
                    params_vec.push(Box::new(val_str));
                }
                None => params_vec.push(Box::new("NULL")),
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
        let conn = self.conn.lock().unwrap();

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
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE cron_jobs SET next_run_at = ?1 WHERE id = ?2",
            params![next_run_at, id],
        )
        .map_err(|e| Error::Agent(format!("Failed to set next run for cron job: {}", e)))?;
        Ok(())
    }

    pub fn delete_job(&self, id: &str) -> Result<bool, Error> {
        let conn = self.conn.lock().unwrap();
        let affected = conn
            .execute("DELETE FROM cron_jobs WHERE id = ?1", params![id])
            .map_err(|e| Error::Agent(format!("Failed to delete cron job: {}", e)))?;
        Ok(affected > 0)
    }

    pub fn get_due_jobs(&self) -> Result<Vec<CronJob>, Error> {
        let conn = self.conn.lock().unwrap();
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
