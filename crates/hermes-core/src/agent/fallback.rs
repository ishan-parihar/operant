//! Automatic model fallback chain for [`ModelClient`].
//!
//! When the primary model fails with a retryable provider error (5xx, 429,
//! network error), [`FallbackModelClient`] automatically tries the next model
//! in the configured fallback chain.  Non-retryable errors (4xx except 429,
//! auth failures) are returned immediately.
//!
//! Fallback is **per-request** — every call starts again with the primary
//! model.  There is no persistent state across requests.

use std::sync::Arc;

use async_trait::async_trait;
use futures::stream::BoxStream;
use tracing::{info, warn};

use super::model_client::{ChatRequest, ModelClient, StreamChunk};
use crate::client::ChatResponse;
use crate::error::{Error, Result};

/// A [`ModelClient`] wrapper that tries fallback models when the primary
/// model fails with a retryable error.
///
/// # Behaviour
///
/// 1. On each request, try the primary model first.
/// 2. If the call succeeds, return the response immediately.
/// 3. If the call fails with a **retryable** error (5xx, 429, network error):
///    log the failure and try the next model in `fallback_models`.
/// 4. If the call fails with a **non-retryable** error (4xx except 429, auth):
///    return the error immediately — changing the model won't help.
/// 5. If all models in the chain are exhausted, return the last error.
///
/// Fallback is per-request — the next request starts again with the primary
/// model.  Different models are assumed to use the same underlying client
/// (provider, base URL, API key); only the `model` field in the request is
/// changed.
#[derive(Clone)]
pub struct FallbackModelClient {
    inner: Arc<dyn ModelClient>,
    primary_model: String,
    fallback_models: Vec<String>,
    fallback_enabled: bool,
}

impl FallbackModelClient {
    /// Create a new fallback client wrapper.
    ///
    /// * `inner` — The underlying model client (must be cheaply clonable via
    ///   `Arc`).  All requests are forwarded to this client; only the
    ///   `model` field changes between attempts.
    /// * `primary_model` — The default model name (e.g. `"gpt-4"`).
    /// * `fallback_models` — Ordered list of fallback models, tried in
    ///   sequence when the current model fails with a retryable error.
    /// * `fallback_enabled` — Set to `false` to bypass the wrapper entirely
    ///   and pass every request straight through.
    pub fn new(
        inner: Arc<dyn ModelClient>,
        primary_model: String,
        fallback_models: Vec<String>,
        fallback_enabled: bool,
    ) -> Self {
        Self {
            inner,
            primary_model,
            fallback_models,
            fallback_enabled,
        }
    }

    /// Build the ordered list of models to try:
    /// `[primary_model, fallback_model_1, fallback_model_2, ...]`.
    fn model_chain(&self) -> Vec<String> {
        let mut models = Vec::with_capacity(1 + self.fallback_models.len());
        models.push(self.primary_model.clone());
        models.extend(self.fallback_models.iter().cloned());
        models
    }

    /// Returns `true` when the error is likely to be resolved by switching
    /// to a different model.
    fn is_fallback_error(err: &Error) -> bool {
        match err {
            // Network / transport errors — transient, may affect specific endpoints
            Error::Network(_) => true,
            // Rate limited — different models may have separate rate-limit buckets
            Error::RateLimited { .. } => true,
            // Server-side provider errors (5xx) — transient, try a different model
            Error::Provider { status, .. } if *status >= 500 => true,
            // Everything else: bad request (4xx), auth (401/403), parse, validation — not retryable
            _ => false,
        }
    }

    /// Core fallback loop shared by `chat` and `chat_streaming`.
    async fn try_models<F, Fut, T>(&self, request: &ChatRequest, call: F) -> Result<T>
    where
        F: Fn(ChatRequest) -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        let models = self.model_chain();
        let mut attempted: Vec<String> = Vec::with_capacity(models.len());
        let mut last_error: Option<Error> = None;

        for model in &models {
            if attempted.contains(model) {
                continue;
            }
            attempted.push(model.clone());

            let mut fallback_req = request.clone();
            fallback_req.model.clone_from(model);

            match call(fallback_req).await {
                Ok(response) => {
                    if attempted.len() > 1 {
                        info!(
                            primary = %self.primary_model,
                            fallback = %model,
                            "Fallback model succeeded"
                        );
                    }
                    return Ok(response);
                }
                Err(e) => {
                    if Self::is_fallback_error(&e) {
                        warn!(
                            primary = %self.primary_model,
                            model = %model,
                            error = %e,
                            "Model failed with retryable error, trying next fallback"
                        );
                        last_error = Some(e);
                    } else {
                        // Non-retryable: return immediately
                        return Err(e);
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| {
            Error::Agent("All models in fallback chain failed".to_string())
        }))
    }
}

#[async_trait]
impl ModelClient for FallbackModelClient {
    fn provider_name(&self) -> &str {
        self.inner.provider_name()
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
        if !self.fallback_enabled || self.fallback_models.is_empty() {
            // Fast path: no fallback configured, passthrough
            return self.inner.chat(request).await;
        }

        self.try_models(&request, |req| self.inner.chat(req))
            .await
    }

    async fn chat_streaming(
        &self,
        request: ChatRequest,
    ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
        if !self.fallback_enabled || self.fallback_models.is_empty() {
            return self.inner.chat_streaming(request).await;
        }

        self.try_models(&request, |req| self.inner.chat_streaming(req))
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use async_trait::async_trait;
    use futures::stream::BoxStream;

    use crate::client::{ChatResponse, Choice, Message, MessageDelta, Role, Usage};

    // ── Mock helpers ────────────────────────────────────────────────

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

    /// Pre-configured result kind for the mock client.
    enum MockResult {
        Ok(ChatResponse),
        ServerError,
        RateLimited,
        AuthError,
        BadRequest,
    }

    /// Mock [`ModelClient`] that returns pre-configured results in order.
    struct MockModelClient {
        provider: &'static str,
        results: Vec<MockResult>,
        call_count: AtomicUsize,
    }

    impl MockModelClient {
        fn new(provider: &'static str, results: Vec<MockResult>) -> Arc<dyn ModelClient> {
            Arc::new(Self {
                provider,
                results,
                call_count: AtomicUsize::new(0),
            })
        }
    }

    #[async_trait]
    impl ModelClient for MockModelClient {
        fn provider_name(&self) -> &str {
            self.provider
        }

        async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
            let idx = self.call_count.fetch_add(1, Ordering::SeqCst);
            match self.results.get(idx) {
                Some(MockResult::Ok(r)) => Ok(r.clone()),
                Some(MockResult::ServerError) => Err(Error::Provider {
                    status: 503,
                    body: "Service Unavailable".into(),
                    retry_after: None,
                }),
                Some(MockResult::RateLimited) => Err(Error::RateLimited {
                    retry_after: Duration::from_secs(5),
                }),
                Some(MockResult::AuthError) => {
                    Err(Error::Authentication("invalid key".into()))
                }
                Some(MockResult::BadRequest) => Err(Error::Provider {
                    status: 400,
                    body: "Bad Request".into(),
                    retry_after: None,
                }),
                None => Err(Error::Agent("mock exhausted".into())),
            }
        }

        async fn chat_streaming(
            &self,
            _request: ChatRequest,
        ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
            Err(Error::Agent("streaming not mocked".into()))
        }
    }

    fn make_request() -> ChatRequest {
        ChatRequest::new("primary", vec![])
    }

    // ── Tests ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn primary_succeeds_no_fallback_attempted() {
        let inner = MockModelClient::new(
            "test",
            vec![MockResult::Ok(chat_response("primary"))],
        );

        let client = FallbackModelClient::new(
            inner,
            "primary".into(),
            vec!["fallback1".into()],
            true,
        );

        let result = client.chat(make_request()).await.unwrap();
        assert_eq!(result.model, "primary");
    }

    #[tokio::test]
    async fn primary_fails_fallback1_succeeds() {
        let inner = MockModelClient::new(
            "test",
            vec![
                MockResult::ServerError,
                MockResult::Ok(chat_response("fallback1")),
            ],
        );

        let client = FallbackModelClient::new(
            inner,
            "primary".into(),
            vec!["fallback1".into()],
            true,
        );

        let result = client.chat(make_request()).await.unwrap();
        // Response model reflects the fallback model that succeeded
        assert_eq!(result.model, "fallback1");
    }

    #[tokio::test]
    async fn all_models_fail_returns_last_error() {
        let inner = MockModelClient::new(
            "test",
            vec![
                MockResult::ServerError,
                MockResult::ServerError,
                MockResult::ServerError,
            ],
        );

        let client = FallbackModelClient::new(
            inner,
            "primary".into(),
            vec!["fallback1".into(), "fallback2".into()],
            true,
        );

        let err = client.chat(make_request()).await.unwrap_err();
        assert!(
            matches!(&err, Error::Provider { status: 503, .. }),
            "expected Provider(503), got {err}"
        );
    }

    #[tokio::test]
    async fn non_retryable_error_no_fallback_attempted() {
        let inner = MockModelClient::new(
            "test",
            vec![MockResult::BadRequest],
        );

        let client = FallbackModelClient::new(
            inner,
            "primary".into(),
            vec!["fallback1".into()],
            true,
        );

        let err = client.chat(make_request()).await.unwrap_err();
        assert!(
            matches!(&err, Error::Provider { status: 400, .. }),
            "expected Provider(400), got {err}"
        );
    }

    #[tokio::test]
    async fn auth_error_no_fallback_attempted() {
        let inner = MockModelClient::new(
            "test",
            vec![MockResult::AuthError],
        );

        let client = FallbackModelClient::new(
            inner,
            "primary".into(),
            vec!["fallback1".into()],
            true,
        );

        let err = client.chat(make_request()).await.unwrap_err();
        assert!(
            matches!(&err, Error::Authentication(_)),
            "expected Authentication error, got {err}"
        );
    }

    #[tokio::test]
    async fn empty_fallback_list_passthrough() {
        let inner = MockModelClient::new(
            "test",
            vec![MockResult::Ok(chat_response("primary"))],
        );

        let client = FallbackModelClient::new(
            inner,
            "primary".into(),
            vec![],
            true,
        );

        let result = client.chat(make_request()).await.unwrap();
        assert_eq!(result.model, "primary");
    }

    #[tokio::test]
    async fn fallback_disabled_passthrough() {
        let inner = MockModelClient::new(
            "test",
            vec![MockResult::Ok(chat_response("primary"))],
        );

        let client = FallbackModelClient::new(
            inner,
            "primary".into(),
            vec!["fallback1".into()],
            false,
        );

        let result = client.chat(make_request()).await.unwrap();
        assert_eq!(result.model, "primary");
    }

    #[tokio::test]
    async fn fallback_disabled_still_returns_errors() {
        // When fallback is disabled, even retryable errors should be
        // returned as-is without attempting fallback.
        let inner = MockModelClient::new(
            "test",
            vec![MockResult::ServerError],
        );

        let client = FallbackModelClient::new(
            inner,
            "primary".into(),
            vec!["fallback1".into()],
            false,
        );

        let err = client.chat(make_request()).await.unwrap_err();
        assert!(
            matches!(&err, Error::Provider { status: 503, .. }),
            "expected Provider(503), got {err}"
        );
    }

    #[tokio::test]
    async fn rate_limit_triggers_fallback() {
        let inner = MockModelClient::new(
            "test",
            vec![
                MockResult::RateLimited,
                MockResult::Ok(chat_response("fallback1")),
            ],
        );

        let client = FallbackModelClient::new(
            inner,
            "primary".into(),
            vec!["fallback1".into()],
            true,
        );

        let result = client.chat(make_request()).await.unwrap();
        assert_eq!(result.model, "fallback1");
    }

    #[tokio::test]
    async fn multiple_fallbacks_second_succeeds() {
        // primary fails -> fallback1 fails -> fallback2 succeeds
        let inner = MockModelClient::new(
            "test",
            vec![
                MockResult::ServerError,
                MockResult::ServerError,
                MockResult::Ok(chat_response("fallback2")),
            ],
        );

        let client = FallbackModelClient::new(
            inner,
            "primary".into(),
            vec!["fallback1".into(), "fallback2".into()],
            true,
        );

        let result = client.chat(make_request()).await.unwrap();
        assert_eq!(result.model, "fallback2");
    }

    #[tokio::test]
    async fn non_retryable_error_mid_chain_stops_fallback() {
        // primary fails (retryable) -> fallback1 fails (non-retryable)
        // The non-retryable error should be returned immediately, fallback2
        // should never be attempted.
        let inner = MockModelClient::new(
            "test",
            vec![
                MockResult::ServerError,
                MockResult::BadRequest,
                MockResult::Ok(chat_response("fallback2")),
            ],
        );

        let client = FallbackModelClient::new(
            inner,
            "primary".into(),
            vec!["fallback1".into(), "fallback2".into()],
            true,
        );

        let err = client.chat(make_request()).await.unwrap_err();
        // Should be 400 from fallback1, not 200 from fallback2
        assert!(
            matches!(&err, Error::Provider { status: 400, .. }),
            "expected Provider(400), got {err}"
        );
    }

    #[tokio::test]
    async fn same_model_request_fields_preserved() {
        // Verify that messages, tools, stream flag etc. are preserved
        // across fallback attempts.
        let inner = MockModelClient::new(
            "test",
            vec![
                MockResult::ServerError,
                MockResult::Ok(chat_response("fallback1")),
            ],
        );

        let client = FallbackModelClient::new(
            inner,
            "primary".into(),
            vec!["fallback1".into()],
            true,
        );

        let mut req = ChatRequest::new("some-model", vec![Message::user("hello")]);
        req.stream = true;
        req.temperature = Some(0.7);
        req.max_tokens = Some(100);

        let result = client.chat(req).await.unwrap();
        assert_eq!(result.model, "fallback1");
    }
}
