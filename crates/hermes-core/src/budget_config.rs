//! Token budget and rate limit configuration.
//!
//! Provides [`BudgetConfig`] with builder-pattern construction, environment
//! variable loading, and validation.

use serde::{Deserialize, Serialize};

/// Default maximum tokens per turn.
pub const DEFAULT_MAX_TOKENS_PER_TURN: u64 = 200_000;
/// Default maximum tokens per tool call.
pub const DEFAULT_MAX_TOKENS_PER_CALL: u64 = 100_000;
/// Default maximum number of tool calls per turn.
pub const DEFAULT_MAX_CALLS_PER_TURN: u32 = 50;
/// Default maximum output size in bytes.
pub const DEFAULT_MAX_OUTPUT_SIZE: u64 = 100_000;
/// Default maximum tool result size in bytes.
pub const DEFAULT_MAX_TOOL_RESULT_SIZE: u64 = 100_000;
/// Default maximum number of parallel tool calls.
pub const DEFAULT_MAX_TOOL_CALLS: u32 = 10;
/// Default cooldown in seconds between retries.
pub const DEFAULT_COOLDOWN_SECONDS: u64 = 5;
/// Default maximum number of retries.
pub const DEFAULT_MAX_RETRIES: u32 = 3;

/// Constants referencing the config field names used by `from_env`.
const ENV_MAX_TOKENS_PER_TURN: &str = "BUDGET_MAX_TOKENS_PER_TURN";
const ENV_MAX_TOKENS_PER_CALL: &str = "BUDGET_MAX_TOKENS_PER_CALL";
const ENV_MAX_CALLS_PER_TURN: &str = "BUDGET_MAX_CALLS_PER_TURN";
const ENV_MAX_OUTPUT_SIZE: &str = "BUDGET_MAX_OUTPUT_SIZE";
const ENV_MAX_TOOL_RESULT_SIZE: &str = "BUDGET_MAX_TOOL_RESULT_SIZE";
const ENV_MAX_TOOL_CALLS: &str = "BUDGET_MAX_TOOL_CALLS";
const ENV_COOLDOWN_SECONDS: &str = "BUDGET_COOLDOWN_SECONDS";
const ENV_MAX_RETRIES: &str = "BUDGET_MAX_RETRIES";

/// Describes where a [`BudgetConfig`] was sourced from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ConfigSource {
    /// Built-in defaults.
    #[default]
    Default,
    /// Loaded from environment variables.
    EnvVar,
    /// Loaded from a configuration file.
    File,
    /// Set explicitly by the caller.
    Explicit,
}

/// Token budget and rate limit configuration.
///
/// Controls the per-turn and per-call resource limits for tool execution,
/// output sizes, and retry behaviour.
///
/// # Examples
///
/// ```ignore
/// use hermes_core::budget_config::BudgetConfig;
///
/// let config = BudgetConfig::default()
///     .with_max_tokens_per_turn(300_000)
///     .with_max_calls_per_turn(100);
///
/// config.validate().unwrap();
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BudgetConfig {
    /// Maximum total tokens consumed across all tool calls in one turn.
    #[serde(default = "default_max_tokens_per_turn")]
    pub max_tokens_per_turn: u64,
    /// Maximum tokens for a single tool call.
    #[serde(default = "default_max_tokens_per_call")]
    pub max_tokens_per_call: u64,
    /// Maximum number of tool calls allowed in a single turn.
    #[serde(default = "default_max_calls_per_turn")]
    pub max_calls_per_turn: u32,
    /// Maximum number of bytes in a tool's text output.
    #[serde(default = "default_max_output_size")]
    pub max_output_size: u64,
    /// Maximum size of a persisted tool result in bytes.
    #[serde(default = "default_max_tool_result_size")]
    pub max_tool_result_size: u64,
    /// Maximum number of tool calls that can run concurrently.
    #[serde(default = "default_max_tool_calls")]
    pub max_tool_calls: u32,
    /// Seconds to wait before retrying a failed tool call.
    #[serde(default = "default_cooldown_seconds")]
    pub cooldown_seconds: u64,
    /// Maximum retry attempts for a failed tool call.
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    /// Source of this configuration (informational).
    #[serde(skip)]
    pub source: ConfigSource,
}

// Separate functions for serde defaults (needs fn pointers).
fn default_max_tokens_per_turn() -> u64 { DEFAULT_MAX_TOKENS_PER_TURN }
fn default_max_tokens_per_call() -> u64 { DEFAULT_MAX_TOKENS_PER_CALL }
fn default_max_calls_per_turn() -> u32 { DEFAULT_MAX_CALLS_PER_TURN }
fn default_max_output_size() -> u64 { DEFAULT_MAX_OUTPUT_SIZE }
fn default_max_tool_result_size() -> u64 { DEFAULT_MAX_TOOL_RESULT_SIZE }
fn default_max_tool_calls() -> u32 { DEFAULT_MAX_TOOL_CALLS }
fn default_cooldown_seconds() -> u64 { DEFAULT_COOLDOWN_SECONDS }
fn default_max_retries() -> u32 { DEFAULT_MAX_RETRIES }

impl Default for BudgetConfig {
    fn default() -> Self {
        Self {
            max_tokens_per_turn: DEFAULT_MAX_TOKENS_PER_TURN,
            max_tokens_per_call: DEFAULT_MAX_TOKENS_PER_CALL,
            max_calls_per_turn: DEFAULT_MAX_CALLS_PER_TURN,
            max_output_size: DEFAULT_MAX_OUTPUT_SIZE,
            max_tool_result_size: DEFAULT_MAX_TOOL_RESULT_SIZE,
            max_tool_calls: DEFAULT_MAX_TOOL_CALLS,
            cooldown_seconds: DEFAULT_COOLDOWN_SECONDS,
            max_retries: DEFAULT_MAX_RETRIES,
            source: ConfigSource::Default,
        }
    }
}

impl BudgetConfig {
    /// Load configuration from environment variables.
    ///
    /// Reads the `BUDGET_*` environment variables.  Any variable that is not
    /// set falls back to the default value.
    pub fn from_env() -> Self {
        Self {
            max_tokens_per_turn: std::env::var(ENV_MAX_TOKENS_PER_TURN)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(DEFAULT_MAX_TOKENS_PER_TURN),
            max_tokens_per_call: std::env::var(ENV_MAX_TOKENS_PER_CALL)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(DEFAULT_MAX_TOKENS_PER_CALL),
            max_calls_per_turn: std::env::var(ENV_MAX_CALLS_PER_TURN)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(DEFAULT_MAX_CALLS_PER_TURN),
            max_output_size: std::env::var(ENV_MAX_OUTPUT_SIZE)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(DEFAULT_MAX_OUTPUT_SIZE),
            max_tool_result_size: std::env::var(ENV_MAX_TOOL_RESULT_SIZE)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(DEFAULT_MAX_TOOL_RESULT_SIZE),
            max_tool_calls: std::env::var(ENV_MAX_TOOL_CALLS)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(DEFAULT_MAX_TOOL_CALLS),
            cooldown_seconds: std::env::var(ENV_COOLDOWN_SECONDS)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(DEFAULT_COOLDOWN_SECONDS),
            max_retries: std::env::var(ENV_MAX_RETRIES)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(DEFAULT_MAX_RETRIES),
            source: ConfigSource::EnvVar,
        }
    }

    /// Validate the configuration for contradictory settings.
    ///
    /// Returns `Ok(())` if the configuration is valid, or a description of
    /// the first problem found.
    pub fn validate(&self) -> std::result::Result<(), String> {
        if self.max_tokens_per_call > self.max_tokens_per_turn {
            return Err(
                "max_tokens_per_call exceeds max_tokens_per_turn".to_string(),
            );
        }
        if self.max_calls_per_turn == 0 {
            return Err("max_calls_per_turn must be > 0".to_string());
        }
        if self.max_tool_calls == 0 {
            return Err("max_tool_calls must be > 0".to_string());
        }
        if self.max_output_size == 0 {
            return Err("max_output_size must be > 0".to_string());
        }
        if self.max_tool_result_size == 0 {
            return Err("max_tool_result_size must be > 0".to_string());
        }
        Ok(())
    }

    // ── Builder-pattern methods ──

    pub fn with_max_tokens_per_turn(mut self, val: u64) -> Self {
        self.max_tokens_per_turn = val;
        self.source = ConfigSource::Explicit;
        self
    }

    pub fn with_max_tokens_per_call(mut self, val: u64) -> Self {
        self.max_tokens_per_call = val;
        self.source = ConfigSource::Explicit;
        self
    }

    pub fn with_max_calls_per_turn(mut self, val: u32) -> Self {
        self.max_calls_per_turn = val;
        self.source = ConfigSource::Explicit;
        self
    }

    pub fn with_max_output_size(mut self, val: u64) -> Self {
        self.max_output_size = val;
        self.source = ConfigSource::Explicit;
        self
    }

    pub fn with_max_tool_result_size(mut self, val: u64) -> Self {
        self.max_tool_result_size = val;
        self.source = ConfigSource::Explicit;
        self
    }

    pub fn with_max_tool_calls(mut self, val: u32) -> Self {
        self.max_tool_calls = val;
        self.source = ConfigSource::Explicit;
        self
    }

    pub fn with_cooldown_seconds(mut self, val: u64) -> Self {
        self.cooldown_seconds = val;
        self.source = ConfigSource::Explicit;
        self
    }

    pub fn with_max_retries(mut self, val: u32) -> Self {
        self.max_retries = val;
        self.source = ConfigSource::Explicit;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_values() {
        let cfg = BudgetConfig::default();
        assert_eq!(cfg.max_tokens_per_turn, DEFAULT_MAX_TOKENS_PER_TURN);
        assert_eq!(cfg.max_tokens_per_call, DEFAULT_MAX_TOKENS_PER_CALL);
        assert_eq!(cfg.max_calls_per_turn, DEFAULT_MAX_CALLS_PER_TURN);
        assert_eq!(cfg.source, ConfigSource::Default);
    }

    #[test]
    fn test_validate_ok() {
        let cfg = BudgetConfig::default();
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn test_validate_per_call_exceeds_turn() {
        let cfg = BudgetConfig::default()
            .with_max_tokens_per_call(500_000)
            .with_max_tokens_per_turn(100_000);
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn test_validate_zero_calls_per_turn() {
        let cfg = BudgetConfig::default().with_max_calls_per_turn(0);
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn test_validate_zero_tool_calls() {
        let cfg = BudgetConfig::default().with_max_tool_calls(0);
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn test_builder_pattern() {
        let cfg = BudgetConfig::default()
            .with_max_tokens_per_turn(300_000)
            .with_max_calls_per_turn(100)
            .with_cooldown_seconds(10)
            .with_max_retries(5);

        assert_eq!(cfg.max_tokens_per_turn, 300_000);
        assert_eq!(cfg.max_calls_per_turn, 100);
        assert_eq!(cfg.cooldown_seconds, 10);
        assert_eq!(cfg.max_retries, 5);
        assert_eq!(cfg.source, ConfigSource::Explicit);
    }

    #[test]
    fn test_serialize_roundtrip() {
        let cfg = BudgetConfig::default()
            .with_max_tokens_per_turn(150_000)
            .with_max_tool_calls(8);

        let json = serde_json::to_string(&cfg).unwrap();
        let deserialized: BudgetConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.max_tokens_per_turn, 150_000);
        assert_eq!(deserialized.max_tool_calls, 8);
        // source is skipped during serialization.
        assert_eq!(deserialized.source, ConfigSource::Default);
    }

    #[test]
    fn test_from_env_empty() {
        // When no env vars are set, from_env falls back to defaults.
        let cfg = BudgetConfig::from_env();
        assert_eq!(cfg.max_tokens_per_turn, DEFAULT_MAX_TOKENS_PER_TURN);
        assert_eq!(cfg.source, ConfigSource::EnvVar);
    }

    #[test]
    fn test_validate_zero_output_size() {
        let cfg = BudgetConfig::default().with_max_output_size(0);
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn test_validate_zero_tool_result_size() {
        let cfg = BudgetConfig::default().with_max_tool_result_size(0);
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn test_config_source_tracking() {
        let mut cfg = BudgetConfig::default();
        assert_eq!(cfg.source, ConfigSource::Default);
        cfg = BudgetConfig::from_env();
        assert_eq!(cfg.source, ConfigSource::EnvVar);
        cfg = cfg.with_max_retries(1);
        assert_eq!(cfg.source, ConfigSource::Explicit);
    }
}
