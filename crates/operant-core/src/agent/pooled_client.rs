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

use async_trait::async_trait;
use futures::stream::BoxStream;
use tracing::warn;

use super::fallback::FallbackModelClient;
use super::model_client::{ChatRequest, ModelClient, StreamChunk};
use crate::client::ChatResponse;
use crate::credential_pool::CredentialPool;
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

    /// The rotation loop shared by `chat` and `chat_streaming`.
    ///
    /// Bounded by the pool size: each iteration consumes one available key
    /// (benched on failure), so the loop terminates when the pool is empty.
    /// Non-rotate errors (4xx other than 429/401-billing, format, policy)
    /// propagate immediately.
    async fn rotate_loop<T>(
        &self,
        request: &ChatRequest,
        call: impl Fn(&dyn ModelClient, ChatRequest) -> futures::future::BoxFuture<'_, Result<T>>,
    ) -> Result<T> {
        let max_attempts = self.pool.len().max(1);
        let mut last_err: Option<Error> = None;

        for attempt in 0..max_attempts {
            let Some(cred) = self.pool.select() else {
                break; // no available key left
            };
            self.inner.set_api_key(&cred.value);
            let result = call(self.inner.as_ref(), request.clone()).await;
            match result {
                Ok(value) => return Ok(value),
                Err(e) => {
                    let classified = FallbackModelClient::classify_error(&e);
                    if !classified.should_rotate_credential {
                        return Err(e);
                    }
                    let reset_at = Self::reset_at_from_error(&e);
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
                        attempt = attempt + 1,
                        max = max_attempts,
                        "Credential exhausted — rotating to next key"
                    );
                    last_err = Some(e);
                }
            }
        }

        Err(last_err.unwrap_or_else(|| {
            Error::Agent(format!(
                "Credential pool for provider '{}' has no available keys",
                self.pool.provider()
            ))
        }))
    }
}

#[async_trait]
impl ModelClient for PooledModelClient {
    fn provider_name(&self) -> &str {
        self.inner.provider_name()
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
        let _guard = self.gate.lock().await;
        self.rotate_loop(&request, |client, req| {
            Box::pin(async move { client.chat(req).await })
        })
        .await
    }

    async fn chat_streaming(
        &self,
        request: ChatRequest,
    ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
        // The gate covers key selection + stream establishment; the returned
        // stream is self-contained and doesn't hold the lock.
        let _guard = self.gate.lock().await;
        self.rotate_loop(&request, |client, req| {
            Box::pin(async move { client.chat_streaming(req).await })
        })
        .await
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
