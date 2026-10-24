//! Todo list tool
//!
//! Task list management tool matching Python's todo_tool.py. Stores todos
//! per session in a global store that is FILE-BACKED: every write persists
//! to `<operant_home>/todos/<hash>.json` and a fresh process rehydrates from
//! disk on first read. Before iter-1234 the store was process-local only,
//! so every TUI restart reported "Todos: 0/0 done, 0 in progress" — an
//! honest description of an empty store that had forgotten yesterday's
//! list.
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
//!
//! [port-jcode] upstream persists per-session todos as JSON under the
//! jcode state dir (`jcode-base/src/todo.rs`: `load_todos`/
//! `save_todos` + `todo_state_path`). Same shape here, resolved through
//! the one home resolver so a profile or `HERMES_HOME` deployment isolates
//! its lists.

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
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

/// Sessions whose list has been loaded from disk this process. A session
/// absent here has not been read yet: the first read pulls its file so a
/// fresh process sees yesterday's list instead of an empty one.
static LOADED_SESSIONS: LazyLock<Mutex<std::collections::HashSet<String>>> =
    LazyLock::new(|| Mutex::new(std::collections::HashSet::new()));

/// The file one session's list persists to, under the one operant home.
///
/// The session id is MODEL-SUPPLIED, so it is not safe as a filename: a
/// crafted id like `../../keys` would steer the write outside the todos
/// directory. A short stable hash of the id names the file instead, and the
/// id itself is stored INSIDE the JSON so a file can always be traced back
/// to its session without trusting the name.
///
/// The directory resolves through an override cell before the one home:
/// the store is process-global, so the lib test suite (parallel threads)
/// must be able to point it at a tempdir WITHOUT mutating HERMES_HOME, which
/// is process-global too and would race every parallel test that resolves
/// a path. Same pattern as `config_shim`'s override cells.
static TODO_DIR_OVERRIDE: std::sync::RwLock<Option<std::path::PathBuf>> =
    std::sync::RwLock::new(None);

/// Test seam: point the todo store at a directory, returning the previous
/// one so the caller can restore it. Production never calls this; the
/// default is `operant_home()/todos`, resolved through the one-home
/// resolver.
#[cfg(test)]
pub(crate) fn set_todo_dir_for_tests(
    dir: Option<std::path::PathBuf>,
) -> Option<std::path::PathBuf> {
    let mut cell = TODO_DIR_OVERRIDE.write().unwrap_or_else(|e| e.into_inner());
    let previous = cell.clone();
    *cell = dir;
    previous
}

/// The todos directory: the test override when set, else the one home.
fn todos_dir() -> std::path::PathBuf {
    if let Ok(cell) = TODO_DIR_OVERRIDE.read()
        && let Some(dir) = cell.as_ref()
    {
        return dir.clone();
    }
    crate::platform::operant_home().join("todos")
}

pub(crate) fn todo_state_path(session_id: &str) -> std::path::PathBuf {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    session_id.hash(&mut hasher);
    let digest = hasher.finish();
    todos_dir().join(format!("{digest:016x}.json"))
}

/// Read a session's list, pulling it from disk the first time this process
/// asks. A missing file or a parse failure degrades to an empty list — the
/// tool must never fail a turn over persistence.
fn load_session(session_id: &str) -> Vec<TodoItem> {
    if let Ok(loaded) = LOADED_SESSIONS.lock()
        && !loaded.contains(session_id)
    {
        drop(loaded);
        let from_disk: Option<Vec<TodoItem>> = std::fs::read(todo_state_path(session_id))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok());
        let mut store = TODO_STORE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(items) = from_disk {
            store.insert(session_id.to_string(), items);
        }
        if let Ok(mut loaded) = LOADED_SESSIONS.lock() {
            loaded.insert(session_id.to_string());
        }
    }
    TODO_STORE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(session_id)
        .cloned()
        .unwrap_or_default()
}

/// Persist a session's list. Failure is logged and swallowed: persistence is
/// an enhancement, never a turn-killer (degrade-don't-die at the store
/// seam). Atomic via temp-file + rename so a crash mid-write can never
/// leave a half-written list that fails to parse on the next boot.
fn persist_session(session_id: &str, items: &[TodoItem]) {
    use std::io::Write;
    let path = todo_state_path(session_id);
    // An explicit clear (empty list) REMOVES the file rather than persisting
    // an empty array: a stale empty file and no file are indistinguishable to
    // the reader, but removal also lets the operator see, in the directory,
    // which sessions actually have lists.
    if items.is_empty() {
        if let Err(e) = std::fs::remove_file(&path)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(target: "todo", session = session_id, error = %e, "todo list removal failed");
        }
        return;
    }
    let write = || -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec(items)
            .map_err(|e| std::io::Error::other(format!("serialize todo list: {e}")))?;
        let tmp = path.with_extension("json.tmp");
        {
            let mut file = std::fs::File::create(&tmp)?;
            file.write_all(&bytes)?;
            file.sync_all().ok();
        }
        std::fs::rename(&tmp, &path)
    };
    if let Err(e) = write() {
        tracing::warn!(target: "todo", session = session_id, error = %e, "todo list persist failed; keeping memory state");
    }
}

/// A single todo item
///
/// The model writes `phase` and `lane` — the card's words for stage and
/// group. Both are carried through to the TUI so a staged plan renders as
/// one; before this they were silently dropped here, which made the todo
/// card's group headers unreachable in practice (the model had no way to
/// name a group).
///
/// [port-jcode] upstream's todo item advertises `phase`/`lane` rather than
/// `stage`/`group` for exactly this reason: the schema words are the ones the
/// model reaches for.
#[derive(Debug, Clone, JsonSchema, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TodoItem {
    /// Unique identifier for this todo
    pub id: String,
    /// Content/description of the task
    pub content: String,
    /// Status: "pending", "in_progress", "completed", or "cancelled"
    pub status: String,
    /// Phase this todo belongs to. Todos that share a phase run as one batch;
    /// the next phase waits for this one. Omit for an unphased list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<u32>,
    /// Lane this todo travels: todos sharing a lane run in parallel inside
    /// one phase and render as one group on the card.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lane: Option<String>,
    /// Model-facing summary shown on the card. Upstream calls this `priority`
    /// and always writes it; operant's tool result never did, which left the
    /// TUI parsing a default for a field no caller could set.
    #[serde(default = "default_priority", skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    /// Optional longer brief: file paths, acceptance criteria, context. Never
    /// drawn on the card (which shows only the short `content` title), but
    /// carried through the write/read round trip so a swarm worker's
    /// assignment is not starved of context by a short title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
    /// Ids of todos that must finish before this one starts (upstream
    /// `blocked_by`). The TUI renders a blocked item as blocked.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocked_by: Vec<String>,
    /// Optional owner label, surfaced on the card (upstream `assigned_to`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assigned_to: Option<String>,
}

fn default_priority() -> Option<String> {
    Some("medium".to_string())
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
        // An empty lane name groups nothing — the trim keeps a stray
        // whitespace label from splitting a lane in two.
        self.lane = self
            .lane
            .take()
            .map(|lane| lane.trim().to_string())
            .filter(|lane| !lane.is_empty());
        self
    }
}

/// Stable summary of one todo for a tool result.
///
/// `phase` and `lane` are the model-facing spellings; the TUI reads them as
/// the card's stage and group, so a written plan survives the round trip
/// instead of arriving with its grouping stripped.
fn todo_summary(t: &TodoItem) -> Value {
    json!({
        "id": t.id,
        "content": t.content,
        "status": t.status,
        "phase": t.phase,
        "lane": t.lane,
        // Every extra field the upstream model contract exposes, so a plan
        // written with them survives the write/read round trip. These were
        // dropped before iter-1201: the TUI already rendered `blocked_by` and
        // carried `assigned_to`, but the tool's struct discarded both, meaning
        // the model could never set them and the two halves disagreed.
        "priority": t.priority,
        "details": t.details,
        "blocked_by": t.blocked_by,
        "assigned_to": t.assigned_to,
    })
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

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: TodoArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("todo", format!("Invalid arguments: {}", e)),
        };
        let session_id = args.session_id.unwrap_or_else(|| "default".to_string());

        // Read mode (hermes: omit `todos` to read the current list).
        // The first read of a session pulls its file from disk, so a fresh
        // process rehydrates the list instead of reporting a false 0/0.
        let Some(todos) = args.todos else {
            let current = load_session(&session_id);
            let summary: Vec<Value> = current.iter().map(todo_summary).collect();
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
            // The existing list is loaded BEFORE the store lock: a merge in a
            // fresh process must merge INTO the persisted list rather than
            // into an empty one that then overwrites the file, and
            // load_session takes the same store lock, so loading inside the
            // lock below would deadlock a plain Mutex.
            let existing = if args.merge {
                Some(load_session(&session_id))
            } else {
                None
            };
            let mut store = TODO_STORE.lock().unwrap_or_else(|e| e.into_inner());
            let next = if let Some(mut merged) = existing {
                // Merge mode: update existing items by id, append new ones.
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
                        // The grouping travels with the item: an update that
                        // omits phase/lane leaves them as they were rather
                        // than dropping the row out of its lane.
                        if item.phase.is_some() {
                            merged[i].phase = item.phase;
                        }
                        if item.lane.is_some() {
                            merged[i].lane = item.lane;
                        }
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

        // Write-through: the list outlives the process. Failure inside is
        // logged, never fatal.
        persist_session(&session_id, &next);
        // A merge that loaded from disk must not re-read it: the memory copy
        // is now the truth.
        if let Ok(mut loaded) = LOADED_SESSIONS.lock() {
            loaded.insert(session_id.clone());
        }

        let summary: Vec<Value> = next.iter().map(todo_summary).collect();

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
    let items = load_session(session_id);
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

/// Counts `(total, completed, in_progress)` read back from a `todo` tool
/// result payload. The agent loop uses this to emit
/// `AgentEvent::TodoUpdated`; returns `None` for any other tool result.
pub fn todo_counts_from_result(content: &str) -> Option<(usize, usize, usize)> {
    let payload: Value = serde_json::from_str(content).ok()?;
    let todos = payload.get("todos")?.as_array()?;
    let completed = todos
        .iter()
        .filter(|t| t.get("status").and_then(Value::as_str) == Some("completed"))
        .count();
    let in_progress = todos
        .iter()
        .filter(|t| t.get("status").and_then(Value::as_str) == Some("in_progress"))
        .count();
    Some((todos.len(), completed, in_progress))
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

    /// Redirect the process-global todo store to a per-test tempdir for the
    /// duration of one test, swapping the in-memory state out with it.
    ///
    /// Why a full swap rather than just pointing the dir elsewhere: the
    /// store, the LOADED_SESSIONS cache and the dir cell are all
    /// PROCESS-GLOBAL, so without the swap a parallel test's session leaks
    /// through memory into this one's assertions — and the test suite must
    /// never write into the operator's real `~/.operant/todos`.
    /// Serialise every test that touches the store. The dir override is ONE
    /// cell and the store is process-global: two guards alive at once would
    /// capture each other as "previous" and restore into chaos, so `new()`
    /// takes STORE_TEST_LOCK first and holds it for the test's whole body.
    static STORE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    // Reentrancy depth for the guard. A test that holds its own StoreGuard
    // and then calls `execute_tool` (which creates another) must NOT take
    // the lock again: std's Mutex is not reentrant, and the second take
    // deadlocked the whole test binary at 0% CPU (measured). A nested guard
    // becomes a no-op that reuses the outer guard's swap. (Plain `//`
    // because thread_local! is a macro — a doc comment there is dead text.)
    thread_local! {
        static GUARD_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
        // The test (thread) name whose dir is currently prepared. The first
        // outermost guard for a NEW name wipes the dir; later guards with the
        // SAME name reuse it unwiped — consecutive `execute_tool` calls in
        // one test read each other's files, while a rerun of the same test
        // (or a new test on a reused worker) starts clean.
        static PREPARED_FOR: std::cell::RefCell<Option<String>> =
            const { std::cell::RefCell::new(None) };
    }

    struct StoreGuard {
        lock: Option<std::sync::MutexGuard<'static, ()>>,
        dir: Option<std::path::PathBuf>,
        previous_dir: Option<std::path::PathBuf>,
        previous_store: HashMap<String, Vec<TodoItem>>,
        previous_loaded: std::collections::HashSet<String>,
    }
    impl StoreGuard {
        fn new() -> Self {
            let nested = GUARD_DEPTH.with(|d| {
                let depth = d.get();
                d.set(depth + 1);
                depth
            }) > 0;
            if nested {
                // The outer guard owns the swap; this one owns nothing and
                // must not restore anything.
                return Self {
                    lock: None,
                    dir: None,
                    previous_dir: None,
                    previous_store: HashMap::new(),
                    previous_loaded: std::collections::HashSet::new(),
                };
            }
            let lock = STORE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            // ONE dir per TEST, keyed by the thread NAME: the rust test
            // harness names each worker thread after its test, so
            // consecutive `execute_tool` calls inside ONE test resolve the
            // same dir and the second reads what the first wrote, while a
            // reused tokio worker running a DIFFERENT test (or the same
            // test in a later session) resolves a fresh one through the
            // PREPARED_FOR wipe below.
            let thread_name = std::thread::current()
                .name()
                .unwrap_or("unnamed")
                .to_string();
            let dir = {
                let thread = thread_name.replace(" > ", "-").replace(' ', "-");
                std::env::temp_dir().join(format!(
                    "operant-todo-tests-{}-{thread}",
                    std::process::id()
                ))
            };
            // Wipe only when this TEST (thread name) has not prepared the
            // dir yet this run: consecutive `execute_tool` calls in one
            // test read each other's files, while a rerun of the same test
            // or a new test on a reused worker starts clean.
            PREPARED_FOR.with(|prepared| {
                let mut prepared = prepared.borrow_mut();
                if prepared.as_deref() != Some(thread_name.as_str()) {
                    let _ = std::fs::remove_dir_all(&dir);
                    *prepared = Some(thread_name);
                }
            });
            let _ = std::fs::create_dir_all(&dir);
            let previous_dir = set_todo_dir_for_tests(Some(dir.clone()));
            let previous_store = TODO_STORE
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .drain()
                .collect();
            let previous_loaded = LOADED_SESSIONS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .drain()
                .collect();
            Self {
                lock: Some(lock),
                dir: Some(dir),
                previous_dir,
                previous_store,
                previous_loaded,
            }
        }
    }
    impl Drop for StoreGuard {
        fn drop(&mut self) {
            let depth = GUARD_DEPTH.with(|d| {
                let next = d.get().saturating_sub(1);
                d.set(next);
                next
            });
            let Some(lock) = self.lock.take() else {
                // Nested guard: it owns nothing and must restore nothing.
                return;
            };
            debug_assert_eq!(depth, 0, "outer guard dropped while a nested one lives");
            if let Ok(mut store) = TODO_STORE.lock() {
                *store = std::mem::take(&mut self.previous_store);
            }
            if let Ok(mut loaded) = LOADED_SESSIONS.lock() {
                *loaded = std::mem::take(&mut self.previous_loaded);
            }
            set_todo_dir_for_tests(self.previous_dir.take());
            // The dir is left in place: PREPARED_FOR's name check wipes it
            // if this test name runs again (new run or reused worker with a
            // different name first).
            self.dir.take();
            drop(lock);
        }
    }

    async fn execute_tool(payload: Value) -> (bool, Value, Option<String>) {
        let _store = StoreGuard::new();
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

        // (iter-1234) The injection reads the store OUTSIDE `execute_tool`, so
        // it needs its own guard: without one the dir cell has been restored
        // to the real home by the previous call's Drop and the read misses
        // the list this test just wrote.
        let _store = StoreGuard::new();
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

    #[test]
    fn schema_offers_phase_and_lane_so_a_can_phase_its_plan() {
        // The todo card has rendered group headers for as long as they have
        // existed, but the tool schema never asked the model for a group name,
        // so the headers could not be populated. The schema now models the
        // grouping with the words a plan is written in: `phase` (which batch)
        // and `lane` (which parallel strand).
        let schema = serde_json::to_value(schemars::schema_for!(TodoArgs))
            .expect("todo args schema serializes");
        let item = &schema["$defs"]["TodoItem"];
        let item_props = &item["properties"];
        assert!(item_props.get("phase").is_some(), "{item_props}");
        assert!(item_props.get("lane").is_some(), "{item_props}");
        // Required stays minimal: a phase-stamped item still needs only its
        // id, content and status, so an unphased list keeps working.
        let required = item["required"].as_array().expect("required is a list");
        for key in required {
            assert_ne!(key.as_str(), Some("phase"), "{required:?}");
            assert_ne!(key.as_str(), Some("lane"), "{required:?}");
        }
    }

    #[test]
    fn schema_offers_the_fields_the_tui_already_renders() {
        // iter-1201: a field diff against upstream found the tool's TodoItem
        // missing eight of upstream's twelve fields. The TUI already rendered
        // `blocked_by` (as a blocked marker) and carried `assigned_to`, so the
        // two halves disagreed and the model had no way to set either. This
        // pins the schema so the fields stay askable, not just readable.
        //
        // Scope, deliberately: `confidence`, `completion_confidence` and
        // `confidence_history` are NOT asserted here. Upstream maintains those
        // from tool-side bookkeeping ("maintained by the todo tool, not the
        // model"), so the port is not obliged to expose them as askable, and
        // the TUI already defaults them. Adding the model-facing fields is the
        // parity gap this closes.
        let schema = serde_json::to_value(schemars::schema_for!(TodoArgs))
            .expect("todo args schema serializes");
        let item_props = &schema["$defs"]["TodoItem"]["properties"];
        // The schema spells these the way the model must WRITE them, which is
        // the tool's camelCase serde rename (`blockedBy` / `assignedTo`) — the
        // round-trip test above feeds exactly those names.
        for key in ["priority", "details", "blockedBy", "assignedTo"] {
            assert!(
                item_props.get(key).is_some(),
                "{key} is in the upstream model contract and must stay askable"
            );
        }
    }

    #[tokio::test]
    async fn writes_and_reads_back_the_metadata_fields_of_the_plan() {
        // The fields the tool used to drop. The TUI parses `blockedBy` /
        // `assignedTo` off this very result, so a round trip that loses them
        // renders a card with no blocked marker and no owner.
        let session = "metadata-roundtrip";
        let (ok, parsed, _) = execute_tool(serde_json::json!({
            "sessionId": session,
            "todos": [
                {
                    "id": "a",
                    "content": "Prep",
                    "status": "completed",
                    "priority": "high",
                    "blockedBy": ["gate"],
                    "assignedTo": "operator",
                    "details": "check ./scripts/check.sh first"
                },
                {"id": "gate", "content": "Gate", "status": "pending"}
            ]
        }))
        .await;
        assert!(ok);
        let items = parsed["todos"].as_array().expect("todos array");
        let first = &items[0];
        assert_eq!(first["priority"], json!("high"));
        assert_eq!(
            first["blocked_by"],
            json!(["gate"]),
            "the dependency must reach the TUI's summary or it renders unblocked"
        );
        assert_eq!(first["assigned_to"], json!("operator"));
        assert_eq!(first["details"], json!("check ./scripts/check.sh first"));
    }

    #[tokio::test]
    async fn writes_and_reads_back_the_phase_and_lane_of_the_plan() {
        let session = "phase-lane-roundtrip";
        let (ok, parsed, _) = execute_tool(serde_json::json!({
            "sessionId": session,
            "todos": [
                {"id": "a", "content": "Prep", "status": "completed", "phase": 1, "lane": "one"},
                {"id": "b", "content": "Build", "status": "in_progress", "phase": 2, "lane": "one"},
                {"id": "c", "content": "Shade", "status": "pending", "phase": 2, "lane": "two"}
            ]
        }))
        .await;
        assert!(ok, "{parsed}");
        let todos = parsed["todos"].as_array().expect("todos array");
        assert_eq!(todos[0]["phase"], serde_json::json!(1));
        assert_eq!(todos[0]["lane"], serde_json::json!("one"));
        assert_eq!(todos[2]["phase"], serde_json::json!(2));
        assert_eq!(todos[2]["lane"], serde_json::json!("two"));

        // A merge that names a new phase rewrites the phase, and one that says
        // nothing about it keeps the row in its lane.
        let (ok, parsed, _) = execute_tool(serde_json::json!({
            "sessionId": session,
            "merge": true,
            "todos": [
                {"id": "a", "content": "Prep done", "status": "completed", "phase": 3},
                {"id": "b", "content": "Build nom", "status": "pending"}
            ]
        }))
        .await;
        assert!(ok, "{parsed}");
        let todos = parsed["todos"].as_array().expect("todos array");
        assert_eq!(todos[0]["phase"], serde_json::json!(3), "{todos:?}");
        assert_eq!(todos[0]["lane"], serde_json::json!("one"), "{todos:?}");
        assert_eq!(todos[1]["phase"], serde_json::json!(2), "{todos:?}");
    }

    // ------------------------------------------------------------------
    // iter-1234: file-backed persistence
    // ------------------------------------------------------------------
    //
    // The defect this block pins: TODO_STORE used to be process-local, so a
    // TUI restart lost the session's list and the footer honestly reported
    // "0/0 done, 0 in progress" for a list that had existed the night
    // before. jcode persists per-session todos as JSON (jcode-base/src/
    // todo.rs); the port now does the same.

    /// A fresh process (empty memory + empty loaded-cache) must rehydrate a
    /// session's list from disk on first read. This is the restart scenario.
    #[tokio::test]
    async fn a_fresh_process_rehydrates_the_list_from_disk() {
        let _store = StoreGuard::new();
        let session = "persist-restart";
        // "Yesterday": the write path persists.
        let (ok, parsed, _) = execute_tool(serde_json::json!({
            "todos": [
                { "id": "1", "content": "Fix scroll", "status": "completed" },
                { "id": "2", "content": "Fix todo counter", "status": "in_progress" },
            ],
            "sessionId": session
        }))
        .await;
        assert!(ok);
        assert_eq!(parsed["count"], 2);

        // "Today": a fresh process — the memory store and the loaded-cache
        // are wiped, exactly what a restart leaves behind.
        TODO_STORE.lock().unwrap_or_else(|e| e.into_inner()).clear();
        LOADED_SESSIONS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();

        let (ok, parsed, _) = execute_tool(serde_json::json!({
            "sessionId": session
        }))
        .await;
        assert!(ok);
        assert_eq!(parsed["count"], 2, "the list must survive the process");
        assert_eq!(parsed["todos"][0]["content"], "Fix scroll");
        assert_eq!(parsed["todos"][1]["status"], "in_progress");
    }

    /// The file must exist at the hashed path under the todos dir, and the
    /// session id must never appear in the FILENAME (model-supplied ids are
    /// untrusted path material).
    #[tokio::test]
    async fn the_list_persists_to_a_hashed_path_without_the_raw_id() {
        let _store = StoreGuard::new();
        let session = "persist-path/../../escape-attempt";
        execute_tool(serde_json::json!({
            "todos": [ { "id": "1", "content": "stay contained", "status": "pending" } ],
            "sessionId": session
        }))
        .await;

        let path = todo_state_path(session);
        let dir = path.parent().unwrap();
        let listed: Vec<_> = std::fs::read_dir(dir).unwrap().collect();
        assert_eq!(listed.len(), 1, "exactly one file per session");
        let name = listed[0]
            .as_ref()
            .expect("read_dir yielded an error")
            .file_name()
            .to_string_lossy()
            .to_string();
        assert!(name.ends_with(".json"), "hash-named json file: {name}");
        assert!(
            !name.contains("escape-attempt") && !name.contains(".."),
            "the raw model-supplied id must not steer the path: {name}"
        );
        // The session's list is traceable through the CONTENT, not the name.
        let bytes = std::fs::read(&path).unwrap();
        let items: Vec<TodoItem> = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].content, "stay contained");
    }

    /// A merge in a fresh process must merge INTO the persisted list, not
    /// into an empty memory copy that then overwrites the file.
    #[tokio::test]
    async fn a_merge_in_a_fresh_process_merges_into_the_persisted_list() {
        let _store = StoreGuard::new();
        let session = "persist-merge";
        execute_tool(serde_json::json!({
            "todos": [
                { "id": "1", "content": "Original", "status": "pending" },
                { "id": "2", "content": "Second", "status": "pending" },
            ],
            "sessionId": session
        }))
        .await;

        // Fresh process.
        TODO_STORE.lock().unwrap_or_else(|e| e.into_inner()).clear();
        LOADED_SESSIONS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();

        let (ok, parsed, _) = execute_tool(serde_json::json!({
            "todos": [ { "id": "1", "content": "Updated", "status": "completed" } ],
            "merge": true,
            "sessionId": session
        }))
        .await;
        assert!(ok);
        assert_eq!(parsed["count"], 2, "merge keeps the off-disk sibling");
        assert_eq!(parsed["todos"][0]["content"], "Updated");
        assert_eq!(parsed["todos"][0]["status"], "completed");
        assert_eq!(parsed["todos"][1]["content"], "Second");
    }

    /// Clearing the list persists the clear: an empty write must remove the
    /// session's file, not leave yesterday's list to resurrect tomorrow.
    #[tokio::test]
    async fn an_empty_write_removes_the_persisted_list() {
        let _store = StoreGuard::new();
        let session = "persist-clear";
        execute_tool(serde_json::json!({
            "todos": [ { "id": "1", "content": "temporal", "status": "pending" } ],
            "sessionId": session
        }))
        .await;
        let path = todo_state_path(session);
        assert!(path.exists(), "write persisted the list");

        let (ok, parsed, _) = execute_tool(serde_json::json!({
            "todos": [],
            "sessionId": session
        }))
        .await;
        assert!(ok);
        assert_eq!(parsed["count"], 0);
        assert!(
            !path.exists(),
            "an explicit clear must not resurrect on the next boot"
        );
    }

    /// todo_injection_for_session rehydrates too: the model keeps its plan
    /// across a compaction AND across a restart.
    #[tokio::test]
    async fn post_compression_injection_survives_a_restart() {
        let _store = StoreGuard::new();
        let session = "persist-injection";
        execute_tool(serde_json::json!({
            "todos": [ { "id": "1", "content": "survive", "status": "in_progress" } ],
            "sessionId": session
        }))
        .await;

        TODO_STORE.lock().unwrap_or_else(|e| e.into_inner()).clear();
        LOADED_SESSIONS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();

        let snapshot = todo_injection_for_session(session);
        assert!(
            snapshot.is_some_and(|s| s.contains("survive")),
            "the injection must read the persisted list, not the dead memory"
        );
    }
}
