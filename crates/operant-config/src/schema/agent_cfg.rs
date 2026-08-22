//! `agent_cfg` configuration surface — extracted verbatim from the
//! former schema.rs monolith (dedup pass). Placement is navigational;
//! every item is re-exported from `schema::`.

use crate::autonomy::AutonomyLevel;
use anyhow::Result;
use operant_macros::Configurable;
#[cfg(feature = "schema-export")]
use serde::{Deserialize, Serialize};

use super::*;

pub(crate) fn normalize_reasoning_effort(value: &str) -> std::result::Result<String, String> {
    let normalized = value.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "minimal" | "low" | "medium" | "high" | "xhigh" => Ok(normalized),
        _ => Err(format!(
            "reasoning_effort {value:?} is invalid (expected one of: minimal, low, medium, high, xhigh)"
        )),
    }
}

pub(crate) fn deserialize_reasoning_effort_opt<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value: Option<String> = Option::deserialize(deserializer)?;
    value
        .map(|raw| normalize_reasoning_effort(&raw).map_err(serde::de::Error::custom))
        .transpose()
}

// ── Pacing ────────────────────────────────────────────────────────

/// Pacing controls for slow/local LLM workloads (`[pacing]` section).
///
/// All fields are optional and default to values that preserve existing
/// behavior. When set, they extend — not replace — the existing timeout
/// and loop-detection subsystems.
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "pacing"]
pub struct PacingConfig {
    /// Per-step timeout in seconds: the maximum time allowed for a single
    /// LLM inference turn, independent of the total message budget.
    /// `None` means no per-step timeout (existing behavior).
    #[serde(default)]
    pub step_timeout_secs: Option<u64>,

    /// Minimum elapsed seconds before loop detection activates.
    /// Tasks completing under this threshold get aggressive loop protection;
    /// longer-running tasks receive a grace period before the detector starts
    /// counting. `None` means loop detection is always active (existing behavior).
    #[serde(default)]
    pub loop_detection_min_elapsed_secs: Option<u64>,

    /// Tool names excluded from identical-output / alternating-pattern loop
    /// detection. Useful for browser workflows where `browser_screenshot`
    /// structurally resembles a loop even when making progress.
    #[serde(default)]
    pub loop_ignore_tools: Vec<String>,

    /// Override for the hardcoded timeout scaling cap (default: 4).
    /// The channel message timeout budget is computed as:
    ///   `message_timeout_secs * min(max_tool_iterations, message_timeout_scale_max)`
    /// Raising this value lets long multi-step tasks with slow local models
    /// receive a proportionally larger budget without inflating the base timeout.
    #[serde(default)]
    pub message_timeout_scale_max: Option<u64>,

    /// Enable pattern-based loop detection (exact repeat, ping-pong,
    /// no-progress). Defaults to `true`.
    #[serde(default = "default_loop_detection_enabled")]
    pub loop_detection_enabled: bool,

    /// Sliding window size for the pattern-based loop detector.
    /// Defaults to 20.
    #[serde(default = "default_loop_detection_window_size")]
    pub loop_detection_window_size: usize,

    /// Number of consecutive identical tool+args calls before the first
    /// escalation (Warning). Defaults to 3.
    #[serde(default = "default_loop_detection_max_repeats")]
    pub loop_detection_max_repeats: usize,
}

impl Default for PacingConfig {
    fn default() -> Self {
        Self {
            step_timeout_secs: None,
            loop_detection_min_elapsed_secs: None,
            loop_ignore_tools: Vec::new(),
            message_timeout_scale_max: None,
            loop_detection_enabled: default_loop_detection_enabled(),
            loop_detection_window_size: default_loop_detection_window_size(),
            loop_detection_max_repeats: default_loop_detection_max_repeats(),
        }
    }
}

// ── Autonomy / Security ──────────────────────────────────────────

/// Autonomy and security policy configuration (`[autonomy]` section).
///
/// Controls what the agent is allowed to do: shell commands, filesystem access,
/// risk approval gates, and per-policy budgets.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "autonomy"]
#[serde(default)]
pub struct AutonomyConfig {
    /// Autonomy level: `read_only`, `supervised` (default), or `full`.
    pub level: AutonomyLevel,
    /// Restrict absolute filesystem paths to workspace-relative references. Default: `true`.
    /// Resolved paths outside the workspace still require `allowed_roots`.
    pub workspace_only: bool,
    /// Allowlist of executable names permitted for shell execution.
    pub allowed_commands: Vec<String>,
    /// Explicit path denylist. Default includes system-critical paths and sensitive dotdirs.
    pub forbidden_paths: Vec<String>,
    /// Maximum actions allowed per hour per policy. Default: `100`.
    pub max_actions_per_hour: u32,
    /// Maximum cost per day in cents per policy. Default: `1000`.
    pub max_cost_per_day_cents: u32,

    /// Require explicit approval for medium-risk shell commands.
    #[serde(default = "default_true")]
    pub require_approval_for_medium_risk: bool,

    /// Block high-risk shell commands even if allowlisted.
    #[serde(default = "default_true")]
    pub block_high_risk_commands: bool,

    /// Additional environment variables allowed for shell tool subprocesses.
    ///
    /// These names are explicitly allowlisted and merged with the built-in safe
    /// baseline (`PATH`, `HOME`, etc.) after `env_clear()`.
    #[serde(default)]
    pub shell_env_passthrough: Vec<String>,

    /// Tools that never require approval (e.g. read-only tools).
    #[serde(default = "default_auto_approve")]
    pub auto_approve: Vec<String>,

    /// Tools that always require interactive approval, even after "Always".
    #[serde(default = "default_always_ask")]
    pub always_ask: Vec<String>,

    /// Extra directory roots the agent may read/write outside the workspace.
    /// Supports absolute, `~/...`, and workspace-relative entries.
    /// Resolved paths under any of these roots pass `is_resolved_path_allowed`.
    #[serde(default, alias = "allowed_path", alias = "allowed_paths")]
    pub allowed_roots: Vec<String>,

    /// Tools to exclude from non-CLI channels (e.g. Telegram, Discord).
    ///
    /// When a tool is listed here, non-CLI channels will not expose it to the
    /// model in tool specs.
    #[serde(default)]
    pub non_cli_excluded_tools: Vec<String>,

    /// Timeout in seconds for shell tool subprocesses. Default: 60.
    #[serde(default = "default_shell_timeout_secs")]
    pub shell_timeout_secs: u64,
}

impl AutonomyConfig {
    /// Merge the built-in default `auto_approve` entries into the current
    /// list, preserving any user-supplied additions.
    pub fn ensure_default_auto_approve(&mut self) {
        let defaults = default_auto_approve();
        for entry in defaults {
            if !self.auto_approve.iter().any(|existing| existing == &entry) {
                self.auto_approve.push(entry);
            }
        }
    }
}

impl Default for AutonomyConfig {
    fn default() -> Self {
        Self {
            level: AutonomyLevel::Supervised,
            workspace_only: true,
            allowed_commands: crate::policy::default_allowed_commands(),
            forbidden_paths: crate::policy::default_forbidden_paths(),
            max_actions_per_hour: 20,
            max_cost_per_day_cents: 500,
            require_approval_for_medium_risk: true,
            block_high_risk_commands: true,
            shell_env_passthrough: vec![],
            auto_approve: default_auto_approve(),
            always_ask: default_always_ask(),
            allowed_roots: Vec::new(),
            non_cli_excluded_tools: Vec::new(),
            shell_timeout_secs: default_shell_timeout_secs(),
        }
    }
}
