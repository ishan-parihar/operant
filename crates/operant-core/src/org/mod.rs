//! The org layer after the Holon supersession (docs/agentic-os/,
//! G+1 by owner order 2026-10-15).
//!
//! What remains is what the boundary gives operant:
//! - [`seat_policy`] — the pure policy engine; the enforcement core for
//!   Holon's `policy.json` (phase C);
//! - [`seat_policy_db`] — personal-mode operator seats; org mode never
//!   consults it (the run's `policy.json` is the policy);
//! - [`seat_budgets`] — budget enforcement and window math; caps come from
//!   `policy.json.budget` in org mode, the table is personal-mode only;
//! - [`seat_authority`] — personal mode's interactive escalation core;
//! - [`context_injection`] — channel-sourced aspects (DM, feed);
//!   `self`/`dept`/`global` sources retired to Holon's brief and memory;
//! - [`worklog`] — the `UsageAccumulator` (feeds `result.usage`) and
//!   outcome signal types; the worklog store itself retired to Holon's
//!   `runs`/`outputs`/`events`.
//!
//! Everything else — the cast, registries, departments, hierarchy,
//! authority grants, notices, DM threads, decisions, the write barrier —
//! retired to Holon (ORG-LAYER-DISPOSITION.md §1). The org stores were
//! archived first (`operant org holon archive`); the data is history.

pub mod context_injection;
pub mod seat_authority;
pub mod seat_budgets;
pub mod seat_policy;
pub mod seat_policy_db;
pub mod worklog;
