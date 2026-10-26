//! Keep the long-lived gateway daemon small. Vendored from jcode
//! (`crates/jcode-harness-api-server/src/allocator.rs`, MIT License,
//! Copyright (c) 2025 Jeremy Huang), trimmed to the two functions the
//! daemon needs: the arena cap and the debounced `malloc_trim`.
//!
//! Why this module exists (2026-10-26 persistent-gateway audit, finding 3):
//! the gateway is a long-lived process with bursty per-turn allocations
//! (transcript rehydration, tool outputs, streaming buffers). glibc's
//! default of one malloc arena per core lets freed-but-retained arena pages
//! accumulate as RSS that no live object explains — jcode measured ~94 MB
//! retained on an idle connection from exactly this, and answered with an
//! arena cap at startup plus a debounced trim after the bursty work.
//! operant's daemon runs the same allocator with neither.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Arena cap applied at startup. `MALLOC_ARENA_MAX` still wins when the
/// operator set it. Four is jcode's value; the gateway's tokio workers plus
/// blocking-pool threads do not need one arena per core on a 12-core box.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
const DEFAULT_ARENA_MAX: i32 = 4;
/// Never trim more often than this; `malloc_trim` walks every arena.
const MIN_TRIM_INTERVAL_MS: u64 = 2_000;

static LAST_TRIM_MS: AtomicU64 = AtomicU64::new(0);

/// Configure the system allocator. Call once, before spawning threads.
pub fn configure() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        if std::env::var_os("MALLOC_ARENA_MAX").is_none() {
            // SAFETY: mallopt only adjusts allocator parameters.
            unsafe {
                libc::mallopt(libc::M_ARENA_MAX, DEFAULT_ARENA_MAX);
            }
        }
    }
}

/// Return freed heap pages to the OS, at most once per interval.
///
/// Debounced because `malloc_trim` walks every arena: calling it per event
/// would spend the sweep cost on every streaming tick instead of once
/// after the burst settles.
pub fn trim() {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0);
    let last = LAST_TRIM_MS.load(Ordering::Relaxed);
    if now.saturating_sub(last) >= MIN_TRIM_INTERVAL_MS
        && LAST_TRIM_MS
            .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
    {
        #[cfg(all(target_os = "linux", target_env = "gnu"))]
        // SAFETY: malloc_trim is thread-safe and only releases free pages.
        unsafe {
            libc::malloc_trim(0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trim_is_idempotent_within_the_debounce_window() {
        // Two immediate calls: the second must be swallowed by the
        // debounce (the sweep is what costs, the cap is what protects).
        trim();
        trim();
    }

    #[test]
    fn configure_is_safe_to_call_twice() {
        configure();
        configure();
    }
}
