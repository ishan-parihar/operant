//! LCM (Lossless Context Management) agent tools.
//!
//! When `agent.context_engine = "lcm"`, the DAG engine is attached to the
//! agent AND its two tools are registered so the model can use the lossless
//! store directly:
//!   - `lcm_recall` — FTS5 phrase search over the DAG (session-scoped by
//!     default, global when `session` is omitted). Returns verbatim nodes
//!     so nothing ever lost to compaction is truly gone.
//!   - `lcm_stats` — diagnostics: engine name, DAG path, node count, and
//!     the D0 tail token budget.
//!
//! hermes-lcm parity: recall tools are the "memory tool" half of the
//! ContextEngine plugin (see `docs/HERMES_LCM_INTEGRATION.md` P2).

use std::sync::Arc;

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::context::{ContextEngine, LcmContextEngine};
use crate::schema::ToolSchema;
use crate::tools::{OperantTool, ToolContext, ToolResult};

/// FTS recall over the lossless DAG.
#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LcmRecallArgs {
    /// Free-text query; matched as an FTS5 phrase (best for verbatim hits).
    pub query: String,
    /// Max hits to return (1..=50, default 5).
    pub limit: Option<usize>,
    /// Scope recall to one session id. Omit for global recall across all
    /// sessions in the DAG.
    pub session: Option<String>,
}

pub struct LcmRecallTool {
    engine: Arc<LcmContextEngine>,
}

impl LcmRecallTool {
    pub fn new(engine: Arc<LcmContextEngine>) -> Self {
        Self { engine }
    }
}

#[async_trait::async_trait]
impl OperantTool for LcmRecallTool {
    fn name(&self) -> &str {
        "lcm_recall"
    }

    fn description(&self) -> &str {
        "Search the lossless context DAG for earlier conversation content that was compacted out of the active window. Returns verbatim messages ranked by relevance. Use when you need details from earlier in this or a previous session."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<LcmRecallArgs>("lcm_recall", "Recall compacted context verbatim")
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: LcmRecallArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::error("lcm_recall", format!("Invalid arguments: {e}"));
            }
        };
        if args.query.trim().is_empty() {
            return ToolResult::error("lcm_recall", "query is required");
        }
        let limit = args.limit.unwrap_or(5).clamp(1, 50);
        let session = args.session.as_deref();
        match self.engine.recall(session, &args.query, limit).await {
            Ok(hits) => {
                if hits.is_empty() {
                    return ToolResult::success(
                        "lcm_recall",
                        json!({
                            "_lcm_tool": "lcm_recall",
                            "hits": [],
                            "note": "no matching nodes in the DAG",
                        }),
                    );
                }
                // Render at most MAX_RENDERED_HITS so the whole result stays
                // well under the agent's tool-result truncation cap (4096
                // bytes) even when the caller requests a large limit.
                // Full-length content previously overflowed the cap and hid
                // the later (often most relevant) hits from the model.
                const MAX_RENDERED_HITS: usize = 5;
                let rendered = hits.iter().take(MAX_RENDERED_HITS);
                let nodes: Vec<Value> = rendered
                    .map(|h| {
                        let mut content = h.content.clone();
                        if content.chars().count() > 600 {
                            content = content.chars().take(600).collect::<String>();
                            content.push_str("...[truncated]");
                        }
                        json!({
                            "node_id": h.node_id,
                            "role": h.role,
                            "content": content,
                            "created_at": h.created_at,
                            "score": h.score,
                        })
                    })
                    .collect();
                ToolResult::success(
                    "lcm_recall",
                    json!({
                        "_lcm_tool": "lcm_recall",
                        "hits": nodes,
                        "total_hits": hits.len(),
                    }),
                )
            }
            Err(e) => ToolResult::error("lcm_recall", format!("recall failed: {e}")),
        }
    }
}

/// Diagnostics for the lossless context engine.
#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LcmStatsArgs {}

pub struct LcmStatsTool {
    engine: Arc<LcmContextEngine>,
}

impl LcmStatsTool {
    pub fn new(engine: Arc<LcmContextEngine>) -> Self {
        Self { engine }
    }
}

#[async_trait::async_trait]
impl OperantTool for LcmStatsTool {
    fn name(&self) -> &str {
        "lcm_stats"
    }

    fn description(&self) -> &str {
        "Report the lossless context engine status: DAG database path, node count, and the fresh-tail (D0) token budget. Use to confirm the context engine is active and to gauge stored history size."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<LcmStatsArgs>("lcm_stats", "LCM engine status")
    }

    async fn execute(&self, _args: Value, _context: ToolContext) -> ToolResult {
        let db_path = self.engine.db_path().display().to_string();
        let nodes = self.engine.node_count_global().unwrap_or(0);
        ToolResult::success(
            "lcm_stats",
            json!({
                "_lcm_tool": "lcm_stats",
                "engine": "lcm",
                "db_path": db_path,
                "dag_nodes": nodes,
                "tail_tokens": self.engine.tail_tokens(),
            }),
        )
    }
}

/// Store or query durable assertions (hermes `assertion_store.py` parity,
/// lightweight). `action = "save"` records a fact; `action = "query"`
/// returns stored facts plus the resolved active state (latest per unique
/// object) and any contradictions (distinct active objects for a predicate).
#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LcmAssertArgs {
    /// `save` (record a fact) or `query` (retrieve facts).
    pub action: String,
    /// Subject key of the fact, e.g. `project:hermes`, `user`, `assistant:self`.
    pub subject: String,
    /// Predicate key, e.g. `prefers`, `uses`, `deadline`. Optional on query.
    pub predicate: Option<String>,
    /// Object value — the fact itself. Required when action = `save`.
    pub object: Option<String>,
    /// Scope to a session id; omitted = global fact scope shared across sessions.
    pub session: Option<String>,
    /// Speaker role recorded with a saved fact (`user`|`assistant`|`tool`).
    pub speaker: Option<String>,
}

pub struct LcmAssertTool {
    engine: Arc<LcmContextEngine>,
}

impl LcmAssertTool {
    pub fn new(engine: Arc<LcmContextEngine>) -> Self {
        Self { engine }
    }
}

#[async_trait::async_trait]
impl OperantTool for LcmAssertTool {
    fn name(&self) -> &str {
        "lcm_assert"
    }

    fn description(&self) -> &str {
        "Store or query durable assertions (facts) in the lossless context store. Save a fact with action=save to persist it across sessions; query with action=query to retrieve stored facts for a subject, including which facts are currently active and any contradictions. Use to remember durable user preferences, project decisions, or constraints."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<LcmAssertArgs>("lcm_assert", "Store or query durable facts")
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: LcmAssertArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::error("lcm_assert", format!("Invalid arguments: {e}"));
            }
        };
        if args.subject.trim().is_empty() {
            return ToolResult::error("lcm_assert", "subject is required");
        }
        let scope = args.session.unwrap_or_else(|| "global".to_string());
        match args.action.trim().to_lowercase().as_str() {
            "save" => {
                let predicate = args.predicate.as_deref().unwrap_or("states").trim();
                let object = args.object.as_deref().unwrap_or("").trim();
                if object.is_empty() {
                    return ToolResult::error(
                        "lcm_assert",
                        "object is required when action = save",
                    );
                }
                let speaker = args.speaker.as_deref().unwrap_or("assistant");
                match self.engine.assert_assertion(
                    &scope,
                    &args.subject,
                    predicate,
                    object,
                    speaker,
                    None,
                ) {
                    Ok(id) => ToolResult::success(
                        "lcm_assert",
                        json!({
                            "_lcm_tool": "lcm_assert",
                            "action": "save",
                            "saved": true,
                            "assertion_id": id,
                            "subject": args.subject,
                            "predicate": predicate,
                            "object": object,
                        }),
                    ),
                    Err(e) => ToolResult::error("lcm_assert", format!("save failed: {e}")),
                }
            }
            "query" => match self.engine.query_assertion_state(
                &scope,
                &args.subject,
                args.predicate.as_deref(),
            ) {
                Ok(records) => {
                    // Active = latest record per (predicate, object_value);
                    // contradictions = predicates with >1 distinct active object.
                    let mut active: Vec<&crate::context::AssertionRecord> = Vec::new();
                    let mut seen: std::collections::HashMap<(String, String), bool> =
                        std::collections::HashMap::new();
                    for r in &records {
                        seen.entry((r.predicate.clone(), r.object_value.clone()))
                            .or_insert(true);
                    }
                    for r in &records {
                        let key = (r.predicate.clone(), r.object_value.clone());
                        if seen.get(&key) == Some(&true) {
                            seen.insert(key, false);
                            active.push(r);
                        }
                    }
                    let mut contradictions: Vec<String> = Vec::new();
                    let mut objects_by_pred: std::collections::HashMap<&str, usize> =
                        std::collections::HashMap::new();
                    for r in &active {
                        *objects_by_pred.entry(r.predicate.as_str()).or_insert(0) += 1;
                    }
                    for (p, n) in objects_by_pred {
                        if n > 1 {
                            contradictions.push(p.to_string());
                        }
                    }
                    let rendered: Vec<Value> = records
                        .iter()
                        .map(|r| {
                            json!({
                                "id": r.id,
                                "subject": r.subject,
                                "predicate": r.predicate,
                                "object": r.object_value,
                                "speaker": r.speaker_role,
                                "created_at": r.created_at,
                            })
                        })
                        .collect();
                    let active_rendered: Vec<Value> = active
                        .iter()
                        .map(|r| {
                            json!({
                                "subject": r.subject,
                                "predicate": r.predicate,
                                "object": r.object_value,
                            })
                        })
                        .collect();
                    ToolResult::success(
                        "lcm_assert",
                        json!({
                            "_lcm_tool": "lcm_assert",
                            "action": "query",
                            "scope": scope,
                            "subject": args.subject,
                            "assertions": rendered,
                            "active": active_rendered,
                            "contradictions": contradictions,
                            "total": records.len(),
                        }),
                    )
                }
                Err(e) => ToolResult::error("lcm_assert", format!("query failed: {e}")),
            },
            other => ToolResult::error(
                "lcm_assert",
                format!("unknown action `{other}` — use save or query"),
            ),
        }
    }
}

/// Multi-round evidence-gated recall (hermes `adaptive_retrieval.py` parity,
/// lightweight). Omit `retrievalId` to start a new retrieval; pass the
/// returned id (optionally refining `query`) to continue until `complete`.
#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LcmRecallRoundArgs {
    /// Omit to start a new retrieval; pass the returned id to continue.
    pub retrieval_id: Option<String>,
    /// Search query. Required on start; optional refinement on continue.
    pub query: Option<String>,
    /// Distinct verbatim evidences required (1..=5, default 2).
    pub evidence_required: Option<usize>,
    /// Scope to a session id; omitted = global DAG.
    pub session: Option<String>,
}

pub struct LcmRecallRoundTool {
    engine: Arc<LcmContextEngine>,
    registry: crate::context::AdaptiveRetrievalRegistry,
}

impl LcmRecallRoundTool {
    pub fn new(engine: Arc<LcmContextEngine>) -> Self {
        Self {
            engine,
            registry: crate::context::AdaptiveRetrievalRegistry::new(),
        }
    }
}

#[async_trait::async_trait]
impl OperantTool for LcmRecallRoundTool {
    fn name(&self) -> &str {
        "lcm_recall_round"
    }

    fn description(&self) -> &str {
        "Multi-round evidence-gated recall over the lossless DAG. Start with a query (omit retrievalId); each round returns exact verbatim evidence plus search leads and a `complete` flag. If not complete, call again with the returned retrievalId, optionally refining the query, until you have gathered the required evidence. Use when one-shot recall did not surface enough proof."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<LcmRecallRoundArgs>(
            "lcm_recall_round",
            "Multi-round evidence-gated recall",
        )
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: LcmRecallRoundArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::error("lcm_recall_round", format!("Invalid arguments: {e}"));
            }
        };
        let session = args.session.as_deref();
        match &args.retrieval_id {
            None => {
                let query = args.query.as_deref().unwrap_or("").trim();
                if query.is_empty() {
                    return ToolResult::error(
                        "lcm_recall_round",
                        "query is required when starting a retrieval",
                    );
                }
                let req = args.evidence_required.unwrap_or(2);
                match self.registry.start(&self.engine, session, query, req).await {
                    Ok(round) => {
                        let mut payload = round_json(round);
                        if let Some(obj) = payload.as_object_mut() {
                            obj.insert("_lcm_tool".into(), json!("lcm_recall_round"));
                        }
                        ToolResult::success("lcm_recall_round", payload)
                    }
                    Err(e) => ToolResult::error("lcm_recall_round", format!("round failed: {e}")),
                }
            }
            Some(id) => match self
                .registry
                .next_round(&self.engine, session, id, args.query.as_deref())
                .await
            {
                Ok(Some(round)) => {
                    let mut payload = round_json(round);
                    if let Some(obj) = payload.as_object_mut() {
                        obj.insert("_lcm_tool".into(), json!("lcm_recall_round"));
                    }
                    ToolResult::success("lcm_recall_round", payload)
                }
                Ok(None) => ToolResult::error(
                    "lcm_recall_round",
                    format!(
                        "unknown or expired retrieval_id `{id}` — start a new retrieval by omitting retrievalId"
                    ),
                ),
                Err(e) => ToolResult::error("lcm_recall_round", format!("round failed: {e}")),
            },
        }
    }
}

/// Serialize a retrieval round into the JSON payload returned to the model.
fn round_json(round: crate::context::RetrievalRound) -> Value {
    json!({
        "retrieval_id": round.retrieval_id,
        "round_number": round.round_number,
        "evidence_required": round.evidence_required,
        "complete": round.complete,
        "evidence": round.evidence_found.iter().map(|e| json!({
            "node_id": e.node_id,
            "role": e.role,
            "snippet": e.snippet,
            "score": e.score,
        })).collect::<Vec<_>>(),
        "leads": round.leads.iter().map(|l| json!({
            "node_id": l.node_id,
            "role": l.role,
            "snippet": l.snippet,
            "score": l.score,
        })).collect::<Vec<_>>(),
    })
}
/// Vector recall over the DAG (hermes `vector_store.py` parity, bounded).
/// Registered only when an embedding model is configured
/// (`agent.context_lcm_embedding_model`) — the tool surface stays clean when
/// the provider cannot embed.
#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LcmVectorRecallArgs {
    /// Query to embed and match by semantic similarity.
    pub query: String,
    /// Max hits to return (1..=20, default 5).
    pub limit: Option<usize>,
    /// Scope to a session id; omitted = semantic search across the whole DAG.
    pub session: Option<String>,
}

pub struct LcmVectorRecallTool {
    engine: Arc<LcmContextEngine>,
    embedder: Arc<dyn crate::context::Embedder>,
}

impl LcmVectorRecallTool {
    pub fn new(engine: Arc<LcmContextEngine>, embedder: Arc<dyn crate::context::Embedder>) -> Self {
        Self { engine, embedder }
    }
}

#[async_trait::async_trait]
impl OperantTool for LcmVectorRecallTool {
    fn name(&self) -> &str {
        "lcm_vector_recall"
    }

    fn description(&self) -> &str {
        "Semantic recall over the lossless DAG: embed the query and return the most similar verbatim nodes by cosine similarity, even when they share no exact words with the query. Complements lcm_recall (exact FTS). Use when reworded phrasing should still match earlier conversation."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<LcmVectorRecallArgs>(
            "lcm_vector_recall",
            "Semantic (vector) recall over the DAG",
        )
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: LcmVectorRecallArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::error("lcm_vector_recall", format!("Invalid arguments: {e}"));
            }
        };
        if args.query.trim().is_empty() {
            return ToolResult::error("lcm_vector_recall", "query is required");
        }
        let limit = args.limit.unwrap_or(5).clamp(1, 20);
        match self
            .engine
            .vector_recall(
                self.embedder.as_ref(),
                args.session.as_deref(),
                &args.query,
                limit,
            )
            .await
        {
            Ok(hits) => {
                if hits.is_empty() {
                    return ToolResult::success(
                        "lcm_vector_recall",
                        json!({
                            "_lcm_tool": "lcm_vector_recall",
                            "hits": [],
                            "note": "no candidate nodes to embed",
                        }),
                    );
                }
                let nodes: Vec<Value> = hits
                    .iter()
                    .map(|h| {
                        let mut content = h.content.clone();
                        if content.chars().count() > 600 {
                            content = content.chars().take(600).collect::<String>();
                            content.push_str("...[truncated]");
                        }
                        json!({
                            "node_id": h.node_id,
                            "role": h.role,
                            "content": content,
                            "created_at": h.created_at,
                            "similarity": h.score,
                        })
                    })
                    .collect();
                ToolResult::success(
                    "lcm_vector_recall",
                    json!({
                        "_lcm_tool": "lcm_vector_recall",
                        "hits": nodes,
                        "total_hits": hits.len(),
                        "model": self.embedder.model_id(),
                    }),
                )
            }
            Err(e) => ToolResult::error("lcm_vector_recall", format!("vector recall failed: {e}")),
        }
    }
}

/// Register the LCM tools into a registry. Only meaningful when the LCM
/// engine is actually attached (config `agent.context_engine = "lcm"`).
/// `embedder` is `None` when no embedding model is configured — the vector
/// tool is then not registered (hermes registers vector tools only when an
/// embedding provider is active).
pub async fn register_lcm_tools(
    registry: &crate::tools::ToolRegistry,
    engine: Arc<LcmContextEngine>,
    embedder: Option<Arc<dyn crate::context::Embedder>>,
) -> crate::error::Result<()> {
    registry
        .register(LcmRecallTool::new(engine.clone()))
        .await?;
    registry.register(LcmStatsTool::new(engine.clone())).await?;
    registry
        .register(LcmAssertTool::new(engine.clone()))
        .await?;
    registry
        .register(LcmRecallRoundTool::new(engine.clone()))
        .await?;
    if let Some(embedder) = embedder {
        registry
            .register(LcmVectorRecallTool::new(engine, embedder))
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn test_engine() -> Arc<LcmContextEngine> {
        let dir = std::env::temp_dir().join(format!(
            "operant_lcm_tools_test_{}_{}",
            std::process::id(),
            Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let engine = LcmContextEngine::new(crate::context::LcmConfig {
            db_path: dir.join("lcm-tools-test.db"),
            tail_tokens: 100,
            auto_recall: false,
            auto_recall_limit: 3,
            auto_recall_max_chars: 4_000,
            rollups_inject: true,
        })
        .unwrap();
        Arc::new(engine)
    }

    #[tokio::test]
    async fn lcm_recall_returns_verbatim_hits() {
        let engine = test_engine();
        let tool = LcmRecallTool::new(engine.clone());

        // Seed the DAG.
        let turn = vec![
            crate::client::Message::user("the deploy pipeline uses rsync over ssh"),
            crate::client::Message::assistant("rsync is configured in ci.yml"),
        ];
        engine.ingest_turn("test-session", &turn).await.unwrap();

        // Happy path: hit returned verbatim, scoped to the session.
        let out = tool
            .execute(
                serde_json::json!({ "query": "deploy pipeline", "session": "test-session" }),
                ToolContext::default(),
            )
            .await;
        let parsed = out.parse_content::<serde_json::Value>().unwrap();
        let hits = parsed["hits"].as_array().unwrap();
        assert!(!hits.is_empty(), "recall should find the seeded node");
        assert!(
            hits.iter()
                .any(|h| h["content"]
                    == serde_json::json!("the deploy pipeline uses rsync over ssh")),
            "hits must be verbatim"
        );
    }

    #[tokio::test]
    async fn lcm_recall_rejects_empty_query() {
        let tool = LcmRecallTool::new(test_engine());
        let out = tool
            .execute(serde_json::json!({ "query": "" }), ToolContext::default())
            .await;
        assert!(!out.success, "empty query must error");
    }
    #[tokio::test]
    async fn lcm_stats_reports_engine_state() {
        let engine = test_engine();
        let tool = LcmStatsTool::new(engine.clone());
        let out = tool
            .execute(serde_json::json!({}), ToolContext::default())
            .await;
        let parsed = out.parse_content::<serde_json::Value>().unwrap();
        assert_eq!(parsed["engine"], serde_json::json!("lcm"));
        assert_eq!(
            parsed["tail_tokens"],
            serde_json::json!(engine.tail_tokens())
        );
        assert!(
            parsed["db_path"]
                .as_str()
                .unwrap()
                .contains("lcm-tools-test")
        );
    }

    #[tokio::test]
    async fn cross_connection_engine_sees_committed_nodes() {
        // Production scenario: the agent-attached engine writes (ingest during
        // assemble/eager turn-end) while the registry's tool engine reads
        // (recall) — two LcmContextEngine instances sharing one WAL file.
        let dir = std::env::temp_dir().join(format!(
            "operant_lcm_cross_conn_{}_{}",
            std::process::id(),
            Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("lcm-cross.db");

        let writer = LcmContextEngine::new(crate::context::LcmConfig {
            db_path: db_path.clone(),
            tail_tokens: 100,
            auto_recall: false,
            auto_recall_limit: 3,
            auto_recall_max_chars: 4_000,
            rollups_inject: true,
        })
        .unwrap();
        let reader = LcmContextEngine::new(crate::context::LcmConfig {
            db_path: db_path.clone(),
            tail_tokens: 100,
            auto_recall: false,
            auto_recall_limit: 3,
            auto_recall_max_chars: 4_000,
            rollups_inject: true,
        })
        .unwrap();

        let turn = vec![crate::client::Message::user(
            "cross-connection freshness marker",
        )];
        writer.ingest_turn("s1", &turn).await.unwrap();

        let hits = reader
            .recall(Some("s1"), "freshness marker", 5)
            .await
            .unwrap();
        assert!(
            hits.iter().any(|h| h.content.contains("cross-connection")),
            "second engine instance must see nodes written by the first (WAL)"
        );
    }

    #[tokio::test]
    async fn lcm_assert_save_then_query_roundtrips_fact() {
        let engine = test_engine();
        let tool = LcmAssertTool::new(engine.clone());

        let saved = tool
            .execute(
                serde_json::json!({
                    "action": "save",
                    "subject": "project:hermes",
                    "predicate": "prefers",
                    "object": "Rust over Python",
                    "session": "sess_a",
                }),
                ToolContext::default(),
            )
            .await;
        assert!(saved.success, "save must succeed: {:?}", saved);
        let saved_parsed = saved.parse_content::<serde_json::Value>().unwrap();
        assert_eq!(saved_parsed["saved"], serde_json::json!(true));

        let queried = tool
            .execute(
                serde_json::json!({
                    "action": "query",
                    "subject": "project:hermes",
                    "session": "sess_a",
                }),
                ToolContext::default(),
            )
            .await;
        assert!(queried.success);
        let q = queried.parse_content::<serde_json::Value>().unwrap();
        assert_eq!(q["total"], serde_json::json!(1));
        assert_eq!(
            q["assertions"][0]["object"],
            serde_json::json!("Rust over Python")
        );
        assert_eq!(q["active"][0]["predicate"], serde_json::json!("prefers"));
        assert!(
            q["contradictions"].as_array().unwrap().is_empty(),
            "one fact, no contradictions"
        );
    }

    #[tokio::test]
    async fn lcm_assert_detects_contradictions_and_requires_object_on_save() {
        let engine = test_engine();
        let tool = LcmAssertTool::new(engine.clone());
        // Two saves of the same predicate with different objects = contradiction.
        for obj in ["vim", "emacs"] {
            let out = tool
                .execute(
                    serde_json::json!({
                        "action": "save",
                        "subject": "user",
                        "predicate": "editor",
                        "object": obj,
                    }),
                    ToolContext::default(),
                )
                .await;
            assert!(out.success, "save {obj} must succeed");
        }
        let queried = tool
            .execute(
                serde_json::json!({ "action": "query", "subject": "user" }),
                ToolContext::default(),
            )
            .await;
        let q = queried.parse_content::<serde_json::Value>().unwrap();
        assert_eq!(q["total"], serde_json::json!(2));
        assert_eq!(q["active"].as_array().unwrap().len(), 2);
        assert_eq!(
            q["contradictions"],
            serde_json::json!(["editor"]),
            "two distinct active objects for `editor`"
        );

        // Save without object must error.
        let bad = tool
            .execute(
                serde_json::json!({
                    "action": "save",
                    "subject": "user",
                    "predicate": "editor",
                }),
                ToolContext::default(),
            )
            .await;
        assert!(!bad.success, "save without object must error");
        // Unknown action must error.
        let bad_action = tool
            .execute(
                serde_json::json!({"action": "delete", "subject": "user"}),
                ToolContext::default(),
            )
            .await;
        assert!(!bad_action.success, "unknown action must error");
    }

    #[tokio::test]
    async fn lcm_recall_round_multi_round_evidence_gathering() {
        let engine = test_engine();
        let tool = LcmRecallRoundTool::new(engine.clone());
        let turn = vec![
            crate::client::Message::assistant("First fact: alpha is set to one."),
            crate::client::Message::assistant("Second fact: beta is set to two."),
            crate::client::Message::assistant("Third fact: gamma is set to three."),
        ];
        engine.ingest_turn("sess_round", &turn).await.unwrap();

        // Start: requirement of 2; round 1 gathers exact phrase evidence only.
        let start = tool
            .execute(
                serde_json::json!({
                    "query": "alpha",
                    "evidenceRequired": 2,
                    "session": "sess_round",
                }),
                ToolContext::default(),
            )
            .await;
        assert!(start.success, "start must succeed: {:?}", start);
        let r1 = start.parse_content::<serde_json::Value>().unwrap();
        assert_eq!(r1["round_number"], serde_json::json!(1));
        assert_eq!(r1["complete"], serde_json::json!(false));
        let id = r1["retrieval_id"].as_str().unwrap().to_string();

        // Continue with a refined query; gather the beta fact as evidence.
        let cont = tool
            .execute(
                serde_json::json!({
                    "retrievalId": id,
                    "query": "beta",
                    "session": "sess_round",
                }),
                ToolContext::default(),
            )
            .await;
        assert!(cont.success);
        let r2 = cont.parse_content::<serde_json::Value>().unwrap();
        assert_eq!(r2["round_number"], serde_json::json!(2));
        assert!(
            r2["evidence"]
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e["snippet"].as_str().unwrap().contains("beta")),
            "round 2 evidence must include the beta fact"
        );
        assert_eq!(r2["complete"], serde_json::json!(true));
    }

    #[tokio::test]
    async fn lcm_recall_round_requires_query_on_start_and_rejects_unknown_id() {
        let engine = test_engine();
        let tool = LcmRecallRoundTool::new(engine.clone());
        let out = tool
            .execute(serde_json::json!({}), ToolContext::default())
            .await;
        assert!(!out.success, "start without query must error");
        let bad = tool
            .execute(
                serde_json::json!({"retrievalId": "retr_nope"}),
                ToolContext::default(),
            )
            .await;
        assert!(!bad.success, "unknown retrieval id must error");
    }

    #[tokio::test]
    async fn lcm_vector_recall_ranks_and_validates() {
        let engine = test_engine();
        let embedder: std::sync::Arc<dyn crate::context::Embedder> =
            std::sync::Arc::new(crate::context::MockEmbedder::default());
        let tool = LcmVectorRecallTool::new(engine.clone(), embedder);
        let turn = vec![
            crate::client::Message::assistant("the deployment uses rsync over ssh"),
            crate::client::Message::assistant("irrelevant filler about the weather"),
        ];
        engine.ingest_turn("sess_vec", &turn).await.unwrap();

        let out = tool
            .execute(
                serde_json::json!({
                    "query": "deploy rsync",
                    "limit": 2,
                    "session": "sess_vec",
                }),
                ToolContext::default(),
            )
            .await;
        assert!(out.success, "vector recall must succeed: {:?}", out);
        let parsed = out.parse_content::<serde_json::Value>().unwrap();
        let hits = parsed["hits"].as_array().unwrap();
        assert!(!hits.is_empty(), "must return ranked hits");
        assert!(
            hits[0]["content"].as_str().unwrap().contains("rsync"),
            "top hit is the semantic match"
        );
        assert_eq!(parsed["model"], serde_json::json!("mock"));

        // Empty query must error.
        let bad = tool
            .execute(serde_json::json!({ "query": "" }), ToolContext::default())
            .await;
        assert!(!bad.success, "empty query must error");
    }
}
