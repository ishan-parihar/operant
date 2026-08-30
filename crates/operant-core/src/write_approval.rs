//! Plan 012 — staged write-approval gate (hermes `write_approval.py` parity).
//!
//! Writes originating from non-interactive contexts (background review, cron
//! jobs, gateway channels) are staged here instead of being executed. The
//! caller receives a `GateDecision::Stage { pending_id }` and the user can
//! later approve/discard the staged write via `list_pending` /
//! `discard_pending`. Interactive (user) origin bypasses the gate.
//!
//! Persistence: this is an in-memory store keyed by `pending_id`. A future
//! round can persist to the database; for now the lifetime is process-local
//! (matches hermes's in-process `STAGED_WRITES` dict).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

/// Per-subsystem write-approval toggle. Defaults to `false` (gating off) —
/// the plan requires "opt-in per subsystem" parity with hermes
/// `write_approval_enabled(subsystem)`.
static ENABLED: LazyLock<RwLock<HashMap<String, bool>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// In-memory pending write store. Keyed by `pending_id`.
static PENDING: LazyLock<RwLock<HashMap<String, StagedWrite>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// A write that was staged instead of executed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StagedWrite {
    pub pending_id: String,
    pub subsystem: String,
    pub origin: String,
    pub payload: String,
    pub staged_at_unix_ms: u64,
}

/// The gate's verdict for a write attempt.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GateDecision {
    /// Write is allowed to proceed (interactive origin, or no approval needed).
    Allow,
    /// Write is hard-blocked (not supported by the gate; this is a safety
    /// surface for skill_guard's hard blocks, not for staged approvals).
    Blocked { message: String },
    /// Write is staged for the user's approval.
    Stage {
        pending_id: String,
        message: String,
    },
}

/// Per-subsystem opt-in. Returns the current toggle (default `false`).
pub fn write_approval_enabled(subsystem: &str) -> bool {
    let guard = ENABLED.read().unwrap_or_else(|e| e.into_inner());
    guard.get(subsystem).copied().unwrap_or(false)
}

/// Set the per-subsystem toggle. Pass `true` to gate writes, `false` to
/// let them through. Thread-safe; takes effect immediately.
pub fn set_write_approval_enabled(subsystem: &str, enabled: bool) {
    let mut guard = ENABLED.write().unwrap_or_else(|e| e.into_inner());
    guard.insert(subsystem.to_string(), enabled);
}

/// Decision helper — the canonical entry point for write sites.
///
/// - interactive origin (user) → `Allow`
/// - background / remote origin AND subsystem not enabled → `Allow`
/// - background / remote origin AND subsystem enabled → `Stage { ... }`
/// - subsystem name is missing/empty → `Allow` (defensive default)
pub fn gate(subsystem: &str, payload: &str) -> GateDecision {
    if subsystem.is_empty() || !write_approval_enabled(subsystem) {
        return GateDecision::Allow;
    }
    let origin = crate::write_origin::current_origin();
    if !crate::write_origin::is_background() {
        return GateDecision::Allow;
    }
    let pending_id = stage_write(subsystem, &origin, payload);
    let id_for_message = pending_id.clone();
    GateDecision::Stage {
        pending_id,
        message: format!(
            "Write to {subsystem} staged for approval (origin: {origin}). \
             Use `/approve {id_for_message}` to apply or `/discard {id_for_message}` to drop."
        ),
    }
}

/// Stage a write for approval. Returns the new `pending_id`.
///
/// Public (not just crate-internal) so the approval / discard tools can
/// list + remove entries by id.
pub fn stage_write(subsystem: &str, origin: &str, payload: &str) -> String {
    let pending_id = format!("{subsystem}_{}", next_id());
    let entry = StagedWrite {
        pending_id: pending_id.clone(),
        subsystem: subsystem.to_string(),
        origin: origin.to_string(),
        payload: payload.to_string(),
        staged_at_unix_ms: now_ms(),
    };
    PENDING
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .insert(pending_id.clone(), entry);
    pending_id
}

/// List all currently-pending writes (most recent first).
pub fn list_pending() -> Vec<StagedWrite> {
    let guard = PENDING.read().unwrap_or_else(|e| e.into_inner());
    let mut out: Vec<StagedWrite> = guard.values().cloned().collect();
    out.sort_by_key(|a| std::cmp::Reverse(a.staged_at_unix_ms));
    out
}

/// Look up a single pending write by id.
pub fn get_pending(pending_id: &str) -> Option<StagedWrite> {
    PENDING
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(pending_id)
        .cloned()
}

/// Remove a pending write. Returns the removed entry (if any).
pub fn discard_pending(pending_id: &str) -> Option<StagedWrite> {
    PENDING
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .remove(pending_id)
}

/// Number of currently-pending writes (across all subsystems).
pub fn pending_count() -> usize {
    PENDING.read().unwrap_or_else(|e| e.into_inner()).len()
}

/// Clear all pending writes (test helper).
pub fn clear_pending_for_tests() {
    PENDING.write().unwrap_or_else(|e| e.into_inner()).clear();
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn next_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    // These tests touch global state (the `WRITE_ORIGIN` slot in
    // `write_origin.rs`, the `PENDING` and `ENABLED` statics here). They
    // therefore share fate across threads and must run serially — the
    // `default` test runner may shuffle unit tests across threads, and
    // tests like `set_write_origin_helper` rely on the global origin
    // staying as they set it. Run with:
    //
    //   cargo test -p operant-core --lib -- --test-threads=1 write_approval
    //
    // The integration tests that use these gates run in their own
    // process and are not affected.
    use super::*;
    use crate::write_origin::{WriteOriginGuard, set_write_origin};

    fn reset() {
        clear_pending_for_tests();
        // Clear all enabled toggles.
        ENABLED.write().unwrap_or_else(|e| e.into_inner()).clear();
    }

    #[test]
    fn interactive_origin_bypasses_gate() {
        reset();
        let _g = WriteOriginGuard::new("user");
        let d = gate("skills", "create skill foo");
        assert!(matches!(d, GateDecision::Allow));
        assert_eq!(pending_count(), 0, "interactive write must not stage");
    }

    #[test]
    fn background_origin_and_disabled_subsystem_allows() {
        reset();
        let _g = WriteOriginGuard::new("background_review");
        let d = gate("skills", "create skill foo");
        assert!(matches!(d, GateDecision::Allow));
        assert_eq!(pending_count(), 0);
    }

    #[test]
    fn background_origin_and_enabled_subsystem_stages() {
        reset();
        set_write_approval_enabled("skills", true);
        let _g = WriteOriginGuard::new("background_review");
        let d = gate("skills", "create skill foo");
        match d {
            GateDecision::Stage { pending_id, message } => {
                assert!(pending_id.starts_with("skills_"));
                assert!(message.contains("staged for approval"));
                assert_eq!(pending_count(), 1);
            }
            other => panic!("expected Stage, got {other:?}"),
        }
    }

    #[test]
    fn gateway_origin_treated_as_background() {
        reset();
        set_write_approval_enabled("skills", true);
        let _g = WriteOriginGuard::new("gateway:telegram");
        let d = gate("skills", "create skill foo");
        assert!(matches!(d, GateDecision::Stage { .. }));
    }

    #[test]
    fn cron_origin_treated_as_background() {
        reset();
        set_write_approval_enabled("skills", true);
        let _g = WriteOriginGuard::new("cron_job");
        let d = gate("skills", "create skill foo");
        assert!(matches!(d, GateDecision::Stage { .. }));
    }

    #[test]
    fn list_pending_orders_by_recency() {
        reset();
        set_write_approval_enabled("skills", true);
        let _g = WriteOriginGuard::new("background_review");
        let _ = gate("skills", "first");
        std::thread::sleep(std::time::Duration::from_millis(2));
        let _ = gate("skills", "second");
        let list = list_pending();
        assert_eq!(list.len(), 2);
        assert!(list[0].payload == "second", "most recent must be first");
    }

    #[test]
    fn discard_removes_by_id() {
        reset();
        set_write_approval_enabled("skills", true);
        let _g = WriteOriginGuard::new("background_review");
        let id = stage_write("skills", "background_review", "create skill bar");
        assert_eq!(pending_count(), 1);
        let removed = discard_pending(&id);
        assert!(removed.is_some());
        assert_eq!(pending_count(), 0);
        assert!(discard_pending(&id).is_none(), "double-discard returns None");
    }

    #[test]
    fn get_pending_returns_clone() {
        reset();
        set_write_approval_enabled("skills", true);
        let _g = WriteOriginGuard::new("background_review");
        let id = stage_write("skills", "background_review", "create skill baz");
        let entry = get_pending(&id).expect("must exist");
        assert_eq!(entry.payload, "create skill baz");
        assert_eq!(entry.subsystem, "skills");
    }

    #[test]
    fn empty_subsystem_allows_defensively() {
        reset();
        let _g = WriteOriginGuard::new("background_review");
        let d = gate("", "anything");
        assert!(matches!(d, GateDecision::Allow));
    }

    #[test]
    fn write_approval_enabled_default_is_false() {
        reset();
        assert!(!write_approval_enabled("never_enabled"));
    }

    #[test]
    fn set_then_clear_round_trip() {
        reset();
        set_write_approval_enabled("skills", true);
        assert!(write_approval_enabled("skills"));
        set_write_approval_enabled("skills", false);
        assert!(!write_approval_enabled("skills"));
    }

    #[test]
    fn set_write_origin_helper() {
        // Direct helper test — WriteOriginGuard is the public API.
        set_write_origin("test_origin");
        assert_eq!(crate::write_origin::current_origin(), "test_origin");
        // The global state leaks across tests, so reset back to "user" to
        // avoid contaminating later tests.
        set_write_origin("user");
    }
}
