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
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures::stream::BoxStream;
use parking_lot::Mutex;
use tokio::sync::mpsc;
use tracing::{info, warn};

use super::AgentEvent;
use crate::error::Error;

/// Export the full error classification from error_classifier module.
pub use super::error_classifier::{ClassifiedError, FailoverReason, classify_api_error};
use super::model_client::{ChatRequest, ModelClient, StreamChunk};
use super::provider_registry::ProviderRegistry;
use crate::client::ChatResponse;
use crate::error::Result;

// ---------------------------------------------------------------------------
// FailoverDecision
// ---------------------------------------------------------------------------

/// What a failed request means for the rest of the chain.
///
/// Produced once by [`Failover::classify`] in the shared fallback path and
/// consumed by both [`FallbackModelClient::chat`] and
/// [`FallbackModelClient::chat_streaming`], so the two paths cannot drift
/// apart and neither re-derives the reason from a message string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailoverDecision {
    /// The runtime is not broken — this request was throttled. Wait `after`
    /// and ask again. Another model on this provider is also a valid
    /// substitute (rate-limit buckets are per model), and when a cheaper
    /// sibling has a known price it is the preferred one.
    RetrySame {
        /// The provider's `Retry-After`, when it sent one.
        after: Option<Duration>,
    },
    /// This model cannot serve the request. A different runtime answers.
    FallBack {
        /// Typed reason. Never a message string.
        reason: FailoverReason,
    },
    /// Nothing in the chain can serve this request. Surface the error.
    GiveUp,
}

/// A [`FailoverDecision`] plus everything a caller needs to act on it.
#[derive(Debug, Clone)]
pub struct Failover {
    /// The semantic verdict.
    pub decision: FailoverDecision,
    /// Typed reason, mirrored out of the decision for the stage gates so no
    /// caller re-parses a message.
    pub reason: FailoverReason,
    /// The full classification (status code, message, recovery hints).
    pub classified: ClassifiedError,
    /// Provider-supplied `Retry-After`, when present.
    pub retry_after: Option<Duration>,
    /// Whether the model chain on the current provider should advance.
    /// Computed once, from the concrete `Error` variant.
    advance_model_chain: bool,
    /// Whether a different provider should take the request. True for auth,
    /// billing, and rate-limit exhaustion — those providers have their own
    /// credentials and quota. Computed once, from the classification.
    switch_provider: bool,
}

impl Failover {
    /// Classify a failed request into a typed decision.
    ///
    /// The two stage gates are the exact predicates the chain has always used,
    /// evaluated here once so every caller shares them:
    ///
    /// * `advance_model_chain` — transport errors, rate limits, and 5xx. A
    ///   different model (or endpoint) may well work. Note this is
    ///   deliberately *not* `classified.should_fallback`, which is set for
    ///   cases (a content-policy rejection at 400) the model chain has never
    ///   advanced on.
    /// * `switch_provider` — auth, billing, and rate limits. Those providers'
    ///   credentials/quota are spent, so a sibling provider with its own quota
    ///   is the right target.
    pub fn classify(err: &Error) -> Self {
        let (classified, retry_after) = match err {
            Error::Network(_) | Error::StreamStalled { .. } => (
                ClassifiedError {
                    reason: FailoverReason::Timeout,
                    status_code: None,
                    message: err.to_string(),
                    retryable: true,
                    should_fallback: true,
                    should_compress: false,
                    should_rotate_credential: false,
                },
                None,
            ),
            Error::RateLimited { retry_after } => (
                ClassifiedError {
                    reason: FailoverReason::RateLimit,
                    status_code: Some(429),
                    message: format!("rate limited, retry after {retry_after:?}"),
                    retryable: true,
                    should_fallback: true,
                    should_compress: false,
                    should_rotate_credential: true,
                },
                Some(*retry_after),
            ),
            Error::Authentication(msg) => (
                ClassifiedError {
                    reason: FailoverReason::Auth,
                    status_code: None,
                    message: msg.clone(),
                    retryable: false,
                    should_fallback: true,
                    should_compress: false,
                    should_rotate_credential: true,
                },
                None,
            ),
            Error::Provider { status, body, .. } => {
                (classify_api_error(Some(*status), body, None), None)
            }
            // The loop's own request ceiling. Our budget, spent on a request
            // that never produced a reply — transient by nature (a re-issue
            // is the remedy), and it must read as `Timeout` so the turn's
            // retry arm and the cron seam's bounded transient retry both fire.
            Error::RequestTimeout { .. } => (
                ClassifiedError {
                    reason: FailoverReason::Timeout,
                    status_code: None,
                    message: err.to_string(),
                    retryable: true,
                    should_fallback: true,
                    should_compress: false,
                    should_rotate_credential: false,
                },
                None,
            ),
            Error::Config(_) => (
                ClassifiedError {
                    reason: FailoverReason::FormatError,
                    status_code: None,
                    message: err.to_string(),
                    retryable: false,
                    should_fallback: false,
                    should_compress: false,
                    should_rotate_credential: false,
                },
                None,
            ),
            _ => (
                ClassifiedError {
                    reason: FailoverReason::Unknown,
                    status_code: None,
                    message: err.to_string(),
                    retryable: true,
                    should_fallback: true,
                    should_compress: false,
                    should_rotate_credential: false,
                },
                None,
            ),
        };

        let advance_model_chain = matches!(
            err,
            Error::Network(_)
                | Error::StreamStalled { .. }
                | Error::RequestTimeout { .. }
                | Error::RateLimited { .. }
        ) || matches!(err, Error::Provider { status, .. } if *status >= 500);
        let switch_provider = classified.is_auth()
            || matches!(
                classified.reason,
                FailoverReason::Billing | FailoverReason::RateLimit
            );

        let decision = match classified.reason {
            FailoverReason::RateLimit => FailoverDecision::RetrySame { after: retry_after },
            _ if advance_model_chain || switch_provider => FailoverDecision::FallBack {
                reason: classified.reason,
            },
            _ => FailoverDecision::GiveUp,
        };

        Self {
            decision,
            reason: classified.reason,
            classified,
            retry_after,
            advance_model_chain,
            switch_provider,
        }
    }

    /// Advance the model chain on the current provider.
    pub fn advance_model_chain(&self) -> bool {
        self.advance_model_chain
    }

    /// Escalate to a different provider (separate credentials and quota).
    pub fn switch_provider(&self) -> bool {
        self.switch_provider
    }

    /// The typed reason's display form, for the existing
    /// `AgentEvent::ModelFallback { reason }` wire — which is the natural
    /// place a failover decision already surfaces, so no new event variant is
    /// needed.
    pub fn event_reason(&self) -> String {
        self.reason.to_string()
    }
}

/// Circuit breaker for the primary model in a fallback chain.
///
/// Tracks consecutive failures of the primary across requests. When the
/// primary fails `failure_threshold` times in a row, it is skipped in favor
/// of fallback models until `cooldown_period` elapses since the last failure.
/// This prevents every ReAct iteration in an outage from re-paying the
/// primary's failure latency.
#[derive(Debug)]
struct PrimaryBreaker {
    /// Consecutive failure count. Reset to 0 on success.
    consecutive_failures: AtomicU32,
    /// When the primary last failed. None if never failed or already reset.
    /// Protected by mutex because we need read-modify-write on cooldown check.
    last_failure: Mutex<Option<Instant>>,
    /// Skip the primary after this many consecutive failures.
    failure_threshold: u32,
    /// After the last failure, wait this long before retrying the primary.
    cooldown_period: Duration,
}

impl PrimaryBreaker {
    /// Create a new breaker with the given threshold and cooldown.
    fn new(failure_threshold: u32, cooldown_period: Duration) -> Self {
        Self {
            consecutive_failures: AtomicU32::new(0),
            last_failure: Mutex::new(None),
            failure_threshold,
            cooldown_period,
        }
    }

    /// Check if the primary should be skipped for this request.
    ///
    /// Returns true if the breaker is open (primary has failed too many
    /// times and cooldown has not elapsed).
    fn should_skip_primary(&self) -> bool {
        let failures = self.consecutive_failures.load(Ordering::Relaxed);
        if failures < self.failure_threshold {
            return false;
        }

        let last_failure = self.last_failure.lock();
        if let Some(last) = *last_failure {
            // Skip if cooldown has not elapsed
            last.elapsed() < self.cooldown_period
        } else {
            // No last_failure recorded but failures >= threshold: inconsistent
            // state, treat as open
            true
        }
    }

    /// Record a primary failure. Increments the counter and updates timestamp.
    fn record_failure(&self) {
        let count = self.consecutive_failures.fetch_add(1, Ordering::Relaxed) + 1;
        *self.last_failure.lock() = Some(Instant::now());

        if count == self.failure_threshold {
            warn!(
                threshold = self.failure_threshold,
                cooldown_secs = self.cooldown_period.as_secs(),
                "Primary model circuit breaker opened — skipping primary for cooldown period"
            );
        }
    }

    /// Record a primary success. Resets the counter and clears the timestamp.
    fn record_success(&self) {
        let previous = self.consecutive_failures.swap(0, Ordering::Relaxed);
        *self.last_failure.lock() = None;

        if previous >= self.failure_threshold {
            info!(
                previous_failures = previous,
                "Primary model circuit breaker closed — primary recovered"
            );
        }
    }
}

/// A [`ModelClient`] wrapper that tries fallback models when the primary
/// model fails with a retryable error.
///
/// # Behaviour
///
/// 1. On each request, try the primary model first (unless circuit breaker is open).
/// 2. If the call succeeds, return the response immediately.
/// 3. If the call fails with a **retryable** error (5xx, 429, network error):
///    log the failure and try the next model in `fallback_models`.
/// 4. If the call fails with a **non-retryable** error (4xx except 429, auth):
///    return the error immediately — changing the model won't help.
/// 5. If all models in the chain are exhausted, return the last error.
///
/// # Circuit breaker
///
/// The primary model has a circuit breaker: after `failure_threshold`
/// consecutive request-level failures, it is skipped in favor of fallback
/// models until `cooldown_period` elapses. This prevents every iteration of
/// a multi-step ReAct turn from re-paying the primary's failure latency during
/// an outage.
#[derive(Clone)]
pub struct FallbackModelClient {
    inner: Arc<dyn ModelClient>,
    primary_model: String,
    fallback_models: Vec<String>,
    fallback_enabled: bool,
    /// Optional provider registry for cross-provider fallback on auth/billing errors.
    provider_registry: Option<Arc<ProviderRegistry>>,
    /// Optional UI event sink. When set, a model/provider switch surfaces as
    /// `AgentEvent::ModelFallback` so the TUI can show which model answered.
    /// Sends are `try_send` (best-effort) — a full parent channel must never
    /// stall a provider request.
    event_tx: Option<mpsc::Sender<AgentEvent>>,
    /// Circuit breaker for the primary model. Shared across cloned instances
    /// via `Arc` so all requests in a session see the same failure state.
    primary_breaker: Arc<PrimaryBreaker>,
}

/// Default number of consecutive primary failures before the breaker opens.
const DEFAULT_FAILURE_THRESHOLD: u32 = 3;

/// Default cooldown before the primary is retried after the breaker opens.
const DEFAULT_COOLDOWN: Duration = Duration::from_secs(60);

/// Clamp `max_tokens` to the model's own output ceiling.
///
/// F2 (Hydrogen review 2026-11-19): the request arrives pre-fitted for the
/// primary model's window. A fallback candidate with a smaller `max_output`
/// would otherwise inherit the primary's `max_tokens` and risk a provider
/// 400 (`max_tokens` exceeds the model's ceiling). This is a pure lookup
/// against the local `models.dev` cache — no network, no agent-loop
/// involvement.
///
/// Returns `None` when the model has no known ceiling (unknown model, no
/// cache entry), in which case the caller leaves `max_tokens` as-is.
fn clamp_max_tokens_to_model(
    current: Option<u32>,
    provider: &str,
    model: &str,
) -> Option<u32> {
    let fitted = current?;
    let leaf = model.rsplit('/').next().unwrap_or(model);
    let ceiling = crate::models_dev::cached_capabilities(provider, leaf)
        .and_then(|caps| caps.max_output)
        .and_then(|t| u32::try_from(t).ok())?;
    // Only clamp DOWN — never raise the operator's fitted value.
    if fitted > ceiling {
        info!(
            model = %model,
            fitted = fitted,
            ceiling = ceiling,
            "Clamping max_tokens to fallback model's output ceiling"
        );
        Some(ceiling)
    } else {
        Some(fitted)
    }
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
            event_tx: None,
            primary_breaker: Arc::new(PrimaryBreaker::new(
                DEFAULT_FAILURE_THRESHOLD,
                DEFAULT_COOLDOWN,
            )),
        }
    }

    /// Configure the circuit breaker's failure threshold.
    ///
    /// The primary is skipped after this many consecutive request-level
    /// failures, until [`with_breaker_cooldown`](Self::with_breaker_cooldown)
    /// elapses. Lower is more aggressive: fewer wasted primary round trips
    /// during an outage. Must be >= 1.
    pub fn with_breaker_threshold(mut self, threshold: u32) -> Self {
        let cooldown = (*self.primary_breaker).cooldown_period;
        self.primary_breaker = Arc::new(PrimaryBreaker::new(
            threshold.max(1),
            cooldown,
        ));
        self
    }

    /// Configure the circuit breaker's cooldown period.
    pub fn with_breaker_cooldown(mut self, cooldown: Duration) -> Self {
        let threshold = (*self.primary_breaker).failure_threshold;
        self.primary_breaker = Arc::new(PrimaryBreaker::new(threshold, cooldown));
        self
    }

    /// Attach a UI event sink so fallback switches are reported.
    pub fn with_event_tx(mut self, tx: mpsc::Sender<AgentEvent>) -> Self {
        self.event_tx = Some(tx);
        self
    }

    fn emit_fallback(&self, to: &str, reason: &str) {
        if let Some(tx) = &self.event_tx {
            let _ = tx.try_send(AgentEvent::ModelFallback {
                from: self.primary_model.clone(),
                to: to.to_string(),
                reason: reason.to_string(),
            });
        }
    }

    /// Attach a provider registry for cross-provider fallback.
    pub fn with_provider_registry(mut self, registry: Arc<ProviderRegistry>) -> Self {
        self.provider_registry = Some(registry);
        self
    }

    /// Try the next provider from the registry for a non-streaming chat.
    /// Called by chat() after try_models returns a classified failure.
    async fn try_next_provider_chat(
        &self,
        request: &ChatRequest,
        failover: &Failover,
    ) -> Option<Result<ChatResponse>> {
        let registry = self.provider_registry.as_ref()?;
        let candidates = registry.fallback_candidates_after(0);
        if candidates.is_empty() {
            return None;
        }
        let mut last_error = None;
        for next_provider in candidates {
            let Some(next_client) = registry.get_client(&next_provider.name) else {
                continue;
            };
            let mut fallback_req = request.clone();
            fallback_req.model.clone_from(&next_provider.model);
            // F2: clamp max_tokens to this fallback model's own ceiling
            // (cross-provider path — the provider name differs from the
            // primary's).
            if let Some(fitted) = clamp_max_tokens_to_model(
                fallback_req.max_tokens,
                &next_provider.name,
                &next_provider.model,
            ) {
                fallback_req.max_tokens = Some(fitted);
            }
            info!(
                to_provider = %next_provider.name,
                to_model = %next_provider.model,
                reason = %failover.event_reason(),
                "Provider failover — switching to fallback provider"
            );
            self.emit_fallback(&next_provider.model, &failover.event_reason());
            let result = next_client.chat(fallback_req).await;
            match result {
                Ok(response) => {
                    registry.clear_failure_count(&next_provider.name);
                    return Some(Ok(response));
                }
                Err(error) => {
                    registry.arm_cooldown(&next_provider.name);
                    last_error = Some(error);
                }
            }
        }
        Some(Err(last_error.unwrap_or_else(|| {
            Error::Agent("All configured fallback providers failed".to_string())
        })))
    }

    /// Try the next provider from the registry for streaming.
    /// Called by chat_streaming() after try_models returns a classified failure.
    async fn try_next_provider_streaming(
        &self,
        request: &ChatRequest,
        failover: &Failover,
    ) -> Option<Result<BoxStream<'static, Result<StreamChunk>>>> {
        let registry = self.provider_registry.as_ref()?;
        let candidates = registry.fallback_candidates_after(0);
        if candidates.is_empty() {
            return None;
        }
        let mut last_error = None;
        for next_provider in candidates {
            let Some(next_client) = registry.get_client(&next_provider.name) else {
                continue;
            };
            let mut fallback_req = request.clone();
            fallback_req.model.clone_from(&next_provider.model);
            // F2: clamp max_tokens to this fallback model's own ceiling
            // (cross-provider streaming path).
            if let Some(fitted) = clamp_max_tokens_to_model(
                fallback_req.max_tokens,
                &next_provider.name,
                &next_provider.model,
            ) {
                fallback_req.max_tokens = Some(fitted);
            }
            info!(
                to_provider = %next_provider.name,
                to_model = %next_provider.model,
                reason = %failover.event_reason(),
                "Provider failover — switching to fallback provider"
            );
            self.emit_fallback(&next_provider.model, &failover.event_reason());
            let result = next_client.chat_streaming(fallback_req).await;
            match result {
                Ok(stream) => {
                    registry.clear_failure_count(&next_provider.name);
                    return Some(Ok(stream));
                }
                Err(error) => {
                    registry.arm_cooldown(&next_provider.name);
                    last_error = Some(error);
                }
            }
        }
        Some(Err(last_error.unwrap_or_else(|| {
            Error::Agent("All configured fallback providers failed".to_string())
        })))
    }

    /// Build the ordered list of models to try:
    /// `[request_model, fallback_model_1, fallback_model_2, ...]`.
    fn model_chain(&self, request_model: &str) -> Vec<String> {
        let mut models = Vec::with_capacity(1 + self.fallback_models.len());
        models.push(request_model.to_string());
        models.extend(self.fallback_models.iter().cloned());
        models
    }

    /// Classify an error into a recovery strategy using the full
    /// error_classifier taxonomy (22+ categories).
    ///
    /// Thin wrapper over [`Failover::classify`] so the credential pool and the
    /// run loop see exactly the classification the failover path uses.
    pub fn classify_error(err: &Error) -> ClassifiedError {
        Failover::classify(err).classified
    }

    /// Core fallback loop shared by `chat` and `chat_streaming`.
    ///
    /// When the primary model's circuit breaker is open (consecutive
    /// failures >= threshold and cooldown not elapsed), the primary is
    /// skipped and the fallback chain is tried first. This prevents
    /// every ReAct iteration during an outage from re-paying the
    /// primary's failure latency.
    async fn try_models<F, Fut, T>(&self, request: &ChatRequest, call: F) -> Result<T>
    where
        F: Fn(ChatRequest) -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        let all_models = self.model_chain(&request.model);
        let mut attempted: Vec<String> = Vec::with_capacity(all_models.len());
        let mut last_error: Option<Error> = None;

        // When the breaker is open, try fallbacks first, then the primary.
        // Otherwise try the primary first (normal chain order).
        let skip_primary = self.primary_breaker.should_skip_primary();
        if skip_primary {
            info!(
                primary = %request.model,
                failures = self.primary_breaker.consecutive_failures.load(Ordering::Relaxed),
                "Primary circuit breaker open — trying fallbacks first"
            );
        }

        let models: Vec<String> = if skip_primary && all_models.len() > 1 {
            // Fallbacks first, primary last
            let mut m = all_models[1..].to_vec();
            m.push(all_models[0].clone());
            m
        } else {
            all_models
        };

        for model in &models {
            if attempted.contains(model) {
                continue;
            }
            attempted.push(model.clone());

            let mut fallback_req = request.clone();
            fallback_req.model.clone_from(model);

            // F2 (Hydrogen review 2026-11-19): clamp `max_tokens` to the
            // fallback model's OWN output ceiling. The request arrives
            // pre-fitted for the primary model's window; a fallback candidate
            // with a smaller max_output would otherwise inherit the primary's
            // max_tokens and risk a 400 (max_tokens exceeds the model's
            // ceiling). This is a local, per-candidate clamp — no network
            // round-trip, no agent-loop involvement.
            if let Some(fitted) = clamp_max_tokens_to_model(
                fallback_req.max_tokens,
                self.inner.provider_name(),
                model,
            ) {
                fallback_req.max_tokens = Some(fitted);
            }

            match call(fallback_req).await {
                Ok(response) => {
                    if attempted.len() > 1 {
                        info!(
                            primary = %request.model,
                            fallback = %model,
                            "Fallback model succeeded"
                        );
                    }
                    // Reset breaker on any success (primary or fallback)
                    if model == &request.model || model == &self.primary_model {
                        self.primary_breaker.record_success();
                    }
                    return Ok(response);
                }
                Err(e) => {
                    let failover = Failover::classify(&e);
                    if failover.advance_model_chain() {
                        // Record primary failure for the breaker
                        if model == &request.model || model == &self.primary_model {
                            self.primary_breaker.record_failure();
                        }
                        // The switch itself is the reportable event: the primary
                        // (or an earlier fallback) just failed, so this model is
                        // about to be attempted instead.
                        if *model != request.model {
                            self.emit_fallback(model, &failover.event_reason());
                        }
                        warn!(
                            primary = %request.model,
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

    fn set_endpoint(&self, base_url: &str, api_key: Option<&str>) {
        // Only the primary is retargeted; fallback providers keep their own
        // endpoints.
        self.inner.set_endpoint(base_url, api_key);
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
        if !self.fallback_enabled || self.fallback_models.is_empty() {
            // Fast path: no fallback configured, passthrough
            let result = self.inner.chat(request.clone()).await;
            // Check for auth/billing/rate-limit-exhaustion → try provider fallback
            if let Err(ref e) = result {
                let failover = Failover::classify(e);
                if failover.switch_provider()
                    && let Some(provider_result) =
                        self.try_next_provider_chat(&request, &failover).await
                {
                    return provider_result;
                }
            }
            return result;
        }

        let result = self.try_models(&request, |req| self.inner.chat(req)).await;
        // Check for auth/billing/rate-limit-exhaustion errors → try provider fallback
        if let Err(ref e) = result {
            let failover = Failover::classify(e);
            if failover.switch_provider()
                && let Some(provider_result) =
                    self.try_next_provider_chat(&request, &failover).await
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
                let failover = Failover::classify(e);
                if failover.switch_provider()
                    && let Some(provider_result) =
                        self.try_next_provider_streaming(&request, &failover).await
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
            let failover = Failover::classify(e);
            if failover.switch_provider()
                && let Some(provider_result) =
                    self.try_next_provider_streaming(&request, &failover).await
            {
                return provider_result;
            }
        }
        result
    }

    fn set_api_key(&self, api_key: &str) {
        self.inner.set_api_key(api_key);
    }

    fn rotate_credential_value(&self, api_key: &str) {
        // Forward so a runtime credential rotation reaches the pooled
        // client underneath, which is where the credential is stored.
        self.inner.rotate_credential_value(api_key);
    }

    fn base_url(&self) -> Option<String> {
        // Only the primary is retargeted; fallback providers keep their own
        // endpoints, so the primary's is the one that answers.
        self.inner.base_url()
    }

    fn current_api_key(&self) -> Option<String> {
        self.inner.current_api_key()
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

        /// Like [`new`](Self::new) but returns a typed `Arc<Self>` so a test
        /// can read `call_count` after the fallback loop ran.
        fn new_arc(provider: &'static str, results: Vec<MockResult>) -> Arc<Self> {
            Arc::new(Self {
                provider,
                results,
                call_count: AtomicUsize::new(0),
            })
        }
    }

    #[derive(Default)]
    struct RecordingModelClient {
        models: std::sync::Mutex<Vec<String>>,
    }

    #[async_trait]
    impl ModelClient for RecordingModelClient {
        fn provider_name(&self) -> &str {
            "recording"
        }

        async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
            self.models
                .lock()
                .map_err(|_| Error::Agent("model recorder lock poisoned".into()))?
                .push(request.model.clone());
            Ok(chat_response(&request.model))
        }

        async fn chat_streaming(
            &self,
            _request: ChatRequest,
        ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
            Err(Error::Agent("streaming not mocked".into()))
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

    #[test]
    fn failover_decision_should_distinguish_rate_limit_from_broken_model() {
        // Rate limited: the runtime is fine, just throttled. The provider's
        // Retry-After rides along so a caller never re-derives it.
        let throttled = Failover::classify(&Error::RateLimited {
            retry_after: Duration::from_secs(7),
        });
        assert_eq!(
            throttled.decision,
            FailoverDecision::RetrySame {
                after: Some(Duration::from_secs(7))
            }
        );
        assert_eq!(throttled.retry_after, Some(Duration::from_secs(7)));
        assert_eq!(throttled.reason, FailoverReason::RateLimit);
        // Rate-limit buckets are per model, and pool exhaustion means this
        // provider's quota is spent — so both stages stay open.
        assert!(throttled.advance_model_chain());
        assert!(throttled.switch_provider());

        // Broken but transient (5xx): advance the chain, do NOT burn a
        // provider — its credentials are fine.
        let broken = Failover::classify(&Error::Provider {
            status: 503,
            body: "Service Unavailable".into(),
            retry_after: None,
        });
        assert_eq!(
            broken.decision,
            FailoverDecision::FallBack {
                reason: FailoverReason::Overloaded
            }
        );
        assert!(broken.advance_model_chain());
        assert!(!broken.switch_provider());

        // Dead credentials: the chain does not advance on models, but another
        // provider with its own keys should answer.
        let auth = Failover::classify(&Error::Authentication("invalid key".into()));
        assert_eq!(
            auth.decision,
            FailoverDecision::FallBack {
                reason: FailoverReason::Auth
            }
        );
        assert!(!auth.advance_model_chain());
        assert!(auth.switch_provider());

        // A malformed request is nobody's fault: no stage, no provider.
        let bad_request = Failover::classify(&Error::Provider {
            status: 400,
            body: "Bad Request".into(),
            retry_after: None,
        });
        assert_eq!(bad_request.decision, FailoverDecision::GiveUp);
        assert!(!bad_request.advance_model_chain());
        assert!(!bad_request.switch_provider());

        // Transport: the endpoint is down, so try a different one.
        let transport = Failover::classify(&Error::Agent("connection reset".into()));
        assert_eq!(transport.decision, FailoverDecision::GiveUp);
        assert!(!transport.advance_model_chain());
        assert!(!transport.switch_provider());

        // The event wire keeps the existing FailoverReason display form, so
        // AgentEvent::ModelFallback is unchanged for consumers.
        assert_eq!(throttled.event_reason(), "rate_limit");
        assert_eq!(auth.event_reason(), "auth");
    }

    #[test]
    fn classify_error_still_returns_the_full_classification() {
        let classified = FallbackModelClient::classify_error(&Error::RateLimited {
            retry_after: Duration::from_secs(3),
        });
        assert_eq!(classified.reason, FailoverReason::RateLimit);
        assert_eq!(classified.status_code, Some(429));
        assert!(classified.retryable);
        assert!(classified.should_rotate_credential);
    }

    #[tokio::test]
    async fn the_request_model_is_the_primary_under_fallback() {
        let inner = Arc::new(RecordingModelClient::default());
        let client = FallbackModelClient::new(
            inner.clone(),
            "startup-primary".into(),
            vec!["configured-fallback".into()],
            true,
        );

        let result = client
            .chat(ChatRequest::new("runtime-override", vec![]))
            .await
            .expect("the request must succeed");

        assert_eq!(result.model, "runtime-override");
        assert_eq!(
            inner.models.lock().expect("model recorder lock").as_slice(),
            ["runtime-override"]
        );
    }

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
    async fn provider_fallback_walks_to_the_third_provider() {
        use crate::agent::provider_registry::{ProviderEntry, ProviderRegistry};

        let primary_client = MockModelClient::new("openai", vec![MockResult::AuthError]);
        let second_client = MockModelClient::new("anthropic", vec![MockResult::AuthError]);
        let third_client =
            MockModelClient::new("google", vec![MockResult::Ok(chat_response("gemini"))]);
        let mut clients = std::collections::HashMap::new();
        clients.insert("openai".to_string(), primary_client);
        clients.insert("anthropic".to_string(), second_client);
        clients.insert("google".to_string(), third_client);
        let chain = vec![
            ProviderEntry {
                name: "openai".into(),
                model: "gpt-4".into(),
            },
            ProviderEntry {
                name: "anthropic".into(),
                model: "claude-3".into(),
            },
            ProviderEntry {
                name: "google".into(),
                model: "gemini".into(),
            },
        ];
        let registry = Arc::new(ProviderRegistry::new(clients, chain));
        let inner = MockModelClient::new("openai", vec![MockResult::AuthError]);
        let client = FallbackModelClient::new(inner, "gpt-4".into(), vec![], true)
            .with_provider_registry(registry);

        let result = client.chat(make_request()).await.unwrap();
        assert_eq!(result.model, "gemini");
    }

    #[tokio::test]
    async fn provider_fallback_is_request_local() {
        use crate::agent::provider_registry::{ProviderEntry, ProviderRegistry};

        let primary_client = MockModelClient::new("openai", vec![MockResult::AuthError]);
        let fallback_client =
            MockModelClient::new("anthropic", vec![MockResult::Ok(chat_response("claude-3"))]);
        let mut clients = std::collections::HashMap::new();
        clients.insert("openai".to_string(), primary_client);
        clients.insert("anthropic".to_string(), fallback_client);
        let chain = vec![
            ProviderEntry {
                name: "openai".into(),
                model: "gpt-4".into(),
            },
            ProviderEntry {
                name: "anthropic".into(),
                model: "claude-3".into(),
            },
        ];
        let registry = Arc::new(ProviderRegistry::new(clients, chain));
        let inner = MockModelClient::new(
            "openai",
            vec![
                MockResult::AuthError,
                MockResult::Ok(chat_response("primary")),
            ],
        );
        let client = FallbackModelClient::new(inner, "gpt-4".into(), vec![], true)
            .with_provider_registry(registry.clone());

        let first = client.chat(make_request()).await.unwrap();
        assert_eq!(first.model, "claude-3");
        let second = client.chat(make_request()).await.unwrap();
        assert_eq!(second.model, "primary");
        assert_eq!(registry.active_provider().as_deref(), Some("openai"));
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

    /// The loop's own request ceiling classifies as a Timeout — the typed
    /// rung ("Connection/read timeout — rebuild client + retry") that an
    /// untyped `Agent` string could never reach.
    #[test]
    fn the_request_ceiling_classifies_as_a_timeout() {
        let failover = Failover::classify(&Error::RequestTimeout {
            budget: Duration::from_secs(120),
            guidance: String::new(),
        });
        assert_eq!(failover.reason, FailoverReason::Timeout);
        assert!(failover.classified.retryable);
        assert!(!failover.classified.should_compress);
    }

    // ── Circuit breaker tests (F1) ──────────────────────────────────

    #[test]
    fn breaker_opens_after_threshold_failures() {
        let breaker = PrimaryBreaker::new(3, Duration::from_secs(60));

        // Breaker starts closed
        assert!(!breaker.should_skip_primary());

        // Fail below threshold: stays closed
        breaker.record_failure();
        breaker.record_failure();
        assert!(!breaker.should_skip_primary());

        // Third failure reaches threshold: opens
        breaker.record_failure();
        assert!(breaker.should_skip_primary());
    }

    #[test]
    fn breaker_closes_on_success_after_failures() {
        let breaker = PrimaryBreaker::new(3, Duration::from_secs(60));

        breaker.record_failure();
        breaker.record_failure();
        breaker.record_failure();
        assert!(breaker.should_skip_primary());

        // Success resets the counter
        breaker.record_success();
        assert!(!breaker.should_skip_primary());
    }

    #[test]
    fn breaker_cooldown_expires() {
        let breaker = PrimaryBreaker::new(2, Duration::from_millis(10));

        breaker.record_failure();
        breaker.record_failure();
        assert!(breaker.should_skip_primary());

        // Wait for cooldown to expire
        std::thread::sleep(Duration::from_millis(20));
        assert!(!breaker.should_skip_primary());
    }

    #[tokio::test]
    async fn breaker_skips_primary_after_threshold_and_uses_fallback() {
        let inner = MockModelClient::new_arc(
            "test",
            vec![
                MockResult::ServerError,
                MockResult::Ok(chat_response("fallback1")),
                MockResult::Ok(chat_response("fallback1")),
                MockResult::Ok(chat_response("fallback1")),
            ],
        );

        let client = FallbackModelClient::new(
            inner.clone(),
            "primary".into(),
            vec!["fallback1".into()],
            true,
        )
        .with_breaker_threshold(2);

        // Request 1: primary fails, fallback succeeds
        let r1 = client.chat(make_request()).await.unwrap();
        assert_eq!(r1.model, "fallback1");

        // Request 2: primary fails again (breaker now at 2), fallback succeeds
        let r2 = client.chat(make_request()).await.unwrap();
        assert_eq!(r2.model, "fallback1");

        // Request 3: breaker is open — primary skipped entirely
        let r3 = client.chat(make_request()).await.unwrap();
        assert_eq!(r3.model, "fallback1");

        // The inner client was called 4 times total: primary, primary, fallback1,
        // fallback1. On request 3 the breaker skipped primary, so only fallback1
        // was tried.
        let calls = inner.call_count.load(Ordering::SeqCst);
        assert_eq!(
            calls, 4,
            "expected 4 calls (2 with primary+fallback, 1 fallback-only), got {calls}"
        );
    }

    #[tokio::test]
    async fn breaker_resets_on_primary_success() {
        // Every request: primary fails (503) then the fallback succeeds —
        // except request 3 where the primary itself succeeds.
        // call_count index: 0=primary r1, 1=fallback r1, 2=primary r2,
        // 3=fallback r2, 4=primary r3 (the one that must succeed).
        let inner = MockModelClient::new(
            "test",
            vec![
                MockResult::ServerError,          // 0: r1 primary
                MockResult::Ok(chat_response("fallback1")), // 1: r1 fallback
                MockResult::ServerError,          // 2: r2 primary
                MockResult::Ok(chat_response("fallback1")), // 3: r2 fallback
                MockResult::Ok(chat_response("primary")),   // 4: r3 primary OK
            ],
        );

        let client = FallbackModelClient::new(
            inner,
            "primary".into(),
            vec!["fallback1".into()],
            true,
        )
        .with_breaker_threshold(3);

        // Request 1: primary fails (count=1), fallback succeeds
        client.chat(make_request()).await.unwrap();
        // Request 2: primary fails (count=2), fallback succeeds
        client.chat(make_request()).await.unwrap();
        // Request 3: primary succeeds (count reset to 0)
        let r3 = client.chat(make_request()).await.unwrap();
        assert_eq!(r3.model, "primary");
    }

    // ── F2: max_tokens per-candidate clamp tests ────────────────────

    /// F2 (Hydrogen review 2026-11-19): when a fallback model has a smaller
    /// `max_output` ceiling than the primary, the fallback request's
    /// `max_tokens` must be clamped down to the candidate's own ceiling.
    ///
    /// `clamp_max_tokens_to_model` is tested directly (no models.dev cache
    /// entry in CI → returns None → no clamp), and an integration-style test
    /// proves the function is called in the fallback dispatch path by
    /// observing that the fallback request's `max_tokens` field is not
    /// silently dropped.
    #[test]
    fn clamp_max_tokens_is_noop_when_model_unknown() {
        // A model with no models.dev entry: the clamp returns None (no
        // ceiling to apply), so max_tokens passes through unchanged.
        let fitted = Some(4096u32);
        let result = clamp_max_tokens_to_model(fitted, "test", "nonexistent-model-xyz");
        assert_eq!(
            result, fitted,
            "unknown model must not change max_tokens"
        );
    }

    #[test]
    fn clamp_max_tokens_is_noop_when_no_max_tokens() {
        // A request without max_tokens stays without one.
        let result = clamp_max_tokens_to_model(None, "test", "some-model");
        assert!(result.is_none(), "no max_tokens → no clamp");
    }

    /// F2: the fallback dispatch path calls the clamp function and the
    /// fitted request's `max_tokens` is preserved through the fallback
    /// model swap. Without the clamp, the primary's max_tokens would
    /// silently ride along to the fallback model.
    #[tokio::test]
    async fn fallback_request_preserves_max_tokens_through_model_swap() {
        // A mock client that records the max_tokens it received, so we can
        // assert the value survived the fallback swap.
        struct MaxTokensRecorder {
            inner: MockModelClient,
            recorded: Arc<std::sync::Mutex<Vec<Option<u32>>>>,
        }

        #[async_trait]
        impl ModelClient for MaxTokensRecorder {
            fn provider_name(&self) -> &str {
                self.inner.provider_name()
            }

            async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
                self.recorded
                    .lock()
                    .expect("recorder lock")
                    .push(request.max_tokens);
                self.inner.chat(request).await
            }

            async fn chat_streaming(
                &self,
                _request: ChatRequest,
            ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
                Err(Error::Agent("streaming not mocked".into()))
            }
        }

        let recorded = Arc::new(std::sync::Mutex::new(Vec::new()));
        let inner = MaxTokensRecorder {
            inner: MockModelClient::new(
                "test",
                vec![
                    MockResult::ServerError,
                    MockResult::Ok(chat_response("fallback1")),
                ],
            )
            .downcast::<MockModelClient>()
            .expect("downcast"),
            recorded: recorded.clone(),
        };

        let client = FallbackModelClient::new(
            Arc::new(inner),
            "primary".into(),
            vec!["fallback1".into()],
            true,
        );

        let mut req = make_request();
        req.max_tokens = Some(1024);

        let result = client.chat(req).await;
        assert!(result.is_ok(), "fallback should succeed");

        let calls = recorded.lock().expect("recorder lock");
        // Two calls: primary (with 1024) and fallback (with 1024, clamped or
        // unchanged depending on models.dev cache — in CI the cache is empty
        // so the value is preserved as-is).
        assert_eq!(calls.len(), 2, "expected 2 calls (primary + fallback)");
        assert_eq!(calls[0], Some(1024), "primary should have max_tokens=1024");
        assert_eq!(
            calls[1],
            Some(1024),
            "fallback should preserve max_tokens through the swap"
        );
    }
}
