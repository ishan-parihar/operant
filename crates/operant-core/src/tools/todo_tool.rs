//! Todo list tool
//!
//! In-memory task list management tool matching Python's todo_tool.py.
//! Stores todos per session using a global LazyLock HashMap.
//!
//! hermes parity (hermes-agent/tools/todo_tool.py):
//! - provide `todos` to write, omit to read the current list
//! - `merge: true` updates existing items by id and appends new ones
//! - duplicate ids are collapsed (last occurrence kept in place)
//! - content capped at MAX_TODO_CONTENT_CHARS, list at MAX_TODO_ITEMS
//!   (hermes caps the persisted state so a single oversized item — whether
//!   authored by the model or replayed from caller-supplied history — cannot
//!   inflate the post-compression re-injection block without bound)
//! - active items are re-injected after context compression via
//!   `todo_injection_for_session` (hermes `TodoStore.format_for_injection`),
//!   so the model keeps its plan across compactions

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolResult};

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

/// Max characters per todo item content (hermes: MAX_TODO_CONTENT_CHARS).
pub const MAX_TODO_CONTENT_CHARS: usize = 4000;
/// Max todo items per session list (hermes: MAX_TODO_ITEMS).
pub const MAX_TODO_ITEMS: usize = 256;
const TRUNCATION_MARKER: &str = "… [truncated]";
/// Stable header marking the synthetic post-compression todo snapshot row.
/// Context compression strips any prior row carrying this header before
/// appending a fresh one, so repeated compactions refresh rather than
/// accumulate (hermes conversation_compression.py).
pub const TODO_INJECTION_HEADER: &str =
    "[Your active task list was preserved across context compression]";

static TODO_STORE: LazyLock<Mutex<HashMap<String, Vec<TodoItem>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// A single todo item
#[derive(Debug, Clone, JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TodoItem {
    /// Unique identifier for this todo
    pub id: String,
    /// Content/description of the task
    pub content: String,
    /// Status: "pending", "in_progress", "completed", or "cancelled"
    pub status: String,
}

impl TodoItem {
    /// Validate that the status is one of the allowed values
    fn is_valid_status(status: &str) -> bool {
        matches!(
            status,
            "pending" | "in_progress" | "completed" | "cancelled"
        )
    }

    /// Validate and normalize a todo item (hermes `TodoStore._validate`):
    /// default empty ids/content, cap content, coerce status to lowercase
    /// with a "pending" fallback for unknown values.
    fn normalized(mut self) -> Self {
        self.id = if self.id.trim().is_empty() {
            "?".to_string()
        } else {
            self.id
        };

        let content = self.content.trim();
        self.content = if content.is_empty() {
            "(no description)".to_string()
        } else {
            cap_content(content)
        };

        let status = self.status.trim().to_lowercase();
        self.status = if Self::is_valid_status(&status) {
            status
        } else {
            "pending".to_string()
        };
        self
    }
}

/// Truncate oversized todo content, keeping the head (the actionable part of
/// a task description) plus a truncation marker (hermes `_cap_content`).
fn cap_content(content: &str) -> String {
    if content.chars().count() <= MAX_TODO_CONTENT_CHARS {
        return content.to_string();
    }
    let keep = MAX_TODO_CONTENT_CHARS.saturating_sub(TRUNCATION_MARKER.chars().count());
    let mut truncated: String = content.chars().take(keep).collect();
    truncated.push_str(TRUNCATION_MARKER);
    truncated
}

/// Collapse duplicate ids, keeping the last occurrence at its original
/// position (hermes `TodoStore._dedupe_by_id`).
fn dedupe_by_id(items: Vec<TodoItem>) -> Vec<TodoItem> {
    let mut last_index: HashMap<String, usize> = HashMap::new();
    for (i, item) in items.iter().enumerate() {
        let key = if item.id.trim().is_empty() {
            "?"
        } else {
            item.id.trim()
        };
        last_index.insert(key.to_string(), i);
    }
    let mut order: Vec<usize> = last_index.values().copied().collect();
    order.sort_unstable();
    order.into_iter().map(|i| items[i].clone()).collect()
}

/// Arguments for the todo tool
#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TodoArgs {
    /// The complete list of todos. Omit to read the current list for this
    /// session; provide an empty list to clear it.
    todos: Option<Vec<TodoItem>>,
    /// If true, update existing items by id and append new ones instead of
    /// replacing the entire list.
    #[serde(default)]
    merge: bool,
    /// Session ID to scope the todo list (defaults to "default")
    session_id: Option<String>,
}

/// Tool for managing an in-memory todo list
pub struct TodoTool;

#[async_trait]
impl OperantTool for TodoTool {
    fn name(&self) -> &str {
        "todo"
    }

    fn description(&self) -> &str {
        "Manage a task list. Provide 'todos' to write the list (replacing it, \
        or merging by id when 'merge' is true); omit 'todos' to read the \
        current list. Supported statuses: pending, in_progress, completed, cancelled."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<TodoArgs>("todo", "Manage task list")
    }

    #[expect(
        clippy::expect_used,
        reason = "poisoned lock: panic is the intended recovery"
    )]
    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: TodoArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("todo", format!("Invalid arguments: {}", e)),
        };
        let session_id = args.session_id.unwrap_or_else(|| "default".to_string());

        // Read mode (hermes: omit `todos` to read the current list)
        let Some(todos) = args.todos else {
            let store = TODO_STORE
                .lock()
                .expect("TODO_STORE mutex poisoned — programmer error");
            let current = store.get(&session_id).cloned().unwrap_or_default();
            let summary: Vec<Value> = current
                .iter()
                .map(|t| {
                    json!({
                        "id": t.id,
                        "content": t.content,
                        "status": t.status,
                    })
                })
                .collect();
            return ToolResult::success(
                "todo",
                json!({
                    "sessionId": session_id,
                    "count": current.len(),
                    "todos": summary,
                }),
            );
        };

        // Write path: dedupe by id, normalize, enforce item cap (hermes caps
        // persisted state so the re-injection block stays bounded). The global
        // store lock is scoped to just the store access — summaries are built
        // outside it.
        let mut items: Vec<TodoItem> = dedupe_by_id(todos)
            .into_iter()
            .map(TodoItem::normalized)
            .collect();
        items.truncate(MAX_TODO_ITEMS);

        let next: Vec<TodoItem> = {
            let mut store = TODO_STORE
                .lock()
                .expect("TODO_STORE mutex poisoned — programmer error");
            let next = if args.merge {
                // Merge mode: update existing items by id, append new ones.
                let mut merged: Vec<TodoItem> = store.get(&session_id).cloned().unwrap_or_default();
                // Owned keys avoid aliasing borrows into `merged` while it is
                // mutated below.
                let mut index: HashMap<String, usize> = HashMap::new();
                for (i, item) in merged.iter().enumerate() {
                    index.insert(item.id.clone(), i);
                }
                for item in items {
                    if let Some(&i) = index.get(item.id.as_str()) {
                        merged[i].content = item.content;
                        merged[i].status = item.status;
                    } else {
                        index.insert(item.id.clone(), merged.len());
                        merged.push(item);
                    }
                }
                merged.truncate(MAX_TODO_ITEMS);
                merged
            } else {
                items
            };
            store.insert(session_id.clone(), next.clone());
            next
        };

        let summary: Vec<Value> = next
            .iter()
            .map(|t| {
                json!({
                    "id": t.id,
                    "content": t.content,
                    "status": t.status,
                })
            })
            .collect();

        ToolResult::success(
            "todo",
            json!({
                "sessionId": session_id,
                "count": summary.len(),
                "todos": summary,
            }),
        )
    }
}

/// Render the active todo list (pending/in_progress only) for re-injection
/// after context compression. Returns None when nothing active remains.
/// Ported from hermes `TodoStore.format_for_injection` — completed/cancelled
/// items are excluded so the model does not re-do finished work.
pub fn todo_injection_for_session(session_id: &str) -> Option<String> {
    let store = TODO_STORE.lock().ok()?;
    let items = store.get(session_id)?;
    let active: Vec<&TodoItem> = items
        .iter()
        .filter(|i| matches!(i.status.as_str(), "pending" | "in_progress"))
        .collect();
    if active.is_empty() {
        return None;
    }

    let mut lines = vec![TODO_INJECTION_HEADER.to_string()];
    for item in active {
        let marker = match item.status.as_str() {
            "completed" => "[x]",
            "in_progress" => "[>]",
            "pending" => "[ ]",
            _ => "[~]",
        };
        lines.push(format!(
            "- {marker} {}. {} ({})",
            item.id, item.content, item.status
        ));
    }
    Some(lines.join("\n"))
}

/// True if a message content is a prior todo-injection snapshot row. Context
/// compression strips these before appending a fresh snapshot so repeated
/// compactions refresh rather than accumulate (hermes conversation_compression.py).
pub fn is_todo_injection_row(content: &str) -> bool {
    content.starts_with(TODO_INJECTION_HEADER)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::ToolContext;

    fn default_context() -> ToolContext {
        ToolContext::default()
    }

    async fn execute_tool(payload: Value) -> (bool, Value, Option<String>) {
        let tool = TodoTool;
        let result = tool.execute(payload, default_context()).await;
        let parsed: Value = serde_json::from_str(&result.content).unwrap_or(Value::Null);
        (result.success, parsed, result.error)
    }

    #[tokio::test]
    async fn test_todo_replace_list() {
        let (ok, parsed, _) = execute_tool(serde_json::json!({
            "todos": [
                { "id": "1", "content": "Write tests", "status": "pending" },
                { "id": "2", "content": "Review PR", "status": "in_progress" },
            ],
            "sessionId": "test-session-1"
        }))
        .await;
        assert!(ok);
        assert_eq!(parsed["count"], 2);
        assert_eq!(parsed["sessionId"], "test-session-1");
    }

    #[tokio::test]
    async fn test_todo_default_session() {
        let (ok, parsed, _) = execute_tool(serde_json::json!({
            "todos": [
                { "id": "a", "content": "Task A", "status": "completed" }
            ]
        }))
        .await;
        assert!(ok);
        assert_eq!(parsed["sessionId"], "default");
    }

    #[tokio::test]
    async fn test_todo_invalid_status_normalized_to_pending() {
        // hermes _validate coerces unknown statuses to "pending" instead of
        // rejecting the whole write.
        let (ok, parsed, _) = execute_tool(serde_json::json!({
            "todos": [
                { "id": "1", "content": "Bad status", "status": "done" }
            ],
            "sessionId": "test-invalid-status"
        }))
        .await;
        assert!(ok);
        assert_eq!(parsed["todos"][0]["status"], "pending");
    }

    #[tokio::test]
    async fn test_todo_empty_list_clears() {
        let (ok, parsed, _) = execute_tool(serde_json::json!({
            "todos": [],
            "sessionId": "test-empty"
        }))
        .await;
        assert!(ok);
        assert_eq!(parsed["count"], 0);
    }

    #[tokio::test]
    async fn test_todo_replaces_previous() {
        let session = "test-replace";
        execute_tool(serde_json::json!({
            "todos": [
                { "id": "1", "content": "First", "status": "pending" },
                { "id": "2", "content": "Second", "status": "pending" },
            ],
            "sessionId": session
        }))
        .await;

        let (ok, parsed, _) = execute_tool(serde_json::json!({
            "todos": [
                { "id": "3", "content": "Third", "status": "completed" }
            ],
            "sessionId": session
        }))
        .await;
        assert!(ok);
        assert_eq!(parsed["count"], 1);
        assert_eq!(parsed["todos"][0]["id"], "3");
    }

    #[tokio::test]
    async fn test_todo_read_mode_returns_current_list() {
        let session = "test-read";
        execute_tool(serde_json::json!({
            "todos": [
                { "id": "1", "content": "Alpha", "status": "pending" },
                { "id": "2", "content": "Beta", "status": "in_progress" },
            ],
            "sessionId": session
        }))
        .await;

        // Omit `todos` → read mode must return the stored list unchanged.
        let (ok, parsed, _) = execute_tool(serde_json::json!({ "sessionId": session })).await;
        assert!(ok);
        assert_eq!(parsed["count"], 2);
        assert_eq!(parsed["todos"][0]["id"], "1");
        assert_eq!(parsed["todos"][1]["content"], "Beta");
    }

    #[tokio::test]
    async fn test_todo_read_mode_empty_session() {
        let (ok, parsed, _) =
            execute_tool(serde_json::json!({ "sessionId": "test-read-empty" })).await;
        assert!(ok);
        assert_eq!(parsed["count"], 0);
    }

    #[tokio::test]
    async fn test_todo_merge_updates_by_id_and_appends() {
        let session = "test-merge";
        execute_tool(serde_json::json!({
            "todos": [
                { "id": "1", "content": "Alpha", "status": "pending" },
                { "id": "2", "content": "Beta", "status": "pending" },
            ],
            "sessionId": session
        }))
        .await;

        let (ok, parsed, _) = execute_tool(serde_json::json!({
            "todos": [
                { "id": "1", "content": "Alpha done", "status": "completed" },
                { "id": "3", "content": "Gamma", "status": "pending" },
            ],
            "merge": true,
            "sessionId": session
        }))
        .await;
        assert!(ok);
        assert_eq!(parsed["count"], 3);
        assert_eq!(parsed["todos"][0]["id"], "1");
        assert_eq!(parsed["todos"][0]["content"], "Alpha done");
        assert_eq!(parsed["todos"][0]["status"], "completed");
        // Existing item 2 untouched
        assert_eq!(parsed["todos"][1]["content"], "Beta");
        // New item appended
        assert_eq!(parsed["todos"][2]["id"], "3");
    }

    #[tokio::test]
    async fn test_todo_dedupe_keeps_last_occurrence() {
        let (ok, parsed, _) = execute_tool(serde_json::json!({
            "todos": [
                { "id": "1", "content": "First", "status": "pending" },
                { "id": "2", "content": "Beta", "status": "pending" },
                { "id": "1", "content": "Last", "status": "in_progress" },
            ],
            "sessionId": "test-dedupe"
        }))
        .await;
        // hermes _dedupe_by_id keeps the LAST occurrence at its original
        // position: [Beta, Last], not [Last, Beta].
        assert!(ok);
        assert_eq!(parsed["count"], 2);
        assert_eq!(parsed["todos"][0]["id"], "2");
        assert_eq!(parsed["todos"][0]["content"], "Beta");
        assert_eq!(parsed["todos"][1]["id"], "1");
        assert_eq!(parsed["todos"][1]["content"], "Last");
    }

    #[tokio::test]
    async fn test_todo_content_capped() {
        let long_content = "x".repeat(MAX_TODO_CONTENT_CHARS + 100);
        let (ok, parsed, _) = execute_tool(serde_json::json!({
            "todos": [
                { "id": "1", "content": long_content, "status": "pending" }
            ],
            "sessionId": "test-cap"
        }))
        .await;
        assert!(ok);
        let stored = parsed["todos"][0]["content"].as_str().unwrap();
        assert!(stored.ends_with(TRUNCATION_MARKER));
        assert!(stored.chars().count() <= MAX_TODO_CONTENT_CHARS);
    }

    #[tokio::test]
    async fn test_todo_item_cap() {
        let todos: Vec<Value> = (0..(MAX_TODO_ITEMS + 50))
            .map(|i| json!({ "id": i.to_string(), "content": "task", "status": "pending" }))
            .collect();
        let (ok, parsed, _) = execute_tool(serde_json::json!({
            "todos": todos,
            "sessionId": "test-item-cap"
        }))
        .await;
        assert!(ok);
        assert_eq!(parsed["count"], MAX_TODO_ITEMS);
    }

    #[tokio::test]
    async fn test_todo_injection_format() {
        let session = "test-inject";
        execute_tool(serde_json::json!({
            "todos": [
                { "id": "1", "content": "Pending task", "status": "pending" },
                { "id": "2", "content": "Active task", "status": "in_progress" },
                { "id": "3", "content": "Finished task", "status": "completed" },
                { "id": "4", "content": "Cancelled task", "status": "cancelled" },
            ],
            "sessionId": session
        }))
        .await;

        let snapshot = todo_injection_for_session(session).expect("active todos present");
        let lines: Vec<&str> = snapshot.lines().collect();
        assert_eq!(lines[0], TODO_INJECTION_HEADER);
        assert_eq!(lines.len(), 3); // header + 2 active items
        // Only pending/in_progress items are injected, with status markers
        assert!(snapshot.contains("- [ ] 1. Pending task (pending)"));
        assert!(snapshot.contains("- [>] 2. Active task (in_progress)"));
        assert!(!snapshot.contains("Finished task"));
        assert!(!snapshot.contains("Cancelled task"));
        assert!(is_todo_injection_row(&snapshot));
    }

    #[tokio::test]
    async fn test_todo_injection_none_when_no_active() {
        let session = "test-inject-none";
        execute_tool(serde_json::json!({
            "todos": [
                { "id": "3", "content": "Finished task", "status": "completed" }
            ],
            "sessionId": session
        }))
        .await;
        assert!(todo_injection_for_session(session).is_none());
    }

    #[test]
    fn test_cap_content_short_passthrough() {
        assert_eq!(cap_content("short"), "short");
    }

    #[test]
    fn test_cap_content_utf8_no_split() {
        let content = "é".repeat(MAX_TODO_CONTENT_CHARS + 10);
        let capped = cap_content(&content);
        assert!(capped.chars().count() <= MAX_TODO_CONTENT_CHARS);
        // No partial multibyte char: the kept head is all intact 'é' and the
        // tail is exactly the truncation marker.
        let (head, tail) = capped.split_at(capped.len() - TRUNCATION_MARKER.len());
        assert!(head.chars().all(|c| c == 'é'));
        assert!(tail.starts_with(TRUNCATION_MARKER));
    }
}
