//! Hybrid retrieval scoring + injectable reranking.
//!
//! ## What retrieval looked like before this module
//!
//! The only memory provider that does real recall is `agentmemory`, reached
//! over REST by `AgentMemoryProvider::prefetch`
//! (`src/agent_memory.rs:548`), which POSTs `smart-search` and hands the
//! response to `format_search_results` (`src/agent_memory.rs:419`). That
//! formatter iterates `results.take(10)` **in whatever order the server
//! returned** and prints `- {text}` per hit. There is no score parsing, no
//! ordering, and no reranking on the operant side: ranking is 100% the
//! server's BM25+vector order, trusted verbatim, and the top-5 `limit` that
//! `prefetch` sends is applied *before* anything operant could reorder.
//!
//! So the ranking that exists today is keyword/BM25 order from a remote
//! service, with no local semantic contribution and no second pass. This module
//! is that missing pass, kept separate from the provider so it can be tested
//! without a server.
//!
//! ## What this adds
//!
//! 1. [`score_candidates`] — a **hybrid** score: keyword overlap and embedding
//!    cosine combined, instead of keyword order alone.
//! 2. [`rerank`] — an **injectable** second pass behind the [`Reranker`]
//!    trait, with a listwise LLM implementation and a deterministic
//!    implementation for tests and offline runs.
//!
//! Both degrade to the pre-existing behaviour: no embedder ⇒ keyword-only
//! scores, no reranker (or a reranker that errors) ⇒ input order preserved.

use std::sync::Arc;

use crate::client::{Message, OpenAIClient};
use crate::context::Embedder;
use crate::error::Result;

// ---------------------------------------------------------------------------
// Candidates + hybrid scoring
// ---------------------------------------------------------------------------

/// One retrievable item handed in by the caller (a memory row, a tool result,
/// a context node). `id` is opaque to the scorer and carried through so a
/// reranked result can be mapped back to its source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub id: String,
    pub text: String,
}

impl Candidate {
    pub fn new(id: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            text: text.into(),
        }
    }
}

/// A candidate with both component scores and their combination.
#[derive(Debug, Clone, PartialEq)]
pub struct ScoredDoc {
    pub candidate: Candidate,
    /// Normalized query-term containment in [0, 1].
    pub keyword: f32,
    /// Cosine against the query embedding, clamped to [0, 1]. `0.0` when no
    /// embedder was available — never a fabricated number.
    pub semantic: f32,
    /// The combined score actually used for ordering.
    pub hybrid: f32,
}

/// Relative weight of the two component scores.
///
/// The two components are on the same [0, 1] scale (containment and clamped
/// cosine), which is what makes a plain weighted sum defensible: no
/// normalization step, no score-probing, one multiplication per pair.
///
/// **These weights are CHOSEN, not tuned, and the measurement does not support
/// them.** The reasoning was that the semantic side is the one that can match a
/// rephrased query — the exact property this change exists to add — while the
/// keyword side is exact and high-precision on a literal term match. 0.6/0.4
/// leans semantic without letting it dominate outright.
///
/// The measured outcome with the only embedder available offline
/// (`MockEmbedder`, a hash trick that scores token overlap) is that 0.6 on the
/// semantic term is actively harmful: it promotes lexically-dense distractors
/// over real hits, and it *lowers* R@3 on a paraphrase query. See the honesty
/// note and `hybrid_does_not_improve_recall_with_the_offline_embedder` in the
/// test module. Treat these weights as a placeholder for a weight sweep against
/// a real embedder and a real query log, not as a tuned value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HybridWeights {
    pub keyword: f32,
    pub semantic: f32,
}

/// Keyword-leaning but not keyword-only. See [`HybridWeights`] for why these
/// are a placeholder chosen from reasoning rather than tuned — and for the
/// measurement that does not support them.
pub const DEFAULT_HYBRID_WEIGHTS: HybridWeights = HybridWeights {
    keyword: 0.4,
    semantic: 0.6,
};

impl HybridWeights {
    /// Keyword-only — the pre-existing behaviour, for A/B measurement.
    pub fn keyword_only() -> Self {
        Self {
            keyword: 1.0,
            semantic: 0.0,
        }
    }

    /// Semantic-only — the upper arm of the sensitivity check.
    pub fn semantic_only() -> Self {
        Self {
            keyword: 0.0,
            semantic: 1.0,
        }
    }
}

/// Lowercase alphanumeric tokens of length > 1.
fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() > 1)
        .map(str::to_string)
        .collect()
}

/// Keyword score: **query-term containment** — the share of the query's
/// distinct terms that appear in the document.
///
/// Containment (`|Q ∩ D| / |Q|`) rather than Jaccard or cosine-on-bigrams
/// because it needs no corpus: a real BM25 needs document frequencies, and no
/// corpus is available at this seam. Normalizing by the *query* side means a
/// long document is not punished for being long, which is the right bias when
/// the query is 5 words and the documents are whole memory rows.
///
/// Returns `0.0` for an empty query — an unanswerable question must not be
/// scored as a match.
pub fn keyword_score(query: &str, doc: &str) -> f32 {
    let q: std::collections::BTreeSet<String> = tokenize(query).into_iter().collect();
    if q.is_empty() {
        return 0.0;
    }
    let d: std::collections::BTreeSet<String> = tokenize(doc).into_iter().collect();
    let hits = q.iter().filter(|t| d.contains(*t)).count();
    hits as f32 / q.len() as f32
}

/// Combine the two component scores as a weighted sum.
///
/// `semantic: None` means "no embedding was available". The weights are then
/// renormalized to `1.0` so a keyword-only run is not silently scaled down by
/// 0.4 and ranked below documents that were scored with an embedder — mixing
/// the two kinds of score in one ordering without renormalizing would be a
/// real bug, not a rounding detail.
pub fn hybrid_score(keyword: f32, semantic: Option<f32>, weights: HybridWeights) -> f32 {
    let sem = semantic.unwrap_or(0.0);
    let (kw, sw) = if semantic.is_some() {
        (weights.keyword, weights.semantic)
    } else {
        // Renormalize: the keyword component carries the whole weight, or
        // keyword-only docs would be ranked below embedded ones.
        let total = weights.keyword + weights.semantic;
        if total <= 0.0 {
            return keyword;
        }
        (weights.keyword / total, 0.0)
    };
    let total = kw + sw;
    if total <= 0.0 {
        return keyword;
    }
    (kw * keyword + sw * sem) / total
}

/// Score every candidate against the query.
///
/// `embedder: None`, an empty query, an embedding error, or a vector that is
/// empty / non-finite / the wrong width all leave `semantic` at `0.0` and
/// renorm to keyword-only. The result is sorted by `hybrid` descending with a
/// stable sort, so equal scores keep the caller's order.
pub async fn score_candidates(
    query: &str,
    candidates: &[Candidate],
    embedder: Option<&dyn Embedder>,
    weights: HybridWeights,
) -> Vec<ScoredDoc> {
    let semantic: Option<Vec<f32>> = match embedder {
        Some(e) if !query.trim().is_empty() && !candidates.is_empty() => {
            let mut texts = Vec::with_capacity(candidates.len() + 1);
            texts.push(query.trim().to_string());
            texts.extend(candidates.iter().map(|c| c.text.clone()));
            match e.embed(&texts).await {
                Ok(v) if v.len() == candidates.len() + 1 => {
                    let q = &v[0];
                    if q.is_empty() || !q.iter().all(|x| x.is_finite()) {
                        None
                    } else {
                        Some(
                            v[1..]
                                .iter()
                                .map(|d| {
                                    if d.len() == q.len() && d.iter().all(|x| x.is_finite()) {
                                        crate::context::embedder::cosine_similarity(q, d).max(0.0)
                                    } else {
                                        0.0
                                    }
                                })
                                .collect(),
                        )
                    }
                }
                Ok(_) => None,
                Err(err) => {
                    tracing::debug!(
                        error = %err,
                        "retrieval: embed failed, scoring keyword-only"
                    );
                    None
                }
            }
        }
        _ => None,
    };

    let mut out: Vec<ScoredDoc> = candidates
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let keyword = keyword_score(query, &c.text);
            let sem = semantic.as_ref().map(|s| s[i]);
            ScoredDoc {
                candidate: c.clone(),
                keyword,
                semantic: sem.unwrap_or(0.0),
                hybrid: hybrid_score(keyword, sem, weights),
            }
        })
        .collect();
    out.sort_by(|a, b| {
        b.hybrid
            .partial_cmp(&a.hybrid)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}

// ---------------------------------------------------------------------------
// Rerank seam
// ---------------------------------------------------------------------------

/// A second-pass reorderer over already-scored results.
///
/// Returns a **permutation of indices into `docs`, best first** — indices, not
/// documents, so an implementation cannot reorder by mutating content and
/// cannot silently drop or duplicate a hit. [`rerank`] validates the
/// permutation and falls back to the input order if it is not one, so a
/// misbehaving implementation degrades instead of corrupting recall.
#[async_trait::async_trait]
pub trait Reranker: Send + Sync {
    /// Identity for diagnostics ("deterministic", "llm-listwise").
    fn name(&self) -> &str;
    /// One call per query. Reordering only — never scoring, never filtering.
    async fn rerank(&self, query: &str, docs: &[ScoredDoc]) -> Result<Vec<usize>>;
}

/// Deterministic reranker: order by `hybrid`, ties keep input order.
///
/// This is the offline / test implementation and the honest default. It cannot
/// improve on the hybrid score it is given — it only makes the ordering
/// explicit and stable, which is a real property (the pre-rerank path depends
/// on the server having sorted correctly) but is *not* a relevance gain. The
/// relevance gain is supposed to come from [`LlmListwiseReranker`], which is
/// **unmeasured** — see the `measure_*` tests and the module report.
#[derive(Debug, Clone, Copy, Default)]
pub struct DeterministicReranker;

#[async_trait::async_trait]
impl Reranker for DeterministicReranker {
    fn name(&self) -> &str {
        "deterministic"
    }

    async fn rerank(&self, _query: &str, docs: &[ScoredDoc]) -> Result<Vec<usize>> {
        // `sort_by` is stable, so equal scores keep input order and a tie
        // never reshuffles results between runs.
        let mut idx: Vec<usize> = (0..docs.len()).collect();
        idx.sort_by(|&a, &b| {
            docs[b]
                .hybrid
                .partial_cmp(&docs[a].hybrid)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(idx)
    }
}

/// Where the listwise rerank stops spending.
///
/// A listwise rerank is **one model call per query** whose prompt carries every
/// candidate. With `n` documents of `snippet_chars` each, the input is
/// `O(n * snippet_chars)` tokens and the output is a permutation of `n`
/// indices. Both are bounded here:
/// - `enabled == false` ⇒ no call at all;
/// - `docs.len() > max_docs` ⇒ no call, input order returned (the win from
///   reranking 5 items does not pay for a call; past `max_docs` the prompt
///   grows faster than the ranking improves);
/// - `snippet_chars` caps each item's contribution.
pub struct RerankBudget {
    pub enabled: bool,
    pub max_docs: usize,
    pub snippet_chars: usize,
}

impl Default for RerankBudget {
    fn default() -> Self {
        // 20 items x 512 chars ≈ 2.5k input tokens + ~120 output tokens. One
        // call per turn. Chosen to cover a typical memory-recall result set
        // (the provider asks for 5) with headroom, and to stop the prompt
        // running away on a large recall.
        Self {
            enabled: true,
            max_docs: 20,
            snippet_chars: 512,
        }
    }
}

/// Listwise LLM reranker: one call, the model returns the full ordering.
///
/// The client and model are injected so the same credentials that drive the
/// agent also drive reranking — no second credential seam, matching
/// `context::assertion_extract::LlmAssertionExtractor`.
pub struct LlmListwiseReranker {
    client: Arc<OpenAIClient>,
    model: String,
    budget: RerankBudget,
}

impl LlmListwiseReranker {
    pub fn new(client: Arc<OpenAIClient>, model: impl Into<String>, budget: RerankBudget) -> Self {
        Self {
            client,
            model: model.into(),
            budget,
        }
    }
}

/// Extract the first balanced `[...]` array from a possibly prose-wrapped
/// model response. Returns the slice, or `None` when there is no array.
fn first_json_array(raw: &str) -> Option<&str> {
    let start = raw.find('[')?;
    let mut depth = 0i32;
    // `char_indices` is relative to the slice, so the closing offset has to be
    // added back onto `start` before indexing `raw`.
    for (i, c) in raw[start..].char_indices() {
        match c {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&raw[start..=start + i]);
                }
            }
            _ => {}
        }
    }
    None
}

#[async_trait::async_trait]
impl Reranker for LlmListwiseReranker {
    fn name(&self) -> &str {
        "llm-listwise"
    }

    async fn rerank(&self, query: &str, docs: &[ScoredDoc]) -> Result<Vec<usize>> {
        let identity: Vec<usize> = (0..docs.len()).collect();
        if !self.budget.enabled || docs.is_empty() {
            return Ok(identity);
        }
        if docs.len() > self.budget.max_docs {
            tracing::debug!(
                docs = docs.len(),
                max = self.budget.max_docs,
                "listwise rerank skipped: over budget"
            );
            return Ok(identity);
        }

        let mut prompt = String::from(
            "Rank the numbered documents by how well they answer the question. \
             Output ONLY a JSON array of the document numbers, best first.\n\n\
             Question: ",
        );
        prompt.push_str(query.trim());
        prompt.push_str("\n\nDocuments:\n");
        for (i, d) in docs.iter().enumerate() {
            prompt.push_str(&format!(
                "{}. {}\n",
                i + 1,
                d.candidate
                    .text
                    .chars()
                    .take(self.budget.snippet_chars)
                    .collect::<String>()
            ));
        }

        // 1024 is enough for a permutation of `max_docs` integers plus a
        // reasoning model's preamble; the ordering is the entire answer.
        let resp = match self
            .client
            .chat(
                &self.model,
                &[
                    Message::system(
                        "You are a precise reranker. Reply with a JSON array of integers only.",
                    ),
                    Message::user(prompt),
                ],
                None,
                Some(1024),
                Some(0.0),
            )
            .await
        {
            Ok(r) => r,
            Err(e) => {
                // A failed rerank must not fail the turn.
                tracing::debug!(error = %e, "listwise rerank call failed, keeping input order");
                return Ok(identity);
            }
        };

        let text = resp
            .choices
            .first()
            .map(|c| {
                c.message
                    .content
                    .clone()
                    .filter(|s| !s.trim().is_empty())
                    .or_else(|| c.message.reasoning_content.clone())
                    .unwrap_or_default()
            })
            .unwrap_or_default();

        let Some(arr) =
            first_json_array(&text).and_then(|s| serde_json::from_str::<Vec<i64>>(s).ok())
        else {
            tracing::debug!(
                raw_len = text.len(),
                "listwise rerank: no parseable ordering"
            );
            return Ok(identity);
        };

        // A partial or duplicated ordering is completed, not obeyed: keep the
        // indices the model gave (deduped, in its order) and append whatever it
        // omitted in input order. Reranking reorders; it never drops a hit.
        let mut out: Vec<usize> = Vec::with_capacity(docs.len());
        let mut seen = vec![false; docs.len()];
        for v in arr {
            if v >= 1 {
                let idx = (v - 1) as usize;
                if idx < docs.len() && !seen[idx] {
                    seen[idx] = true;
                    out.push(idx);
                }
            }
        }
        for (idx, was_seen) in seen.iter().enumerate() {
            if !was_seen {
                out.push(idx);
            }
        }
        Ok(out)
    }
}

/// Apply a reranker to already-scored results.
///
/// `None` ⇒ input order, unchanged. A reranker that returns `Err` or a
/// permutation that is not a permutation of `0..docs.len()` ⇒ input order,
/// unchanged. The returned vector is always a permutation of the input, so no
/// implementation can drop or duplicate a memory.
pub async fn rerank(
    reranker: Option<&dyn Reranker>,
    query: &str,
    docs: Vec<ScoredDoc>,
) -> Vec<ScoredDoc> {
    let Some(r) = reranker else {
        return docs;
    };
    let n = docs.len();
    let order = match r.rerank(query, &docs).await {
        Ok(o) => o,
        Err(e) => {
            tracing::debug!(error = %e, reranker = r.name(), "rerank failed, keeping input order");
            return docs;
        }
    };
    let is_permutation = order.len() == n && {
        let mut seen = vec![false; n];
        let mut ok = true;
        for &i in &order {
            if i >= n || seen[i] {
                ok = false;
                break;
            }
            seen[i] = true;
        }
        ok
    };
    if !is_permutation {
        tracing::warn!(
            reranker = r.name(),
            got = order.len(),
            want = n,
            "reranker returned a non-permutation, keeping input order"
        );
        return docs;
    }
    order
        .into_iter()
        .filter_map(|i| docs.get(i).cloned())
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::MockEmbedder;

    fn docs(specs: &[(&str, &str, f32)]) -> Vec<ScoredDoc> {
        specs
            .iter()
            .map(|(id, text, hybrid)| ScoredDoc {
                candidate: Candidate::new(*id, *text),
                keyword: *hybrid,
                semantic: 0.0,
                hybrid: *hybrid,
            })
            .collect()
    }

    #[test]
    fn keyword_score_is_query_term_containment() {
        // Full match, partial match, no match, empty query.
        assert_eq!(
            keyword_score("deploy pipeline", "we fixed the deploy pipeline"),
            1.0
        );
        assert_eq!(keyword_score("deploy pipeline", "deploy notes only"), 0.5);
        assert_eq!(
            keyword_score("deploy pipeline", "unrelated weather text"),
            0.0
        );
        assert_eq!(keyword_score("   ", "anything"), 0.0);
    }

    #[test]
    fn hybrid_score_should_combine_keyword_and_semantic() {
        let w = DEFAULT_HYBRID_WEIGHTS;
        // A doc with perfect keyword but no semantic relevance must lose to
        // one with the reverse profile — that is the entire point of combining.
        let keyword_only_hit = hybrid_score(1.0, Some(0.0), w);
        let semantic_only_hit = hybrid_score(0.0, Some(1.0), w);
        assert!(
            (keyword_only_hit - 0.4).abs() < 1e-6,
            "got {keyword_only_hit}"
        );
        assert!(
            (semantic_only_hit - 0.6).abs() < 1e-6,
            "got {semantic_only_hit}"
        );
        assert!(semantic_only_hit > keyword_only_hit);
        // Both components present ⇒ the weighted mean of the two.
        let both = hybrid_score(1.0, Some(0.5), w);
        assert!((both - 0.7).abs() < 1e-6, "got {both}");
    }

    #[test]
    fn hybrid_score_renormalizes_when_no_embedding_is_available() {
        let w = DEFAULT_HYBRID_WEIGHTS;
        // A *missing* semantic score is not the same as a *measured* zero, and
        // the two must not be conflated: `None` means the embedding endpoint
        // was unavailable, so the keyword component carries the whole weight
        // and a perfect keyword hit still scores 1.0. Without that, a
        // keyword-only document would be ranked below every embedded document
        // purely because of which pipeline produced it.
        let missing = hybrid_score(0.5, None, w);
        assert!((missing - 0.5).abs() < 1e-6, "got {missing}");
        assert!((hybrid_score(1.0, None, w) - 1.0).abs() < 1e-6);

        // `Some(0.0)` is a real measurement of "no similarity" and is weighted
        // like any other value, so it scores strictly below the missing case.
        let measured_zero = hybrid_score(0.5, Some(0.0), w);
        assert!((measured_zero - 0.2).abs() < 1e-6, "got {measured_zero}");
        assert!(measured_zero < missing);
    }

    #[tokio::test]
    async fn score_candidates_degrades_to_keyword_only_without_embedder() {
        let candidates = vec![
            Candidate::new("a", "the deploy pipeline is broken"),
            Candidate::new("b", "unrelated weather"),
        ];
        let scored =
            score_candidates("deploy pipeline", &candidates, None, DEFAULT_HYBRID_WEIGHTS).await;
        assert_eq!(scored[0].candidate.id, "a");
        assert_eq!(scored[1].candidate.id, "b");
        assert!(scored.iter().all(|d| d.semantic == 0.0));
        // Renormalized: a full keyword match is 1.0, not 0.4.
        assert!(
            (scored[0].hybrid - 1.0).abs() < 1e-6,
            "got {}",
            scored[0].hybrid
        );
    }

    #[tokio::test]
    async fn score_candidates_uses_the_embedder_when_present() {
        let candidates = vec![
            Candidate::new("a", "the deploy pipeline is broken"),
            Candidate::new("b", "unrelated weather"),
        ];
        let mock = MockEmbedder::default();
        let scored = score_candidates(
            "deploy pipeline",
            &candidates,
            Some(&mock),
            DEFAULT_HYBRID_WEIGHTS,
        )
        .await;
        assert_eq!(scored.len(), 2);
        assert!(scored[0].candidate.id == "a" && scored[1].candidate.id == "b");
    }

    /// A reranker that records what it saw, proving the seam is injectable and
    /// that `rerank` actually consults the injected implementation.
    struct RecordingReranker {
        calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        order: Vec<usize>,
    }

    #[async_trait::async_trait]
    impl Reranker for RecordingReranker {
        fn name(&self) -> &str {
            "recording"
        }
        async fn rerank(&self, _query: &str, docs: &[ScoredDoc]) -> Result<Vec<usize>> {
            self.calls
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            assert!(!docs.is_empty(), "reranker must see the candidates");
            Ok(self.order.clone())
        }
    }

    #[tokio::test]
    async fn rerank_should_be_injectable_and_deterministic_under_test() {
        let input = docs(&[("a", "alpha", 0.1), ("b", "beta", 0.9), ("c", "gamma", 0.5)]);
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

        // (1) `None` reranker ⇒ untouched input order.
        let none = rerank(None, "q", input.clone()).await;
        assert_eq!(
            none.iter()
                .map(|d| d.candidate.id.as_str())
                .collect::<Vec<_>>(),
            ["a", "b", "c"]
        );

        // (2) Injected reranker is called and its permutation is honoured.
        let injected = rerank(
            Some(&RecordingReranker {
                calls: calls.clone(),
                order: vec![1, 2, 0],
            }),
            "q",
            input.clone(),
        )
        .await;
        assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 1);
        assert_eq!(
            injected
                .iter()
                .map(|d| d.candidate.id.as_str())
                .collect::<Vec<_>>(),
            ["b", "c", "a"]
        );

        // (3) The deterministic implementation is the offline default, and
        //     repeated runs on identical input give identical output.
        let det = DeterministicReranker;
        let first = rerank(Some(&det), "q", input.clone()).await;
        let second = rerank(Some(&det), "q", input.clone()).await;
        assert_eq!(first, second);
        assert_eq!(
            first
                .iter()
                .map(|d| d.candidate.id.as_str())
                .collect::<Vec<_>>(),
            ["b", "c", "a"]
        );
    }

    #[tokio::test]
    async fn rerank_should_not_reorder_when_scores_are_equal() {
        let input = docs(&[
            ("a", "first", 0.5),
            ("b", "second", 0.5),
            ("c", "third", 0.5),
            ("d", "fourth", 0.5),
        ]);
        let det = DeterministicReranker;
        let out = rerank(Some(&det), "q", input.clone()).await;
        assert_eq!(
            out.iter()
                .map(|d| d.candidate.id.as_str())
                .collect::<Vec<_>>(),
            ["a", "b", "c", "d"],
            "an all-ties ordering must be the identity, not an arbitrary shuffle"
        );
        // Same query, two runs ⇒ same ordering.
        let again = rerank(Some(&det), "q", input.clone()).await;
        assert_eq!(out, again);
    }

    /// A reranker that drops a document must not be able to lose a memory.
    #[tokio::test]
    async fn rerank_rejects_a_non_permutation() {
        let input = docs(&[("a", "alpha", 0.1), ("b", "beta", 0.9)]);
        let out = rerank(
            Some(&RecordingReranker {
                calls: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                order: vec![1], // dropped "a"
            }),
            "q",
            input.clone(),
        )
        .await;
        assert_eq!(
            out, input,
            "a short permutation must be refused, not obeyed"
        );
    }

    #[tokio::test]
    async fn llm_reranker_respects_its_budget_without_a_model_call() {
        // A client pointed at an unroutable address: if the budget guard were
        // bypassed, `rerank` would try to dial it and the test would be slow
        // and flaky instead of deterministic. The guard returns first, so no
        // request is ever attempted and the ordering is the input ordering.
        let client = Arc::new(OpenAIClient::new(crate::client::ClientConfig {
            base_url: "http://127.0.0.1:1/v1".to_string(),
            ..Default::default()
        }));

        // (a) disabled entirely.
        let disabled = LlmListwiseReranker::new(
            client.clone(),
            "never-called",
            RerankBudget {
                enabled: false,
                ..RerankBudget::default()
            },
        );
        let input = docs(&[("a", "alpha", 0.1), ("b", "beta", 0.9)]);
        assert_eq!(rerank(Some(&disabled), "q", input.clone()).await, input);

        // (b) over `max_docs` — the call is skipped, not truncated.
        let tiny = LlmListwiseReranker::new(
            client,
            "never-called",
            RerankBudget {
                enabled: true,
                max_docs: 1,
                snippet_chars: 64,
            },
        );
        assert_eq!(rerank(Some(&tiny), "q", input.clone()).await, input);
    }

    #[test]
    fn first_json_array_handles_fences_prose_and_absence() {
        assert_eq!(first_json_array("```json\n[3,1,2]\n```"), Some("[3,1,2]"));
        assert_eq!(
            first_json_array("Here you go: [2, 1] — hope that helps"),
            Some("[2, 1]")
        );
        assert_eq!(first_json_array("no array here"), None);
        assert_eq!(first_json_array("[1, [2, 3], 4]"), Some("[1, [2, 3], 4]"));
    }

    // -----------------------------------------------------------------------
    // Measurement — clearly-labelled SYNTHETIC fixture
    // -----------------------------------------------------------------------
    //
    // HONESTY NOTE, read before believing any number below.
    //
    // This fixture is SYNTHETIC and I wrote it. It is not a real query log, not
    // drawn from operant sessions, and not relevance-judged by anyone but me. It
    // is a mechanism check: does adding a semantic term to the score change
    // the ranking, and does that change the metric?
    //
    // The MEASURED RESULT, on the only embedder available offline, is
    // NEGATIVE and is recorded as such:
    //
    //   arm A (query terms present in the good docs)
    //     R@1 1/5 -> 1/5   R@3 3/5 -> 3/5   MRR 1.000 -> 1.000
    //     the ordering DID change, the metric did not move.
    //   arm B (query is a paraphrase; only meaning connects it to the goods)
    //     R@1 1/5 -> 1/5   R@3 2/5 -> 1/5   MRR 1.000 -> 1.000
    //     the semantic term made recall WORSE at 0.4/0.6, and semantic-only
    //     made it worse still (R@1 0/5, MRR 0.500).
    //
    // The mechanism is not bad luck. The offline embedder is `MockEmbedder`, a
    // **hash trick** (`context/embedder.rs:126`) that scores *token overlap* —
    // the same signal the keyword score already uses. Combined with the
    // keyword score it is not extra information, only a re-weighting toward
    // query-dense documents, and a query-dense document can be a distractor
    // (`d-2` "pipeline of water through the old castle walls" outranks
    // `a-exact-2` once the semantic term is weighted at 0.6). The paraphrase
    // arm is precisely where a real embedding model would pay off, and the hash
    // trick structurally cannot: it has no meaning to draw on.
    //
    // This is the same negative result jcode's audit recorded for a local
    // cross-encoder reranker — recall did not improve. Recording that is the
    // correct outcome, not a failed writeup.
    //
    // CONCLUSION, stated plainly: **hybrid retrieval scoring is UNPROVEN on the
    // only embedder this repository can run without a network, and on the
    // paraphrase arm it measurably hurts.** It is implemented, tested,
    // injectable and correctly degrading, but it must NOT be enabled in the
    // prefetch path on the strength of this evidence. Enabling it requires
    // `OpenAIEmbedder` (a real embedding model) plus a re-run of
    // `measure_hybrid_scoring_on_synthetic_fixture` against a real query log
    // and a weight sweep — at which point the weights below are the *first
    // thing to re-tune*, since they were chosen blind.
    //
    // The `reordered` column in the output is the one that matters: if it is
    // false, the semantic term contributed nothing, whatever recall says.

    /// (id, text, is_relevant)
    const FIXTURE: &[(&str, &str, bool)] = &[
        // --- relevant, sharing terms with query A ("deploy pipeline") ---
        (
            "a-exact-1",
            "the deploy pipeline runs release.sh over ssh",
            true,
        ),
        (
            "a-exact-2",
            "deploy pipeline retry policy after a failed stage",
            true,
        ),
        (
            "a-exact-3",
            "the deploy pipeline log is written to /var/log/deploy",
            true,
        ),
        // --- relevant by MEANING only; no query-A term appears ---
        (
            "a-para-1",
            "shipping the built artifact to the production host",
            true,
        ),
        (
            "a-para-2",
            "the release copies files to the target machine",
            true,
        ),
        // --- distractors, lexically noisy on query A ---
        (
            "d-1",
            "deploy notes: the weather forecast is irrelevant here",
            false,
        ),
        (
            "d-2",
            "pipeline of water through the old castle walls",
            false,
        ),
        ("d-3", "a release without notes is not supported", false),
        ("d-4", "cooking pasta requires salt and patience", false),
        (
            "d-5",
            "the server room temperature is cold in winter",
            false,
        ),
        ("d-6", "ssh config for the jump host lives in ~/.ssh", false),
        ("d-7", "retry storms can overload the machine", false),
    ];

    const QUERY_A: &str = "deploy pipeline";
    const QUERY_B: &str = "how does the built code get onto the production host";

    /// Reciprocal rank of the first relevant hit (0.0 when there is none).
    fn reciprocal_rank(order: &[String]) -> f64 {
        match order
            .iter()
            .position(|id| FIXTURE.iter().any(|(i, _, r)| *i == id.as_str() && *r))
        {
            Some(r) => 1.0 / (r + 1) as f64,
            None => 0.0,
        }
    }

    fn hits_at(k: usize, order: &[String]) -> usize {
        order
            .iter()
            .take(k)
            .filter(|id| FIXTURE.iter().any(|(i, _, r)| *i == id.as_str() && *r))
            .count()
    }

    async fn fixture_order(
        query: &str,
        embedder: Option<&dyn Embedder>,
        w: HybridWeights,
    ) -> Vec<String> {
        let candidates: Vec<Candidate> = FIXTURE
            .iter()
            .map(|(id, text, _)| Candidate::new(*id, *text))
            .collect();
        let scored = score_candidates(query, &candidates, embedder, w).await;
        let det = DeterministicReranker;
        rerank(Some(&det), query, scored)
            .await
            .iter()
            .map(|d| d.candidate.id.clone())
            .collect()
    }

    /// Reports, for each query and each weighting, the retrieval metric with
    /// and without the semantic term, plus whether the hybrid ordering
    /// actually *differs* from the keyword-only ordering.
    #[tokio::test]
    async fn measure_hybrid_scoring_on_synthetic_fixture() {
        let mock = MockEmbedder::default();
        let total = FIXTURE.iter().filter(|(_, _, r)| *r).count();
        for (arm, query) in [("A-term", QUERY_A), ("B-paraphrase", QUERY_B)] {
            for (label, w) in [
                ("keyword-only", HybridWeights::keyword_only()),
                ("hybrid 0.4/0.6", DEFAULT_HYBRID_WEIGHTS),
                ("semantic-only", HybridWeights::semantic_only()),
            ] {
                let kw = fixture_order(query, None, w).await;
                let hy = fixture_order(query, Some(&mock), w).await;
                println!(
                    "MEASURE arm={arm:<13} weights={label:<15} of {total} relevant  \
                     R@1 {}/{} -> {}/{}   R@3 {}/{} -> {}/{}   MRR {:.3} -> {:.3}   \
                     reordered={}   top5={:?}",
                    hits_at(1, &kw),
                    total,
                    hits_at(1, &hy),
                    total,
                    hits_at(3, &kw),
                    total,
                    hits_at(3, &hy),
                    total,
                    reciprocal_rank(&kw),
                    reciprocal_rank(&hy),
                    kw != hy,
                    hy.iter().take(5).collect::<Vec<_>>()
                );
            }
        }
    }

    /// Pins the measured outcome so it cannot be quietly rewritten as an
    /// improvement: with the in-tree hash-trick embedder, the semantic term
    /// does **not** improve the retrieval metric, and on the paraphrase arm it
    /// degrades it.
    ///
    /// This asserts a NON-improvement on purpose. If a real embedding model
    /// (`OpenAIEmbedder`) is ever wired into this path, these assertions fail
    /// — which is the intended alarm: at that point the metric must be
    /// re-measured on a real query log, [`HybridWeights`] must be re-tuned
    /// from measurements instead of from reasoning, and this test replaced
    /// with one that asserts the improvement that was actually observed.
    #[tokio::test]
    async fn hybrid_does_not_improve_recall_with_the_offline_embedder() {
        let mock = MockEmbedder::default();
        for (arm, query) in [("A-term", QUERY_A), ("B-paraphrase", QUERY_B)] {
            let kw = fixture_order(query, None, HybridWeights::keyword_only()).await;
            let hy = fixture_order(query, Some(&mock), DEFAULT_HYBRID_WEIGHTS).await;
            println!(
                "MEASURE no-improvement check arm={arm:<13} \
                 R@3 {} -> {}   MRR {:.3} -> {:.3}   reordered={}",
                hits_at(3, &kw),
                hits_at(3, &hy),
                reciprocal_rank(&kw),
                reciprocal_rank(&hy),
                kw != hy
            );
            assert!(
                hits_at(3, &hy) <= hits_at(3, &kw),
                "arm {arm}: hybrid improved R@3 from {} to {} with the hash \
                 embedder — the honesty note in this module is now wrong and the \
                 weights must be re-measured, not defended",
                hits_at(3, &kw),
                hits_at(3, &hy)
            );
            assert!(
                reciprocal_rank(&hy) <= reciprocal_rank(&kw),
                "arm {arm}: hybrid improved MRR with the hash embedder — same \
                 consequence: re-measure, then re-tune the weights"
            );
        }
    }

    // -----------------------------------------------------------------------
    // The UNPROVEN gate
    // -----------------------------------------------------------------------

    /// Hybrid retrieval scoring is UNPROVEN, and on the paraphrase arm it
    /// measurably HURTS (see the module's conclusion and
    /// `measure_hybrid_scoring_on_synthetic_fixture`). The module doc says it
    /// must not be enabled in the prefetch path. A comment is not a gate,
    /// though — a future contributor wiring it up would hit a wall of prose at
    /// the definition site and keep scrolling.
    ///
    /// This is the runnable form of "grep for a caller": it fails the moment any
    /// file outside `retrieval.rs` reaches for `score_candidates` or `rerank`.
    /// Enabling it deliberately means deleting this test, which shows up in the
    /// diff — and a diff that deletes the warning is a diff a reviewer can see.
    #[test]
    fn hybrid_retrieval_should_have_no_production_caller() {
        let sources: [(&str, &str); 6] = [
            ("memory_provider.rs", include_str!("memory_provider.rs")),
            ("agent_memory.rs", include_str!("agent_memory.rs")),
            ("agent/mod.rs", include_str!("agent/mod.rs")),
            ("agent/run.rs", include_str!("agent/run.rs")),
            ("agent/stream.rs", include_str!("agent/stream.rs")),
            ("agent/builders.rs", include_str!("agent/builders.rs")),
        ];
        for (name, src) in sources {
            assert!(
                !src.contains("retrieval::score_candidates"),
                "{name} now calls retrieval::score_candidates. Hybrid scoring is \
                 UNPROVEN and hurts on the paraphrase arm. Re-run \
                 measure_hybrid_scoring_on_synthetic_fixture against a REAL query \
                 log with a real embedding model, re-tune DEFAULT_HYBRID_WEIGHTS \
                 (they were chosen blind), and delete this test in the same commit \
                 so the change is visible in review."
            );
            assert!(
                !src.contains("retrieval::rerank"),
                "{name} now calls retrieval::rerank. The LLM listwise reranker is \
                 UNMEASURED — it cannot be measured without a live model budget. \
                 Establish a recall@k improvement first, and delete this test in \
                 the same commit so the change is visible in review."
            );
        }
    }
}
