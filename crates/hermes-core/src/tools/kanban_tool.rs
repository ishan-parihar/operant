use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use tracing::{debug, error, info};

use crate::error::{Error, Result};
use crate::kanban::{KanbanDb, TaskStatus};
use crate::schema::ToolSchema;
use crate::tools::{HermesTool, ToolContext, ToolResult};

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

    fn handle(&self, args: KanbanToolArgs) -> Result<String> {
        let action = args.action.to_lowercase();
        
        match action.as_str() {
            "show" => {
                let tid = args.task_id.clone().ok_or_else(|| Error::Agent("task_id is required".into()))?;
                let task = self.db.get_task(&tid)?.ok_or_else(|| Error::Agent(format!("task {} not found", tid)))?;
                
                let result = json!({
                    "task": task,
                    "parents": self.db.parent_ids(&tid)?,
                    "children": self.db.child_ids(&tid)?,
                    "comments": self.db.list_comments(&tid)?,
                    "events": self.db.list_events(&tid)?,
                    "runs": self.db.list_runs(&tid)?,
                    "worker_context": self.db.build_worker_context(&tid)?,
                });
                Ok(serde_json::to_string_pretty(&result).unwrap())
            }
            "complete" => {
                let tid = args.task_id.clone().ok_or_else(|| Error::Agent("task_id is required".into()))?;
                let success = self.db.complete_task(
                    &tid,
                    args.result.as_deref(),
                    args.summary.as_deref(),
                    args.metadata.as_ref(),
                    args.created_cards.as_deref(),
                    None
                )?;
                
                if success {
                    Ok(json!({"ok": true, "task_id": tid}).to_string())
                } else {
                    Err(Error::Agent("Could not complete task (unknown id or already terminal)".into()))
                }
            }
            "block" => {
                let tid = args.task_id.clone().ok_or_else(|| Error::Agent("task_id is required".into()))?;
                let reason = args.reason.clone().ok_or_else(|| Error::Agent("reason is required".into()))?;
                let success = self.db.block_task(&tid, &reason, None)?;
                
                if success {
                    Ok(json!({"ok": true, "task_id": tid}).to_string())
                } else {
                    Err(Error::Agent("Could not block task (unknown id or not in running/ready)".into()))
                }
            }
            "heartbeat" => {
                let tid = args.task_id.clone().ok_or_else(|| Error::Agent("task_id is required".into()))?;
                let success = self.db.heartbeat_worker(&tid, args.note.as_deref(), None)?;
                
                if success {
                    Ok(json!({"ok": true, "task_id": tid}).to_string())
                } else {
                    Err(Error::Agent("Could not heartbeat task (unknown id or not running)".into()))
                }
            }
            "comment" => {
                let tid = args.task_id.clone().ok_or_else(|| Error::Agent("task_id is required".into()))?;
                let body = args.body.clone().ok_or_else(|| Error::Agent("body is required".into()))?;
                let author = "worker";
                let cid = self.db.add_comment(&tid, author, &body)?;
                Ok(json!({"ok": true, "task_id": tid, "comment_id": cid}).to_string())
            }
            "create" => {
                let title = args.title.clone().ok_or_else(|| Error::Agent("title is required".into()))?;
                let assignee = args.assignee.clone().ok_or_else(|| Error::Agent("assignee is required".into()))?;
                
                let id = self.db.create_task(
                    &title,
                    args.body.as_deref(),
                    Some(&assignee),
                    Some("worker"),
                    args.workspace_kind.as_deref().unwrap_or("scratch"),
                    args.workspace_path.as_deref(),
                    args.tenant.as_deref(),
                    args.priority.unwrap_or(0),
                    &args.parents.clone().unwrap_or_default(),
                    args.triage.unwrap_or(false),
                    args.idempotency_key.as_deref(),
                    args.max_runtime_seconds,
                    args.skills.as_deref(),
                    args.max_retries,
                )?;
                Ok(json!({"ok": true, "task_id": id}).to_string())
            }
            "link" => {
                let parent_id = args.parent_id.clone().ok_or_else(|| Error::Agent("parent_id is required".into()))?;
                let child_id = args.child_id.clone().ok_or_else(|| Error::Agent("child_id is required".into()))?;
                self.db.link_tasks(&parent_id, &child_id)?;
                Ok(json!({"ok": true, "parent_id": parent_id, "child_id": child_id}).to_string())
            }
            _ => Err(Error::Agent(format!("Unknown kanban action '{}'", action))),
        }
    }
}

#[async_trait]
impl HermesTool for KanbanTool {
    fn name(&self) -> &str {
        "kanban"
    }

    fn description(&self) -> &str {
        "Structured tool-call surface for Kanban task management. Supports showing, completing, blocking, heartbeating, commenting, creating, and linking tasks."
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
}
