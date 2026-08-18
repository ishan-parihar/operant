use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

use crate::error::{Error, Result};
use crate::kanban::KanbanDb;
use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolResult};

#[derive(Deserialize, Serialize, Debug, JsonSchema)]
pub struct KanbanToolArgs {
    pub action: String,
    pub task_id: Option<String>,
    pub title: Option<String>,
    pub body: Option<String>,
    pub assignee: Option<String>,
    pub summary: Option<String>,
    pub metadata: Option<Value>,
    pub result: Option<String>,
    pub created_cards: Option<Vec<String>>,
    pub reason: Option<String>,
    pub note: Option<String>,
    pub parents: Option<Vec<String>>,
    pub tenant: Option<String>,
    pub priority: Option<i32>,
    pub workspace_kind: Option<String>,
    pub workspace_path: Option<String>,
    pub triage: Option<bool>,
    pub idempotency_key: Option<String>,
    pub max_runtime_seconds: Option<i32>,
    pub skills: Option<Vec<String>>,
    pub max_retries: Option<i32>,
    pub parent_id: Option<String>,
    pub child_id: Option<String>,
}

pub struct KanbanTool {
    db: Arc<KanbanDb>,
}

impl KanbanTool {
    pub fn new(db: Arc<KanbanDb>) -> Self {
        Self { db }
    }

    #[expect(
        clippy::expect_used,
        reason = "invariant guaranteed by surrounding validation"
    )]
    fn handle(&self, args: KanbanToolArgs) -> Result<String> {
        let action = args.action.to_lowercase();

        match action.as_str() {
            "list" => {
                let tasks = self.db.list_tasks()?;
                let result = json!({
                    "tasks": tasks,
                });
                Ok(serde_json::to_string_pretty(&result).expect("kanban result is serializable"))
            }
            "show" => {
                // `kanban show` without a task_id is treated as a list — the
                // agent frequently interprets "show" as "show me all tasks"
                // and previously hit a hard "task_id is required" error (the
                // live loop reported "task_id required even for list action").
                let Some(tid) = args.task_id.clone() else {
                    let tasks = self.db.list_tasks()?;
                    return Ok(serde_json::to_string_pretty(&json!({ "tasks": tasks }))
                        .expect("kanban result is serializable"));
                };
                let task = self
                    .db
                    .get_task(&tid)?
                    .ok_or_else(|| Error::Agent(format!("task {} not found", tid)))?;

                let result = json!({
                    "task": task,
                    "parents": self.db.parent_ids(&tid)?,
                    "children": self.db.child_ids(&tid)?,
                    "comments": self.db.list_comments(&tid)?,
                    "events": self.db.list_events(&tid)?,
                    "runs": self.db.list_runs(&tid)?,
                    "worker_context": self.db.build_worker_context(&tid)?,
                });
                Ok(serde_json::to_string_pretty(&result).expect("kanban result is serializable"))
            }
            "complete" => {
                let tid = args
                    .task_id
                    .clone()
                    .ok_or_else(|| Error::Agent("task_id is required".into()))?;
                let success = self.db.complete_task(
                    &tid,
                    args.result.as_deref(),
                    args.summary.as_deref(),
                    args.metadata.as_ref(),
                    args.created_cards.as_deref(),
                    None,
                )?;

                if success {
                    Ok(json!({"ok": true, "task_id": tid}).to_string())
                } else {
                    Err(Error::Agent(
                        "Could not complete task (unknown id or already terminal)".into(),
                    ))
                }
            }
            "block" => {
                let tid = args
                    .task_id
                    .clone()
                    .ok_or_else(|| Error::Agent("task_id is required".into()))?;
                let reason = args
                    .reason
                    .clone()
                    .ok_or_else(|| Error::Agent("reason is required".into()))?;
                let success = self.db.block_task(&tid, &reason, None)?;

                if success {
                    Ok(json!({"ok": true, "task_id": tid}).to_string())
                } else {
                    Err(Error::Agent(
                        "Could not block task (unknown id or not in running/ready)".into(),
                    ))
                }
            }
            "heartbeat" => {
                let tid = args
                    .task_id
                    .clone()
                    .ok_or_else(|| Error::Agent("task_id is required".into()))?;
                let success = self.db.heartbeat_worker(&tid, args.note.as_deref(), None)?;

                if success {
                    Ok(json!({"ok": true, "task_id": tid}).to_string())
                } else {
                    Err(Error::Agent(
                        "Could not heartbeat task (unknown id or not running)".into(),
                    ))
                }
            }
            "comment" => {
                let tid = args
                    .task_id
                    .clone()
                    .ok_or_else(|| Error::Agent("task_id is required".into()))?;
                let body = args
                    .body
                    .clone()
                    .ok_or_else(|| Error::Agent("body is required".into()))?;
                let author = "worker";
                let cid = self.db.add_comment(&tid, author, &body)?;
                Ok(json!({"ok": true, "task_id": tid, "comment_id": cid}).to_string())
            }
            "create" => {
                let title = args
                    .title
                    .clone()
                    .ok_or_else(|| Error::Agent("title is required".into()))?;
                let assignee = args
                    .assignee
                    .clone()
                    .ok_or_else(|| Error::Agent("assignee is required".into()))?;

                let id = self.db.create_task(crate::kanban::db::CreateTaskParams {
                    title: &title,
                    body: args.body.as_deref(),
                    assignee: Some(&assignee),
                    created_by: Some("worker"),
                    workspace_kind: args.workspace_kind.as_deref().unwrap_or("scratch"),
                    workspace_path: args.workspace_path.as_deref(),
                    tenant: args.tenant.as_deref(),
                    priority: args.priority.unwrap_or(0),
                    parents: &args.parents.clone().unwrap_or_default(),
                    triage: args.triage.unwrap_or(false),
                    idempotency_key: args.idempotency_key.as_deref(),
                    max_runtime_seconds: args.max_runtime_seconds,
                    skills: args.skills.as_deref(),
                    max_retries: args.max_retries,
                })?;
                Ok(json!({"ok": true, "task_id": id}).to_string())
            }
            "link" => {
                let parent_id = args
                    .parent_id
                    .clone()
                    .ok_or_else(|| Error::Agent("parent_id is required".into()))?;
                let child_id = args
                    .child_id
                    .clone()
                    .ok_or_else(|| Error::Agent("child_id is required".into()))?;
                self.db.link_tasks(&parent_id, &child_id)?;
                Ok(json!({"ok": true, "parent_id": parent_id, "child_id": child_id}).to_string())
            }
            _ => Err(Error::Agent(format!("Unknown kanban action '{}'", action))),
        }
    }
}

#[async_trait]
impl OperantTool for KanbanTool {
    fn name(&self) -> &str {
        "kanban"
    }

    fn description(&self) -> &str {
        "Structured tool-call surface for Kanban task management. \
         Actions: 'list' — all tasks (no task_id needed); 'show' — task detail \
         (show without task_id lists all tasks); 'create' — new task; \
         'complete'/'block'/'heartbeat'/'comment' — require task_id; 'link' — \
         parent_id + child_id."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<KanbanToolArgs>("kanban", self.description())
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: KanbanToolArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("kanban", format!("Invalid arguments: {}", e)),
        };

        match self.handle(args) {
            Ok(content) => ToolResult::success("kanban", content),
            Err(e) => ToolResult::error("kanban", e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kanban_schema() {
        let schema = ToolSchema::from_type::<KanbanToolArgs>("kanban", "test");
        let json = serde_json::to_value(&schema).unwrap();
        assert!(json.is_object());
        assert_eq!(json["name"], "kanban");
    }

    #[test]
    fn test_kanban_show_without_task_id_lists_tasks() {
        // Regression: the live loop reported "task_id required even for list
        // action" — the model calls `kanban show` expecting a listing. Show
        // without a task_id must fall back to listing, never error.
        let db = Arc::new(
            KanbanDb::init(std::path::PathBuf::from("test_kanban_show_fallback.db")).unwrap(),
        );
        let tool = KanbanTool::new(db);
        let args: KanbanToolArgs = serde_json::from_value(json!({"action": "show"})).unwrap();
        let out = tool.handle(args).unwrap();
        assert!(out.contains("tasks"), "show-without-id must return a task list");
    }
}
