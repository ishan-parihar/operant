//! Client-layer per-provider credential rotation.
//!
//! [`PooledModelClient`] wraps a [`ModelClient`] together with the provider's
//! [`CredentialPool`] and performs the hermes `mark_exhausted_and_rotate`
//! loop *inside* the client: on a classified credential error (429 rate
//! limit, 401 auth, billing), the key that failed is marked exhausted (with
//! an error-class cooldown — 401 → 5m, 429 → 1h, sole-credential transient →
//! 60s) and the next available key from the pool is tried, bounded by the
//! pool size. When every key is benched, the original error propagates so
//! the outer layers (model fallback chain / provider registry / agent loop)
//! can switch providers — the same composition hermes uses (pool rotation
//! first, `load_pool(fb_provider)` on provider switch).
//!
//! Key selection + runtime key swap on the shared inner client are
//! serialized under an internal mutex so concurrent callers can't race the
//! client's runtime key.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use futures::stream::BoxStream;
use tracing::warn;

use super::fallback::FallbackModelClient;
use super::model_client::{ChatRequest, ModelClient, StreamChunk};
use crate::client::ChatResponse;
use crate::credential_pool::{CredentialPool, PooledCredential};
use crate::error::{Error, Result};

/// A [`ModelClient`] wrapper that rotates credentials within a per-provider
/// pool on classified credential errors (hermes pool parity).
#[derive(Clone)]
pub struct PooledModelClient {
    inner: Arc<dyn ModelClient>,
    pool: Arc<CredentialPool>,
    /// Serializes key selection + runtime swap + request so concurrent
    /// callers can't race the shared client's runtime key.
    gate: Arc<tokio::sync::Mutex<()>>,
}

impl PooledModelClient {
    /// Create a pooled client over `inner` with the provider's pool.
    ///
    /// An empty pool is valid — the wrapper degenerates to a plain passthrough
    /// (select returns `None` on the first iteration and the error surfaces
    /// immediately), so callers can wrap unconditionally.
    pub fn new(inner: Arc<dyn ModelClient>, pool: Arc<CredentialPool>) -> Self {
        Self {
            inner,
            pool,
            gate: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    /// Extract a provider-supplied recovery timestamp from the error, when
    /// present (hermes `last_error_reset_at` parity). The gateway's
    /// `retry-after` beats the error-class TTL so a free-tier per-minute
    /// throttle benches the key for seconds, not an hour.
    fn reset_at_from_error(err: &Error) -> Option<chrono::DateTime<chrono::Utc>> {
        let retry_after = match err {
            Error::RateLimited { retry_after } => Some(*retry_after),
            Error::Provider {
                retry_after: Some(d),
                ..
            } => Some(*d),
            _ => None,
        };
        retry_after.map(|d| chrono::Utc::now() + chrono::Duration::from_std(d).unwrap_or_default())
    }

    /// Bench the given credential for a classified credential error (hermes
    /// `mark_exhausted_and_rotate`): sizes the cooldown by error class and
    /// honors a provider-supplied retry-after.
    fn bench_credential(&self, cred: &PooledCredential, err: &Error) {
        let classified = FallbackModelClient::classify_error(err);
        let reset_at = Self::reset_at_from_error(err);
        self.pool.invalidate_with_reset(
            &cred.id,
            classified.status_code.map(|s| s as i32),
            Some(classified.reason.as_str()),
            Some(&classified.message),
            reset_at,
            false,
        );
        warn!(
            provider = %self.pool.provider(),
            credential = %cred.name,
            status = ?classified.status_code,
            reason = %classified.reason,
            "Credential exhausted — benched"
        );
    }

    /// Build the error surfaced when the pool has no available credential.
    ///
    /// If a classified error was recorded during this rotation loop it is
    /// returned as-is (it carries the underlying error class — normally
    /// `RateLimited`). Otherwise the pool was *already* exhausted — e.g. all
    /// keys benched by a previous call in the same turn — so surface the
    /// earliest remaining bench as a `RateLimited` instead of an opaque
    /// `Agent("no available keys")` error (hermes pool-exhaustion parity):
    /// the outer fallback chain can then act on the rate-limit class and
    /// switch providers instead of failing on an unclassifiable error.
    fn pool_exhausted_error(&self, last_err: Option<Error>) -> Error {
        if let Some(err) = last_err {
            return err;
        }
        let now = chrono::Utc::now();
        let earliest = self
            .pool
            .list()
            .iter()
            .filter_map(|c| c.error_reset_at.map(|r| (r - now).num_seconds().max(0)))
            .min();
        if let Some(secs) = earliest {
            return Error::RateLimited {
                retry_after: Duration::from_secs(secs.max(1) as u64),
            };
        }
        Error::Agent(format!(
            "Credential pool for provider '{}' has no available keys",
            self.pool.provider()
        ))
    }

    /// The non-streaming rotation loop.
    ///
    /// Bounded by the pool size: each iteration consumes one available key
    /// (benched on failure), so the loop terminates when the pool is empty.
    /// Non-rotate errors (4xx other than 429/401-billing, format, policy)
    /// propagate immediately.
    async fn rotate_loop_chat(&self, request: &ChatRequest) -> Result<ChatResponse> {
        let max_attempts = self.pool.len().max(1);
        let mut last_err: Option<Error> = None;

        for attempt in 0..max_attempts {
            let Some(cred) = self.pool.select() else {
                break; // no available key left
            };
            self.inner.set_api_key(&cred.value);
            match self.inner.chat(request.clone()).await {
                Ok(value) => return Ok(value),
                Err(e) => {
                    let classified = FallbackModelClient::classify_error(&e);
                    if !classified.should_rotate_credential {
                        return Err(e);
                    }
                    warn!(
                        attempt = attempt + 1,
                        max = max_attempts,
                        "Credential exhausted — rotating to next key"
                    );
                    self.bench_credential(&cred, &e);
                    last_err = Some(e);
                }
            }
        }

        Err(self.pool_exhausted_error(last_err))
    }
}

#[async_trait]
impl ModelClient for PooledModelClient {
    fn provider_name(&self) -> &str {
        self.inner.provider_name()
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
        let _guard = self.gate.lock().await;
        self.rotate_loop_chat(&request).await
    }

    async fn chat_streaming(
        &self,
        request: ChatRequest,
    ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
        // The gate covers key selection + stream establishment; the returned
        // stream is self-contained and doesn't hold the lock.
        let _guard = self.gate.lock().await;
        let max_attempts = self.pool.len().max(1);
        let mut last_err: Option<Error> = None;

        for attempt in 0..max_attempts {
            let Some(cred) = self.pool.select() else {
                break; // no available key left
            };
            self.inner.set_api_key(&cred.value);
            match self.inner.chat_streaming(request.clone()).await {
                Ok(stream) => {
                    // Wrap the stream so a rotate-classified MID-STREAM error
                    // (e.g. a 429 chunk after the connection was established)
                    // benches the selected credential. The agent's mid-stream
                    // recovery re-issues the request, and the re-issue picks
                    // the next available key — rotation fires on the retry
                    // (hermes `mark_exhausted_and_rotate` parity for the
                    // stream lifecycle, not just establishment). Transport
                    // drops (Network) and format errors are not benched.
                    let pool = self.pool.clone();
                    let cred_id = cred.id.clone();
                    let cred_name = cred.name.clone();
                    let provider = self.pool.provider().to_string();
                    let wrapped = stream.map(move |item| {
                        if let Err(e) = &item {
                            let classified = FallbackModelClient::classify_error(e);
                            if classified.should_rotate_credential {
                                let reset_at = Self::reset_at_from_error(e);
                                pool.invalidate_with_reset(
                                    &cred_id,
                                    classified.status_code.map(|s| s as i32),
                                    Some(classified.reason.as_str()),
                                    Some(&classified.message),
                                    reset_at,
                                    false,
                                );
                                warn!(
                                    provider = %provider,
                                    credential = %cred_name,
                                    status = ?classified.status_code,
                                    reason = %classified.reason,
                                    "Credential exhausted mid-stream — benched for the retry"
                                );
                            }
                        }
                        item
                    });
                    return Ok(Box::pin(wrapped));
                }
                Err(e) => {
                    let classified = FallbackModelClient::classify_error(&e);
                    if !classified.should_rotate_credential {
                        return Err(e);
                    }
                    warn!(
                        attempt = attempt + 1,
                        max = max_attempts,
                        "Credential exhausted — rotating to next key"
                    );
                    self.bench_credential(&cred, &e);
                    last_err = Some(e);
                }
            }
        }

        Err(self.pool_exhausted_error(last_err))
    }

    fn set_api_key(&self, api_key: &str) {
        // Forward so the agent-loop rotation (last-resort path) also swaps
        // the runtime key through this wrapper.
        self.inner.set_api_key(api_key);
    }
}

impl std::fmt::Debug for PooledModelClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PooledModelClient")
            .field("provider", &self.inner.provider_name())
            .field("pool", &self.pool)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use crate::client::{ChatResponse, Choice, Message, MessageDelta, Role, Usage};
    use crate::credential_pool::{AuthType, PooledCredential};

    fn chat_response(model: &str) -> ChatResponse {
        ChatResponse {
            id: "resp_1".into(),
            object: "chat.completion".into(),
            created: 0,
            model: model.into(),
            choices: vec![Choice {
                index: 0,
                message: MessageDelta {
                    role: Some(Role::Assistant),
                    content: Some("Hello!".into()),
                    reasoning_content: None,
                    tool_calls: None,
                },
                finish_reason: Some("stop".into()),
            }],
            usage: Usage {
                prompt_tokens: 10,
                completion_tokens: 5,
                total_tokens: 15,
            },
        }
    }

    /// Scripted outcomes for the mock (avoids cloning `Result`/`Error`).
    enum Outcome {
        Ok(ChatResponse),
        /// 429 with a 5s retry-after.
        RateLimited,
        /// 402 billing exhaustion.
        Billing,
        /// 400 bad request (non-rotate).
        BadRequest,
    }

    /// Mock that returns a scripted sequence of outcomes; records the api key
    /// observed on each request into a shared buffer the test reads back.
    struct ScriptedClient {
        results: Vec<Outcome>,
        call_count: AtomicUsize,
        seen_keys: Arc<std::sync::Mutex<Vec<String>>>,
    }

    impl ScriptedClient {
        #[allow(clippy::new_ret_no_self)]
        fn new(
            results: Vec<Outcome>,
            seen_keys: Arc<std::sync::Mutex<Vec<String>>>,
        ) -> Arc<dyn ModelClient> {
            Arc::new(Self {
                results,
                call_count: AtomicUsize::new(0),
                seen_keys,
            })
        }
    }

    #[async_trait]
    impl ModelClient for ScriptedClient {
        fn provider_name(&self) -> &str {
            "mock"
        }

        fn set_api_key(&self, api_key: &str) {
            if let Ok(mut keys) = self.seen_keys.lock() {
                keys.push(api_key.to_string());
            }
        }

        async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse> {
            let idx = self.call_count.fetch_add(1, Ordering::SeqCst);
            match self.results.get(idx) {
                Some(Outcome::Ok(r)) => Ok(r.clone()),
                Some(Outcome::RateLimited) => Err(Error::RateLimited {
                    retry_after: Duration::from_secs(5),
                }),
                Some(Outcome::Billing) => Err(Error::Provider {
                    status: 402,
                    body: "{\"error\":{\"message\":\"insufficient credits\"}}".into(),
                    retry_after: None,
                }),
                Some(Outcome::BadRequest) => Err(Error::Provider {
                    status: 400,
                    body: "Bad Request".into(),
                    retry_after: None,
                }),
                None => Err(Error::Agent("script exhausted".into())),
            }
        }

        async fn chat_streaming(
            &self,
            _request: ChatRequest,
        ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
            Err(Error::Agent("streaming not mocked".into()))
        }
    }

    fn pool_with(keys: &[&str]) -> Arc<CredentialPool> {
        let pool = Arc::new(CredentialPool::new("mock"));
        for (i, key) in keys.iter().enumerate() {
            pool.add(PooledCredential::new(
                &format!("key-{i}"),
                AuthType::ApiKey,
                key,
                "test",
            ));
        }
        pool
    }

    fn request() -> ChatRequest {
        ChatRequest::new("mock-model", vec![Message::user("hi")])
    }

    fn recording() -> Arc<std::sync::Mutex<Vec<String>>> {
        Arc::new(std::sync::Mutex::new(Vec::new()))
    }

    #[tokio::test]
    async fn rotates_to_next_key_on_429_and_succeeds() {
        let seen = recording();
        let client = ScriptedClient::new(
            vec![Outcome::RateLimited, Outcome::Ok(chat_response("mock"))],
            seen.clone(),
        );
        let pooled = PooledModelClient::new(client, pool_with(&["k1", "k2"]));

        let result = pooled.chat(request()).await.unwrap();
        assert_eq!(result.model, "mock");
        // First attempt used k1, rotated, second attempt used k2.
        let keys = seen.lock().unwrap().clone();
        assert_eq!(keys, vec!["k1", "k2"]);
        // k1 benched (reset ~5s from retry-after); k2 still available.
        let list = pooled.pool.list();
        assert_eq!(
            list.iter().filter(|c| c.is_available()).count(),
            1,
            "k1 should be benched, k2 available"
        );
        let reset_at = list[0].error_reset_at.unwrap();
        let bench_secs = (reset_at - chrono::Utc::now()).num_seconds();
        assert!(
            bench_secs <= 10,
            "retry-after (5s) should override the 1h TTL, got {bench_secs}s"
        );
    }

    #[tokio::test]
    async fn single_key_failure_propagates_immediately() {
        let seen = recording();
        let client = ScriptedClient::new(vec![Outcome::RateLimited], seen);
        let pooled = PooledModelClient::new(client, pool_with(&["k1"]));

        let err = pooled.chat(request()).await.unwrap_err();
        assert!(matches!(err, Error::RateLimited { .. }));
        // Sole credential → retry-after (5s) honored, never the 1h bench.
        let cred = pooled.pool.list().into_iter().next().unwrap();
        let reset_at = cred.error_reset_at.unwrap();
        let bench_secs = (reset_at - chrono::Utc::now()).num_seconds();
        assert!(
            bench_secs <= 10,
            "sole credential should bench ~retry-after, got {bench_secs}s"
        );
    }

    #[tokio::test]
    async fn non_rotate_error_propagates_without_rotation() {
        let seen = recording();
        let client = ScriptedClient::new(vec![Outcome::BadRequest], seen.clone());
        let pooled = PooledModelClient::new(client, pool_with(&["k1", "k2"]));

        let err = pooled.chat(request()).await.unwrap_err();
        assert!(matches!(err, Error::Provider { status: 400, .. }));
        // Only one key was touched (no rotation on non-rotate errors).
        let keys = seen.lock().unwrap().clone();
        assert_eq!(keys, vec!["k1"]);
        assert!(pooled.pool.has_available());
    }

    #[tokio::test]
    async fn all_keys_benched_returns_last_error() {
        let seen = recording();
        let client = ScriptedClient::new(vec![Outcome::RateLimited, Outcome::RateLimited], seen);
        let pooled = PooledModelClient::new(client, pool_with(&["k1", "k2"]));

        let err = pooled.chat(request()).await.unwrap_err();
        assert!(matches!(err, Error::RateLimited { .. }));
        // Both keys benched → pool exhausted.
        assert!(!pooled.pool.has_available());
    }

    #[tokio::test]
    async fn empty_pool_passthrough_error() {
        let seen = recording();
        let client = ScriptedClient::new(vec![Outcome::Ok(chat_response("mock"))], seen);
        let pool = Arc::new(CredentialPool::new("mock"));
        let pooled = PooledModelClient::new(client, pool);
        let err = pooled.chat(request()).await.unwrap_err();
        assert!(err.to_string().contains("no available keys"));
    }

    #[tokio::test]
    async fn exhausted_pool_surfaces_rate_limited_with_remaining_bench() {
        // Regression: after a previous call benched every key, the NEXT call
        // found the pool empty and surfaced an opaque `Agent("no available
        // keys")` error — unclassifiable, so the outer fallback chain never
        // switched providers. Now it must surface `RateLimited` with the
        // earliest remaining bench (hermes pool-exhaustion parity), letting
        // the chain act on the rate-limit class.
        let seen = recording();
        let client = ScriptedClient::new(vec![Outcome::Ok(chat_response("mock"))], seen);
        let pooled = PooledModelClient::new(client, pool_with(&["k1", "k2"]));

        // Simulate a previous call that benched both keys for ~10s.
        let reset_at = chrono::Utc::now() + chrono::Duration::seconds(10);
        for cred in pooled.pool.list() {
            pooled.pool.invalidate_with_reset(
                &cred.id,
                Some(429),
                Some("rate_limit"),
                Some("rate limit"),
                Some(reset_at),
                false,
            );
        }
        assert!(!pooled.pool.has_available());

        let err = pooled.chat(request()).await.unwrap_err();
        match err {
            Error::RateLimited { retry_after } => {
                let secs = retry_after.as_secs();
                assert!(
                    (1..=10).contains(&secs),
                    "expected remaining bench ~10s, got {secs}s"
                );
            }
            other => panic!("expected RateLimited, got {other:?}"),
        }
    }

    /// Streaming mock: each `chat_streaming` call pops the next scripted
    /// sequence of stream items.
    struct StreamingScriptedClient {
        scripts: std::sync::Mutex<std::collections::VecDeque<Vec<Result<StreamChunk>>>>,
        seen_keys: Arc<std::sync::Mutex<Vec<String>>>,
    }

    impl StreamingScriptedClient {
        #[allow(clippy::new_ret_no_self)]
        fn new(
            scripts: Vec<Vec<Result<StreamChunk>>>,
            seen_keys: Arc<std::sync::Mutex<Vec<String>>>,
        ) -> Arc<dyn ModelClient> {
            Arc::new(Self {
                scripts: std::sync::Mutex::new(scripts.into()), // VecDeque::from
                seen_keys,
            })
        }
    }

    #[async_trait]
    impl ModelClient for StreamingScriptedClient {
        fn provider_name(&self) -> &str {
            "mock-stream"
        }

        fn set_api_key(&self, api_key: &str) {
            if let Ok(mut keys) = self.seen_keys.lock() {
                keys.push(api_key.to_string());
            }
        }

        async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse> {
            Err(Error::Agent("non-streaming not used".into()))
        }

        async fn chat_streaming(
            &self,
            _request: ChatRequest,
        ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
            let script = self
                .scripts
                .lock()
                .map(|mut s| s.pop_front().unwrap_or_default())
                .unwrap_or_default();
            Ok(Box::pin(futures::stream::iter(script)))
        }
    }

    #[tokio::test]
    async fn mid_stream_rotate_error_benches_key_and_next_request_rotates() {
        let seen = recording();
        let client = StreamingScriptedClient::new(
            vec![
                // Request 1: stream established with k1, then a mid-stream
                // 429 chunk (connection succeeded, body failed).
                vec![
                    Err(Error::RateLimited {
                        retry_after: Duration::from_secs(5),
                    }),
                    Ok(StreamChunk::new(Some("partial".to_string()), None, None)),
                ],
                // Request 2 (the retry): clean stream on the rotated key.
                vec![
                    Ok(StreamChunk::new(Some("rotated".to_string()), None, None)),
                    Ok(StreamChunk::new(Some(" answer".to_string()), None, None)),
                ],
            ],
            seen.clone(),
        );
        let pooled = PooledModelClient::new(client, pool_with(&["k1", "k2"]));

        // Request 1: the mid-stream 429 surfaces AND benches k1.
        let mut s1 = pooled.chat_streaming(request()).await.unwrap();
        let first = s1.next().await.unwrap();
        assert!(
            matches!(first, Err(Error::RateLimited { .. })),
            "mid-stream 429 must surface from the stream"
        );
        let _ = s1.next().await.unwrap().unwrap(); // remaining chunk still flows
        assert_eq!(
            pooled
                .pool
                .list()
                .iter()
                .filter(|c| c.is_available())
                .count(),
            1,
            "k1 benched by the mid-stream 429; only k2 available"
        );

        // Request 2: rotation fires — the retry selects k2.
        let mut s2 = pooled.chat_streaming(request()).await.unwrap();
        let mut text = String::new();
        while let Some(item) = s2.next().await {
            if let Ok(chunk) = item {
                text.push_str(chunk.content.as_deref().unwrap_or(""));
            }
        }
        assert_eq!(text, "rotated answer");
        let keys = seen.lock().unwrap().clone();
        assert_eq!(keys, vec!["k1", "k2"], "rotation fired on the retry");
    }

    #[tokio::test]
    async fn mid_stream_network_drop_does_not_bench_key() {
        let seen = recording();
        // reqwest transport error (provider closing the SSE connection
        // mid-body) — the realistic mid-stream drop.
        let net_err = reqwest::Client::new()
            .get("http://127.0.0.1:9/")
            .send()
            .await
            .unwrap_err();
        let client = StreamingScriptedClient::new(
            vec![
                // Transport drop (network) — not the key's fault.
                vec![Err(Error::Network(net_err))],
                // Retry succeeds with the SAME key (no rotation).
                vec![Ok(StreamChunk::new(
                    Some("recovered".to_string()),
                    None,
                    None,
                ))],
            ],
            seen.clone(),
        );
        let pooled = PooledModelClient::new(client, pool_with(&["k1"]));

        let mut s1 = pooled.chat_streaming(request()).await.unwrap();
        assert!(matches!(s1.next().await.unwrap(), Err(Error::Network(_))));
        // Sole key NOT benched — a transport blip isn't a key failure.
        assert!(pooled.pool.has_available());

        // Next request reuses k1 and succeeds.
        let mut s2 = pooled.chat_streaming(request()).await.unwrap();
        let text = s2.next().await.unwrap().unwrap();
        assert_eq!(text.content.as_deref(), Some("recovered"));
        let keys = seen.lock().unwrap().clone();
        assert_eq!(keys, vec!["k1", "k1"], "transport drop does not rotate");
    }

    #[tokio::test]
    async fn billing_error_benches_full_ttl_even_when_sole() {
        let seen = recording();
        let client = ScriptedClient::new(vec![Outcome::Billing], seen);
        let pooled = PooledModelClient::new(client, pool_with(&["k1"]));

        let _ = pooled.chat(request()).await.unwrap_err();
        // Billing keeps the full bench (1h) even for a sole credential.
        let cred = pooled.pool.list().into_iter().next().unwrap();
        let reset_at = cred.error_reset_at.unwrap();
        let bench_secs = (reset_at - chrono::Utc::now()).num_seconds();
        assert!(
            bench_secs >= 3000,
            "billing should bench ~1h, got {bench_secs}s"
        );
    }
}
