pub mod db;
pub mod dispatcher;
pub mod notify;

pub use db::{Comment, CreateTaskParams, Event, KanbanDb, Run, Task, TaskStatus};
pub use dispatcher::Dispatcher;
pub use notify::{NotifyManager, NotifySubscription};

pub mod diagnostics;
pub use diagnostics::{DiagnosticIssue, KanbanDiagnostics};

pub mod triage;
pub use triage::{TriageContext, TriageSpecifier};

use crate::error::Error;
use serde::Serialize;
use std::path::{Path, PathBuf};

/// Information about a single kanban board
#[derive(Debug, Clone, Serialize)]
pub struct BoardInfo {
    pub slug: String,
    pub task_count: usize,
    pub exists: bool,
}

/// Manager for multi-board kanban lifecycle.
///
/// Handles path resolution, board enumeration, creation, and deletion.
/// Boards map to separate SQLite database files named `operant_kanban_<slug>.db`.
/// The "default" board uses the backward-compatible name `operant_kanban.db`.
pub struct KanbanManager {
    kanban_dir: PathBuf,
}

impl KanbanManager {
    /// Create a manager rooted at `kanban_dir` (the directory containing board DB files).
    pub fn new(kanban_dir: PathBuf) -> Self {
        Self { kanban_dir }
    }

    /// Convenience: construct a manager from the main app database path.
    /// The kanban directory is the parent of the main database file.
    pub fn from_app_config(db_path: &Path) -> Self {
        let kanban_dir = db_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        Self::new(kanban_dir)
    }

    /// Resolve the database file path for a given board slug.
    /// "default" → `operant_kanban.db` (backward-compatible).
    /// other   → `operant_kanban_<slug>.db`.
    pub fn resolve_path(&self, slug: &str) -> PathBuf {
        if slug.is_empty() || slug == "default" {
            self.kanban_dir.join("operant_kanban.db")
        } else {
            self.kanban_dir.join(format!("operant_kanban_{}.db", slug))
        }
    }

    /// Open (or create) a board by slug.
    pub fn open_board(&self, slug: &str) -> Result<KanbanDb, Error> {
        KanbanDb::init(self.resolve_path(slug))
    }

    /// List all available boards by scanning the kanban directory.
    /// Always includes "default" (even if the file doesn't exist yet).
    pub fn list_boards(&self) -> Result<Vec<BoardInfo>, Error> {
        let mut boards: Vec<BoardInfo> = Vec::new();

        let default_path = self.resolve_path("default");
        let exists = default_path.exists();
        let count = if exists {
            self.open_board("default")?.list_tasks()?.len()
        } else {
            0
        };
        boards.push(BoardInfo {
            slug: "default".to_string(),
            task_count: count,
            exists,
        });

        if let Ok(entries) = std::fs::read_dir(&self.kanban_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str.starts_with("operant_kanban_")
                    && name_str.ends_with(".db")
                    && let Some(slug) = name_str
                        .strip_prefix("operant_kanban_")
                        .and_then(|s| s.strip_suffix(".db"))
                {
                    if slug == "default" {
                        continue;
                    }
                    let path = entry.path();
                    let count = match KanbanDb::init(path) {
                        Ok(db) => db.list_tasks().unwrap_or_default().len(),
                        Err(_) => 0,
                    };
                    boards.push(BoardInfo {
                        slug: slug.to_string(),
                        task_count: count,
                        exists: true,
                    });
                }
            }
        }

        boards.sort_by(|a, b| a.slug.cmp(&b.slug));
        Ok(boards)
    }

    /// Create a new empty board (fails if slug is invalid or board already exists).
    pub fn create_board(&self, slug: &str) -> Result<(), Error> {
        if slug.is_empty() {
            return Err(Error::Agent("Board slug cannot be empty".to_string()));
        }
        if slug.contains('/') || slug.contains('\\') || slug.contains(' ') || slug.contains('.') {
            return Err(Error::Agent(format!(
                "Invalid board slug '{}': must be a simple identifier",
                slug
            )));
        }
        let path = self.resolve_path(slug);
        if path.exists() {
            return Err(Error::Agent(format!("Board '{}' already exists", slug)));
        }
        KanbanDb::init(path)?;
        Ok(())
    }

    /// Delete a board database file (cannot delete "default").
    ///
    /// RT-421: the guard compared the SLUG, so it missed the empty slug —
    /// which `resolve_path` treats as an alias of "default". `operant kanban
    /// delete-board ""` therefore deleted the DEFAULT board's entire
    /// database. The guard now compares the RESOLVED path, so every alias
    /// that reaches the default board is refused.
    pub fn delete_board(&self, slug: &str) -> Result<(), Error> {
        let path = self.resolve_path(slug);
        if path == self.resolve_path("default") {
            return Err(Error::Agent("Cannot delete the default board".to_string()));
        }
        if !path.exists() {
            return Err(Error::Agent(format!("Board '{}' does not exist", slug)));
        }
        std::fs::remove_file(&path)
            .map_err(|e| Error::Agent(format!("Failed to delete board '{}': {}", slug, e)))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_kanban_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "operant_kanban_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// RT-276: completing a CLAIMED task must close its run.
    ///
    /// The live completion path (`KanbanDb::complete_task`) marked the task
    /// done but left `tasks.current_run_id` set and the `task_runs` row
    /// 'running' forever — a state the subsystem's own diagnostics classify
    /// as a ZOMBIE run that needs hand-run SQL. The only code that closed a
    /// run (`Dispatcher::complete_run`) had no production caller.
    #[test]
    fn completing_a_claimed_task_closes_its_run() {
        let dir = temp_kanban_dir();
        let mgr = KanbanManager::new(dir);
        mgr.open_board("default").unwrap();
        let db = KanbanDb::init(mgr.resolve_path("default")).unwrap();

        let task_id = db
            .create_task(CreateTaskParams {
                title: "a claimed task",
                body: None,
                assignee: None,
                created_by: None,
                workspace_kind: "personal",
                workspace_path: None,
                tenant: None,
                priority: 0,
                parents: &[],
                triage: false,
                idempotency_key: None,
                max_runtime_seconds: None,
                skills: None,
                max_retries: None,
            })
            .unwrap();

        let dispatcher = Dispatcher::new(db.conn().clone());
        let run_id = dispatcher.claim_task(&task_id, "test-worker").unwrap();
        let task = db.get_task(&task_id).unwrap().unwrap();
        assert_eq!(task.current_run_id, Some(run_id), "the claim opens a run");

        assert!(
            db.complete_task(&task_id, Some("done"), None, None, None, None)
                .unwrap()
        );

        // The task no longer points at a run.
        let task = db.get_task(&task_id).unwrap().unwrap();
        assert_eq!(
            task.current_run_id, None,
            "completion must clear current_run_id"
        );

        // The run row is closed, not left running.
        let runs = db.list_runs(&task_id).unwrap();
        let run = runs.iter().find(|r| r.id == run_id).expect("the run row");
        assert_eq!(run.status, "completed", "the run must be closed: {run:?}");
        assert!(run.ended_at.is_some(), "the run must carry an end time");

        // And the subsystem's own detector agrees: no zombie run.
        let issues = KanbanDiagnostics::new(db.conn().clone())
            .run_checks()
            .unwrap();
        assert!(
            !issues.iter().any(|i| i.description.contains("zombie")
                || i.description.contains("is 'running'")),
            "the board must carry no zombie run: {issues:?}"
        );
    }

    #[test]
    fn test_resolve_path_default() {
        let dir = temp_kanban_dir();
        let mgr = KanbanManager::new(dir.clone());
        assert_eq!(mgr.resolve_path("default"), dir.join("operant_kanban.db"));
        assert_eq!(mgr.resolve_path(""), dir.join("operant_kanban.db"));
    }

    /// RT-421: `delete_board("")` deleted the DEFAULT board's database —
    /// the guard compared the slug, and the empty slug is an alias of
    /// "default" (the code resolves both to `operant_kanban.db`).
    #[test]
    fn deleting_the_empty_slug_cannot_delete_the_default_board() {
        let dir = temp_kanban_dir();
        let mgr = KanbanManager::new(dir.clone());
        mgr.open_board("default").unwrap();
        assert!(mgr.resolve_path("default").exists());

        let err = mgr
            .delete_board("")
            .expect_err("the empty slug names the default board");
        assert!(
            err.to_string().contains("default"),
            "the error must name the board it refused: {err}"
        );
        assert!(
            mgr.resolve_path("default").exists(),
            "the default board's database must still be there"
        );

        // The named slug is refused the same way, for the same reason.
        assert!(mgr.delete_board("default").is_err());
        assert!(mgr.resolve_path("default").exists());

        // And a real board still deletes.
        mgr.create_board("work").unwrap();
        assert!(mgr.delete_board("work").is_ok());
        assert!(!mgr.resolve_path("work").exists());
    }

    #[test]
    fn test_resolve_path_named() {
        let dir = temp_kanban_dir();
        let mgr = KanbanManager::new(dir.clone());
        assert_eq!(mgr.resolve_path("work"), dir.join("operant_kanban_work.db"));
        assert_eq!(
            mgr.resolve_path("personal"),
            dir.join("operant_kanban_personal.db")
        );
    }

    #[test]
    fn test_create_and_open_board() {
        let dir = temp_kanban_dir();
        let mgr = KanbanManager::new(dir);

        mgr.open_board("default").unwrap();
        assert!(mgr.resolve_path("default").exists());

        mgr.create_board("work").unwrap();
        assert!(mgr.resolve_path("work").exists());

        assert!(mgr.create_board("work").is_err());
    }

    #[test]
    fn test_delete_board() {
        let dir = temp_kanban_dir();
        let mgr = KanbanManager::new(dir);

        mgr.create_board("scratch").unwrap();
        assert!(mgr.resolve_path("scratch").exists());

        mgr.delete_board("scratch").unwrap();
        assert!(!mgr.resolve_path("scratch").exists());
    }

    #[test]
    fn test_cannot_delete_default() {
        let dir = temp_kanban_dir();
        let mgr = KanbanManager::new(dir);
        mgr.open_board("default").unwrap();
        assert!(mgr.delete_board("default").is_err());
    }

    #[test]
    fn test_list_boards() {
        let dir = temp_kanban_dir();
        let mgr = KanbanManager::new(dir);

        mgr.open_board("default").unwrap();
        mgr.create_board("alpha").unwrap();
        mgr.create_board("beta").unwrap();

        let boards = mgr.list_boards().unwrap();
        assert_eq!(boards.len(), 3);
        assert!(boards.iter().any(|b| b.slug == "default"));
        assert!(boards.iter().any(|b| b.slug == "alpha"));
        assert!(boards.iter().any(|b| b.slug == "beta"));
    }

    #[test]
    fn test_task_isolation_between_boards() {
        let dir = temp_kanban_dir();
        let mgr = KanbanManager::new(dir);

        let db_a = mgr.open_board("board_a").unwrap();
        let db_b = mgr.open_board("board_b").unwrap();

        let task_a_id = db_a
            .create_task(crate::kanban::db::CreateTaskParams {
                title: "Task for A",
                body: Some("Body A"),
                assignee: None,
                created_by: Some("test"),
                workspace_kind: "test",
                workspace_path: None,
                tenant: None,
                priority: 1,
                parents: &[],
                triage: false,
                idempotency_key: None,
                max_runtime_seconds: None,
                skills: None,
                max_retries: None,
            })
            .unwrap();

        let task_b_id = db_b
            .create_task(crate::kanban::db::CreateTaskParams {
                title: "Task for B",
                body: Some("Body B"),
                assignee: None,
                created_by: Some("test"),
                workspace_kind: "test",
                workspace_path: None,
                tenant: None,
                priority: 1,
                parents: &[],
                triage: false,
                idempotency_key: None,
                max_runtime_seconds: None,
                skills: None,
                max_retries: None,
            })
            .unwrap();

        let tasks_a = db_a.list_tasks().unwrap();
        assert_eq!(tasks_a.len(), 1);
        assert_eq!(tasks_a[0].id, task_a_id);

        let tasks_b = db_b.list_tasks().unwrap();
        assert_eq!(tasks_b.len(), 1);
        assert_eq!(tasks_b[0].id, task_b_id);

        assert_ne!(task_a_id, task_b_id);
    }

    #[test]
    fn test_invalid_slug_rejected() {
        let dir = temp_kanban_dir();
        let mgr = KanbanManager::new(dir);

        assert!(mgr.create_board("has space").is_err());
        assert!(mgr.create_board("has/slash").is_err());
        assert!(mgr.create_board("has.dot").is_err());
        assert!(mgr.create_board("").is_err());
    }

    fn new_board_with_task(title: &str) -> (KanbanDb, String, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = KanbanDb::init(dir.path().join("kanban.db")).unwrap();
        let task_id = db
            .create_task(CreateTaskParams {
                title,
                body: None,
                assignee: None,
                created_by: None,
                workspace_kind: "personal",
                workspace_path: None,
                tenant: None,
                priority: 0,
                parents: &[],
                triage: false,
                idempotency_key: None,
                max_runtime_seconds: None,
                skills: None,
                max_retries: None,
            })
            .unwrap();
        (db, task_id, dir)
    }

    /// Same, but the caller owns the tempdir so a SECOND handle can be opened on
    /// the same file — which is the only way to exercise the database-level guard
    /// rather than two views of one connection.
    fn new_board_with_task_at(path: std::path::PathBuf) -> (KanbanDb, String) {
        let db = KanbanDb::init(path).unwrap();
        let task_id = db
            .create_task(CreateTaskParams {
                title: "claimed once",
                body: None,
                assignee: None,
                created_by: None,
                workspace_kind: "personal",
                workspace_path: None,
                tenant: None,
                priority: 0,
                parents: &[],
                triage: false,
                idempotency_key: None,
                max_runtime_seconds: None,
                skills: None,
                max_retries: None,
            })
            .unwrap();
        (db, task_id)
    }

    fn count_running_runs(db: &KanbanDb, task_id: &str) -> i64 {
        db.conn()
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM task_runs WHERE task_id = ?1 AND status = 'running'",
                rusqlite::params![task_id],
                |r| r.get(0),
            )
            .unwrap()
    }

    /// iter-2453 (F1): two dispatchers claiming one task must not both win.
    ///
    /// `claim_task` used to be three autocommit statements, so two callers that
    /// both read `todo` both inserted a `running` run and both updated the task.
    /// Two SEPARATE `Arc<Mutex<Connection>>` handles are used on purpose: one
    /// shared handle would serialise on the same mutex and pass without proving
    /// anything about the database-level guard.
    ///
    /// MUTATION-CHECKED: with the v2 migration's unique index downgraded to a
    /// plain index, this test fails with `left: 2, right: 1` — two running runs,
    /// the F1 corruption. So the assertion is load-bearing and the INDEX is what
    /// the assertion actually guards. The `BEGIN IMMEDIATE` in `claim_task` is
    /// the in-process discipline; the index is the backstop that holds across
    /// processes, which is the case the fleet actually hits (the board is a
    /// machine-wide file and 20+ worktrees are live).
    #[test]
    fn a_second_concurrent_claim_of_one_task_cannot_win() {
        let _dir = tempfile::tempdir().unwrap();
        let path = _dir.path().join("kanban.db");
        let (db_a, task_id) = new_board_with_task_at(path.clone());
        let db_b = KanbanDb::init(path).unwrap();

        let a = Dispatcher::new(db_a.conn().clone());
        let b = Dispatcher::new(db_b.conn().clone());

        let run_a = a
            .claim_task(&task_id, "worker-a")
            .expect("the first claim wins");
        let second = b.claim_task(&task_id, "worker-b");

        assert_eq!(count_running_runs(&db_a, &task_id), 1);
        assert_eq!(count_running_runs(&db_b, &task_id), 1);
        // The loser must be told, not silently ignored — either the database
        // refuses it (the partial unique index) or the claim reports a failure.
        assert!(
            second.is_err(),
            "a second claim of one task must not succeed, got run {:?}",
            second.ok()
        );
        let task = db_a.get_task(&task_id).unwrap().unwrap();
        assert_eq!(
            task.current_run_id,
            Some(run_a),
            "the task must still point at the winning run"
        );
    }

    /// iter-2453 (F1): the unique index is PARTIAL, so a retry is still allowed.
    ///
    /// `fail_run` resets the TASK to `todo` and leaves the old run `failed`. A
    /// full unique index on `task_runs(task_id)` would make every retry fail
    /// forever, which is why the index is `WHERE status = 'running'`.
    #[test]
    fn a_failed_task_can_be_claimed_again() {
        let (db, task_id, _keep) = new_board_with_task("retried");
        let dispatcher = Dispatcher::new(db.conn().clone());

        let first = dispatcher.claim_task(&task_id, "worker-1").unwrap();
        dispatcher.fail_run(&task_id, first, "transient").unwrap();

        let task = db.get_task(&task_id).unwrap().unwrap();
        assert_eq!(task.status, TaskStatus::Todo, "failure re-queues the task");

        let second = dispatcher
            .claim_task(&task_id, "worker-2")
            .expect("a retry must be claimable — the index is partial");
        assert_ne!(first, second, "the retry opens a NEW run");
        assert_eq!(count_running_runs(&db, &task_id), 1);
    }

    /// iter-2453 (F2): the retired lease columns are gone from a fresh schema.
    ///
    /// The v2 migration drops them, so a NEWLY created database must not carry
    /// them either. Asserting on `PRAGMA table_info` is what proves the schema
    /// stopped advertising a boundary nothing enforces.
    #[test]
    fn a_fresh_schema_carries_no_retired_lease_columns() {
        let (db, _task_id, _keep) = new_board_with_task("schema");
        let conn = db.conn().lock().unwrap();

        let cols = |table: &str| -> Vec<String> {
            let mut stmt = conn
                .prepare(&format!("PRAGMA table_info({table})"))
                .unwrap();
            stmt.query_map([], |r| r.get::<_, String>(1))
                .unwrap()
                .collect::<std::result::Result<Vec<_>, _>>()
                .unwrap()
        };

        for table in ["tasks", "task_runs"] {
            let names = cols(table);
            for retired in ["claim_lock", "claim_expires", "worker_pid"] {
                assert!(
                    !names.iter().any(|c| c == retired),
                    "{table} still declares the retired column {retired}"
                );
            }
        }
        // `task_runs.last_heartbeat_at` was never written by any code path.
        assert!(
            !cols("task_runs").iter().any(|c| c == "last_heartbeat_at"),
            "task_runs still declares the never-written last_heartbeat_at"
        );
        // These two MUST survive: `tasks.last_heartbeat_at` is written by
        // `heartbeat()`, and `max_runtime_seconds` is operator intent without
        // enforcement (a separate finding, not this one).
        let task_cols = cols("tasks");
        assert!(
            task_cols.iter().any(|c| c == "last_heartbeat_at"),
            "tasks.last_heartbeat_at is live and must not be dropped"
        );
        assert!(
            cols("task_runs").iter().any(|c| c == "max_runtime_seconds"),
            "max_runtime_seconds is deliberately retained"
        );
    }
}
