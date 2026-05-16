pub mod db;
pub mod dispatcher;
pub mod notify;

pub use db::{Comment, Event, KanbanDb, Run, Task, TaskStatus};
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
/// Boards map to separate SQLite database files named `hermes_kanban_<slug>.db`.
/// The "default" board uses the backward-compatible name `hermes_kanban.db`.
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
    /// "default" → `hermes_kanban.db` (backward-compatible).
    /// other   → `hermes_kanban_<slug>.db`.
    pub fn resolve_path(&self, slug: &str) -> PathBuf {
        if slug.is_empty() || slug == "default" {
            self.kanban_dir.join("hermes_kanban.db")
        } else {
            self.kanban_dir.join(format!("hermes_kanban_{}.db", slug))
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
                if name_str.starts_with("hermes_kanban_") && name_str.ends_with(".db") {
                    if let Some(slug) = name_str
                        .strip_prefix("hermes_kanban_")
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
    pub fn delete_board(&self, slug: &str) -> Result<(), Error> {
        if slug == "default" {
            return Err(Error::Agent("Cannot delete the default board".to_string()));
        }
        let path = self.resolve_path(slug);
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
            "hermes_kanban_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_resolve_path_default() {
        let dir = temp_kanban_dir();
        let mgr = KanbanManager::new(dir.clone());
        assert_eq!(mgr.resolve_path("default"), dir.join("hermes_kanban.db"));
        assert_eq!(mgr.resolve_path(""), dir.join("hermes_kanban.db"));
    }

    #[test]
    fn test_resolve_path_named() {
        let dir = temp_kanban_dir();
        let mgr = KanbanManager::new(dir.clone());
        assert_eq!(mgr.resolve_path("work"), dir.join("hermes_kanban_work.db"));
        assert_eq!(
            mgr.resolve_path("personal"),
            dir.join("hermes_kanban_personal.db")
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
            .create_task(
                "Task for A",
                Some("Body A"),
                None,
                Some("test"),
                "test",
                None,
                None,
                1,
                &[],
                false,
                None,
                None,
                None,
                None,
            )
            .unwrap();

        let task_b_id = db_b
            .create_task(
                "Task for B",
                Some("Body B"),
                None,
                Some("test"),
                "test",
                None,
                None,
                1,
                &[],
                false,
                None,
                None,
                None,
                None,
            )
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
}
