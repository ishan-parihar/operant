//! Wave 1 — the org layer.
//!
//! **Packet A: the employee registry.** Operant had no employee or
//! department concept before this module; the registry is the first one.
//!
//! ## What lives here
//!
//! - [`employee`] — the pure types (`Employee`, `AgentType`,
//!   `EmployeeCronJob`) and the `employee_id` derivation. No I/O.
//! - [`employee_db`] — the sqlite store, the DDL, and the deterministic
//!   backfill from the live `CronJob`.
//! - [`worklog`] — the worklog types (`WorklogEntry`, `TurnObservation`,
//!   `WorkflowKind`, `Outcome`, `KaizenProposal`) and the sealed entry
//!   constructor that makes framework-only authorship a type fact. No I/O.
//! - [`worklog_db`] — the sqlite store, the §3.4 DDL, the framework-only
//!   append, and the append-only triggers.
//! - [`notice`] — the notice board's pure types: typed recipients, the
//!   `Notice` row, retention constants, and the GC caps. No I/O.
//! - [`notice_db`] — the notice board's sqlite store, inbox query, ack
//!   transaction, and the batched retention GC.
//!
//! ## Storage (WAVE1-DECISIONS §3, Q2)
//!
//! The registry lives in the **existing** kanban sqlite file, derived as
//! `config.database_path.parent()/operant_kanban.db` — the same sibling-DB
//! convention `main.rs:967-971` and `cmd_cron.rs:82-92` already use. No new
//! database file, no JSONL.
//!
//! ## Boundaries (this packet deliberately does NOT do these)
//!
//! - **No gate.** WAVE1-DECISIONS §3.2 defines the fail-closed pre-execution
//!   gate; that is packet B. The one thing this packet owns of that
//!   contract is [`Employee::missing_required_fields`], the required-field
//!   predicate the gate consumes. Backfill records empty `skills` faithfully
//!   as `[]` and never repairs it — the gate is what surfaces those rows.
//! - **No harness provider.** Per §1.4 the *tools* are mounted through the
//!   `tool` seam; durable state is not. The store is a plain native
//!   subsystem in operant-core that is never unmounted.
//! - **No mutation of `CronJob`.** §3.1.1 backfills from the live struct as
//!   it is. The `employee` / `agent_type` / `continuity` columns on
//!   `CronJob` (OUTLINE §4 Wave 1 item 2) are a later wave.

pub mod employee;
pub mod employee_db;
pub mod identity_gate;
pub mod notice;
pub mod notice_db;
// RESTORED BY PACKET D — packet A temporarily disabled these because of a
// compile error in worklog_db.rs. That error is fixed (the `named_params!`
// macro was used without its `use` import; see worklog_db.rs:49).
pub mod worklog;
pub mod worklog_db;

pub use employee::{
    AgentType, EMPLOYEE_ID_HEX_LEN, EMPLOYEE_ID_PREFIX, Employee, EmployeeCronJob,
    derive_employee_id,
};
pub use employee_db::{BackfillReport, EmployeeDb, org_db_path};
pub use identity_gate::{
    EmployeeDbLookup, EmployeeLookup, GATE_BLOCK_PREFIX, GateBlock, GateDecision, IdentityGate,
    REQUIRED_IDENTITY_FIELDS,
};
pub use notice::{
    ACK_TAG, BROADCAST_SELECTOR, GcReport, InboxQuery, NOTICE_BOARD_DB_FILE, Notice,
    NoticeIdentity, NoticeInboxMatcher, PostNotice, RAW_RETENTION_DAYS, Recipient, RetentionLimits,
    SUMMARY_RETENTION_WEEKS, UNKNOWN_DEPT, WEEK_SECONDS, WEEKLY_SUMMARY_TAG, rfc3339,
};
pub use notice_db::NoticeBoard;
pub use worklog::{
    KAIZEN_LIMIT, KaizenProposal, Outcome, OutcomeSignals, TurnObservation, UNKNOWN_EMPLOYEE,
    UsageAccumulator, WORKLOG_DB_FILE, WorkflowKind, WorklogEntry, WorklogRecord,
};
pub use worklog_db::{WorklogDb, WorklogQuery, worklog_db_path};
