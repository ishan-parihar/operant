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

use crate::error::Error;

/// Export the full error classification from error_classifier module.
pub use super::error_classifier::{ClassifiedError, FailoverReason, classify_api_error};
use super::model_client::{ChatRequest, ModelClient, StreamChunk};
use super::provider_registry::ProviderRegistry;
use crate::client::ChatResponse;
use crate::error::Result;

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
    /// Optional provider registry for cross-provider fallback on auth/billing errors.
    provider_registry: Option<Arc<ProviderRegistry>>,
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
            provider_registry: None,
        }
    }

    /// Attach a provider registry for cross-provider fallback.
    pub fn with_provider_registry(mut self, registry: Arc<ProviderRegistry>) -> Self {
        self.provider_registry = Some(registry);
        self
    }

    /// Try the next provider from the registry for a non-streaming chat.
    /// Called by chat() after try_models returns an auth/billing error.
    async fn try_next_provider_chat(&self, request: &ChatRequest) -> Option<Result<ChatResponse>> {
        let registry = self.provider_registry.as_ref()?;
        // switch_to_next() skips providers in cooldown and returns None if all exhausted.
        let next_provider = registry.switch_to_next()?;
        let next_client = registry.get_client(&next_provider.name)?;
        let mut fallback_req = request.clone();
        fallback_req.model.clone_from(&next_provider.model);
        info!(
            to_provider = %next_provider.name,
            to_model = %next_provider.model,
            "Auth/billing error — switching to fallback provider"
        );
        let result = next_client.chat(fallback_req).await;
        if result.is_err() {
            registry.arm_cooldown(&next_provider.name);
        }
        Some(result)
    }

    /// Whether an error warrants switching to the next provider in the
    /// fallback chain (hermes parity).
    ///
    /// The switch fires on auth/billing errors AND on credential-pool
    /// exhaustion, which surfaces as `RateLimited` once every key is benched
    /// (the pooled client reports the underlying class instead of an opaque
    /// "no keys" error). A fully rate-limited provider is exactly when a
    /// fallback provider's separate quota should be tried; the registry's
    /// anti-thrash cooldown bounds repeat switching.
    fn should_switch_provider(classified: &ClassifiedError) -> bool {
        classified.is_auth()
            || matches!(
                classified.reason,
                FailoverReason::Billing | FailoverReason::RateLimit
            )
    }

    /// Try the next provider from the registry for streaming.
    /// Called by chat_streaming() after try_models returns an auth/billing error.
    async fn try_next_provider_streaming(
        &self,
        request: &ChatRequest,
    ) -> Option<Result<BoxStream<'static, Result<StreamChunk>>>> {
        let registry = self.provider_registry.as_ref()?;
        let next_provider = registry.switch_to_next()?;
        let next_client = registry.get_client(&next_provider.name)?;
        let mut fallback_req = request.clone();
        fallback_req.model.clone_from(&next_provider.model);
        info!(
            to_provider = %next_provider.name,
            to_model = %next_provider.model,
            "Auth/billing error — switching to fallback provider"
        );
        let result = next_client.chat_streaming(fallback_req).await;
        if result.is_err() {
            registry.arm_cooldown(&next_provider.name);
        }
        Some(result)
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

    /// Classify an error into a recovery strategy using the full
    /// error_classifier taxonomy (22+ categories).
    ///
    /// Extracts status code, body, and error code from the Error enum
    /// and delegates to `classify_api_error` for pattern matching.
    pub fn classify_error(err: &Error) -> ClassifiedError {
        match err {
            Error::Network(_) => ClassifiedError {
                reason: FailoverReason::Timeout,
                status_code: None,
                message: err.to_string(),
                retryable: true,
                should_fallback: true,
                should_compress: false,
                should_rotate_credential: false,
            },

            Error::RateLimited { retry_after } => ClassifiedError {
                reason: FailoverReason::RateLimit,
                status_code: Some(429),
                message: format!("rate limited, retry after {:?}", retry_after),
                retryable: true,
                should_fallback: true,
                should_compress: false,
                should_rotate_credential: true,
            },

            Error::Authentication(msg) => ClassifiedError {
                reason: FailoverReason::Auth,
                status_code: None,
                message: msg.clone(),
                retryable: false,
                should_fallback: true,
                should_compress: false,
                should_rotate_credential: true,
            },

            Error::Provider { status, body, .. } => classify_api_error(Some(*status), body, None),

            Error::Config(_) => ClassifiedError {
                reason: FailoverReason::FormatError,
                status_code: None,
                message: err.to_string(),
                retryable: false,
                should_fallback: false,
                should_compress: false,
                should_rotate_credential: false,
            },

            _ => ClassifiedError {
                reason: FailoverReason::Unknown,
                status_code: None,
                message: err.to_string(),
                retryable: true,
                should_fallback: true,
                should_compress: false,
                should_rotate_credential: false,
            },
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

        Err(last_error
            .unwrap_or_else(|| Error::Agent("All models in fallback chain failed".to_string())))
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
            let result = self.inner.chat(request.clone()).await;
            // Check for auth/billing errors → try provider fallback
            if let Err(ref e) = result {
                let classified = Self::classify_error(e);
                if Self::should_switch_provider(&classified)
                    && let Some(provider_result) = self.try_next_provider_chat(&request).await
                {
                    return provider_result;
                }
            }
            return result;
        }

        let result = self.try_models(&request, |req| self.inner.chat(req)).await;
        // Check for auth/billing/rate-limit-exhaustion errors → try provider fallback
        if let Err(ref e) = result {
            let classified = Self::classify_error(e);
            if Self::should_switch_provider(&classified)
                && let Some(provider_result) = self.try_next_provider_chat(&request).await
            {
                return provider_result;
            }
        }
        result
    }

    async fn chat_streaming(
        &self,
        request: ChatRequest,
    ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
        if !self.fallback_enabled || self.fallback_models.is_empty() {
            let result = self.inner.chat_streaming(request.clone()).await;
            if let Err(ref e) = result {
                let classified = Self::classify_error(e);
                if Self::should_switch_provider(&classified)
                    && let Some(provider_result) = self.try_next_provider_streaming(&request).await
                {
                    return provider_result;
                }
            }
            return result;
        }

        let result = self
            .try_models(&request, |req| self.inner.chat_streaming(req))
            .await;
        if let Err(ref e) = result {
            let classified = Self::classify_error(e);
            if Self::should_switch_provider(&classified)
                && let Some(provider_result) = self.try_next_provider_streaming(&request).await
            {
                return provider_result;
            }
        }
        result
    }

    fn set_api_key(&self, api_key: &str) {
        self.inner.set_api_key(api_key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
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
        #[allow(clippy::new_ret_no_self)]
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

        async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse> {
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
                Some(MockResult::AuthError) => Err(Error::Authentication("invalid key".into())),
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
        let inner = MockModelClient::new("test", vec![MockResult::Ok(chat_response("primary"))]);

        let client =
            FallbackModelClient::new(inner, "primary".into(), vec!["fallback1".into()], true);

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

        let client =
            FallbackModelClient::new(inner, "primary".into(), vec!["fallback1".into()], true);

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
        let inner = MockModelClient::new("test", vec![MockResult::BadRequest]);

        let client =
            FallbackModelClient::new(inner, "primary".into(), vec!["fallback1".into()], true);

        let err = client.chat(make_request()).await.unwrap_err();
        assert!(
            matches!(&err, Error::Provider { status: 400, .. }),
            "expected Provider(400), got {err}"
        );
    }

    #[tokio::test]
    async fn auth_error_no_fallback_attempted() {
        let inner = MockModelClient::new("test", vec![MockResult::AuthError]);

        let client =
            FallbackModelClient::new(inner, "primary".into(), vec!["fallback1".into()], true);

        let err = client.chat(make_request()).await.unwrap_err();
        assert!(
            matches!(&err, Error::Authentication(_)),
            "expected Authentication error, got {err}"
        );
    }

    #[tokio::test]
    async fn empty_fallback_list_passthrough() {
        let inner = MockModelClient::new("test", vec![MockResult::Ok(chat_response("primary"))]);

        let client = FallbackModelClient::new(inner, "primary".into(), vec![], true);

        let result = client.chat(make_request()).await.unwrap();
        assert_eq!(result.model, "primary");
    }

    #[tokio::test]
    async fn fallback_disabled_passthrough() {
        let inner = MockModelClient::new("test", vec![MockResult::Ok(chat_response("primary"))]);

        let client =
            FallbackModelClient::new(inner, "primary".into(), vec!["fallback1".into()], false);

        let result = client.chat(make_request()).await.unwrap();
        assert_eq!(result.model, "primary");
    }

    #[tokio::test]
    async fn fallback_disabled_still_returns_errors() {
        // When fallback is disabled, even retryable errors should be
        // returned as-is without attempting fallback.
        let inner = MockModelClient::new("test", vec![MockResult::ServerError]);

        let client =
            FallbackModelClient::new(inner, "primary".into(), vec!["fallback1".into()], false);

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

        let client =
            FallbackModelClient::new(inner, "primary".into(), vec!["fallback1".into()], true);

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

        let client =
            FallbackModelClient::new(inner, "primary".into(), vec!["fallback1".into()], true);

        let mut req = ChatRequest::new("some-model", vec![Message::user("hello")]);
        req.stream = true;
        req.temperature = Some(0.7);
        req.max_tokens = Some(100);

        let result = client.chat(req).await.unwrap();
        assert_eq!(result.model, "fallback1");
    }

    // ── Provider-level fallback tests ──────────────────────────────

    #[tokio::test]
    async fn auth_error_triggers_provider_switch() {
        use crate::agent::provider_registry::{ProviderEntry, ProviderRegistry};

        // Primary provider returns auth error → should switch to fallback provider
        let primary_client = MockModelClient::new("openai", vec![MockResult::AuthError]);
        let fallback_client =
            MockModelClient::new("anthropic", vec![MockResult::Ok(chat_response("claude-3"))]);

        let mut clients = std::collections::HashMap::new();
        clients.insert("openai".to_string(), primary_client);
        clients.insert("anthropic".to_string(), fallback_client);

        let chain = vec![
            ProviderEntry {
                name: "openai".to_string(),
                model: "gpt-4".to_string(),
            },
            ProviderEntry {
                name: "anthropic".to_string(),
                model: "claude-3".to_string(),
            },
        ];
        let registry = Arc::new(ProviderRegistry::new(clients, chain));

        // Inner client is the openai one (returns auth error)
        let inner = MockModelClient::new("openai", vec![MockResult::AuthError]);
        let client = FallbackModelClient::new(inner, "gpt-4".into(), vec![], true)
            .with_provider_registry(registry);

        let result = client.chat(make_request()).await.unwrap();
        assert_eq!(result.model, "claude-3");
    }

    #[tokio::test]
    async fn rate_limit_exhaustion_triggers_provider_switch() {
        use crate::agent::provider_registry::{ProviderEntry, ProviderRegistry};

        // Primary returns RateLimited — the class a credential pool surfaces
        // once every key is benched (pool exhaustion) — and the fallback
        // provider succeeds. Hermes parity: a fully rate-limited provider
        // should switch to the fallback provider's separate quota.
        let primary_client = MockModelClient::new("openai", vec![MockResult::RateLimited]);
        let fallback_client =
            MockModelClient::new("anthropic", vec![MockResult::Ok(chat_response("claude-3"))]);

        let mut clients = std::collections::HashMap::new();
        clients.insert("openai".to_string(), primary_client);
        clients.insert("anthropic".to_string(), fallback_client);

        let chain = vec![
            ProviderEntry {
                name: "openai".to_string(),
                model: "gpt-4".to_string(),
            },
            ProviderEntry {
                name: "anthropic".to_string(),
                model: "claude-3".to_string(),
            },
        ];
        let registry = Arc::new(ProviderRegistry::new(clients, chain));

        let inner = MockModelClient::new("openai", vec![MockResult::RateLimited]);
        let client = FallbackModelClient::new(inner, "gpt-4".into(), vec![], true)
            .with_provider_registry(registry);

        let result = client.chat(make_request()).await.unwrap();
        assert_eq!(result.model, "claude-3");
    }

    #[tokio::test]
    async fn non_auth_error_does_not_trigger_provider_switch() {
        use crate::agent::provider_registry::{ProviderEntry, ProviderRegistry};

        // Primary provider returns 503 (server error) → should NOT trigger provider switch
        // (provider switch is only for auth/billing/rate-limit-exhaustion errors)
        let primary_client = MockModelClient::new("openai", vec![MockResult::ServerError]);
        let fallback_client =
            MockModelClient::new("anthropic", vec![MockResult::Ok(chat_response("claude-3"))]);

        let mut clients = std::collections::HashMap::new();
        clients.insert("openai".to_string(), primary_client);
        clients.insert("anthropic".to_string(), fallback_client);

        let chain = vec![
            ProviderEntry {
                name: "openai".to_string(),
                model: "gpt-4".to_string(),
            },
            ProviderEntry {
                name: "anthropic".to_string(),
                model: "claude-3".to_string(),
            },
        ];
        let registry = Arc::new(ProviderRegistry::new(clients, chain));

        // Inner client returns 503 (not auth error)
        let inner = MockModelClient::new("openai", vec![MockResult::ServerError]);
        let client = FallbackModelClient::new(inner, "gpt-4".into(), vec![], true)
            .with_provider_registry(registry);

        let err = client.chat(make_request()).await.unwrap_err();
        // Should return the 503 error, NOT switch providers
        assert!(matches!(&err, Error::Provider { status: 503, .. }));
    }

    #[tokio::test]
    async fn exhausted_provider_chain_returns_original_error() {
        use crate::agent::provider_registry::{ProviderEntry, ProviderRegistry};

        // Both providers return auth errors → chain exhausted → original error returned
        let primary_client = MockModelClient::new("openai", vec![MockResult::AuthError]);
        let fallback_client = MockModelClient::new("anthropic", vec![MockResult::AuthError]);

        let mut clients = std::collections::HashMap::new();
        clients.insert("openai".to_string(), primary_client);
        clients.insert("anthropic".to_string(), fallback_client);

        let chain = vec![
            ProviderEntry {
                name: "openai".to_string(),
                model: "gpt-4".to_string(),
            },
            ProviderEntry {
                name: "anthropic".to_string(),
                model: "claude-3".to_string(),
            },
        ];
        let registry = Arc::new(ProviderRegistry::new(clients, chain));

        let inner = MockModelClient::new("openai", vec![MockResult::AuthError]);
        let client = FallbackModelClient::new(inner, "gpt-4".into(), vec![], true)
            .with_provider_registry(registry);

        let err = client.chat(make_request()).await.unwrap_err();
        // Should return auth error from the fallback provider (anthropic)
        assert!(matches!(&err, Error::Authentication(_)));
    }
}
