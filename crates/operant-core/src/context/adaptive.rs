//! Adaptive evidence-gated retrieval (hermes `adaptive_retrieval.py` parity,
//! lightweight port).
//!
//! hermes runs multi-round recall where each round returns **exact evidence**
//! (verbatim snippets that satisfy the evidence requirement) plus **search
//! leads** for the next round, and the retrieval completes once enough
//! distinct evidence is gathered. The full hermes machinery (persisted query
//! views, tool-metadata resolution, budget ledger) is replaced here by an
//! in-memory registry keyed by a caller-supplied `retrieval_id`, with TTL
//! purge — enough to make rounds genuinely interactive inside the agentic
//! loop without the cross-session persistence surface (YAGNI).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::context::{ContextEngine, LcmContextEngine};
use crate::error::{Error, Result};

/// Maximum retrieval rounds before a session is force-completed.
pub const MAX_ROUNDS: usize = 3;
/// Idle TTL for in-memory retrieval state (purged on access past this).
const STATE_TTL_MILLIS: u64 = 5 * 60 * 1000;

/// A verbatim snippet that satisfies the evidence requirement
/// (hermes `ExactEvidence` parity).
#[derive(Debug, Clone)]
pub struct ExactEvidence {
    pub node_id: i64,
    pub snippet: String,
    pub role: String,
    pub score: f64,
}

/// A candidate node worth searching next round (hermes `SearchLead` parity).
#[derive(Debug, Clone)]
pub struct SearchLead {
    pub node_id: i64,
    pub snippet: String,
    pub role: String,
    pub score: f64,
}

/// One retrieval round's result, returned to the model.
#[derive(Debug, Clone)]
pub struct RetrievalRound {
    pub retrieval_id: String,
    pub round_number: usize,
    pub evidence_required: usize,
    pub evidence_found: Vec<ExactEvidence>,
    pub leads: Vec<SearchLead>,
    /// True when the evidence requirement is met (or rounds exhausted).
    pub complete: bool,
}

/// In-memory state for one active retrieval (hermes `RetrievalState` parity).
#[derive(Debug, Clone)]
struct RetrievalState {
    query: String,
    evidence_required: usize,
    rounds_done: usize,
    evidence_node_ids: Vec<i64>,
    last_used_millis: u64,
}

/// Multi-round retrieval registry. Rounds are stateful and interactive: the
/// model calls `lcm_recall_round` again with the returned `retrieval_id`
/// (optionally refining the query) until `complete` is true.
#[derive(Default)]
pub struct AdaptiveRetrievalRegistry {
    states: Mutex<HashMap<String, RetrievalState>>,
}

impl AdaptiveRetrievalRegistry {
    pub fn new() -> Self {
        Self {
            states: Mutex::new(HashMap::new()),
        }
    }

    fn now_millis() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    fn purge(&self) {
        let now = Self::now_millis();
        if let Ok(mut states) = self.states.lock() {
            states.retain(|_, s| now.saturating_sub(s.last_used_millis) < STATE_TTL_MILLIS);
        }
    }

    /// Start a new retrieval: run round 1 against `query` and return the
    /// round payload. `session` scopes the search (None = global DAG).
    pub async fn start(
        &self,
        engine: &LcmContextEngine,
        session: Option<&str>,
        query: &str,
        evidence_required: usize,
    ) -> Result<RetrievalRound> {
        self.purge();
        // Compact id: `retr_<12 hex>` — short enough for agent round-trips.
        let retrieval_id = format!("retr_{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
        let req = evidence_required.clamp(1, 5);
        {
            let mut states = self
                .states
                .lock()
                .map_err(|_| Error::Agent("lcm: adaptive registry poisoned".into()))?;
            states.insert(
                retrieval_id.clone(),
                RetrievalState {
                    query: query.trim().to_string(),
                    evidence_required: req,
                    rounds_done: 0,
                    evidence_node_ids: Vec::new(),
                    last_used_millis: Self::now_millis(),
                },
            );
        }
        self.run_round(engine, session, &retrieval_id, Some(query))
            .await
    }

    /// Continue an existing retrieval with an optional refined query.
    /// Returns `None` when the id is unknown or the retrieval already
    /// completed.
    pub async fn next_round(
        &self,
        engine: &LcmContextEngine,
        session: Option<&str>,
        retrieval_id: &str,
        refined_query: Option<&str>,
    ) -> Result<Option<RetrievalRound>> {
        self.purge();
        let mut query = None;
        {
            let mut states = self
                .states
                .lock()
                .map_err(|_| Error::Agent("lcm: adaptive registry poisoned".into()))?;
            let Some(state) = states.get_mut(retrieval_id) else {
                return Ok(None);
            };
            if state.rounds_done >= MAX_ROUNDS {
                return Ok(None);
            }
            state.last_used_millis = Self::now_millis();
            if let Some(q) = refined_query
                && !q.trim().is_empty()
            {
                state.query = q.trim().to_string();
                query = Some(state.query.clone());
            }
        }
        match query {
            Some(q) => self
                .run_round(engine, session, retrieval_id, Some(q.as_str()))
                .await
                .map(Some),
            None => self
                .run_round(engine, session, retrieval_id, None)
                .await
                .map(Some),
        }
    }

    /// Drop all in-memory retrieval state (tests / session reset).
    pub fn clear(&self) {
        if let Ok(mut states) = self.states.lock() {
            states.clear();
        }
    }

    /// Run one search round and fold new evidence into the state.
    async fn run_round(
        &self,
        engine: &LcmContextEngine,
        session: Option<&str>,
        retrieval_id: &str,
        query: Option<&str>,
    ) -> Result<RetrievalRound> {
        let (req, seen, rounds_done, q) = {
            let states = self
                .states
                .lock()
                .map_err(|_| Error::Agent("lcm: adaptive registry poisoned".into()))?;
            let Some(state) = states.get(retrieval_id) else {
                return Err(Error::Agent(format!(
                    "lcm: unknown retrieval_id {retrieval_id} — start a new retrieval by omitting it"
                )));
            };
            (
                state.evidence_required,
                state.evidence_node_ids.clone(),
                state.rounds_done,
                query
                    .map(str::to_string)
                    .unwrap_or_else(|| state.query.clone()),
            )
        };

        let hits = engine.recall(session, &q, 10).await.unwrap_or_default();
        let round_number = rounds_done + 1;
        let mut evidence = Vec::new();
        let mut leads = Vec::new();
        let mut known = seen;
        let query_lower = q.to_lowercase();

        // Evidence = verbatim hits containing the query terms; leads = the
        // rest (candidates for the next, possibly refined, round).
        for h in &hits {
            let snippet = bounded_snippet(&h.content);
            if known.contains(&h.node_id) {
                continue;
            }
            if !query_lower.is_empty() && h.content.to_lowercase().contains(&query_lower) {
                known.push(h.node_id);
                evidence.push(ExactEvidence {
                    node_id: h.node_id,
                    snippet,
                    role: h.role.clone(),
                    score: h.score,
                });
            } else {
                leads.push(SearchLead {
                    node_id: h.node_id,
                    snippet,
                    role: h.role.clone(),
                    score: h.score,
                });
            }
        }

        // Evidence accumulates across rounds: `known` includes this round's
        // new exact-evidence node ids, so the requirement is judged against
        // the cumulative set (hermes evidence-gating semantics).
        let complete = known.len() >= req || round_number >= MAX_ROUNDS;
        let out = RetrievalRound {
            retrieval_id: retrieval_id.to_string(),
            round_number,
            evidence_required: req,
            evidence_found: evidence.clone(),
            leads,
            complete,
        };

        let mut states = self
            .states
            .lock()
            .map_err(|_| Error::Agent("lcm: adaptive registry poisoned".into()))?;
        if let Some(state) = states.get_mut(retrieval_id) {
            state.rounds_done = round_number;
            state.last_used_millis = Self::now_millis();
            for e in evidence {
                if !state.evidence_node_ids.contains(&e.node_id) {
                    state.evidence_node_ids.push(e.node_id);
                }
            }
        }
        Ok(out)
    }
}

/// Bound a node's content to a short verbatim snippet.
fn bounded_snippet(content: &str) -> String {
    let mut s: String = content.chars().take(300).collect();
    if content.chars().count() > 300 {
        s.push_str("...");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::LcmConfig;
    use std::sync::Arc;
    use uuid::Uuid;

    fn test_engine() -> Arc<LcmContextEngine> {
        let dir = std::env::temp_dir().join(format!(
            "operant_lcm_adaptive_{}_{}",
            std::process::id(),
            Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Arc::new(
            LcmContextEngine::new(LcmConfig {
                db_path: dir.join("adaptive.db"),
                tail_tokens: 100,
                auto_recall: false,
                auto_recall_limit: 3,
                auto_recall_max_chars: 4_000,
                rollups_inject: true,
            })
            .unwrap(),
        )
    }

    async fn seed(engine: &LcmContextEngine, session: &str, contents: &[&str]) {
        // One turn per fact: assistant messages are indexed (system is not).
        for c in contents {
            engine
                .ingest_turn(
                    session,
                    &[crate::client::Message::assistant((*c).to_string())],
                )
                .await
                .expect("seed node");
        }
    }

    #[tokio::test]
    async fn start_returns_evidence_and_completes_when_met() {
        let engine = test_engine();
        let session = "sess_adaptive_test";
        seed(
            &engine,
            session,
            &[
                "The magic number for this session is forty-two-point-seven.",
                "The deploy date is set to August 20th.",
                "Unrelated chit-chat about the weather.",
            ],
        )
        .await;
        let reg = AdaptiveRetrievalRegistry::new();
        let round = reg
            .start(&engine, Some(session), "forty-two-point-seven", 1)
            .await
            .unwrap();
        assert_eq!(round.round_number, 1);
        assert_eq!(round.evidence_required, 1);
        assert!(
            !round.evidence_found.is_empty(),
            "expected exact evidence, got {:#?}",
            round.evidence_found
        );
        assert!(
            round
                .evidence_found
                .iter()
                .any(|e| e.snippet.contains("forty-two-point-seven")),
            "evidence must contain the exact phrase"
        );
        assert!(round.complete, "requirement of 1 met in round 1");
    }

    #[tokio::test]
    async fn continue_rounds_accumulate_until_requirement() {
        let engine = test_engine();
        let session = "sess_adaptive_rounds";
        seed(
            &engine,
            session,
            &[
                "First fact: alpha is set to one.",
                "Second fact: beta is set to two.",
                "Third fact: gamma is set to three.",
                "Noise node with nothing relevant here.",
            ],
        )
        .await;
        let reg = AdaptiveRetrievalRegistry::new();
        // Requirement 3 > single-round hits: round 1 gathers what it can.
        let r1 = reg.start(&engine, Some(session), "alpha", 3).await.unwrap();
        assert_eq!(r1.round_number, 1);
        assert!(!r1.complete, "3 required but round 1 found fewer");
        // Round 2 with a refined query gathers more exact evidence.
        let r2 = reg
            .next_round(&engine, Some(session), &r1.retrieval_id, Some("beta"))
            .await
            .unwrap()
            .expect("round 2");
        assert_eq!(r2.round_number, 2);
        assert!(
            r2.evidence_found.iter().any(|e| e.snippet.contains("beta")),
            "round 2 must surface the beta fact"
        );
        // Cumulative evidence: alpha (r1) + beta (r2) = 2 of 3 required.
        assert!(!r2.complete, "round 2 has 2/3 evidence, not yet complete");
        // Round 3 with gamma completes the 3-evidence requirement.
        let r3 = reg
            .next_round(&engine, Some(session), &r1.retrieval_id, Some("gamma"))
            .await
            .unwrap()
            .expect("round 3");
        assert_eq!(r3.round_number, 3);
        assert!(
            r3.evidence_found
                .iter()
                .any(|e| e.snippet.contains("gamma")),
            "round 3 must surface the gamma fact"
        );
        assert!(r3.complete, "round 3 completes the 3-evidence requirement");
    }

    #[tokio::test]
    async fn unknown_id_is_rejected_and_clear_resets() {
        let engine = test_engine();
        let reg = AdaptiveRetrievalRegistry::new();
        let r = reg
            .next_round(&engine, None, "does-not-exist", None)
            .await
            .unwrap();
        assert!(r.is_none(), "unknown id -> None, not an error");
        let round = reg.start(&engine, None, "alpha", 1).await.unwrap();
        reg.clear();
        let r2 = reg
            .next_round(&engine, None, &round.retrieval_id, None)
            .await
            .unwrap();
        assert!(r2.is_none(), "state cleared -> None");
    }

    #[tokio::test]
    async fn evidence_required_is_clamped() {
        let engine = test_engine();
        seed(
            &engine,
            "sess_clamp",
            &["Fact alpha one.", "Fact alpha two."],
        )
        .await;
        let reg = AdaptiveRetrievalRegistry::new();
        let round = reg
            .start(&engine, Some("sess_clamp"), "alpha", 99)
            .await
            .unwrap();
        assert_eq!(round.evidence_required, 5, "clamped to max");
    }
}
