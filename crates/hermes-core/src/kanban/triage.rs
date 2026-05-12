use crate::error::Error;
use rusqlite::{params, Connection};
use std::sync::{Arc, Mutex};

/// Context about a task for triage decisions
#[derive(Debug, Clone)]
pub struct TriageContext {
    pub task_id: String,
    pub title: String,
    pub description: String,
    pub status: String,
    pub priority: i32,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub comments: Vec<(String, String, i64)>, // (author, content, created_at)
    pub linked_tasks: Vec<String>,
    pub recent_events: Vec<(String, String, String, i64)>, // (kind, payload, outcome, created_at)
}

/// Builds triage context from the database for LLM prompting
pub struct TriageSpecifier {
    db: Arc<Mutex<Connection>>,
}

impl TriageSpecifier {
    pub fn new(db: Arc<Mutex<Connection>>) -> Self {
        Self { db }
    }

    /// Gather full context for a task
    pub fn build_context(&self, task_id: &str) -> Result<TriageContext, Error> {
        let conn = self.db.lock().unwrap();

        // Get task
        let task = conn
            .query_row(
                "SELECT id, title, body, status, priority, created_at, started_at FROM tasks WHERE id = ?1",
                params![task_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, i32>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, Option<i64>>(6)?,
                    ))
                },
            )
            .map_err(|e| Error::Agent(format!("Task not found: {}", e)))?;

        // Get comments
        let mut comment_stmt = conn
            .prepare(
                "SELECT author, body, created_at FROM task_comments WHERE task_id = ?1 ORDER BY created_at ASC",
            )
            .map_err(|e| Error::Agent(format!("Failed to prepare comments query: {}", e)))?;

        let comments: Vec<(String, String, i64)> = comment_stmt
            .query_map(params![task_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
            .map_err(|e| Error::Agent(format!("Query error: {}", e)))?
            .filter_map(|r| r.ok())
            .collect();

        // Get parent IDs
        let mut parent_stmt = conn
            .prepare("SELECT parent_id FROM task_links WHERE child_id = ?1")
            .map_err(|e| Error::Agent(format!("Failed to prepare parent query: {}", e)))?;

        let parent_ids: Vec<String> = parent_stmt
            .query_map(params![task_id], |row| row.get::<_, String>(0))
            .map_err(|e| Error::Agent(format!("Query error: {}", e)))?
            .filter_map(|r| r.ok())
            .collect();

        // Get child IDs
        let mut child_stmt = conn
            .prepare("SELECT child_id FROM task_links WHERE parent_id = ?1")
            .map_err(|e| Error::Agent(format!("Failed to prepare child query: {}", e)))?;

        let child_ids: Vec<String> = child_stmt
            .query_map(params![task_id], |row| row.get::<_, String>(0))
            .map_err(|e| Error::Agent(format!("Query error: {}", e)))?
            .filter_map(|r| r.ok())
            .collect();

        let mut linked_tasks = parent_ids;
        linked_tasks.extend(child_ids);

        // Get events (last 20)
        let mut event_stmt = conn
            .prepare(
                "SELECT kind, payload, created_at FROM task_events WHERE task_id = ?1 ORDER BY created_at DESC LIMIT 20",
            )
            .map_err(|e| Error::Agent(format!("Failed to prepare events query: {}", e)))?;

        let recent_events: Vec<(String, String, String, i64)> = event_stmt
            .query_map(params![task_id], |row| {
                let payload_raw: Option<String> = row.get(1)?;
                let payload_str = payload_raw.unwrap_or_default();
                Ok((
                    row.get::<_, String>(0)?,
                    payload_str,
                    String::new(), // events have no outcome field
                    row.get::<_, i64>(2)?,
                ))
            })
            .map_err(|e| Error::Agent(format!("Query error: {}", e)))?
            .filter_map(|r| r.ok())
            .collect();

        let (id, title, body, status, priority, created_at, started_at) = task;

        Ok(TriageContext {
            task_id: id,
            title,
            description: body.unwrap_or_default(),
            status,
            priority,
            created_at,
            started_at,
            comments,
            linked_tasks,
            recent_events,
        })
    }

    /// Build a prompt string for an LLM to analyze the task
    pub fn build_prompt(&self, task_id: &str, instruction: Option<&str>) -> Result<String, Error> {
        let ctx = self.build_context(task_id)?;

        let instruction_text =
            instruction.unwrap_or("Suggest the next action for this task.");

        let started_str = ctx
            .started_at
            .map(|t| t.to_string())
            .unwrap_or_else(|| "Not started".to_string());

        let comments_str = ctx
            .comments
            .iter()
            .map(|(a, c, _t)| format!("  - {}: {}", a, c))
            .collect::<Vec<_>>()
            .join("\n");

        let events_str = ctx
            .recent_events
            .iter()
            .map(|(k, p, _o, _t)| format!("  [{}] {}", k, p))
            .collect::<Vec<_>>()
            .join("\n");

        let prompt = format!(
            r#"Triage Task Analysis
========================
Instruction: {}

Task ID: {}
Title: {}
Description: {}
Status: {}
Priority: {}
Created: {}
Started: {}

Comments:
{}

Linked Tasks: {}

Recent Events (last 20):
{}

Please analyze this task and suggest:
1. Should this task be prioritized, deprioritized, or blocked?
2. Is there any risk of this task becoming stale?
3. What is the recommended next action?
"#,
            instruction_text,
            ctx.task_id,
            ctx.title,
            ctx.description,
            ctx.status,
            ctx.priority,
            ctx.created_at,
            started_str,
            comments_str,
            ctx.linked_tasks.join(", "),
            events_str,
        );

        Ok(prompt)
    }
}
