//! `sop` configuration surface — extracted verbatim from the
//! former schema.rs monolith (dedup pass). Placement is navigational;
//! every item is re-exported from `schema::`.

use anyhow::Result;
use operant_macros::Configurable;
#[cfg(feature = "schema-export")]
use serde::{Deserialize, Serialize};

use super::*;

// ── SOP engine configuration ───────────────────────────────────

/// Standard Operating Procedures engine configuration (`[sop]`).
///
/// The `default_execution_mode` field uses the `SopExecutionMode` type from
/// `sop::types` (re-exported via `sop::SopExecutionMode`). To avoid circular
/// module references, config stores it using the same enum definition.
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "sop"]
pub struct SopConfig {
    /// Directory containing SOP definitions (subdirs with SOP.toml + SOP.md).
    /// Required to enable runtime SOP loading. When omitted, no SOPs are loaded
    /// at runtime; CLI commands (`sop list`, `sop validate`, `sop show`) still
    /// resolve the default `<workspace>/sops` for offline inspection.
    #[serde(default)]
    pub sops_dir: Option<String>,

    /// Default execution mode for SOPs that omit `execution_mode`.
    /// Values: `auto`, `supervised` (default), `step_by_step`,
    /// `priority_based`, `deterministic`.
    #[serde(default = "default_sop_execution_mode")]
    pub default_execution_mode: String,

    /// Maximum total concurrent SOP runs across all SOPs.
    #[serde(default = "default_sop_max_concurrent_total")]
    pub max_concurrent_total: usize,

    /// Approval timeout in seconds. When a run waits for approval longer than
    /// this, Critical/High-priority SOPs auto-approve; others stay waiting.
    /// Set to 0 to disable timeout.
    #[serde(default = "default_sop_approval_timeout_secs")]
    pub approval_timeout_secs: u64,

    /// Maximum number of finished runs kept in memory for status queries.
    /// Oldest runs are evicted when over capacity. 0 = unlimited.
    #[serde(default = "default_sop_max_finished_runs")]
    pub max_finished_runs: usize,
}

impl Default for SopConfig {
    fn default() -> Self {
        Self {
            sops_dir: None,
            default_execution_mode: default_sop_execution_mode(),
            max_concurrent_total: default_sop_max_concurrent_total(),
            approval_timeout_secs: default_sop_approval_timeout_secs(),
            max_finished_runs: default_sop_max_finished_runs(),
        }
    }
}
