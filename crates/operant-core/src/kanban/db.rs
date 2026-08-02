use rusqlite::{Connection, Result, params};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::error::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum TaskStatus {
    Triage,
    Todo,
    Ready,
    Running,
    Blocked,
    Done,
    Archived,
}

impl TaskStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Triage => "triage",
            Self::Todo => "todo",
            Self::Ready => "ready",
            Self::Running => "running",
            Self::Blocked => "blocked",
            Self::Done => "done",
            Self::Archived => "archived",
        }
    }

    pub fn parse_status(s: &str) -> Option<Self> {
        match s {
            "triage" => Some(Self::Triage),
            "todo" => Some(Self::Todo),
            "ready" => Some(Self::Ready),
            "running" => Some(Self::Running),
            "blocked" => Some(Self::Blocked),
            "done" => Some(Self::Done),
            "archived" => Some(Self::Archived),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub body: Option<String>,
    pub assignee: Option<String>,
    pub status: TaskStatus,
    pub priority: i32,
    pub created_by: Option<String>,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub completed_at: Option<i64>,
    pub workspace_kind: String,
    pub workspace_path: Option<String>,
    pub claim_lock: Option<String>,
    pub claim_expires: Option<i64>,
    pub tenant: Option<String>,
    pub result: Option<String>,
    pub idempotency_key: Option<String>,
    pub consecutive_failures: i32,
    pub worker_pid: Option<i32>,
    pub last_failure_error: Option<String>,
    pub max_runtime_seconds: Option<i32>,
    pub last_heartbeat_at: Option<i64>,
    pub current_run_id: Option<i64>,
    pub workflow_template_id: Option<String>,
    pub current_step_key: Option<String>,
    pub skills: Option<Vec<String>>,
    pub max_retries: Option<i32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Run {
    pub id: i64,
    pub task_id: String,
    pub profile: Option<String>,
    pub step_key: Option<String>,
    pub status: String,
    pub claim_lock: Option<String>,
    pub claim_expires: Option<i64>,
    pub worker_pid: Option<i32>,
    pub max_runtime_seconds: Option<i32>,
    pub last_heartbeat_at: Option<i64>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub outcome: Option<String>,
    pub summary: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Comment {
    pub id: i64,
    pub task_id: String,
    pub author: String,
    pub body: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Event {
    pub id: i64,
    pub task_id: String,
    pub kind: String,
    pub payload: Option<serde_json::Value>,
    pub created_at: i64,
    pub run_id: Option<i64>,
}

/// Parameters for creating a new kanban task.
pub struct CreateTaskParams<'a> {
    pub title: &'a str,
    pub body: Option<&'a str>,
    pub assignee: Option<&'a str>,
    pub created_by: Option<&'a str>,
    pub workspace_kind: &'a str,
    pub workspace_path: Option<&'a str>,
    pub tenant: Option<&'a str>,
    pub priority: i32,
    pub parents: &'a [String],
    pub triage: bool,
    pub idempotency_key: Option<&'a str>,
    pub max_runtime_seconds: Option<i32>,
    pub skills: Option<&'a [String]>,
    pub max_retries: Option<i32>,
}

pub struct KanbanDb {
    conn: Arc<Mutex<Connection>>,
}

impl KanbanDb {
    /// Lock the SQLite connection, converting mutex poisoning into a
    /// recoverable error instead of panicking (same pattern as database.rs).
    fn lock_conn(&self) -> Result<std::sync::MutexGuard<'_, Connection>, Error> {
        self.conn
            .lock()
            .map_err(|_| Error::Agent("kanban db mutex poisoned".to_string()))
    }
}

impl KanbanDb {
    pub fn init(path: PathBuf) -> Result<Self, Error> {
        let conn = Connection::open(path)
            .map_err(|e| Error::Agent(format!("Failed to open kanban database: {}", e)))?;

        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
        };

        db.setup_schema()?;
        Ok(db)
    }

    pub fn conn(&self) -> &Arc<Mutex<Connection>> {
        &self.conn
    }

    fn setup_schema(&self) -> Result<(), Error> {
        let conn = self.lock_conn()?;

        let schema = r#"
            CREATE TABLE IF NOT EXISTS tasks (
                id                   TEXT PRIMARY KEY,
                title                TEXT NOT NULL,
                body                 TEXT,
                assignee             TEXT,
                status               TEXT NOT NULL,
                priority             INTEGER DEFAULT 0,
                created_by           TEXT,
                created_at           INTEGER NOT NULL,
                started_at           INTEGER,
                completed_at         INTEGER,
                workspace_kind       TEXT NOT NULL DEFAULT 'scratch',
                workspace_path       TEXT,
                claim_lock           TEXT,
                claim_expires        INTEGER,
                tenant               TEXT,
                result               TEXT,
                idempotency_key      TEXT,
                consecutive_failures INTEGER NOT NULL DEFAULT 0,
                worker_pid           INTEGER,
                last_failure_error   TEXT,
                max_runtime_seconds  INTEGER,
                last_heartbeat_at    INTEGER,
                current_run_id       INTEGER,
                workflow_template_id TEXT,
                current_step_key     TEXT,
                skills               TEXT,
                max_retries          INTEGER
            );

            CREATE TABLE IF NOT EXISTS task_links (
                parent_id  TEXT NOT NULL,
                child_id   TEXT NOT NULL,
                PRIMARY KEY (parent_id, child_id),
                FOREIGN KEY(parent_id) REFERENCES tasks(id),
                FOREIGN KEY(child_id) REFERENCES tasks(id)
            );

            CREATE TABLE IF NOT EXISTS task_comments (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                task_id    TEXT NOT NULL,
                author     TEXT NOT NULL,
                body       TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                FOREIGN KEY(task_id) REFERENCES tasks(id)
            );

            CREATE TABLE IF NOT EXISTS task_events (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                task_id    TEXT NOT NULL,
                run_id     INTEGER,
                kind       TEXT NOT NULL,
                payload    TEXT,
                created_at INTEGER NOT NULL,
                FOREIGN KEY(task_id) REFERENCES tasks(id)
            );

            CREATE TABLE IF NOT EXISTS task_runs (
                id                  INTEGER PRIMARY KEY AUTOINCREMENT,
                task_id             TEXT NOT NULL,
                profile             TEXT,
                step_key            TEXT,
                status              TEXT NOT NULL,
                claim_lock          TEXT,
                claim_expires       INTEGER,
                worker_pid          INTEGER,
                max_runtime_seconds INTEGER,
                last_heartbeat_at   INTEGER,
                started_at          INTEGER NOT NULL,
                ended_at            INTEGER,
                outcome             TEXT,
                summary             TEXT,
                metadata            TEXT,
                error               TEXT,
                FOREIGN KEY(task_id) REFERENCES tasks(id)
            );

            CREATE TABLE IF NOT EXISTS kanban_notify_subs (
                task_id       TEXT NOT NULL,
                platform      TEXT NOT NULL,
                chat_id       TEXT NOT NULL,
                thread_id     TEXT NOT NULL DEFAULT '',
                user_id       TEXT,
                created_at    INTEGER NOT NULL,
                last_event_id INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (task_id, platform, chat_id, thread_id)
            );

            CREATE INDEX IF NOT EXISTS idx_tasks_assignee_status ON tasks(assignee, status);
            CREATE INDEX IF NOT EXISTS idx_tasks_status          ON tasks(status);
            CREATE INDEX IF NOT EXISTS idx_tasks_tenant          ON tasks(tenant);
            CREATE INDEX IF NOT EXISTS idx_tasks_idempotency     ON tasks(idempotency_key);
            CREATE INDEX IF NOT EXISTS idx_links_child           ON task_links(child_id);
            CREATE INDEX IF NOT EXISTS idx_links_parent          ON task_links(parent_id);
            CREATE INDEX IF NOT EXISTS idx_comments_task         ON task_comments(task_id, created_at);
            CREATE INDEX IF NOT EXISTS idx_events_task           ON task_events(task_id, created_at);
            CREATE INDEX IF NOT EXISTS idx_events_run            ON task_events(run_id, id);
            CREATE INDEX IF NOT EXISTS idx_runs_task             ON task_runs(task_id, started_at);
            CREATE INDEX IF NOT EXISTS idx_runs_status           ON task_runs(status);
            CREATE INDEX IF NOT EXISTS idx_notify_task           ON kanban_notify_subs(task_id);

            CREATE TABLE IF NOT EXISTS task_assignees (
                task_id   TEXT NOT NULL,
                assignee  TEXT NOT NULL,
                PRIMARY KEY (task_id, assignee),
                FOREIGN KEY(task_id) REFERENCES tasks(id)
            );
        "#;

        conn.execute_batch(schema)
            .map_err(|e| Error::Agent(format!("Failed to initialize kanban schema: {}", e)))?;

        Ok(())
    }

    pub fn get_task(&self, id: &str) -> Result<Option<Task>, Error> {
        let conn = self.lock_conn()?;
        let mut stmt = conn
            .prepare("SELECT * FROM tasks WHERE id = ?1")
            .map_err(|e| Error::Agent(format!("Failed to prepare get_task: {}", e)))?;

        let task = stmt
            .query_row(params![id], |row| {
                let skills_raw: Option<String> = row.get("skills")?;
                let skills = skills_raw.and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok());

                Ok(Task {
                    id: row.get("id")?,
                    title: row.get("title")?,
                    body: row.get("body")?,
                    assignee: row.get("assignee")?,
                    status: TaskStatus::parse_status(&row.get::<_, String>("status")?)
                        .unwrap_or(TaskStatus::Todo),
                    priority: row.get("priority")?,
                    created_by: row.get("created_by")?,
                    created_at: row.get("created_at")?,
                    started_at: row.get("started_at")?,
                    completed_at: row.get("completed_at")?,
                    workspace_kind: row.get("workspace_kind")?,
                    workspace_path: row.get("workspace_path")?,
                    claim_lock: row.get("claim_lock")?,
                    claim_expires: row.get("claim_expires")?,
                    tenant: row.get("tenant")?,
                    result: row.get("result")?,
                    idempotency_key: row.get("idempotency_key")?,
                    consecutive_failures: row.get("consecutive_failures")?,
                    worker_pid: row.get("worker_pid")?,
                    last_failure_error: row.get("last_failure_error")?,
                    max_runtime_seconds: row.get("max_runtime_seconds")?,
                    last_heartbeat_at: row.get("last_heartbeat_at")?,
                    current_run_id: row.get("current_run_id")?,
                    workflow_template_id: row.get("workflow_template_id")?,
                    current_step_key: row.get("current_step_key")?,
                    skills,
                    max_retries: row.get("max_retries")?,
                })
            })
            .map_err(|e| Error::Agent(format!("Error fetching task: {}", e)))?;

        Ok(Some(task))
    }

    pub fn list_tasks(&self) -> Result<Vec<Task>, Error> {
        let conn = self.lock_conn()?;
        let mut stmt = conn
            .prepare("SELECT * FROM tasks ORDER BY created_at DESC")
            .map_err(|e| Error::Agent(format!("Failed to prepare list_tasks: {}", e)))?;

        let tasks = stmt
            .query_map([], |row| {
                let skills_raw: Option<String> = row.get("skills")?;
                let skills = skills_raw.and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok());

                Ok(Task {
                    id: row.get("id")?,
                    title: row.get("title")?,
                    body: row.get("body")?,
                    assignee: row.get("assignee")?,
                    status: TaskStatus::parse_status(&row.get::<_, String>("status")?)
                        .unwrap_or(TaskStatus::Todo),
                    priority: row.get("priority")?,
                    created_by: row.get("created_by")?,
                    created_at: row.get("created_at")?,
                    started_at: row.get("started_at")?,
                    completed_at: row.get("completed_at")?,
                    workspace_kind: row.get("workspace_kind")?,
                    workspace_path: row.get("workspace_path")?,
                    claim_lock: row.get("claim_lock")?,
                    claim_expires: row.get("claim_expires")?,
                    tenant: row.get("tenant")?,
                    result: row.get("result")?,
                    idempotency_key: row.get("idempotency_key")?,
                    consecutive_failures: row.get("consecutive_failures")?,
                    worker_pid: row.get("worker_pid")?,
                    last_failure_error: row.get("last_failure_error")?,
                    max_runtime_seconds: row.get("max_runtime_seconds")?,
                    last_heartbeat_at: row.get("last_heartbeat_at")?,
                    current_run_id: row.get("current_run_id")?,
                    workflow_template_id: row.get("workflow_template_id")?,
                    current_step_key: row.get("current_step_key")?,
                    skills,
                    max_retries: row.get("max_retries")?,
                })
            })
            .map_err(|e| Error::Agent(format!("Failed to query tasks: {}", e)))?;

        let mut result = Vec::new();
        for task in tasks {
            result.push(task.map_err(|e| Error::Agent(format!("Error reading task row: {}", e)))?);
        }
        Ok(result)
    }

    pub fn create_task(&self, p: CreateTaskParams<'_>) -> Result<String, Error> {
        let conn = self.lock_conn()?;

        if let Some(key) = p.idempotency_key {
            let existing: Option<String> = conn
                .query_row(
                    "SELECT id FROM tasks WHERE idempotency_key = ?1 AND status != 'archived'",
                    params![key],
                    |row| row.get(0),
                )
                .ok();
            if let Some(id) = existing {
                return Ok(id);
            }
        }

        let id = format!(
            "t_{}",
            uuid::Uuid::new_v4().to_string()[..8].replace('-', "")
        );
        let created_at = chrono::Utc::now().timestamp();
        let status = if p.triage { "triage" } else { "todo" };
        let skills_json = p.skills.and_then(|s| serde_json::to_string(s).ok());

        conn.execute(
            "INSERT INTO tasks (id, title, body, assignee, status, priority, created_by, created_at,
             workspace_kind, workspace_path, tenant, idempotency_key, max_runtime_seconds, skills, max_retries)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![id, p.title, p.body, p.assignee, status, p.priority, p.created_by, created_at,
                    p.workspace_kind, p.workspace_path, p.tenant, p.idempotency_key, p.max_runtime_seconds, skills_json, p.max_retries],
        ).map_err(|e| Error::Agent(format!("Failed to create task: {}", e)))?;

        for parent in p.parents {
            conn.execute(
                "INSERT INTO task_links (parent_id, child_id) VALUES (?1, ?2)",
                params![parent, id],
            )
            .map_err(|e| Error::Agent(format!("Failed to link task: {}", e)))?;
        }

        Ok(id)
    }

    pub fn complete_task(
        &self,
        tid: &str,
        result: Option<&str>,
        _summary: Option<&str>,
        metadata: Option<&serde_json::Value>,
        created_cards: Option<&[String]>,
        expected_run_id: Option<i64>,
    ) -> Result<bool, Error> {
        let conn = self.lock_conn()?;

        if let Some(run_id) = expected_run_id {
            let current_run: Option<i64> = conn
                .query_row(
                    "SELECT current_run_id FROM tasks WHERE id = ?1",
                    params![tid],
                    |row| row.get(0),
                )
                .ok();
            if current_run != Some(run_id) {
                return Ok(false);
            }
        }

        let metadata_json = metadata.and_then(|m| serde_json::to_string(m).ok());

        conn.execute(
            "UPDATE tasks SET status = 'done', completed_at = ?1, result = ?2 WHERE id = ?3",
            params![chrono::Utc::now().timestamp(), result, tid],
        )
        .map_err(|e| Error::Agent(format!("Failed to complete task: {}", e)))?;

        if let Some(cards) = created_cards {
            for card in cards {
                conn.execute(
                    "INSERT INTO task_links (parent_id, child_id) VALUES (?1, ?2)",
                    params![tid, card],
                )
                .map_err(|e| Error::Agent(format!("Failed to link created card: {}", e)))?;
            }
        }

        conn.execute(
            "INSERT INTO task_events (task_id, run_id, kind, payload, created_at)
             VALUES (?1, ?2, 'completed', ?3, ?4)",
            params![
                tid,
                expected_run_id,
                metadata_json,
                chrono::Utc::now().timestamp()
            ],
        )
        .map_err(|e| Error::Agent(format!("Failed to record completion event: {}", e)))?;

        Ok(true)
    }

    pub fn block_task(
        &self,
        tid: &str,
        reason: &str,
        expected_run_id: Option<i64>,
    ) -> Result<bool, Error> {
        let conn = self.lock_conn()?;

        if let Some(run_id) = expected_run_id {
            let current_run: Option<i64> = conn
                .query_row(
                    "SELECT current_run_id FROM tasks WHERE id = ?1",
                    params![tid],
                    |row| row.get(0),
                )
                .ok();
            if current_run != Some(run_id) {
                return Ok(false);
            }
        }

        conn.execute(
            "UPDATE tasks SET status = 'blocked' WHERE id = ?1",
            params![tid],
        )
        .map_err(|e| Error::Agent(format!("Failed to block task: {}", e)))?;

        let payload: Option<String> =
            serde_json::to_string(&serde_json::json!({"reason": reason})).ok();
        conn.execute(
            "INSERT INTO task_events (task_id, run_id, kind, payload, created_at)
             VALUES (?1, ?2, 'blocked', ?3, ?4)",
            params![
                tid,
                expected_run_id,
                payload,
                chrono::Utc::now().timestamp()
            ],
        )
        .map_err(|e| Error::Agent(format!("Failed to record block event: {}", e)))?;

        Ok(true)
    }

    pub fn heartbeat_worker(
        &self,
        tid: &str,
        note: Option<&str>,
        expected_run_id: Option<i64>,
    ) -> Result<bool, Error> {
        let conn = self.lock_conn()?;

        if let Some(run_id) = expected_run_id {
            let current_run: Option<i64> = conn
                .query_row(
                    "SELECT current_run_id FROM tasks WHERE id = ?1",
                    params![tid],
                    |row| row.get(0),
                )
                .ok();
            if current_run != Some(run_id) {
                return Ok(false);
            }
        }

        conn.execute(
            "UPDATE tasks SET last_heartbeat_at = ?1 WHERE id = ?2",
            params![chrono::Utc::now().timestamp(), tid],
        )
        .map_err(|e| Error::Agent(format!("Failed to heartbeat task: {}", e)))?;

        let payload = note.map(|n| {
            serde_json::to_string(&serde_json::json!({ "note": n }))
                .expect("serializable JSON object always serializes")
        });

        conn.execute(
            "INSERT INTO task_events (task_id, run_id, kind, payload, created_at)
             VALUES (?1, ?2, 'heartbeat', ?3, ?4)",
            params![
                tid,
                expected_run_id,
                payload,
                chrono::Utc::now().timestamp()
            ],
        )
        .map_err(|e| Error::Agent(format!("Failed to record heartbeat event: {}", e)))?;

        Ok(true)
    }

    pub fn add_comment(&self, tid: &str, author: &str, body: &str) -> Result<i64, Error> {
        let conn = self.lock_conn()?;
        conn.execute(
            "INSERT INTO task_comments (task_id, author, body, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![tid, author, body, chrono::Utc::now().timestamp()],
        )
        .map_err(|e| Error::Agent(format!("Failed to add comment: {}", e)))?;

        Ok(conn.last_insert_rowid())
    }

    pub fn link_tasks(&self, parent_id: &str, child_id: &str) -> Result<(), Error> {
        let conn = self.lock_conn()?;
        conn.execute(
            "INSERT INTO task_links (parent_id, child_id) VALUES (?1, ?2)",
            params![parent_id, child_id],
        )
        .map_err(|e| Error::Agent(format!("Failed to link tasks: {}", e)))?;
        Ok(())
    }

    pub fn unlink_tasks(&self, parent_id: &str, child_id: &str) -> Result<(), Error> {
        let conn = self.lock_conn()?;
        conn.execute(
            "DELETE FROM task_links WHERE parent_id = ?1 AND child_id = ?2",
            params![parent_id, child_id],
        )
        .map_err(|e| Error::Agent(format!("Failed to unlink tasks: {}", e)))?;
        Ok(())
    }

    pub fn add_assignee(&self, task_id: &str, assignee: &str) -> Result<(), Error> {
        let conn = self.lock_conn()?;
        conn.execute(
            "INSERT OR IGNORE INTO task_assignees (task_id, assignee) VALUES (?1, ?2)",
            params![task_id, assignee],
        )
        .map_err(|e| Error::Agent(format!("Failed to add assignee: {}", e)))?;
        Ok(())
    }

    pub fn remove_assignee(&self, task_id: &str, assignee: &str) -> Result<(), Error> {
        let conn = self.lock_conn()?;
        conn.execute(
            "DELETE FROM task_assignees WHERE task_id = ?1 AND assignee = ?2",
            params![task_id, assignee],
        )
        .map_err(|e| Error::Agent(format!("Failed to remove assignee: {}", e)))?;
        Ok(())
    }

    pub fn list_assignees(&self, task_id: &str) -> Result<Vec<String>, Error> {
        let conn = self.lock_conn()?;
        let mut stmt = conn
            .prepare("SELECT assignee FROM task_assignees WHERE task_id = ?1 ORDER BY assignee ASC")
            .map_err(|e| Error::Agent(format!("Failed to prepare list_assignees: {}", e)))?;

        let rows = stmt
            .query_map(params![task_id], |row| row.get(0))
            .map_err(|e| Error::Agent(format!("Query error: {}", e)))?;

        let mut assignees = Vec::new();
        for row in rows {
            assignees.push(row.map_err(|e| Error::Agent(format!("Row error: {}", e)))?);
        }
        Ok(assignees)
    }

    pub fn list_comments(&self, tid: &str) -> Result<Vec<Comment>, Error> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare("SELECT id, task_id, author, body, created_at FROM task_comments WHERE task_id = ?1 ORDER BY created_at ASC")
            .map_err(|e| Error::Agent(format!("Failed to prepare list_comments: {}", e)))?;

        let rows = stmt
            .query_map(params![tid], |row| {
                Ok(Comment {
                    id: row.get(0)?,
                    task_id: row.get(1)?,
                    author: row.get(2)?,
                    body: row.get(3)?,
                    created_at: row.get(4)?,
                })
            })
            .map_err(|e| Error::Agent(format!("Query error: {}", e)))?;

        let mut comments = Vec::new();
        for row in rows {
            comments.push(row.map_err(|e| Error::Agent(format!("Row error: {}", e)))?);
        }
        Ok(comments)
    }

    pub fn list_events(&self, tid: &str) -> Result<Vec<Event>, Error> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare("SELECT id, task_id, kind, payload, created_at, run_id FROM task_events WHERE task_id = ?1 ORDER BY created_at ASC")
            .map_err(|e| Error::Agent(format!("Failed to prepare list_events: {}", e)))?;

        let rows = stmt
            .query_map(params![tid], |row| {
                let payload_raw: Option<String> = row.get(3)?;
                let payload =
                    payload_raw.and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok());

                Ok(Event {
                    id: row.get(0)?,
                    task_id: row.get(1)?,
                    kind: row.get(2)?,
                    payload,
                    created_at: row.get(4)?,
                    run_id: row.get(5)?,
                })
            })
            .map_err(|e| Error::Agent(format!("Query error: {}", e)))?;

        let mut events = Vec::new();
        for row in rows {
            events.push(row.map_err(|e| Error::Agent(format!("Row error: {}", e)))?);
        }
        Ok(events)
    }

    pub fn list_runs(&self, tid: &str) -> Result<Vec<Run>, Error> {
        let conn = self.lock_conn()?;
        let mut stmt = conn
            .prepare("SELECT * FROM task_runs WHERE task_id = ?1 ORDER BY started_at ASC")
            .map_err(|e| Error::Agent(format!("Failed to prepare list_runs: {}", e)))?;

        let rows = stmt
            .query_map(params![tid], |row| {
                let metadata_raw: Option<String> = row.get("metadata")?;
                let metadata =
                    metadata_raw.and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok());

                Ok(Run {
                    id: row.get("id")?,
                    task_id: row.get("task_id")?,
                    profile: row.get("profile")?,
                    step_key: row.get("step_key")?,
                    status: row.get("status")?,
                    claim_lock: row.get("claim_lock")?,
                    claim_expires: row.get("claim_expires")?,
                    worker_pid: row.get("worker_pid")?,
                    max_runtime_seconds: row.get("max_runtime_seconds")?,
                    last_heartbeat_at: row.get("last_heartbeat_at")?,
                    started_at: row.get("started_at")?,
                    ended_at: row.get("ended_at")?,
                    outcome: row.get("outcome")?,
                    summary: row.get("summary")?,
                    metadata,
                    error: row.get("error")?,
                })
            })
            .map_err(|e| Error::Agent(format!("Query error: {}", e)))?;

        let mut runs = Vec::new();
        for row in rows {
            runs.push(row.map_err(|e| Error::Agent(format!("Row error: {}", e)))?);
        }
        Ok(runs)
    }

    pub fn parent_ids(&self, tid: &str) -> Result<Vec<String>, Error> {
        let conn = self.lock_conn()?;
        let mut stmt = conn
            .prepare("SELECT parent_id FROM task_links WHERE child_id = ?1")
            .map_err(|e| Error::Agent(format!("Failed to prepare parent_ids: {}", e)))?;

        let rows = stmt
            .query_map(params![tid], |row| row.get(0))
            .map_err(|e| Error::Agent(format!("Query error: {}", e)))?;

        let mut parents = Vec::new();
        for row in rows {
            parents.push(row.map_err(|e| Error::Agent(format!("Row error: {}", e)))?);
        }
        Ok(parents)
    }

    pub fn child_ids(&self, tid: &str) -> Result<Vec<String>, Error> {
        let conn = self.lock_conn()?;
        let mut stmt = conn
            .prepare("SELECT child_id FROM task_links WHERE parent_id = ?1")
            .map_err(|e| Error::Agent(format!("Failed to prepare child_ids: {}", e)))?;

        let rows = stmt
            .query_map(params![tid], |row| row.get(0))
            .map_err(|e| Error::Agent(format!("Query error: {}", e)))?;

        let mut children = Vec::new();
        for row in rows {
            children.push(row.map_err(|e| Error::Agent(format!("Row error: {}", e)))?);
        }
        Ok(children)
    }

    pub fn build_worker_context(&self, tid: &str) -> Result<String, Error> {
        let task = self
            .get_task(tid)?
            .ok_or_else(|| Error::Agent(format!("Task {} not found", tid)))?;
        let comments = self.list_comments(tid)?;
        let runs = self.list_runs(tid)?;

        let mut context = format!(
            "Task ID: {}\nTitle: {}\nBody: {}\nAssignee: {:?}\nStatus: {:?}\n\n",
            task.id,
            task.title,
            task.body.as_deref().unwrap_or(""),
            task.assignee.as_deref().unwrap_or(""),
            task.status.as_str()
        );

        context.push_str("--- Comments ---\n");
        for c in comments {
            context.push_str(&format!(
                "**{}** ({}): {}\n",
                c.author, c.created_at, c.body
            ));
        }

        context.push_str("\n--- Prior Attempts ---\n");
        for run in runs.iter().rev().take(10) {
            context.push_str(&format!(
                "Run {}: Status={}, Outcome={:?}, Summary={:?}\n",
                run.id, run.status, run.outcome, run.summary
            ));
        }

        Ok(context)
    }
}
