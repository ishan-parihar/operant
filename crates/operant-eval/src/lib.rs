#![deny(missing_docs)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! Golden-task eval harness for agent-loop regression catching.
//!
//! Much smaller than upstream `hermes-eval`: the goal is a deterministic
//! pass/fail gate over golden prompt→tool-sequence→answer pairs, not a
//! benchmark suite.
//!
//! This crate ships the **pure-function core** — task loading, the
//! subsequence/keyword verifier, and the text reporter — all testable with a
//! mock trace and no model endpoint. The endpoint-driven runner (driving the
//! real agent loop) is intentionally left to the integration layer so the
//! default dev loop stays fast; see `docs/PHASE7_PARITY_DESIGN.md` C3.
//!
//! # Usage
//!
//! 1. Load tasks from `tasks/*.yaml` via [`task::EvalTask::load_from_dir`].
//! 2. Run the agent headlessly, collecting the tool-call sequence + final
//!    answer into a [`verifier::AgentTrace`].
//! 3. Verify with [`verifier::verify`], report with
//!    [`reporter::render_report`], gate on [`reporter::summary_pass`].

pub mod reporter;
pub mod task;
pub mod verifier;

pub use reporter::{render_report, summary_pass};
pub use task::EvalTask;
pub use verifier::{AgentTrace, CheckResult, TaskVerdict, verify};
