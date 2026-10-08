// Vendored from jcode (crates/operant-app-core/src/catchup.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805.
// [port-decision] only `CatchupSeenSnapshot` is ported: that is the whole of
// what the ported session-picker loading path calls
// (operant_ui/session_picker/loading.rs:1716, :1853). Its upstream file also
// carries `needs_catchup` (:11), `mark_seen` (:62), `build_brief` (:70) and
// `render_markdown` (:104), which need `operant_task_types::{CatchupBrief,
// PersistedCatchupState}` and the `Session` message list — neither is ported
// here, and no ported call site reaches them. The private helpers the ported
// arm does need are ported verbatim (`state_path` :252, `load_seen_state` :256,
// `is_attention_status` :275, `needs_catchup_with_seen` :52).
// [port-decision] `PersistedCatchupState` (operant-task-types/src/lib.rs:646) is
// a two-line `HashMap<String, i64>` wrapper with `#[serde(default)]`, so it is
// inlined here as the private `PersistedCatchupState` below rather than pulling
// in a new dependency for it. It keeps the same serde shape.
use std::collections::HashMap;

use chrono::{DateTime, Utc};

use crate::tui::operant_app::storage::operant_dir;
use crate::tui::operant_model::vendor_types::SessionStatus;

const CATCHUP_STATE_FILE: &str = "catchup_seen.json";

/// Snapshot of the persisted catch-up "seen" state, so callers that need to
/// evaluate many sessions at once (e.g. the session picker building its list)
/// can avoid re-reading and re-parsing `catchup_seen.json` once per session.
#[derive(Clone, Default)]
pub struct CatchupSeenSnapshot {
    state: PersistedCatchupState,
}

impl CatchupSeenSnapshot {
    /// Load the persisted seen-state once from disk.
    pub fn load() -> Self {
        Self {
            state: load_seen_state(),
        }
    }

    /// Same semantics as [`needs_catchup`] but uses this preloaded snapshot
    /// instead of re-reading the state file for every call.
    pub fn needs_catchup(
        &self,
        session_id: &str,
        updated_at: DateTime<Utc>,
        status: &SessionStatus,
    ) -> bool {
        if !is_attention_status(status) {
            return false;
        }
        let seen = self.state.seen_at_ms_by_session.get(session_id).copied();
        needs_catchup_with_seen(updated_at.timestamp_millis(), seen, status)
    }
}

pub(crate) fn needs_catchup_with_seen(
    updated_at_ms: i64,
    seen_at_ms: Option<i64>,
    status: &SessionStatus,
) -> bool {
    is_attention_status(status) && seen_at_ms.unwrap_or_default() < updated_at_ms
}

fn state_path() -> anyhow::Result<std::path::PathBuf> {
    Ok(operant_dir()?.join(CATCHUP_STATE_FILE))
}

fn load_seen_state() -> PersistedCatchupState {
    let Ok(path) = state_path() else {
        return PersistedCatchupState::default();
    };
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn is_attention_status(status: &SessionStatus) -> bool {
    matches!(
        status,
        SessionStatus::Closed
            | SessionStatus::Reloaded
            | SessionStatus::Compacted
            | SessionStatus::RateLimited
            | SessionStatus::Crashed { .. }
            | SessionStatus::Error { .. }
    )
}

/// Verbatim from operant-task-types/src/lib.rs:645-:649.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct PersistedCatchupState {
    #[serde(default)]
    pub seen_at_ms_by_session: HashMap<String, i64>,
}
