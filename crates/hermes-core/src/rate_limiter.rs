//! Token-bucket rate limiter per model/provider.
//!
//! Provides proactive rate limiting to avoid HTTP 429 responses,
//! with automatic retry support and status querying for the gateway.
//!
//! Each model or provider gets its own token bucket. Tokens are
//! consumed on each API request and refilled at a fixed rate.
//! On a 429 response the bucket is drained and the caller waits
//! for the Retry-After duration plus exponential backoff.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::Mutex;

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// Errors specific to rate limiting operations.
#[derive(Debug, Clone)]
pub enum RateLimitError {
    /// Too many requests — wait before retrying.
    TooManyRequests {
        /// Recommended wait time in seconds before retrying.
        retry_after_secs: u64,
    },
    /// Hard quota exceeded — no further requests allowed.
    QuotaExceeded,
}

impl std::fmt::Display for RateLimitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooManyRequests { retry_after_secs } => {
                write!(
                    f,
                    "Rate limit exceeded, retry after {}s",
                    retry_after_secs
                )
            }
            Self::QuotaExceeded => write!(f, "Rate limit quota exceeded"),
        }
    }
}

impl std::error::Error for RateLimitError {}

// ---------------------------------------------------------------------------
// Status
// ---------------------------------------------------------------------------

/// Current status of a rate limit bucket.
#[derive(Debug, Clone)]
pub struct RateLimitStatus {
    /// Remaining tokens in the bucket.
    pub remaining: u32,
    /// Maximum capacity of the bucket.
    pub limit: u32,
    /// Approximate instant when the bucket will be fully replenished.
    pub reset_at: Option<Instant>,
}

// ---------------------------------------------------------------------------
// TokenBucket
// ---------------------------------------------------------------------------

/// A token bucket that governs request frequency for a single model/provider.
///
/// Tokens are consumed on each outbound request and gradually refilled
/// at a fixed rate (`refill_rate` tokens per second).  A bucket starts
/// full and never exceeds its `capacity`.
#[derive(Debug, Clone)]
pub struct TokenBucket {
    /// Maximum number of tokens the bucket can hold.
    capacity: u32,
    /// Current number of available tokens.
    remaining: u32,
    /// Tokens added per second during refill.
    refill_rate: f64,
    /// Wall-clock instant of the most recent refill.
    last_refill: Instant,
}

impl TokenBucket {
    /// Create a new token bucket, starting full.
    pub fn new(capacity: u32, refill_rate: f64) -> Self {
        Self {
            capacity,
            remaining: capacity,
            refill_rate,
            last_refill: Instant::now(),
        }
    }

    /// Refill the bucket based on real time elapsed since the last refill.
    ///
    /// The number of tokens added is `elapsed_seconds * refill_rate`,
    /// capped at `capacity`.  No-op when no time has passed or when the
    /// bucket is already full.
    pub fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill);
        let tokens_to_add = (elapsed.as_secs_f64() * self.refill_rate) as u32;
        if tokens_to_add > 0 {
            self.remaining = (self.remaining + tokens_to_add).min(self.capacity);
            self.last_refill = now;
        }
    }

    /// Try to consume one token from the bucket.
    ///
    /// Returns `true` if a token was available (and consumed), `false` if
    /// the bucket is empty.  Call [`refill`](Self::refill) first to
    /// replenish.
    pub fn try_consume(&mut self) -> bool {
        if self.remaining > 0 {
            self.remaining -= 1;
            true
        } else {
            false
        }
    }

    /// Return the number of tokens currently available.
    pub fn remaining(&self) -> u32 {
        self.remaining
    }

    /// Drain all remaining tokens (simulate immediate exhaustion).
    pub fn drain(&mut self) {
        self.remaining = 0;
    }

    /// Minimum wall-clock duration until the next token becomes available.
    ///
    /// Returns [`Duration::ZERO`] when tokens are already available.
    pub fn time_until_next_token(&self) -> std::time::Duration {
        if self.remaining > 0 {
            return std::time::Duration::ZERO;
        }
        if self.refill_rate <= 0.0 {
            return std::time::Duration::MAX;
        }
        std::time::Duration::from_secs_f64(1.0 / self.refill_rate)
    }

    /// Approximate instant when the bucket will be back at full capacity.
    pub fn full_reset_instant(&self) -> Instant {
        let replenish_secs = self.capacity as f64 / self.refill_rate.max(0.001);
        self.last_refill + std::time::Duration::from_secs_f64(replenish_secs)
    }
}

// ---------------------------------------------------------------------------
// RateLimiter
// ---------------------------------------------------------------------------

/// Thread-safe collection of token buckets, keyed by model or provider name.
///
/// All interior mutation is protected by a [`tokio::sync::Mutex`] so the
/// rate limiter can be shared across concurrent gateway tasks via [`Arc`].
#[derive(Debug, Clone)]
pub struct RateLimiter {
    inner: Arc<Mutex<HashMap<String, TokenBucket>>>,
    default_capacity: u32,
    default_refill_rate: f64,
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new(60, 1.0)
    }
}

impl RateLimiter {
    /// Create a new rate limiter with the given default bucket parameters.
    ///
    /// * `default_capacity` — starting and maximum tokens for new buckets.
    /// * `default_refill_rate` — tokens replenished per second.
    pub fn new(default_capacity: u32, default_refill_rate: f64) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            default_capacity,
            default_refill_rate,
        }
    }

    /// Check whether a request for `model` is allowed right now.
    ///
    /// On success a token is consumed from the model's bucket.
    /// On failure (bucket empty) a [`RateLimitError`] describing the
    /// necessary wait time is returned.
    pub async fn check_rate_limit(
        &self,
        model: &str,
    ) -> std::result::Result<(), RateLimitError> {
        let mut buckets = self.inner.lock().await;
        let bucket = self.bucket_or_create(&mut buckets, model);

        bucket.refill();
        if bucket.try_consume() {
            Ok(())
        } else {
            let retry_after = bucket.time_until_next_token().as_secs().max(1);
            Err(RateLimitError::TooManyRequests { retry_after_secs: retry_after })
        }
    }

    /// Record artificial token consumption for `model`.
    ///
    /// Used after receiving a 429 response to drain the bucket so
    /// subsequent requests are blocked until the server's Retry-After
    /// window passes.
    pub async fn record_usage(&self, model: &str, tokens: u32) {
        let mut buckets = self.inner.lock().await;
        let bucket = self.bucket_or_create(&mut buckets, model);
        if tokens > 0 {
            bucket.remaining = bucket.remaining.saturating_sub(tokens);
        }
    }

    /// Drain all remaining tokens for `model` (simulate rate-limit hit).
    pub async fn drain_bucket(&self, model: &str) {
        let mut buckets = self.inner.lock().await;
        self.bucket_or_create(&mut buckets, model).drain();
    }

    /// Query the current rate-limit status for `model`.
    ///
    /// Returns a default (full) status when no bucket exists for the
    /// requested model yet.
    pub async fn get_status(&self, model: &str) -> RateLimitStatus {
        let buckets = self.inner.lock().await;
        match buckets.get(model) {
            Some(bucket) => RateLimitStatus {
                remaining: bucket.remaining(),
                limit: bucket.capacity,
                reset_at: Some(bucket.full_reset_instant()),
            },
            None => RateLimitStatus {
                remaining: self.default_capacity,
                limit: self.default_capacity,
                reset_at: None,
            },
        }
    }

    /// A list of all models currently being tracked.
    pub async fn tracked_models(&self) -> Vec<String> {
        let buckets = self.inner.lock().await;
        buckets.keys().cloned().collect()
    }

    // -- internal helpers ------------------------------------------------

    fn bucket_or_create<'a>(
        &self,
        buckets: &'a mut HashMap<String, TokenBucket>,
        model: &str,
    ) -> &'a mut TokenBucket {
        buckets.entry(model.to_string()).or_insert_with(|| {
            TokenBucket::new(self.default_capacity, self.default_refill_rate)
        })
    }
}

// ---------------------------------------------------------------------------
// Helper: parse `Retry-After` header
// ---------------------------------------------------------------------------

/// Parse the `Retry-After` response header into a [`std::time::Duration`].
///
/// Supports two formats defined in RFC 7231:
///
/// 1. **Delta-seconds** — an integer number of seconds to wait.
/// 2. **HTTP-date** — an absolute timestamp after which to retry (parsed
///    as IMF-fixdate / RFC 1123 / RFC 2822 via chrono).
///
/// Returns `None` when the header is absent or unparseable.
pub fn parse_retry_after_header(
    headers: &reqwest::header::HeaderMap,
) -> Option<std::time::Duration> {
    let value = headers.get(reqwest::header::RETRY_AFTER)?;
    let text = value.to_str().ok()?.trim();

    // 1. Integer seconds.
    if let Ok(secs) = text.parse::<u64>() {
        return Some(std::time::Duration::from_secs(secs));
    }

    // 2. HTTP-date (IMF-fixdate / RFC 2822).
    if let Ok(dt) = chrono::DateTime::parse_from_rfc2822(text) {
        let now = chrono::Utc::now();
        let target = dt.with_timezone(&chrono::Utc);
        let diff = target.signed_duration_since(now);
        let secs = diff.num_seconds().max(0) as u64;
        return Some(std::time::Duration::from_secs(secs));
    }

    None
}

/// Compute exponential-backoff delay (in seconds) for the given attempt.
///
/// Formula: `delay = min(base_delay * 2^(attempt-1), max_delay)`
///
/// Used both for retry-after on 429 responses and for transient server
/// errors (5xx, network blips).
pub fn exponential_backoff_secs(attempt: u32, base_delay: u64, max_delay: u64) -> u64 {
    if attempt == 0 {
        return base_delay;
    }
    let factor = 2u64.saturating_pow(attempt - 1);
    base_delay.saturating_mul(factor).min(max_delay)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    // -----------------------------------------------------------------------
    // TokenBucket
    // -----------------------------------------------------------------------

    #[test]
    fn token_bucket_starts_full() {
        let tb = TokenBucket::new(10, 1.0);
        assert_eq!(tb.remaining(), 10);
    }

    #[test]
    fn token_bucket_consume_returns_true_when_available() {
        let mut tb = TokenBucket::new(5, 1.0);
        assert!(tb.try_consume());
        assert_eq!(tb.remaining(), 4);
    }

    #[test]
    fn token_bucket_consume_returns_false_when_empty() {
        let mut tb = TokenBucket::new(1, 1.0);
        assert!(tb.try_consume());
        assert!(!tb.try_consume());
        assert_eq!(tb.remaining(), 0);
    }

    #[test]
    fn token_bucket_refill_adds_tokens() {
        let mut tb = TokenBucket::new(100, 1000.0); // very fast refill
        tb.try_consume(); // 99 remaining
        // Force last_refill into the past
        tb.last_refill = Instant::now() - Duration::from_millis(100);
        tb.refill();
        // Should have added >= 100 tokens, but capped at capacity
        assert_eq!(tb.remaining(), 100);
    }

    #[test]
    fn token_bucket_refill_does_not_exceed_capacity() {
        let mut tb = TokenBucket::new(10, 1000.0);
        tb.last_refill = Instant::now() - Duration::from_secs(10);
        tb.refill();
        assert_eq!(tb.remaining(), 10);
    }

    #[test]
    fn token_bucket_drain_sets_to_zero() {
        let mut tb = TokenBucket::new(10, 1.0);
        tb.drain();
        assert_eq!(tb.remaining(), 0);
    }

    #[test]
    fn token_bucket_time_until_next_token_when_full() {
        let tb = TokenBucket::new(10, 1.0);
        assert_eq!(tb.time_until_next_token(), Duration::ZERO);
    }

    #[test]
    fn token_bucket_time_until_next_token_when_empty() {
        let mut tb = TokenBucket::new(10, 2.0);
        tb.drain();
        let t = tb.time_until_next_token();
        assert!(t > Duration::ZERO);
        assert!(t <= Duration::from_millis(501)); // 1/2 sec
    }

    #[test]
    fn token_bucket_full_reset_instant_is_in_future() {
        let mut tb = TokenBucket::new(60, 1.0);
        tb.drain();
        let reset = tb.full_reset_instant();
        assert!(reset > Instant::now());
    }

    // -----------------------------------------------------------------------
    // RateLimiter
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn rate_limiter_allows_request_when_bucket_full() {
        let limiter = RateLimiter::new(10, 10.0);
        assert!(limiter.check_rate_limit("gpt-4").await.is_ok());
    }

    #[tokio::test]
    async fn rate_limiter_blocks_when_bucket_empty() {
        let limiter = RateLimiter::new(1, 0.01); // very slow refill
        // consume the single token
        assert!(limiter.check_rate_limit("gpt-4").await.is_ok());
        // second call should fail
        let err = limiter.check_rate_limit("gpt-4").await.unwrap_err();
        assert!(matches!(err, RateLimitError::TooManyRequests { .. }));
    }

    #[tokio::test]
    async fn rate_limiter_tracks_models_independently() {
        let limiter = RateLimiter::new(5, 5.0);
        assert!(limiter.check_rate_limit("model-a").await.is_ok());
        assert!(limiter.check_rate_limit("model-b").await.is_ok());
        let status_a = limiter.get_status("model-a").await;
        let status_b = limiter.get_status("model-b").await;
        assert_eq!(status_a.remaining, 4);
        assert_eq!(status_b.remaining, 4);
    }

    #[tokio::test]
    async fn rate_limiter_drain_bucket_blocks_subsequent_calls() {
        let limiter = RateLimiter::new(10, 10.0);
        limiter.drain_bucket("gpt-4").await;
        let err = limiter.check_rate_limit("gpt-4").await.unwrap_err();
        assert!(matches!(err, RateLimitError::TooManyRequests { .. }));
    }

    #[tokio::test]
    async fn rate_limiter_record_usage_reduces_remaining() {
        let limiter = RateLimiter::new(10, 10.0);
        limiter.record_usage("gpt-4", 3).await;
        let status = limiter.get_status("gpt-4").await;
        assert_eq!(status.remaining, 7);
    }

    #[tokio::test]
    async fn rate_limiter_get_status_returns_default_for_unknown_model() {
        let limiter = RateLimiter::new(60, 1.0);
        let status = limiter.get_status("unknown").await;
        assert_eq!(status.remaining, 60);
        assert_eq!(status.limit, 60);
        assert!(status.reset_at.is_none());
    }

    #[tokio::test]
    async fn rate_limiter_tracked_models_returns_known_keys() {
        let limiter = RateLimiter::new(10, 1.0);
        limiter.check_rate_limit("alpha").await.ok();
        limiter.check_rate_limit("beta").await.ok();
        let models = limiter.tracked_models().await;
        assert!(models.contains(&"alpha".to_string()));
        assert!(models.contains(&"beta".to_string()));
    }

    #[tokio::test]
    async fn rate_limiter_default_works() {
        let limiter = RateLimiter::default();
        assert!(limiter.check_rate_limit("gpt-4").await.is_ok());
    }

    // -----------------------------------------------------------------------
    // Exponential backoff
    // -----------------------------------------------------------------------

    #[test]
    fn backoff_attempt_1_equals_base() {
        assert_eq!(exponential_backoff_secs(1, 5, 120), 5);
    }

    #[test]
    fn backoff_attempt_2_equals_2x_base() {
        assert_eq!(exponential_backoff_secs(2, 5, 120), 10);
    }

    #[test]
    fn backoff_attempt_3_equals_4x_base() {
        assert_eq!(exponential_backoff_secs(3, 5, 120), 20);
    }

    #[test]
    fn backoff_caps_at_max_delay() {
        assert_eq!(exponential_backoff_secs(10, 5, 120), 120);
    }

    #[test]
    fn backoff_zero_attempt_returns_base() {
        assert_eq!(exponential_backoff_secs(0, 5, 120), 5);
    }

    #[test]
    fn backoff_different_base_and_max() {
        assert_eq!(exponential_backoff_secs(1, 2, 30), 2);
        assert_eq!(exponential_backoff_secs(5, 2, 30), 30);
    }

    // -----------------------------------------------------------------------
    // Retry-After header parsing
    // -----------------------------------------------------------------------

    #[test]
    fn parse_retry_after_seconds() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::RETRY_AFTER,
            reqwest::header::HeaderValue::from_static("30"),
        );
        let dur = parse_retry_after_header(&headers).unwrap();
        assert_eq!(dur, Duration::from_secs(30));
    }

    #[test]
    fn parse_retry_after_missing_returns_none() {
        let headers = reqwest::header::HeaderMap::new();
        assert!(parse_retry_after_header(&headers).is_none());
    }

    #[test]
    fn parse_retry_after_invalid_returns_none() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::RETRY_AFTER,
            reqwest::header::HeaderValue::from_static("not-a-number"),
        );
        assert!(parse_retry_after_header(&headers).is_none());
    }
}
