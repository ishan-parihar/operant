//! Rate limit header parsing and state tracking.
//!
//! Ported from hermes-agent's `agent/rate_limit_tracker.py`.
//! Parses `x-ratelimit-*` headers from HTTP responses into structured state.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A single rate limit bucket.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitBucket {
    pub limit: u64,
    pub remaining: u64,
    pub reset_seconds: f64,
    pub captured_at: DateTime<Utc>,
}

impl RateLimitBucket {
    pub fn used(&self) -> u64 {
        self.limit.saturating_sub(self.remaining)
    }

    pub fn usage_pct(&self) -> f64 {
        if self.limit == 0 {
            100.0
        } else {
            (self.used() as f64 / self.limit as f64) * 100.0
        }
    }
}

/// Aggregated rate limit state across all buckets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitState {
    pub requests_min: Option<RateLimitBucket>,
    pub requests_hour: Option<RateLimitBucket>,
    pub tokens_min: Option<RateLimitBucket>,
    pub tokens_hour: Option<RateLimitBucket>,
    pub captured_at: DateTime<Utc>,
    pub provider: String,
}

impl RateLimitState {
    /// Parse rate limit headers from a `reqwest::HeaderMap`.
    pub fn from_headers(headers: &reqwest::header::HeaderMap, provider: &str) -> Option<Self> {
        let captured_at = Utc::now();

        let requests_min = parse_bucket(headers, "x-ratelimit-limit-requests-minute", "x-ratelimit-remaining-requests-minute", "x-ratelimit-reset-requests-minute")
            .or_else(|| parse_bucket(headers, "x-ratelimit-limit-requests", "x-ratelimit-remaining-requests", "x-ratelimit-reset-requests"));

        let requests_hour = parse_bucket(headers, "x-ratelimit-limit-requests-hour", "x-ratelimit-remaining-requests-hour", "x-ratelimit-reset-requests-hour");

        let tokens_min = parse_bucket(headers, "x-ratelimit-limit-tokens-minute", "x-ratelimit-remaining-tokens-minute", "x-ratelimit-reset-tokens-minute")
            .or_else(|| parse_bucket(headers, "x-ratelimit-limit-tokens", "x-ratelimit-remaining-tokens", "x-ratelimit-reset-tokens"));

        let tokens_hour = parse_bucket(headers, "x-ratelimit-limit-tokens-hour", "x-ratelimit-remaining-tokens-hour", "x-ratelimit-reset-tokens-hour");

        if requests_min.is_none()
            && requests_hour.is_none()
            && tokens_min.is_none()
            && tokens_hour.is_none()
        {
            return None;
        }

        Some(RateLimitState {
            requests_min,
            requests_hour,
            tokens_min,
            tokens_hour,
            captured_at,
            provider: provider.to_string(),
        })
    }

    /// One-line compact summary for status bars.
    pub fn compact_summary(&self) -> String {
        let mut parts = Vec::new();

        if let Some(b) = &self.requests_min {
            parts.push(format!("req/min: {}/{}", b.remaining, b.limit));
        }
        if let Some(b) = &self.tokens_min {
            parts.push(format!("tok/min: {}/{}", b.remaining, b.limit));
        }
        if let Some(b) = &self.requests_hour {
            parts.push(format!("req/hr: {}/{}", b.remaining, b.limit));
        }
        if let Some(b) = &self.tokens_hour {
            parts.push(format!("tok/hr: {}/{}", b.remaining, b.limit));
        }

        if parts.is_empty() {
            "no rate limit headers".to_string()
        } else {
            parts.join(" | ")
        }
    }

    /// Check if any bucket is exhausted.
    pub fn is_exhausted(&self) -> bool {
        [
            self.requests_min.as_ref(),
            self.requests_hour.as_ref(),
            self.tokens_min.as_ref(),
            self.tokens_hour.as_ref(),
        ]
        .into_iter()
        .flatten()
        .any(|b| b.remaining == 0)
    }

    /// Get the shortest reset time among exhausted buckets.
    pub fn shortest_reset_seconds(&self) -> Option<f64> {
        [
            self.requests_min.as_ref(),
            self.requests_hour.as_ref(),
            self.tokens_min.as_ref(),
            self.tokens_hour.as_ref(),
        ]
        .into_iter()
        .flatten()
        .filter(|b| b.remaining == 0)
        .map(|b| b.reset_seconds)
        .reduce(f64::min)
    }
}

fn parse_bucket(
    headers: &reqwest::header::HeaderMap,
    limit_name: &str,
    remaining_name: &str,
    reset_name: &str,
) -> Option<RateLimitBucket> {
    let limit = headers
        .get(limit_name)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())?;
    let remaining = headers
        .get(remaining_name)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())?;
    let reset_seconds = headers
        .get(reset_name)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<f64>().ok())?;

    Some(RateLimitBucket {
        limit,
        remaining,
        reset_seconds,
        captured_at: Utc::now(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

    fn make_headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(HeaderName::from_bytes(k.as_bytes()).unwrap(), HeaderValue::from_str(v).unwrap());
        }
        h
    }

    #[test]
    fn test_parse_all_buckets() {
        let headers = make_headers(&[
            ("x-ratelimit-limit-requests-minute", "100"),
            ("x-ratelimit-remaining-requests-minute", "50"),
            ("x-ratelimit-reset-requests-minute", "30"),
            ("x-ratelimit-limit-tokens-minute", "50000"),
            ("x-ratelimit-remaining-tokens-minute", "25000"),
            ("x-ratelimit-reset-tokens-minute", "15"),
        ]);
        let state = RateLimitState::from_headers(&headers, "anthropic").unwrap();
        assert!(state.requests_min.is_some());
        assert!(state.tokens_min.is_some());
        assert_eq!(state.requests_min.as_ref().unwrap().limit, 100);
        assert_eq!(state.requests_min.as_ref().unwrap().remaining, 50);
    }

    #[test]
    fn test_no_headers_returns_none() {
        let headers = HeaderMap::new();
        assert!(RateLimitState::from_headers(&headers, "anthropic").is_none());
    }

    #[test]
    fn test_compact_summary() {
        let headers = make_headers(&[
            ("x-ratelimit-limit-requests-minute", "100"),
            ("x-ratelimit-remaining-requests-minute", "50"),
            ("x-ratelimit-reset-requests-minute", "30"),
        ]);
        let state = RateLimitState::from_headers(&headers, "anthropic").unwrap();
        let summary = state.compact_summary();
        assert!(summary.contains("req/min: 50/100"));
    }

    #[test]
    fn test_is_exhausted() {
        let headers = make_headers(&[
            ("x-ratelimit-limit-requests-minute", "100"),
            ("x-ratelimit-remaining-requests-minute", "0"),
            ("x-ratelimit-reset-requests-minute", "60"),
        ]);
        let state = RateLimitState::from_headers(&headers, "anthropic").unwrap();
        assert!(state.is_exhausted());
        assert_eq!(state.shortest_reset_seconds(), Some(60.0));
    }
}
