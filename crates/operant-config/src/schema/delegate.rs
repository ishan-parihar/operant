//! `delegate` configuration surface — extracted verbatim from the
//! former schema.rs monolith (dedup pass). Placement is navigational;
//! every item is re-exported from `schema::`.

use anyhow::Result;
use operant_macros::Configurable;
#[cfg(feature = "schema-export")]
use serde::{Deserialize, Serialize};

use super::*;

// ── Delegate Tool Configuration ─────────────────────────────────

/// Global delegate tool configuration for default timeout values.
#[derive(Debug, Clone, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "delegate"]
pub struct DelegateToolConfig {
    /// Default timeout in seconds for non-agentic sub-agent provider calls.
    /// Can be overridden per-agent in `[agents.<name>]` config.
    /// Default: 120 seconds.
    #[serde(default = "default_delegate_timeout_secs")]
    pub timeout_secs: u64,
    /// Default timeout in seconds for agentic sub-agent runs.
    /// Can be overridden per-agent in `[agents.<name>]` config.
    /// Default: 300 seconds.
    #[serde(default = "default_delegate_agentic_timeout_secs")]
    pub agentic_timeout_secs: u64,
}

impl Default for DelegateToolConfig {
    fn default() -> Self {
        Self {
            timeout_secs: DEFAULT_DELEGATE_TIMEOUT_SECS,
            agentic_timeout_secs: DEFAULT_DELEGATE_AGENTIC_TIMEOUT_SECS,
        }
    }
}

// ── Delegate Agents ──────────────────────────────────────────────

/// Configuration for a delegate sub-agent used by the `delegate` tool.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Configurable)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[prefix = "delegate-agent"]
pub struct DelegateAgentConfig {
    /// Provider name (e.g. "ollama", "openrouter", "anthropic")
    pub provider: String,
    /// Model name
    pub model: String,
    /// Optional system prompt for the sub-agent
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Optional API key override
    #[serde(default)]
    #[secret]
    pub api_key: Option<String>,
    /// Temperature override
    #[serde(default)]
    pub temperature: Option<f64>,
    /// Max recursion depth for nested delegation
    #[serde(default = "default_max_depth")]
    pub max_depth: u32,
    /// Enable agentic sub-agent mode (multi-turn tool-call loop).
    #[serde(default)]
    pub agentic: bool,
    /// Allowlist of tool names available to the sub-agent in agentic mode.
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// Maximum tool-call iterations in agentic mode.
    #[serde(default = "default_max_tool_iterations")]
    pub max_iterations: usize,
    /// Optional timeout in seconds for non-agentic sub-agent provider calls.
    /// When `None`, falls back to `[delegate].timeout_secs` (default: 120).
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    /// Optional timeout in seconds for agentic sub-agent runs.
    /// When `None`, falls back to `[delegate].agentic_timeout_secs` (default: 300).
    #[serde(default)]
    pub agentic_timeout_secs: Option<u64>,
    /// Optional skills directory path (relative to workspace root) for scoped skill loading.
    /// When unset or empty, the sub-agent falls back to the default workspace `skills/` directory.
    #[serde(default)]
    pub skills_directory: Option<String>,
    /// Optional memory namespace for isolation.
    /// When set, the sub-agent's memory operations are isolated to this namespace,
    /// preventing cross-contamination with memory from other agents.
    #[serde(default)]
    pub memory_namespace: Option<String>,
}

// ── Swarms ──────────────────────────────────────────────────────

/// Orchestration strategy for a swarm of agents.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum SwarmStrategy {
    /// Run agents sequentially; each agent's output feeds into the next.
    Sequential,
    /// Run agents in parallel; collect all outputs.
    Parallel,
    /// Use the LLM to pick the best agent for the task.
    Router,
}

/// Configuration for a swarm of coordinated agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema-export", derive(schemars::JsonSchema))]
pub struct SwarmConfig {
    /// Ordered list of agent names (must reference keys in `agents`).
    pub agents: Vec<String>,
    /// Orchestration strategy.
    pub strategy: SwarmStrategy,
    /// System prompt for router strategy (used to pick the best agent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub router_prompt: Option<String>,
    /// Optional description shown to the LLM when choosing swarms.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Maximum total timeout for the swarm execution in seconds.
    #[serde(default = "default_swarm_timeout_secs")]
    pub timeout_secs: u64,
}
