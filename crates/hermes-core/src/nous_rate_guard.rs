//! Cross-session rate limit guard for Nous Research provider.
//!
//! Ported from hermes-agent's `agent/nous_rate_guard.py`.
//! Distinguishes genuine account rate limits from upstream provider
//! capacity issues by checking exhausted buckets with meaningful reset windows.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::rate_limit_tracker::RateLimitState;

/// Shared state file for cross-session coordination.
fn nous_state_path() -> PathBuf {
    let mut path = dirs::home_dir().unwrap_or_default();
    path.push(".hermes");
    path.push("rate_limits");
    path.push("nous.json");
    path
}

/// Rate limit state persisted to disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NousRateLimitState {
    pub rate_limited: bool,
    pub rate_limit_until: f64,
    pub rate_limit_reason: String,
    pub captured_at: f64,
    pub exhausted_buckets: Vec<String>,
}

impl NousRateLimitState {
    pub fn is_active(&self) -> bool {
        if !self.rate_limited {
            return false;
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        now < self.rate_limit_until
    }

    pub fn remaining_seconds(&self) -> Option<f64> {
        if !self.is_active() {
            return None;
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        let remaining = self.rate_limit_until - now;
        if remaining > 0.0 {
            Some(remaining)
        } else {
            None
        }
    }
}

/// Record a rate limit event for Nous provider.
///
/// Writes state atomically to the shared file so other sessions see it.
pub fn record_nous_rate_limit(
    rate_limit_state: Option<&RateLimitState>,
    default_cooldown_secs: f64,
) -> std::io::Result<()> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();

    let mut exhausted_buckets = Vec::new();
    let mut shortest_reset = default_cooldown_secs;

    if let Some(state) = rate_limit_state {
        if let Some(buckets) = state.exhausted_bucket_names() {
            exhausted_buckets = buckets;
        }
        if let Some(reset) = state.shortest_reset_seconds() {
            shortest_reset = reset;
        }
    }

    let record = NousRateLimitState {
        rate_limited: true,
        rate_limit_until: now + shortest_reset,
        rate_limit_reason: format!(
            "Rate limited with {} exhausted bucket(s)",
            exhausted_buckets.len()
        ),
        captured_at: now,
        exhausted_buckets,
    };

    let json = serde_json::to_string_pretty(&record)?;
    let path = nous_state_path();

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let dir = path.parent().unwrap_or(std::path::Path::new("."));
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    std::io::Write::write_all(&mut tmp, json.as_bytes())?;
    tmp.persist(&path)?;

    Ok(())
}

/// Check if Nous is currently rate-limited. Returns seconds remaining or None.
pub fn nous_rate_limit_remaining() -> Option<f64> {
    let path = nous_state_path();
    if !path.exists() {
        return None;
    }

    let content = fs::read_to_string(&path).ok()?;
    let record: NousRateLimitState = serde_json::from_str(&content).ok()?;

    record.remaining_seconds()
}

/// Clear the Nous rate limit state after a successful request.
pub fn clear_nous_rate_limit() -> std::io::Result<()> {
    let path = nous_state_path();
    if path.exists() {
        fs::remove_file(&path)?;
    }
    Ok(())
}

/// Determine if a rate limit is a genuine account limit vs upstream capacity issue.
///
/// A genuine account rate limit has exhausted buckets with reset windows >= 60s.
/// An upstream provider capacity issue (e.g. DeepSeek out of capacity on Nous)
/// typically has very short reset windows or no exhausted buckets at all.
pub fn is_genuine_nous_rate_limit(rate_limit_state: Option<&RateLimitState>) -> bool {
    let Some(state) = rate_limit_state else {
        return false;
    };

    // Check for exhausted buckets with meaningful reset windows (>= 60s)
    let has_meaningful_exhaustion = [
        state.requests_min.as_ref(),
        state.requests_hour.as_ref(),
        state.tokens_min.as_ref(),
        state.tokens_hour.as_ref(),
    ]
    .into_iter()
    .flatten()
    .any(|b| b.remaining == 0 && b.reset_seconds >= 60.0);

    has_meaningful_exhaustion
}

impl RateLimitState {
    fn exhausted_bucket_names(&self) -> Option<Vec<String>> {
        let mut names = Vec::new();
        if let Some(b) = &self.requests_min {
            if b.remaining == 0 {
                names.push("requests/minute".to_string());
            }
        }
        if let Some(b) = &self.requests_hour {
            if b.remaining == 0 {
                names.push("requests/hour".to_string());
            }
        }
        if let Some(b) = &self.tokens_min {
            if b.remaining == 0 {
                names.push("tokens/minute".to_string());
            }
        }
        if let Some(b) = &self.tokens_hour {
            if b.remaining == 0 {
                names.push("tokens/hour".to_string());
            }
        }
        if names.is_empty() {
            None
        } else {
            Some(names)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nous_state_serialization() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        let state = NousRateLimitState {
            rate_limited: true,
            rate_limit_until: now + 300.0,
            rate_limit_reason: "test".to_string(),
            captured_at: now,
            exhausted_buckets: vec!["tokens/minute".to_string()],
        };
        let json = serde_json::to_string(&state).unwrap();
        let parsed: NousRateLimitState = serde_json::from_str(&json).unwrap();
        assert!(parsed.is_active());
    }

    #[test]
    fn test_is_genuine_with_exhausted_bucket() {
        use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_bytes(b"x-ratelimit-limit-tokens-minute").unwrap(),
            HeaderValue::from_str("100000").unwrap(),
        );
        headers.insert(
            HeaderName::from_bytes(b"x-ratelimit-remaining-tokens-minute").unwrap(),
            HeaderValue::from_str("0").unwrap(),
        );
        headers.insert(
            HeaderName::from_bytes(b"x-ratelimit-reset-tokens-minute").unwrap(),
            HeaderValue::from_str("120").unwrap(),
        );
        let state = RateLimitState::from_headers(&headers, "nous").unwrap();
        assert!(is_genuine_nous_rate_limit(Some(&state)));
    }
}
