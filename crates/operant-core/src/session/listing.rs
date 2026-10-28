//! The unified session listing record — one shape for every surface.
//!
//! Plan v2 (2026-10-28): the TUI browser showed 5 fields, the CLI printed a
//! different projection, and the gateway listed its own table — three shapes,
//! three limits, one database (audit finding RT-A). This module owns the ONE
//! record (`SessionEntryV2`) and its loader (`Database::list_sessions_rich`),
//! so the session hub, the CLI subcommands, and the gateway's read-only API
//! routes all render the same rows.
//!
//! `employee_id` is the operator's "who is executing this session" tag: the
//! agent's bylaws seat (org/seat_policy_db stays the source of truth for
//! names) written at session start, NULL for personal-mode sessions and for
//! every row created before the column existed — those rows honestly carry
//! no attribution rather than a backfilled guess.

use chrono::{DateTime, Utc};

/// Lifecycle status of a listed session row, derived at read time.
///
/// `Live` marks the session the *current process* is streaming into (the TUI
/// is same-process, so presence is `is_streaming`, not jcode's cross-process
/// pid registry).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowStatus {
    /// The session this process is actively streaming into right now.
    Live,
    /// An ordinary, at-rest session row.
    Idle,
    /// Archived by the operator.
    Archived,
}

/// What a row in the unified list IS.
///
/// Sessions, rewind/branch forks (they ARE sessions — the fork creates one),
/// and live background delegations all appear as peer rows (omp's AgentHub
/// model: main and subagents are one record kind, not two surfaces).
#[derive(Debug, Clone)]
pub struct SessionEntryV2 {
    /// `sessions.id`, or the delegation handle for a `Subagent` row.
    pub id: String,
    /// `sessions.parent_session_id` — set for branches and forked sessions.
    pub parent_id: Option<String>,
    pub title: Option<String>,
    /// Parsed `ended_at` (falls back to `started_at` when the session never
    /// ended). The DB stores RFC3339 strings; the picker formats relative
    /// time from the typed value.
    pub updated_at: DateTime<Utc>,
    /// The operator's "who is executing" tag. None = personal mode, or a
    /// row that predates the column (no backfill lie).
    pub employee_id: Option<String>,
    /// `sessions.source` (local | gateway | cron | tui …).
    pub source: String,
    pub message_count: usize,
    pub cost_usd: f64,
    /// input + output + cache_read + cache_write, summed from the columns
    /// the schema already carries.
    pub tokens: u64,
    pub cwd: Option<String>,
    pub status: RowStatus,
    pub kind: RowKind,
}

/// The row's kind. A `Subagent` row carries the live delegation registry's
/// snapshot fields; it is NOT persisted in `sessions` (the registry is
/// process-local and prunes with its worker).
#[derive(Debug, Clone)]
pub enum RowKind {
    Session,
    /// A rewind/branch fork — rendered indented under its parent when the
    /// parent is in the same list window.
    Branch,
    /// A live-or-terminal background `delegate_task` worker, joined from
    /// the async-delegation registry.
    Subagent {
        goal: String,
        model: String,
        /// Unix seconds at dispatch.
        dispatch_time: u64,
        /// Pending | Completed | Failed, mirroring
        /// `AsyncDelegationStatus` without the dependency.
        status: &'static str,
    },
}
