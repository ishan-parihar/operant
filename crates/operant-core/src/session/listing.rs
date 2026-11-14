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
///
/// `Interrupted` is the one state NOT derived from the row's own columns: it
/// comes from the exit marker (`crate::session_interrupted`) materialised onto
/// the row at exit, and it answers a question no column answered before — "was
/// this session live when the owning process went away?" That is jcode's
/// `SessionStatus::Crashed{..}`, and operant could not previously say it at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowStatus {
    /// The session this process is actively streaming into right now.
    Live,
    /// The session the previous process was attached to when it exited, named
    /// by the interrupted-session marker. Cleared when that session's next
    /// turn starts.
    Interrupted,
    /// An ordinary, at-rest session row.
    Idle,
    /// Archived by the operator.
    Archived,
}

impl RowStatus {
    /// The wire label for the read-only session API (iter-E): lowercase,
    /// stable, distinct from the TUI's display label.
    pub fn api_label(&self) -> &'static str {
        match self {
            RowStatus::Live => "live",
            RowStatus::Interrupted => "interrupted",
            RowStatus::Idle => "idle",
            RowStatus::Archived => "archived",
        }
    }

    /// The TUI's display label, which `api_label`'s own doc has promised since
    /// it was written. Title-case because it sits in a right-aligned Status
    /// column under a `Status` header, where a lowercase `interrupted` reads as
    /// a continuation of the sentence beside it rather than as a column value.
    pub fn label(&self) -> &'static str {
        match self {
            RowStatus::Live => "Live",
            RowStatus::Interrupted => "Interrupted",
            RowStatus::Idle => "Idle",
            RowStatus::Archived => "Archived",
        }
    }
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
    /// The two columns `updated_at` stands in for (2026-11-14, plan iteration
    /// 3b). The runtime is `ended_at - started_at`, and `updated_at` alone
    /// cannot answer it: it is `COALESCE(ended_at, started_at)`, so it says
    /// when the session last changed, not how long it ran. Either may be None on
    /// a legacy row — 3,308 of 3,314 live rows have no `started_at`.
    pub started_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
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
