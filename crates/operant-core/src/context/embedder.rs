//! Embedder abstraction for vector recall (hermes `embedding_provider.py`
//! parity, bounded port).
//!
//! hermes exposes an `EmbeddingProvider` protocol (`model_id`, `dim`,
//! `embed_documents`, `embed_query`) backed by Voyage/etc. and a heavyweight
//! `VectorStore` with int8 packing, identity hashing and provenance
//! verification. Here the contract is collapsed to one `embed` call so tests
//! can inject a deterministic mock; the store side (caching, cosine rank) is
//! `LcmContextEngine::vector_recall` (see `lcm.rs`).

use crate::client::OpenAIClient;
use crate::error::Result;

/// Produces dense vector embeddings for text.
#[async_trait::async_trait]
pub trait Embedder: Send + Sync {
    /// Embedding model id (recorded with cached vectors so a model change
    /// invalidates the cache).
    fn model_id(&self) -> &str;

    /// Embed each text; returns one vector per input, same order. The caller
    /// is responsible for batching (the engine batches at most 32 texts).
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
}

/// Real embedder backed by the OpenAI-compatible `/embeddings` endpoint.
pub struct OpenAIEmbedder {
    client: OpenAIClient,
    model: String,
}

impl OpenAIEmbedder {
    pub fn new(client: OpenAIClient, model: impl Into<String>) -> Self {
        Self {
            client,
            model: model.into(),
        }
    }
}

#[async_trait::async_trait]
impl Embedder for OpenAIEmbedder {
    fn model_id(&self) -> &str {
        &self.model
    }

    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        self.client.embeddings(&self.model, texts).await
    }
}

/// Cosine similarity in [0, 1] (vectors must share a dimension; 0 on
/// mismatch/empty so a bad vector never dominates ranking).
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f64;
    let mut na = 0.0f64;
    let mut nb = 0.0f64;
    for (x, y) in a.iter().zip(b) {
        dot += (*x as f64) * (*y as f64);
        na += (*x as f64) * (*x as f64);
        nb += (*y as f64) * (*y as f64);
    }
    let denom = (na.sqrt() * nb.sqrt()).max(f64::EPSILON);
    (dot / denom) as f32
}

/// Deterministic, token-overlap-sensitive mock embedder for tests. Two texts
/// sharing tokens get higher cosine similarity than unrelated texts, so the
/// ranking behavior is testable without a network.
pub struct MockEmbedder {
    pub dim: usize,
    pub calls: std::sync::atomic::AtomicUsize,
}

impl Default for MockEmbedder {
    fn default() -> Self {
        Self {
            dim: 32,
            calls: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

/// Hash-trick embedding: each alphanumeric token adds weight to a bucket and
/// its smoothed neighbor, then the vector is L2-normalized.
fn hash_embed(text: &str, dim: usize) -> Vec<f32> {
    let mut v = vec![0.0f32; dim];
    for token in text.to_lowercase().split(|c: char| !c.is_alphanumeric()) {
        if token.is_empty() {
            continue;
        }
        let idx = (fnv1a(token) % dim as u64) as usize;
        v[idx] += 1.0;
        v[(idx + 1) % dim] += 0.5;
    }
    let norm = v
        .iter()
        .map(|x| x * x)
        .sum::<f32>()
        .sqrt()
        .max(f32::EPSILON);
    v.iter_mut().for_each(|x| *x /= norm);
    v
}

#[async_trait::async_trait]
impl Embedder for MockEmbedder {
    fn model_id(&self) -> &str {
        "mock"
    }

    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        self.calls
            .fetch_add(texts.len(), std::sync::atomic::Ordering::Relaxed);
        Ok(texts.iter().map(|t| hash_embed(t, self.dim)).collect())
    }
}

/// Shared by the mock and the vector-recall tests.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;

    #[test]
    fn cosine_ranks_shared_tokens_above_unrelated() {
        let a = hash_embed("alpha beta gamma project", 32);
        let b = hash_embed("alpha beta gamma project", 32);
        let c = hash_embed("the weather is nice today", 32);
        let sim_same = cosine_similarity(&a, &b);
        let sim_unrelated = cosine_similarity(&a, &c);
        assert!(
            sim_same > 0.999,
            "identical text must be near-1, got {sim_same}"
        );
        assert!(
            sim_unrelated < sim_same,
            "unrelated text must rank lower: {sim_unrelated} < {sim_same}"
        );
        assert!(sim_unrelated > 0.0, "unrelated still has some overlap");
    }

    #[test]
    fn cosine_mismatched_dims_is_zero() {
        assert_eq!(cosine_similarity(&[1.0], &[1.0, 2.0]), 0.0);
        assert_eq!(cosine_similarity(&[], &[]), 0.0);
    }

    #[tokio::test]
    async fn mock_embedder_returns_normalized_vectors() {
        let mock = MockEmbedder::default();
        let out = mock
            .embed(&["hello".to_string(), "world".to_string()])
            .await
            .unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].len(), mock.dim);
        let norm: f32 = out[0].iter().map(|x| x * x).sum();
        assert!(
            (norm - 1.0).abs() < 1e-3,
            "vectors are normalized, got {norm}"
        );
        assert_eq!(
            mock.calls.load(std::sync::atomic::Ordering::Relaxed),
            2,
            "each embedded text counts a call"
        );
    }

    #[test]
    fn error_type_is_constructible_for_embedder() {
        // The embedder surface returns crate Result; sanity-check the error
        // type flows (prevents accidental Result alias drift).
        let _: Result<Vec<Vec<f32>>> = Err(Error::Agent("embedder: test error".to_string()));
    }
}
