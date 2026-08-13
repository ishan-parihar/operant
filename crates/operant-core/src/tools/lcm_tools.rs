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
    embedder: Option<Arc<dyn crate::context::Embedder>>,
}

impl LcmRecallTool {
    pub fn new(
        engine: Arc<LcmContextEngine>,
        embedder: Option<Arc<dyn crate::context::Embedder>>,
    ) -> Self {
        Self { engine, embedder }
    }
}

/// Reciprocal Rank Fusion (hermes-lcm parity: `lcm_recall` fuses its FTS and
/// semantic arms with RRF). Each arm contributes `1/(K + rank)` — pure ranks,
/// so the two arms need no score calibration (FTS bm25 is lower-is-better,
/// cosine similarity is higher-is-better). Hits are deduped by node id.
fn rrf_fuse(
    fts: Vec<crate::context::RecallHit>,
    vector: Vec<crate::context::RecallHit>,
    limit: usize,
) -> Vec<crate::context::RecallHit> {
    const K: f64 = 60.0;
    let mut scores: std::collections::HashMap<i64, f64> = std::collections::HashMap::new();
    let mut hits: std::collections::HashMap<i64, crate::context::RecallHit> =
        std::collections::HashMap::new();
    let mut arms: Vec<Vec<crate::context::RecallHit>> = Vec::with_capacity(2);
    if !fts.is_empty() {
        arms.push(fts);
    }
    if !vector.is_empty() {
        arms.push(vector);
    }
    for arm in arms {
        for (rank, hit) in arm.into_iter().enumerate() {
            *scores.entry(hit.node_id).or_insert(0.0) += 1.0 / (K + rank as f64 + 1.0);
            hits.entry(hit.node_id).or_insert(hit);
        }
    }
    let mut ranked: Vec<(i64, f64)> = scores.into_iter().collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    ranked
        .into_iter()
        .take(limit)
        .filter_map(|(id, _)| hits.remove(&id))
        .collect()
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
        let fts_hits = match self.engine.recall(session, &args.query, limit).await {
            Ok(hits) => hits,
            Err(e) => return ToolResult::error("lcm_recall", format!("recall failed: {e}")),
        };
        // hermes-lcm parity: when an embedding backend is configured, fuse the
        // FTS and semantic arms with Reciprocal Rank Fusion (RRF). RRF uses
        // ranks only, so the two arms need no score calibration (bm25 is
        // lower-is-better, cosine is higher-is-better). FTS-only otherwise.
        let hits = match &self.embedder {
            Some(embedder) => match self
                .engine
                .vector_recall(embedder.as_ref(), session, &args.query, limit)
                .await
            {
                Ok(vector) if !vector.is_empty() => rrf_fuse(fts_hits, vector, limit),
                Ok(_) => fts_hits,
                Err(e) => {
                    tracing::warn!(error = %e, "lcm_recall: semantic arm failed — FTS only");
                    fts_hits
                }
            },
            None => fts_hits,
        };
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

/// Store, query, or auto-extract durable assertions (hermes
/// `assertion_store.py` + `assertion_extraction.py` parity, lightweight).
/// `action = "save"` records a fact; `action = "query"` returns stored facts
/// plus the resolved active state (latest per unique object) and any
/// contradictions (distinct active objects for a predicate);
/// `action = "extract"` runs the LLM assertion extractor over the most
/// recent message nodes of the session (or the whole DAG) and stores the
/// discovered facts (opt-in via `agent.context_lcm_assertion_extraction`).
#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LcmAssertArgs {
    /// `save` (record a fact), `query` (retrieve facts), or `extract`
    /// (LLM-mine durable facts from recent DAG content).
    pub action: String,
    /// Subject key of the fact, e.g. `project:hermes`, `user`, `assistant:self`.
    /// Optional when action = `extract` (the extractor picks subjects).
    pub subject: Option<String>,
    /// Predicate key, e.g. `prefers`, `uses`, `deadline`. Optional on query.
    pub predicate: Option<String>,
    /// Object value — the fact itself. Required when action = `save`.
    pub object: Option<String>,
    /// Scope to a session id; omitted = global fact scope shared across sessions.
    pub session: Option<String>,
    /// Speaker role recorded with a saved fact (`user`|`assistant`|`tool`).
    pub speaker: Option<String>,
    /// Max recent message nodes scanned when action = `extract` (1..=200,
    /// default 40).
    pub limit: Option<usize>,
}

pub struct LcmAssertTool {
    engine: Arc<LcmContextEngine>,
    /// LLM assertion extractor; `None` when the opt-in config gate
    /// (`agent.context_lcm_assertion_extraction`) is off — the `extract`
    /// action then returns a clear "not configured" error.
    extractor: Option<Arc<dyn crate::context::AssertionExtractor>>,
}

impl LcmAssertTool {
    pub fn new(engine: Arc<LcmContextEngine>) -> Self {
        Self {
            engine,
            extractor: None,
        }
    }

    pub fn with_extractor(
        mut self,
        extractor: Option<Arc<dyn crate::context::AssertionExtractor>>,
    ) -> Self {
        self.extractor = extractor;
        self
    }
}

#[async_trait::async_trait]
impl OperantTool for LcmAssertTool {
    fn name(&self) -> &str {
        "lcm_assert"
    }

    fn description(&self) -> &str {
        "Store, query, or auto-extract durable assertions (facts) in the lossless context store. Save a fact with action=save to persist it across sessions; query with action=query to retrieve stored facts for a subject (including active state and contradictions); extract with action=extract to LLM-mine durable facts from the most recent conversation and persist them automatically. Use to remember durable user preferences, project decisions, or constraints."
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
        let scope = args.session.clone().unwrap_or_else(|| "global".to_string());
        match args.action.trim().to_lowercase().as_str() {
            "extract" => {
                // Hermes ModelAssertionExtractor parity, opt-in: when the
                // gate is off, return a clear actionable error instead of a
                // cryptic one.
                let Some(extractor) = &self.extractor else {
                    return ToolResult::error(
                        "lcm_assert",
                        "assertion extraction is not configured — set agent.context_lcm_assertion_extraction = true to enable LLM fact mining",
                    );
                };
                // Shared backend with the background maintenance scheduler
                // (hermes _assertion_extraction parity): scan the recent
                // message nodes, LLM-mine durable facts, persist them.
                let limit = args.limit.unwrap_or(40);
                let report = match crate::context::assertion_extract::run_assertion_extraction(
                    &self.engine,
                    extractor.as_ref(),
                    args.session.as_deref(),
                    limit,
                )
                .await
                {
                    Ok(r) => r,
                    Err(e) => {
                        return ToolResult::error("lcm_assert", format!("extract failed: {e}"));
                    }
                };
                let assertions: Vec<Value> = report
                    .assertions
                    .iter()
                    .map(|a| {
                        json!({
                            "id": a.id,
                            "subject": a.subject,
                            "predicate": a.predicate,
                            "object": a.object,
                            "speaker": a.speaker,
                        })
                    })
                    .collect();
                let mut payload = json!({
                    "_lcm_tool": "lcm_assert",
                    "action": "extract",
                    "saved": report.saved,
                    "scanned_nodes": report.scanned_nodes,
                    "extractor": extractor.name(),
                    "assertions": assertions,
                });
                if report.saved == 0 && report.scanned_nodes == 0 {
                    payload["note"] = json!("no message nodes to mine in scope");
                } else if report.saved == 0 {
                    payload["note"] = json!("no durable assertions found in the scanned nodes");
                }
                ToolResult::success("lcm_assert", payload)
            }
            "save" => {
                let subject = args.subject.as_deref().unwrap_or("").trim().to_string();
                if subject.is_empty() {
                    return ToolResult::error(
                        "lcm_assert",
                        "subject is required when action = save",
                    );
                }
                let predicate = args.predicate.as_deref().unwrap_or("states").trim();
                let object = args.object.as_deref().unwrap_or("").trim();
                if object.is_empty() {
                    return ToolResult::error(
                        "lcm_assert",
                        "object is required when action = save",
                    );
                }
                let speaker = args.speaker.as_deref().unwrap_or("assistant");
                match self
                    .engine
                    .assert_assertion(&scope, &subject, predicate, object, speaker, None)
                {
                    Ok(id) => ToolResult::success(
                        "lcm_assert",
                        json!({
                            "_lcm_tool": "lcm_assert",
                            "action": "save",
                            "saved": true,
                            "assertion_id": id,
                            "subject": subject,
                            "predicate": predicate,
                            "object": object,
                        }),
                    ),
                    Err(e) => ToolResult::error("lcm_assert", format!("save failed: {e}")),
                }
            }
            "query" => {
                let subject = args.subject.as_deref().unwrap_or("").trim();
                if subject.is_empty() {
                    return ToolResult::error(
                        "lcm_assert",
                        "subject is required when action = query",
                    );
                }
                match self
                    .engine
                    .query_assertion_state(&scope, subject, args.predicate.as_deref())
                {
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
                }
            }
            other => ToolResult::error(
                "lcm_assert",
                format!("unknown action `{other}` — use save, query, or extract"),
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
/// Temporal memory recall — hermes-lcm `lcm_recent` parity. Returns the most
/// recent context summaries by natural UTC period (`day` / `week` / `month`),
/// preferring ready rollups and falling back to the most recent message nodes
/// when no rollup covers the period yet.
#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LcmRecentArgs {
    /// Optional period kind filter: `day` | `week` | `month`. Omit for all.
    pub period: Option<String>,
    /// Max entries to return (1..=20, default 5).
    pub limit: Option<usize>,
}

pub struct LcmRecentTool {
    engine: Arc<LcmContextEngine>,
}

impl LcmRecentTool {
    pub fn new(engine: Arc<LcmContextEngine>) -> Self {
        Self { engine }
    }
}

#[async_trait::async_trait]
impl OperantTool for LcmRecentTool {
    fn name(&self) -> &str {
        "lcm_recent"
    }

    fn description(&self) -> &str {
        "Retrieve recent context summaries by natural time period (day/week/month) from the lossless DAG. Prefers ready rollups; falls back to the most recent message nodes when no rollup exists yet. Use for temporal memory: what happened this week, last month, etc."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<LcmRecentArgs>(
            "lcm_recent",
            "Recent context by natural time period",
        )
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: LcmRecentArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::error("lcm_recent", format!("Invalid arguments: {e}"));
            }
        };
        let limit = args.limit.unwrap_or(5).clamp(1, 20);
        let period = args.period.as_deref();
        let rollups = match self.engine.list_rollups_global(period, limit) {
            Ok(r) => r,
            Err(e) => return ToolResult::error("lcm_recent", format!("rollup lookup failed: {e}")),
        };
        let entries: Vec<Value> = rollups
            .iter()
            .map(|r| {
                json!({
                    "type": "rollup",
                    "period_kind": r.period_kind,
                    "period_start": r.period_start,
                    "summary": r.summary,
                    "source_count": r.source_count,
                    "created_at": r.created_at,
                })
            })
            .collect();
        // Fallback: no ready rollups → most recent message nodes (leaf
        // summaries), time-bounded by the request limit.
        let (entries, fallback) = if entries.is_empty() {
            let nodes = match self.engine.recent_message_nodes(None, limit) {
                Ok(n) => n,
                Err(e) => {
                    return ToolResult::error("lcm_recent", format!("node lookup failed: {e}"));
                }
            };
            (
                nodes
                    .iter()
                    .map(|(node_id, role, content, created_at)| {
                        let mut snippet = content.clone();
                        if snippet.chars().count() > 300 {
                            snippet = snippet.chars().take(300).collect::<String>();
                            snippet.push_str("...[truncated]");
                        }
                        json!({
                            "type": "message",
                            "node_id": node_id,
                            "role": role,
                            "snippet": snippet,
                            "created_at": created_at,
                        })
                    })
                    .collect::<Vec<Value>>(),
                true,
            )
        } else {
            (entries, false)
        };
        ToolResult::success(
            "lcm_recent",
            json!({
                "_lcm_tool": "lcm_recent",
                "period": period.unwrap_or("all"),
                "rollup_fallback": fallback,
                "entries": entries,
                "total": entries.len(),
            }),
        )
    }
}

/// DB / index / lifecycle health diagnostics — hermes-lcm `lcm_doctor` parity.
#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LcmDoctorArgs {}

pub struct LcmDoctorTool {
    engine: Arc<LcmContextEngine>,
}

impl LcmDoctorTool {
    pub fn new(engine: Arc<LcmContextEngine>) -> Self {
        Self { engine }
    }
}

#[async_trait::async_trait]
impl OperantTool for LcmDoctorTool {
    fn name(&self) -> &str {
        "lcm_doctor"
    }

    fn description(&self) -> &str {
        "Run health diagnostics on the lossless DAG store: SQLite integrity check, node/rollup/assertion/embedding counts, FTS index coverage, and store size. Use to verify the context engine is healthy and detect drift or corruption early."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<LcmDoctorArgs>("lcm_doctor", "LCM store health diagnostics")
    }

    async fn execute(&self, _args: Value, _context: ToolContext) -> ToolResult {
        match self.engine.doctor() {
            Ok(diag) => ToolResult::success(
                "lcm_doctor",
                json!({
                    "_lcm_tool": "lcm_doctor",
                    "diagnostics": diag,
                }),
            ),
            Err(e) => ToolResult::error("lcm_doctor", format!("diagnostics failed: {e}")),
        }
    }
}

/// Load an ordered, bounded raw-message transcript page for one explicit
/// session (hermes-lcm `lcm_load_session` parity). Paged with an
/// `after_store_id` cursor; the returned `next_cursor` continues the page.
#[derive(JsonSchema, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LcmLoadSessionArgs {
    /// The session id whose raw transcript to page through.
    pub session_id: String,
    /// Cursor: resume after this node id (omit for the first page).
    pub after_store_id: Option<i64>,
    /// Max rows per page (1..=200, default 50).
    pub limit: Option<usize>,
    /// Include exact slice refs (`content_hash` + `position`) for each row
    /// (opt-in; default false keeps the payload lean).
    #[serde(default)]
    pub include_exact_ref: bool,
}

pub struct LcmLoadSessionTool {
    engine: Arc<LcmContextEngine>,
}

impl LcmLoadSessionTool {
    pub fn new(engine: Arc<LcmContextEngine>) -> Self {
        Self { engine }
    }
}

#[async_trait::async_trait]
impl OperantTool for LcmLoadSessionTool {
    fn name(&self) -> &str {
        "lcm_load_session"
    }

    fn description(&self) -> &str {
        "Load an ordered, bounded page of the raw message transcript for one explicit session_id from the lossless DAG. Paged with afterStoreId (the returned nextCursor continues). Opt into includeExactRef=true for content hashes + positions. Use to read exactly what was said in a past session, message by message."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<LcmLoadSessionArgs>(
            "lcm_load_session",
            "Load a paged raw session transcript",
        )
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: LcmLoadSessionArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return ToolResult::error("lcm_load_session", format!("Invalid arguments: {e}"));
            }
        };
        let session_id = args.session_id.trim();
        if session_id.is_empty() {
            return ToolResult::error("lcm_load_session", "session_id is required");
        }
        let limit = args.limit.unwrap_or(50).clamp(1, 200);
        match self
            .engine
            .load_session_page(session_id, args.after_store_id, limit)
        {
            Ok((rows, next_cursor)) => {
                if rows.is_empty() {
                    let note = if next_cursor.is_none() && args.after_store_id.is_some() {
                        "end of transcript reached — no more rows after the cursor"
                    } else {
                        "no message nodes found for this session"
                    };
                    return ToolResult::success(
                        "lcm_load_session",
                        json!({
                            "_lcm_tool": "lcm_load_session",
                            "session_id": session_id,
                            "rows": [],
                            "next_cursor": null,
                            "note": note,
                        }),
                    );
                }
                let rendered: Vec<Value> = rows
                    .iter()
                    .map(|(id, role, content, hash, position, created_at)| {
                        let mut obj = json!({
                            "store_id": id,
                            "role": role,
                            "content": content,
                            "created_at": created_at,
                        });
                        if args.include_exact_ref
                            && let Some(obj) = obj.as_object_mut()
                        {
                            obj.insert("content_hash".into(), json!(hash));
                            obj.insert("position".into(), json!(position));
                        }
                        obj
                    })
                    .collect();
                ToolResult::success(
                    "lcm_load_session",
                    json!({
                        "_lcm_tool": "lcm_load_session",
                        "session_id": session_id,
                        "rows": rendered,
                        "next_cursor": next_cursor,
                        "page_size": rows.len(),
                        "include_exact_ref": args.include_exact_ref,
                        "hint": "Pass nextCursor as afterStoreId to load the next page.",
                    }),
                )
            }
            Err(e) => ToolResult::error("lcm_load_session", format!("page load failed: {e}")),
        }
    }
}

/// `embedder` is `None` when no embedding model is configured — the vector
/// tool is then not registered (hermes registers vector tools only when an
/// embedding provider is active).
pub async fn register_lcm_tools(
    registry: &crate::tools::ToolRegistry,
    engine: Arc<LcmContextEngine>,
    embedder: Option<Arc<dyn crate::context::Embedder>>,
    extractor: Option<Arc<dyn crate::context::AssertionExtractor>>,
) -> crate::error::Result<()> {
    registry
        .register(LcmRecallTool::new(engine.clone(), embedder.clone()))
        .await?;
    registry.register(LcmStatsTool::new(engine.clone())).await?;
    registry
        .register(LcmAssertTool::new(engine.clone()).with_extractor(extractor))
        .await?;
    registry
        .register(LcmRecallRoundTool::new(engine.clone()))
        .await?;
    registry
        .register(LcmRecentTool::new(engine.clone()))
        .await?;
    registry
        .register(LcmDoctorTool::new(engine.clone()))
        .await?;
    registry
        .register(LcmLoadSessionTool::new(engine.clone()))
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
            ignore_session_patterns: Vec::new(),
            readonly_sessions: Vec::new(),
        })
        .unwrap();
        Arc::new(engine)
    }

    fn hit(node_id: i64, content: &str) -> crate::context::RecallHit {
        crate::context::RecallHit {
            node_id,
            role: "user".to_string(),
            content: content.to_string(),
            created_at: 1,
            score: 0.0,
        }
    }

    #[test]
    fn rrf_fuse_merges_dedupes_and_reranks() {
        // FTS arm ranks A best; vector arm ranks B best; C appears in both.
        let fts = vec![hit(1, "a"), hit(2, "b"), hit(3, "c")];
        let vector = vec![hit(4, "d"), hit(3, "c-vec"), hit(2, "b-vec")];
        let fused = rrf_fuse(fts, vector, 5);
        assert_eq!(fused.len(), 4, "overlap (node 3) must be deduped");
        let ids: Vec<i64> = fused.iter().map(|h| h.node_id).collect();
        // Rank sums: node2: 1/62 + 1/63 and node3: 1/63 + 1/62 are tied for
        // first — both overlaps must lead, in either order.
        let (a, b) = (ids[0], ids[1]);
        assert!(
            (a == 2 && b == 3) || (a == 3 && b == 2),
            "both in-both-arms hits must lead, got {ids:?}"
        );
        // FTS-only arms degrade to plain ranking.
        let solo = rrf_fuse(vec![hit(9, "x"), hit(8, "y")], Vec::new(), 5);
        assert_eq!(solo.len(), 2);
        assert_eq!(solo[0].node_id, 9);
        // Empty both arms → empty.
        assert!(rrf_fuse(Vec::new(), Vec::new(), 5).is_empty());
    }

    #[tokio::test]
    async fn lcm_recall_fuses_fts_and_vector_when_embedder_present() {
        let engine = test_engine();
        let embedder: Arc<dyn crate::context::Embedder> =
            Arc::new(crate::context::MockEmbedder::default());
        let tool = LcmRecallTool::new(engine.clone(), Some(embedder.clone()));
        let turn = vec![
            crate::client::Message::assistant("the deployment uses rsync over ssh"),
            crate::client::Message::assistant("irrelevant filler about the weather"),
        ];
        engine.ingest_turn("sess_fuse", &turn).await.unwrap();
        let out = tool
            .execute(
                serde_json::json!({
                    "query": "deploy rsync",
                    "limit": 5,
                    "session": "sess_fuse",
                }),
                ToolContext::default(),
            )
            .await;
        assert!(out.success, "fused recall must succeed: {:?}", out);
        let parsed = out.parse_content::<serde_json::Value>().unwrap();
        let hits = parsed["hits"].as_array().unwrap();
        assert!(!hits.is_empty(), "fused recall must return hits");
        // Semantic arm is a mock (same vector for all), so FTS ranks first;
        // the point is the tool still works with an embedder attached.
        assert!(hits[0]["content"].as_str().unwrap().contains("rsync"));
    }

    #[tokio::test]
    async fn lcm_recent_falls_back_to_recent_messages() {
        let engine = test_engine();
        let tool = LcmRecentTool::new(engine.clone());
        let turn = vec![
            crate::client::Message::assistant("alpha summary content"),
            crate::client::Message::assistant("beta summary content"),
        ];
        engine.ingest_turn("sess_rec", &turn).await.unwrap();
        let out = tool
            .execute(serde_json::json!({ "limit": 3 }), ToolContext::default())
            .await;
        assert!(out.success, "lcm_recent must succeed: {:?}", out);
        let parsed = out.parse_content::<serde_json::Value>().unwrap();
        let entries = parsed["entries"].as_array().unwrap();
        assert!(
            !entries.is_empty(),
            "must fall back to recent message nodes"
        );
        assert_eq!(parsed["rollup_fallback"], serde_json::json!(true));
        // Bad period still returns a result (filtered or fallback).
        let bad = tool
            .execute(
                serde_json::json!({ "period": "month", "limit": 3 }),
                ToolContext::default(),
            )
            .await;
        assert!(bad.success);
    }

    #[tokio::test]
    async fn lcm_doctor_reports_store_health() {
        let engine = test_engine();
        let tool = LcmDoctorTool::new(engine.clone());
        let turn = vec![crate::client::Message::assistant("seed node for doctor")];
        engine.ingest_turn("sess_doc", &turn).await.unwrap();
        let out = tool
            .execute(serde_json::json!({}), ToolContext::default())
            .await;
        assert!(out.success, "lcm_doctor must succeed: {:?}", out);
        let parsed = out.parse_content::<serde_json::Value>().unwrap();
        let diag = &parsed["diagnostics"];
        assert_eq!(diag["engine"], serde_json::json!("lcm"));
        assert_eq!(diag["integrity_check"], serde_json::json!("ok"));
        assert!(diag["nodes"].as_i64().unwrap() >= 1);
        assert!(diag["fts_coverage_pct"].as_i64().unwrap() >= 0);
    }

    #[tokio::test]
    async fn lcm_recall_returns_verbatim_hits() {
        let engine = test_engine();
        let tool = LcmRecallTool::new(engine.clone(), None);

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
        let tool = LcmRecallTool::new(test_engine(), None);
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
            ignore_session_patterns: Vec::new(),
            readonly_sessions: Vec::new(),
        })
        .unwrap();
        let reader = LcmContextEngine::new(crate::context::LcmConfig {
            db_path: db_path.clone(),
            tail_tokens: 100,
            auto_recall: false,
            auto_recall_limit: 3,
            auto_recall_max_chars: 4_000,
            rollups_inject: true,
            ignore_session_patterns: Vec::new(),
            readonly_sessions: Vec::new(),
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
    async fn lcm_assert_extract_stores_mined_facts_and_errors_when_not_configured() {
        let engine = test_engine();
        engine
            .ingest_turn(
                "sess_ex",
                &[
                    crate::client::Message::assistant("The deploy window is every Tuesday."),
                    crate::client::Message::assistant("The stack is Rust and SQLite."),
                ],
            )
            .await
            .unwrap();

        // Without an extractor, the action must fail with a clear message.
        let unconfigured = LcmAssertTool::new(engine.clone());
        let out = unconfigured
            .execute(
                serde_json::json!({"action": "extract", "session": "sess_ex"}),
                ToolContext::default(),
            )
            .await;
        assert!(!out.success, "extract without extractor must error");
        assert!(
            out.error
                .as_ref()
                .unwrap()
                .contains("context_lcm_assertion_extraction"),
            "error must name the config gate"
        );

        // With a fake extractor, mined facts are persisted + attributed.
        let fake: std::sync::Arc<dyn crate::context::AssertionExtractor> =
            std::sync::Arc::new(crate::context::assertion_extract::tests::FakeExtractor {
                assertions: vec![
                    crate::context::ExtractedAssertion {
                        subject: "project:operant".to_string(),
                        predicate: "stack".to_string(),
                        object: "Rust and SQLite".to_string(),
                        speaker: "assistant".to_string(),
                    },
                    crate::context::ExtractedAssertion {
                        subject: "project:operant".to_string(),
                        predicate: "deploy".to_string(),
                        object: "every Tuesday".to_string(),
                        speaker: "assistant".to_string(),
                    },
                ],
            });
        let tool = LcmAssertTool::new(engine.clone()).with_extractor(Some(fake));
        let out = tool
            .execute(
                serde_json::json!({"action": "extract", "session": "sess_ex", "limit": 10}),
                ToolContext::default(),
            )
            .await;
        assert!(out.success, "extract must succeed: {:?}", out);
        let p = out.parse_content::<serde_json::Value>().unwrap();
        assert_eq!(p["action"], serde_json::json!("extract"));
        assert_eq!(p["saved"], serde_json::json!(2));
        assert_eq!(p["extractor"], serde_json::json!("fake"));
        assert!(p["scanned_nodes"].as_i64().unwrap() >= 2);

        // The mined facts are now queryable through the normal query path.
        let q = LcmAssertTool::new(engine.clone())
            .execute(
                serde_json::json!({
                    "action": "query",
                    "subject": "project:operant",
                    "session": "sess_ex",
                }),
                ToolContext::default(),
            )
            .await;
        let qp = q.parse_content::<serde_json::Value>().unwrap();
        assert_eq!(qp["total"], serde_json::json!(2));
        assert!(
            qp["assertions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|a| a["object"] == serde_json::json!("Rust and SQLite")),
            "mined fact must be queryable"
        );
        // Source attribution: the mined assertions point at a real node.
        assert!(
            qp["assertions"]
                .as_array()
                .unwrap()
                .iter()
                .all(|a| a["id"].as_i64().is_some()),
            "stored assertions carry ids"
        );
    }

    #[tokio::test]
    async fn lcm_assert_extract_reports_empty_scope() {
        let engine = test_engine();
        let fake: std::sync::Arc<dyn crate::context::AssertionExtractor> =
            std::sync::Arc::new(crate::context::assertion_extract::tests::FakeExtractor {
                assertions: vec![],
            });
        let tool = LcmAssertTool::new(engine.clone()).with_extractor(Some(fake));
        // Empty DAG scope → clean "nothing to mine" success, not an error.
        let out = tool
            .execute(
                serde_json::json!({"action": "extract", "session": "ghost"}),
                ToolContext::default(),
            )
            .await;
        assert!(out.success);
        let p = out.parse_content::<serde_json::Value>().unwrap();
        assert_eq!(p["saved"], serde_json::json!(0));
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
    async fn lcm_load_session_pages_through_a_transcript() {
        let engine = test_engine();
        let tool = LcmLoadSessionTool::new(engine.clone());
        let turn = vec![
            crate::client::Message::user("first question"),
            crate::client::Message::assistant("first answer"),
            crate::client::Message::user("second question"),
            crate::client::Message::assistant("second answer"),
        ];
        engine.ingest_turn("sess_load", &turn).await.unwrap();

        // Page 1: 2 rows, cursor returned. (serde camelCase: sessionId)
        let p1 = tool
            .execute(
                serde_json::json!({ "sessionId": "sess_load", "limit": 2 }),
                ToolContext::default(),
            )
            .await;
        assert!(p1.success, "page 1 must succeed: {:?}", p1);
        let r1 = p1.parse_content::<serde_json::Value>().unwrap();
        let rows1 = r1["rows"].as_array().unwrap();
        assert_eq!(rows1.len(), 2);
        assert_eq!(rows1[0]["content"], serde_json::json!("first question"));
        assert_eq!(rows1[1]["content"], serde_json::json!("first answer"));
        let cursor = r1["next_cursor"].as_i64().unwrap();

        // Page 2 continues after the cursor (oldest-first ordering).
        let p2 = tool
            .execute(
                serde_json::json!({
                    "sessionId": "sess_load",
                    "afterStoreId": cursor,
                    "limit": 2,
                }),
                ToolContext::default(),
            )
            .await;
        assert!(p2.success, "page 2 must succeed: {:?}", p2);
        let r2 = p2.parse_content::<serde_json::Value>().unwrap();
        let rows2 = r2["rows"].as_array().unwrap();
        assert_eq!(rows2.len(), 2);
        assert_eq!(rows2[0]["content"], serde_json::json!("second question"));
        assert_eq!(rows2[1]["content"], serde_json::json!("second answer"));
        assert!(r2["next_cursor"].is_null(), "no third page");
    }

    #[tokio::test]
    async fn lcm_load_session_include_exact_ref_and_empty_session() {
        let engine = test_engine();
        let tool = LcmLoadSessionTool::new(engine.clone());
        engine
            .ingest_turn(
                "sess_ref",
                &[crate::client::Message::assistant("hello dag")],
            )
            .await
            .unwrap();

        // includeExactRef=true surfaces content_hash + position.
        let out = tool
            .execute(
                serde_json::json!({
                    "sessionId": "sess_ref",
                    "includeExactRef": true,
                }),
                ToolContext::default(),
            )
            .await;
        assert!(out.success);
        let p = out.parse_content::<serde_json::Value>().unwrap();
        let row = &p["rows"][0];
        assert!(row["content_hash"].is_string());
        assert!(row["position"].is_i64());
        assert_eq!(p["include_exact_ref"], serde_json::json!(true));

        // Unknown session → clean empty result, not an error.
        let ghost = tool
            .execute(
                serde_json::json!({ "sessionId": "ghost-session" }),
                ToolContext::default(),
            )
            .await;
        assert!(ghost.success);
        let g = ghost.parse_content::<serde_json::Value>().unwrap();
        assert!(g["rows"].as_array().unwrap().is_empty());
        assert!(g["note"].as_str().unwrap().contains("no message nodes"));

        // Empty session_id → error.
        let bad = tool
            .execute(
                serde_json::json!({ "sessionId": "" }),
                ToolContext::default(),
            )
            .await;
        assert!(!bad.success);
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
