//! Agent-facing context tools: `ctx_status` and `ctx_search`.
//!
//! ## Why these two, and why they are read-only
//!
//! The context an agent is carrying is invisible to it. The agent cannot see
//! how much of its window is tool output, how much the provider is charging
//! for the prompt it just sent, or what it said three turns ago that a
//! compaction may have summarized away. Every one of those is a decision the
//! agent SHOULD be making — "I am carrying 40 000 tokens of dead `grep`
//! output; I should search for the one thing I still need" — and none of them
//! is a decision it can make today.
//!
//! These two tools give it that view, and NOTHING else:
//!
//! - `ctx_status` — what is in my window right now: the provider's own token
//!   count for the last request, the engine's estimate of the assembled
//!   context, the split between tool output and conversation, the budget, and
//!   the session's totals so far.
//! - `ctx_search` — token-efficient recall of MY OWN session context, scoped
//!   to the session I am in, with a hard token budget on the evidence it
//!   returns.
//!
//! They are read-only ON PURPOSE. The write half of the family — the
//! mark/restore/pin/sweep tools, where the agent asks for context to be
//! dropped — needs a state model (a queue of marks, a sweep that applies them,
//! an undo), and every one of those is a way to corrupt a live request if it
//! is wrong. That family is a separate iteration, with the safety contract
//! written down first (see `docs/HERMES_LCM_INTEGRATION.md` and the
//! context-architecture audit). Two tools that CANNOT change anything are
//! worth shipping first: they answer the question the write family would be
//! answering, and they carry no hazard at all.
//!
//! ## The session they talk about
//!
//! The session is resolved in this order: the `session` argument, the
//! `session_id` the agent's tool context carries, then the newest session in
//! the database. The first two are exact; the last is a fallback for a host
//! that does not wire the session id (the agent's `tool_context()` puts it in
//! the metadata today).

use std::sync::Arc;

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::database::Database;
use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolResult};

/// The default token budget for `ctx_search`'s returned evidence.
pub const CTX_SEARCH_DEFAULT_BUDGET_TOKENS: usize = 1_000;

/// The most hits `ctx_search` will return.
pub const CTX_SEARCH_MAX_HITS: usize = 10;

/// The arguments of `ctx_status`.
#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CtxStatusArgs {
    /// Session id. Omit for the session you are in.
    pub session: Option<String>,
}

/// The arguments of `ctx_search`.
#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CtxSearchArgs {
    /// Free-text query. Matched as an FTS5 phrase, so quoted phrases are
    /// verbatim and bare words are ANDed.
    pub query: String,
    /// Max hits to return (1..=10, default 5).
    pub limit: Option<usize>,
    /// Token budget for the returned snippets (default 1000).
    pub budget_tokens: Option<usize>,
    /// Session id. Omit for the session you are in.
    pub session: Option<String>,
}

/// Resolve which session a context tool is talking about.
///
/// The argument wins, then the session the agent's tool context carries, then
/// the newest session in the database. The last is a FALLBACK for a host that
/// does not wire the session id; it is stated in the tool's own output so a
/// reader can tell which rule answered.
fn resolve_session(
    db: &Database,
    requested: Option<&str>,
    context: &ToolContext,
) -> Result<(String, &'static str), String> {
    if let Some(sid) = requested.map(str::trim).filter(|s| !s.is_empty()) {
        return Ok((sid.to_string(), "argument"));
    }
    // The id arrives in two shapes: the `ToolContext::session_id` field, and
    // the `session_id` metadata the agent's `tool_context()` sets. Both are
    // read, because a host wires one or the other and a tool that resolves
    // the wrong session is worse than no tool at all.
    if let Some(sid) = context
        .session_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return Ok((sid.to_string(), "tool context"));
    }
    if let Some(sid) = context.get("session_id").map(str::trim).filter(|s| !s.is_empty()) {
        return Ok((sid.to_string(), "tool context"));
    }
    // The fallback is allowed ONLY when it is unambiguous — exactly one
    // session in the database. The gateway runs agent SLOTS (many sessions
    // sharing one process), and there "the newest session" is somebody
    // else's conversation: reporting another seat's context to an agent that
    // asked for its own is the one failure this tool must not have.
    let sessions = db
        .list_sessions(2)
        .map_err(|e| format!("could not read the session list: {e}"))?;
    match sessions.len() {
        0 => Err("no sessions in the database yet".to_string()),
        1 => Ok((
            sessions[0].id.clone(),
            "the only session (no session id was wired)",
        )),
        _ => Err(
            "no session id was wired and the database holds more than one session — pass \
             one explicitly (`session`), because guessing would report another \
             conversation's context"
                .to_string(),
        ),
    }
}

/// `ctx_status` — what is in my context window right now.
pub struct CtxStatusTool {
    db: Arc<Database>,
}

impl CtxStatusTool {
    /// Build the tool over a database handle.
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }
}

#[async_trait::async_trait]
impl OperantTool for CtxStatusTool {
    fn name(&self) -> &str {
        "ctx_status"
    }

    fn description(&self) -> &str {
        "Show what your context window currently holds: the provider's own token count for the last request, the engine's estimate of the assembled context, how that context splits between tool output and conversation, the budget it is fitted to, and this session's totals. Read-only: it changes nothing. Use it when you suspect you are carrying a lot of tool output, before deciding what to keep and what to drop."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<CtxStatusArgs>("ctx_status", "Report the shape and cost of the current context")
    }

    async fn execute(&self, args: Value, context: ToolContext) -> ToolResult {
        let args: CtxStatusArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("ctx_status", format!("Invalid arguments: {e}")),
        };
        let (session_id, resolved_by) =
            match resolve_session(&self.db, args.session.as_deref(), &context) {
                Ok(v) => v,
                Err(e) => return ToolResult::error("ctx_status", e),
            };
        let usage = match self.db.session_usage(&session_id) {
            Ok(u) => u,
            Err(e) => return ToolResult::error("ctx_status", format!("could not read the session: {e}")),
        };
        let rows = match self.db.context_ledger_for_session(&session_id, 1) {
            Ok(r) => r,
            Err(e) => {
                return ToolResult::error("ctx_status", format!("could not read the ledger: {e}"));
            }
        };
        let last = rows.last();
        let (budget_tokens, window_tokens, estimated, tool_result_tokens, message_count, ladder_fired) =
            match last {
                Some(r) => (
                    r.budget_tokens,
                    r.window_tokens,
                    r.estimated_context_tokens,
                    r.tool_result_tokens,
                    r.message_count,
                    r.ladder_fired,
                ),
                None => (0, 0, 0, 0, 0, false),
            };
        let share = if estimated > 0 {
            (tool_result_tokens as f64 * 100.0 / estimated as f64).round() as i64
        } else {
            0
        };
        let gap = last
            .map(|r| r.prompt_tokens - r.estimated_context_tokens)
            .unwrap_or(0);
        // The two halves of the gap, named once, for the drift flag and the note.
        let (r_prompt, r_estimate) = last
            .map(|r| (r.prompt_tokens, r.estimated_context_tokens))
            .unwrap_or((0, 0));
        ToolResult::success(
            "ctx_status",
            json!({
                "_ctx_tool": "ctx_status",
                "session_id": session_id,
                "session_resolved_by": resolved_by,
                "last_request": {
                    "provider_prompt_tokens": last.map(|r| r.prompt_tokens).unwrap_or(0),
                    "provider_completion_tokens": last.map(|r| r.completion_tokens).unwrap_or(0),
                    "engine_estimate_tokens": estimated,
                    "estimate_gap_tokens": gap,
                    "message_count": message_count,
                    "tool_result_tokens": tool_result_tokens,
                    "tool_result_share_percent": share,
                    "context_was_compacted": ladder_fired,
                },
                "budget_tokens": budget_tokens,
                "window_tokens": window_tokens,
                "session_totals": {
                    "input_tokens": usage.input_tokens,
                    "output_tokens": usage.output_tokens,
                    "requests": usage.api_call_count,
                    "messages": usage.message_count,
                    "tool_calls": usage.tool_call_count,
                },
                "note": "provider_prompt_tokens is what the provider charged for the last request; \
                         engine_estimate_tokens is the engine's own estimate of the assembled \
                         context. The gap is the part of the bill the estimate does not model \
                         (the system prompt and the tool schemas). tool_result_share_percent is \
                         how much of the context is tool output.",
                "estimate_drifts": gap > 0 && r_prompt > 0 && r_prompt > r_estimate * 2,
            }),
        )
    }
}

/// `ctx_search` — token-efficient recall of my own session context.
pub struct CtxSearchTool {
    db: Arc<Database>,
}

impl CtxSearchTool {
    /// Build the tool over a database handle.
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }
}

#[async_trait::async_trait]
impl OperantTool for CtxSearchTool {
    fn name(&self) -> &str {
        "ctx_search"
    }

    fn description(&self) -> &str {
        "Search THIS session's own context for earlier content — your own words, tool output, and anything a compaction may have summarized away. Returns the matching messages with their ids, under a hard token budget so the evidence you get back is cheap. Read-only: it changes nothing. Use it instead of re-running a command whose output you may already have."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<CtxSearchArgs>("ctx_search", "Recall earlier content from this session's context")
    }

    async fn execute(&self, args: Value, context: ToolContext) -> ToolResult {
        let args: CtxSearchArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("ctx_search", format!("Invalid arguments: {e}")),
        };
        if args.query.trim().is_empty() {
            return ToolResult::error("ctx_search", "query is required");
        }
        let limit = args.limit.unwrap_or(5).clamp(1, CTX_SEARCH_MAX_HITS);
        let budget = args
            .budget_tokens
            .unwrap_or(CTX_SEARCH_DEFAULT_BUDGET_TOKENS)
            .clamp(100, 8_000);
        let (session_id, resolved_by) =
            match resolve_session(&self.db, args.session.as_deref(), &context) {
                Ok(v) => v,
                Err(e) => return ToolResult::error("ctx_search", e),
            };
        let hits = match self
            .db
            .search_messages_fts(&args.query, Some(&session_id), Some(limit as u32))
        {
            Ok(h) => h,
            Err(e) => return ToolResult::error("ctx_search", format!("search failed: {e}")),
        };
        if hits.is_empty() {
            return ToolResult::success(
                "ctx_search",
                json!({
                    "_ctx_tool": "ctx_search",
                    "session_id": session_id,
                    "hits": [],
                    "note": "no match in this session's context",
                }),
            );
        }
        // The evidence is budgeted: each hit contributes a snippet that fits
        // what is left of the budget, and a hit that does not fit is reported
        // as skipped rather than silently truncated to nothing. The ids are
        // always returned — an id costs almost nothing and it is what makes a
        // hit actionable (`ctx_search` again with a narrower query).
        let mut remaining = budget;
        let mut out = Vec::new();
        let mut skipped = 0usize;
        for hit in &hits {
            let content = hit.content.trim();
            let cost = crate::context_management::estimate_tokens(content);
            if cost > remaining {
                skipped += 1;
                continue;
            }
            remaining -= cost;
            out.push(json!({
                "message_id": hit.message_id,
                "role": hit.role,
                "timestamp": hit.created_at,
                "tokens": cost,
                "content": content,
            }));
        }
        ToolResult::success(
            "ctx_search",
            json!({
                "_ctx_tool": "ctx_search",
                "session_id": session_id,
                "session_resolved_by": resolved_by,
                "hits": out,
                "hits_skipped_for_budget": skipped,
                "budget_tokens": budget,
                "tokens_returned": budget - remaining,
                "note": "hits are verbatim from this session's context; each carries the \
                         message id, so a narrower query can find it again.",
            }),
        )
    }
}

/// Register the read-only context tools.
///
/// Unlike the LCM family, these are NOT gated on `agent.context_engine`: they
/// read the session store and the context ledger, which exist under every
/// engine. The agent's ability to see its own context should not depend on
/// which engine is assembling it.
pub async fn register_ctx_tools(
    registry: &crate::tools::ToolRegistry,
    db: Arc<Database>,
) -> crate::error::Result<()> {
    registry.register(CtxStatusTool::new(db.clone())).await?;
    registry.register(CtxSearchTool::new(db)).await?;
    Ok(())
}


// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::ContextRequestRecord;
    use crate::tools::ToolRegistry;

    fn temp_db() -> (tempfile::TempDir, Database) {
        let temp = tempfile::tempdir().expect("a temp dir");
        let db = Database::init(temp.path().join("ctx.sqlite")).expect("init the database");
        (temp, db)
    }

    /// A session with two messages and one metered request.
    fn seed(db: &Database, sid: &str) {
        let now = "2026-11-12T00:00:00Z";
        db.save_session(sid, None, "agent", now, now).unwrap();
        db.save_message(sid, "user", "how does the compaction ladder work", now).unwrap();
        db.save_message(sid, "assistant", "it evicts the oldest tool results first", now).unwrap();
        db.add_session_usage(sid, 29_067, 5, 1).unwrap();
        db.record_context_request(&ContextRequestRecord {
            session_id: sid.to_string(),
            request_seq: 1,
            created_at: now.to_string(),
            prompt_tokens: 29_067,
            completion_tokens: 5,
            total_tokens: 29_072,
            estimated_context_tokens: 10_162,
            message_count: 2,
            tool_result_tokens: 6_000,
            user_tokens: 327,
            assistant_tokens: 200,
            tool_call_count: 1,
            budget_tokens: 995_623,
            window_tokens: 1_000_000,
            ladder_fired: false,
            note: None,
        })
        .unwrap();
    }

    #[tokio::test]
    async fn ctx_status_reports_the_last_request() {
        let (_tmp, db) = temp_db();
        seed(&db, "s-ctx");
        let tool = CtxStatusTool::new(Arc::new(db));
        let result = tool
            .execute(json!({"session": "s-ctx"}), ToolContext::default())
            .await;
        assert!(result.success, "ctx_status must succeed: {}", result.content);
        let v: serde_json::Value = serde_json::from_str(&result.content).expect("json");
        assert_eq!(v["session_id"], "s-ctx");
        assert_eq!(v["session_resolved_by"], "argument");
        assert_eq!(v["last_request"]["provider_prompt_tokens"], 29_067);
        assert_eq!(v["last_request"]["engine_estimate_tokens"], 10_162);
        assert_eq!(v["last_request"]["estimate_gap_tokens"], 29_067 - 10_162);
        assert_eq!(v["last_request"]["tool_result_tokens"], 6_000);
        // 6 000 of 10 162 is ~59 %.
        assert_eq!(v["last_request"]["tool_result_share_percent"], 59);
        assert_eq!(v["session_totals"]["requests"], 1);
        assert_eq!(v["window_tokens"], 1_000_000);
    }

    #[tokio::test]
    async fn ctx_status_uses_the_session_from_the_tool_context() {
        let (_tmp, db) = temp_db();
        seed(&db, "s-one");
        seed(&db, "s-two");
        let tool = CtxStatusTool::new(Arc::new(db));
        let result = tool
            .execute(json!({}), ToolContext::default().with_session_id("s-two"))
            .await;
        let v: serde_json::Value = serde_json::from_str(&result.content).expect("json");
        assert_eq!(v["session_id"], "s-two", "the tool context decides");
        assert_eq!(v["session_resolved_by"], "tool context");
    }

    #[tokio::test]
    async fn ctx_status_without_a_request_reports_zeros_rather_than_failing() {
        let (_tmp, db) = temp_db();
        let now = "2026-11-12T00:00:00Z";
        db.save_session("s-empty", None, "agent", now, now).unwrap();
        let tool = CtxStatusTool::new(Arc::new(db));
        let result = tool
            .execute(json!({"session": "s-empty"}), ToolContext::default())
            .await;
        assert!(result.success);
        let v: serde_json::Value = serde_json::from_str(&result.content).expect("json");
        assert_eq!(v["last_request"]["provider_prompt_tokens"], 0);
        assert_eq!(v["last_request"]["tool_result_share_percent"], 0,
                   "no request means no share, and no divide-by-zero");
    }

    #[tokio::test]
    async fn ctx_status_refuses_to_guess_when_several_sessions_exist() {
        // RT-C01: the gateway runs agent SLOTS, so "the newest session" is
        // another seat's conversation. With no session id wired and more than
        // one session present, the tool must REFUSE rather than report
        // somebody else's context.
        let (_tmp, db) = temp_db();
        seed(&db, "s-one");
        seed(&db, "s-two");
        let tool = CtxStatusTool::new(Arc::new(db));
        let result = tool.execute(json!({}), ToolContext::default()).await;
        assert!(!result.success, "the tool must not guess");
        // `ToolResult::error` carries the message in the error field.
        let message = result.error.as_deref().unwrap_or_default();
        assert!(
            message.contains("more than one session"),
            "the refusal says why: {message}"
        );
    }

    #[tokio::test]
    async fn ctx_status_flags_the_estimator_drift() {
        // RT-C05: the provider charged 29 067 for a context the engine
        // estimated at 10 162 — the estimate does not model the frozen prefix
        // or the tool schemas. A ratio above two is the signal, and it is
        // reported rather than left in a column.
        let (_tmp, db) = temp_db();
        seed(&db, "s-ctx");
        let tool = CtxStatusTool::new(Arc::new(db));
        let result = tool
            .execute(json!({"session": "s-ctx"}), ToolContext::default())
            .await;
        let v: serde_json::Value = serde_json::from_str(&result.content).expect("json");
        assert_eq!(
            v["estimate_drifts"], true,
            "a 3x gap between the provider's count and the estimate is drift"
        );
    }

    #[tokio::test]
    async fn ctx_search_finds_earlier_content_in_the_session() {
        let (_tmp, db) = temp_db();
        seed(&db, "s-ctx");
        let tool = CtxSearchTool::new(Arc::new(db));
        let result = tool
            .execute(json!({"query": "compaction"}), ToolContext::default().with_session_id("s-ctx"))
            .await;
        assert!(result.success);
        let v: serde_json::Value = serde_json::from_str(&result.content).expect("json");
        let hits = v["hits"].as_array().expect("hits array");
        assert!(!hits.is_empty(), "the phrase is in the seeded user message");
        assert_eq!(hits[0]["role"], "user");
        assert!(
            v["tokens_returned"].as_i64().expect("tokens") <= v["budget_tokens"].as_i64().expect("budget"),
            "the evidence respects its budget"
        );
    }

    #[tokio::test]
    async fn ctx_search_skips_a_hit_that_does_not_fit_the_budget() {
        let (_tmp, db) = temp_db();
        let now = "2026-11-12T00:00:00Z";
        db.save_session("s-big", None, "agent", now, now).unwrap();
        db.save_message("s-big", "tool", &"verdict ".repeat(20_000), now).unwrap();
        let tool = CtxSearchTool::new(Arc::new(db));
        let result = tool
            .execute(
                json!({"query": "verdict", "budget_tokens": 500, "session": "s-big"}),
                ToolContext::default(),
            )
            .await;
        assert!(result.success);
        let v: serde_json::Value = serde_json::from_str(&result.content).expect("json");
        assert_eq!(
            v["hits_skipped_for_budget"].as_u64(),
            Some(1),
            "a hit that cannot fit is reported as skipped, not truncated to nothing"
        );
        assert_eq!(v["hits"].as_array().map(Vec::len).unwrap_or(0), 0);
    }

    #[tokio::test]
    async fn ctx_search_is_scoped_to_the_session_it_is_asked_about() {
        let (_tmp, db) = temp_db();
        let now = "2026-11-12T00:00:00Z";
        db.save_session("s-a", None, "agent", now, now).unwrap();
        db.save_session("s-b", None, "agent", now, now).unwrap();
        db.save_message("s-a", "user", "the alpha token appears here", now).unwrap();
        db.save_message("s-b", "user", "the alpha token appears there", now).unwrap();
        let tool = CtxSearchTool::new(Arc::new(db));
        let result = tool
            .execute(json!({"query": "alpha", "session": "s-a"}), ToolContext::default())
            .await;
        let v: serde_json::Value = serde_json::from_str(&result.content).expect("json");
        let hits = v["hits"].as_array().expect("hits");
        assert_eq!(hits.len(), 1, "only the asked-about session is searched");
    }

    #[tokio::test]
    async fn the_context_tools_are_registered_for_every_engine() {
        let (_tmp, db) = temp_db();
        let registry = ToolRegistry::new(std::time::Duration::from_secs(5));
        register_ctx_tools(&registry, Arc::new(db))
            .await
            .expect("register the context tools");
        assert!(registry.contains("ctx_status").await);
        assert!(registry.contains("ctx_search").await);
        // Both are read-only by construction: they say so, and neither can
        // change anything they read.
        let status = registry.get("ctx_status").await.expect("the status tool");
        assert!(status.description().contains("Read-only"));
        let search = registry.get("ctx_search").await.expect("the search tool");
        assert!(search.description().contains("Read-only"));
    }
}
