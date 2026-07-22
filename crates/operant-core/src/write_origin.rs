//! Write-origin provenance — async-safe tracking for background review context.
//!
//! Mirrors `hermes-agent/tools/skill_provenance.py`. The background review
//! agent fork sets the origin to `"background_review"` so that the skill
//! manager can enforce write guards (no editing protected/hub skills, no
//! creating skills outside the review's scope).
//!
//! Uses `Arc<RwLock<String>>` so the origin survives Tokio task migration
//! across OS threads. Thread-local storage would lose the value if a task
//! is polled on a different thread by the work-stealing runtime.
//!
//! # Usage
//!
//! ```rust,ignore
//! use crate::write_origin::{set_write_origin, reset_write_origin, is_background_review};
//!
//! let token = set_write_origin("background_review");
//! assert!(is_background_review());
//! reset_write_origin(token);
//! assert!(!is_background_review());
//! ```

use std::sync::{Arc, LazyLock, RwLock};

/// Shared origin string. `Arc` so it can be cloned into spawned tasks;
/// `RwLock` so reads (the hot path) don't block and writes are rare.
static WRITE_ORIGIN: LazyLock<Arc<RwLock<String>>> =
    LazyLock::new(|| Arc::new(RwLock::new("assistant_tool".to_string())));

/// Token returned by [`set_write_origin`] for scoped reset.
#[derive(Debug, Clone)]
pub struct WriteOriginToken {
    /// The previous origin value, used for scoped restore.
    previous: String,
}

/// Set the current write origin and return a token for scoped reset.
///
/// The token must be passed to [`reset_write_origin`] when the scoped
/// context ends. This prevents accidental leaking of the review origin
/// into subsequent foreground turns.
pub fn set_write_origin(origin: &str) -> WriteOriginToken {
    let previous = {
        let lock = WRITE_ORIGIN.read().unwrap_or_else(|e| e.into_inner());
        lock.clone()
    };
    {
        let mut lock = WRITE_ORIGIN.write().unwrap_or_else(|e| e.into_inner());
        *lock = origin.to_string();
    }
    WriteOriginToken { previous }
}

/// Reset the write origin to the value before [`set_write_origin`] was called.
pub fn reset_write_origin(token: WriteOriginToken) {
    let mut lock = WRITE_ORIGIN.write().unwrap_or_else(|e| e.into_inner());
    *lock = token.previous;
}

/// Get the current write origin string.
pub fn get_write_origin() -> String {
    let lock = WRITE_ORIGIN.read().unwrap_or_else(|e| e.into_inner());
    lock.clone()
}

/// Returns `true` when the current execution context is the background review
/// agent fork. This is the primary guard-check used by the skill manager.
pub fn is_background_review() -> bool {
    let lock = WRITE_ORIGIN.read().unwrap_or_else(|e| e.into_inner());
    *lock == "background_review"
}

/// Scoping guard — sets the origin on creation and resets it on drop.
///
/// # Example
///
/// ```rust,ignore
/// {
///     let _guard = WriteOriginGuard::background_review();
///     assert!(is_background_review());
/// } // _guard drops here, origin resets
/// assert!(!is_background_review());
/// ```
pub struct WriteOriginGuard {
    token: WriteOriginToken,
}

impl WriteOriginGuard {
    /// Create a guard that sets the origin to `"background_review"`.
    pub fn background_review() -> Self {
        let token = set_write_origin("background_review");
        Self { token }
    }

    /// Create a guard with a custom origin string.
    pub fn new(origin: &str) -> Self {
        let token = set_write_origin(origin);
        Self { token }
    }
}

impl Drop for WriteOriginGuard {
    fn drop(&mut self) {
        let token = WriteOriginToken {
            previous: self.token.previous.clone(),
        };
        reset_write_origin(token);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Consolidated test: exercises set/reset, guard scoping, and nested
    /// operations in a single sequential block so parallel test threads
    /// don't race on the global static.
    #[test]
    fn test_write_origin_roundtrip() {
        let before = get_write_origin();

        // ── set / reset ──
        let token = set_write_origin("background_review");
        assert!(is_background_review());
        assert_eq!(get_write_origin(), "background_review");
        reset_write_origin(token);
        assert_eq!(get_write_origin(), before);

        // ── guard scoping ──
        {
            let _guard = WriteOriginGuard::background_review();
            assert!(is_background_review());
        }
        assert_eq!(get_write_origin(), before);

        // ── custom origin guard ──
        {
            let _guard = WriteOriginGuard::new("curator");
            assert_eq!(get_write_origin(), "curator");
            assert!(!is_background_review());
        }
        assert_eq!(get_write_origin(), before);

        // ── nested set/reset ──
        let outer = set_write_origin("background_review");
        assert!(is_background_review());

        let inner = set_write_origin("curator");
        assert_eq!(get_write_origin(), "curator");

        reset_write_origin(inner);
        assert!(is_background_review());

        reset_write_origin(outer);
        assert_eq!(get_write_origin(), before);
    }
}
