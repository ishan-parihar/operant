//! `scheduler` configuration surface — extracted verbatim from the
//! former schema.rs monolith (dedup pass). Placement is navigational;
//! every item is re-exported from `schema::`.

use anyhow::Result;
use operant_macros::Configurable;
#[cfg(feature = "schema-export")]
use serde::{Deserialize, Serialize};

use super::*;

// ── Scheduler ────────────────────────────────────────────────────

/// Scheduler configuration for periodic task execution (`[scheduler]` section).
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "scheduler"]
pub struct SchedulerConfig {
    /// Enable the built-in scheduler loop.
    #[serde(default = "default_scheduler_enabled")]
    pub enabled: bool,
    /// Maximum number of persisted scheduled tasks.
    #[serde(default = "default_scheduler_max_tasks")]
    pub max_tasks: usize,
    /// Maximum tasks executed per scheduler polling cycle.
    #[serde(default = "default_scheduler_max_concurrent")]
    pub max_concurrent: usize,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            enabled: default_scheduler_enabled(),
            max_tasks: default_scheduler_max_tasks(),
            max_concurrent: default_scheduler_max_concurrent(),
        }
    }
}

// ── Heartbeat ────────────────────────────────────────────────────

/// Heartbeat configuration for periodic health pings (`[heartbeat]` section).
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "heartbeat"]
#[allow(clippy::struct_excessive_bools)]
pub struct HeartbeatConfig {
    /// Enable periodic heartbeat pings. Default: `true`.
    pub enabled: bool,
    /// Interval in minutes between heartbeat pings. Minimum: `1`. Default: `30`.
    #[serde(default = "default_heartbeat_interval")]
    pub interval_minutes: u32,
    /// Enable two-phase heartbeat: Phase 1 asks LLM whether to run, Phase 2
    /// executes only when the LLM decides there is work to do. Saves API cost
    /// during quiet periods. Default: `true`.
    #[serde(default = "default_two_phase")]
    pub two_phase: bool,
    /// Optional fallback task text when `HEARTBEAT.md` has no task entries.
    #[serde(default)]
    pub message: Option<String>,
    /// Optional delivery channel for heartbeat output (for example: `telegram`).
    /// When omitted, auto-selects the first configured channel.
    #[serde(default, alias = "channel")]
    pub target: Option<String>,
    /// Optional delivery recipient/chat identifier (required when `target` is
    /// explicitly set).
    #[serde(default, alias = "recipient")]
    pub to: Option<String>,
    /// Enable adaptive intervals that back off on failures and speed up for
    /// high-priority tasks. Default: `false`.
    #[serde(default)]
    pub adaptive: bool,
    /// Minimum interval in minutes when adaptive mode is enabled. Default: `5`.
    #[serde(default = "default_heartbeat_min_interval")]
    pub min_interval_minutes: u32,
    /// Maximum interval in minutes when adaptive mode backs off. Default: `120`.
    #[serde(default = "default_heartbeat_max_interval")]
    pub max_interval_minutes: u32,
    /// Dead-man's switch timeout in minutes. If the heartbeat has not ticked
    /// within this window, an alert is sent. `0` disables. Default: `0`.
    #[serde(default)]
    pub deadman_timeout_minutes: u32,
    /// Channel for dead-man's switch alerts (e.g. `telegram`). Falls back to
    /// the heartbeat delivery channel.
    #[serde(default)]
    pub deadman_channel: Option<String>,
    /// Recipient for dead-man's switch alerts. Falls back to `to`.
    #[serde(default)]
    pub deadman_to: Option<String>,
    /// Maximum number of heartbeat run history records to retain. Default: `100`.
    #[serde(default = "default_heartbeat_max_run_history")]
    pub max_run_history: u32,
    /// Load the channel session history before each heartbeat task execution so
    /// the LLM has conversational context. Default: `false`.
    ///
    /// When `true`, the session file for the configured `target`/`to` is passed
    /// to the agent as `session_state_file`, giving it access to the recent
    /// conversation history — just as if the user had sent a message.
    #[serde(default)]
    pub load_session_context: bool,
    /// Maximum wall-clock seconds allowed for a single agent invocation
    /// (Phase 1 decision or Phase 2 task execution). `0` disables.
    /// Default: `600` (10 minutes).
    #[serde(default = "default_heartbeat_task_timeout")]
    pub task_timeout_secs: u64,
}

impl Default for HeartbeatConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            interval_minutes: default_heartbeat_interval(),
            two_phase: true,
            message: None,
            target: None,
            to: None,
            adaptive: false,
            min_interval_minutes: default_heartbeat_min_interval(),
            max_interval_minutes: default_heartbeat_max_interval(),
            deadman_timeout_minutes: 0,
            deadman_channel: None,
            deadman_to: None,
            max_run_history: default_heartbeat_max_run_history(),
            load_session_context: false,
            task_timeout_secs: default_heartbeat_task_timeout(),
        }
    }
}

/// A declarative cron job definition for the `[[cron.jobs]]` config array.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
pub struct CronJobDecl {
    /// Stable identifier used for merge semantics across syncs.
    pub id: String,
    /// Human-readable name.
    #[serde(default)]
    pub name: Option<String>,
    /// Job type: `"shell"` (default) or `"agent"`.
    #[serde(default = "default_job_type_decl")]
    pub job_type: String,
    /// Schedule for the job.
    pub schedule: CronScheduleDecl,
    /// Shell command to run (required when `job_type = "shell"`).
    #[serde(default)]
    pub command: Option<String>,
    /// Agent prompt (required when `job_type = "agent"`).
    #[serde(default)]
    pub prompt: Option<String>,
    /// Whether the job is enabled. Default: `true`.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Model override for agent jobs.
    #[serde(default)]
    pub model: Option<String>,
    /// Allowlist of tool names for agent jobs.
    #[serde(default)]
    pub allowed_tools: Option<Vec<String>>,
    /// Whether to recall and inject memory context before this agent job runs.
    /// Defaults to `true`; set to `false` for stateless digest jobs.
    #[serde(default = "default_true")]
    pub uses_memory: bool,
    /// Session target: `"isolated"` (default) or `"main"`.
    #[serde(default)]
    pub session_target: Option<String>,
    /// Delivery configuration.
    #[serde(default)]
    pub delivery: Option<DeliveryConfigDecl>,
}

/// Schedule variant for declarative cron jobs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum CronScheduleDecl {
    /// Classic cron expression.
    Cron {
        /// Cron expression in standard five-field form.
        expr: String,
        /// Optional timezone for the expression (IANA name).
        #[serde(default)]
        tz: Option<String>,
    },
    /// Interval in milliseconds.
    Every {
        /// Repeat interval in milliseconds.
        every_ms: u64,
    },
    /// One-shot at an RFC 3339 timestamp.
    At {
        /// RFC 3339 timestamp of the single run.
        at: String,
    },
}

impl Default for CronConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            catch_up_on_startup: true,
            max_run_history: default_max_run_history(),
            jobs: Vec::new(),
        }
    }
}
