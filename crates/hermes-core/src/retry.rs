//! Jittered exponential backoff for retry loops.
//!
//! Ported from hermes-agent's `agent/retry_utils.py`.
//! Uses decorrelated exponential backoff to prevent thundering herd
//! when multiple sessions retry simultaneously.

use std::sync::Mutex;

lazy_static::lazy_static! {
    static ref JITTER_COUNTER: Mutex<u64> = Mutex::new(0);
}

/// Compute a jittered backoff delay for the given attempt number.
///
/// Uses decorrelated exponential backoff:
/// `delay = min(base * 2^(attempt-1), max_delay) + jitter`
///
/// The jitter is derived from a thread-safe counter seeded with the
/// current time, ensuring different processes get different jitter values
/// even when retrying at the same time.
///
/// # Arguments
/// * `attempt` - The retry attempt number (1-based)
/// * `base_delay` - Base delay in seconds (default: 5)
/// * `max_delay` - Maximum delay in seconds (default: 120)
/// * `jitter_ratio` - Jitter as a fraction of the computed delay (default: 0.5)
pub fn jittered_backoff(attempt: u32, base_delay: f64, max_delay: f64, jitter_ratio: f64) -> f64 {
    let exponential = base_delay * 2.0_f64.powi(attempt as i32 - 1);
    let capped = exponential.min(max_delay);

    let mut counter = JITTER_COUNTER.lock().unwrap();
    *counter += 1;
    let seed = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64)
        .wrapping_add(*counter);
    drop(counter);

    // Simple LCG for deterministic jitter without external RNG dependency
    let lcg = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
    let jitter = ((lcg as f64 / u64::MAX as f64) * 2.0 - 1.0) * jitter_ratio * capped;

    (capped + jitter).max(0.0)
}

/// Default jittered backoff with standard parameters.
pub fn default_backoff(attempt: u32) -> f64 {
    jittered_backoff(attempt, 5.0, 120.0, 0.5)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_attempt_1_near_base() {
        let delay = jittered_backoff(1, 5.0, 120.0, 0.5);
        // base=5, jitter up to ±2.5, so range is [2.5, 7.5]
        assert!((2.5..=7.5).contains(&delay));
    }

    #[test]
    fn test_attempt_grows_exponentially() {
        let d1 = jittered_backoff(1, 5.0, 120.0, 0.0); // zero jitter for deterministic check
        let d2 = jittered_backoff(2, 5.0, 120.0, 0.0);
        let d3 = jittered_backoff(3, 5.0, 120.0, 0.0);
        assert!((d1 - 5.0).abs() < 0.001);
        assert!((d2 - 10.0).abs() < 0.001);
        assert!((d3 - 20.0).abs() < 0.001);
    }

    #[test]
    fn test_caps_at_max_delay() {
        let delay = jittered_backoff(10, 5.0, 120.0, 0.0);
        assert!((delay - 120.0).abs() < 0.001);
    }

    #[test]
    fn test_default_backoff_sane() {
        let delay = default_backoff(1);
        assert!(delay > 0.0 && delay <= 120.0);
    }
}
