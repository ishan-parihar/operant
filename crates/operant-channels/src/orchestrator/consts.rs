//! `consts` — extracted verbatim from the former orchestrator/mod.rs monolith.
//! Re-exported from `orchestrator` so every import path is unchanged.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use super::*;

/// Live channel registry populated by `start_channels()`. Used by `deliver_announcement()` to
/// reuse authenticated channel instances (critical for Matrix E2EE — avoids re-running session
/// restore on every cron delivery).
///
/// Set once at startup; valid for the process lifetime. Daemon restart is required to pick up
/// channel-config changes — there's no in-flight refresh path. Callers must tolerate the
/// `OnceLock::get()` returning `None` during the brief window before `start_channels` populates
/// it; `deliver_announcement` falls back to per-call channel reconstruction in that case.
pub(crate) static CRON_CHANNEL_REGISTRY: OnceLock<Arc<HashMap<String, Arc<dyn Channel>>>> =
    OnceLock::new();
/// Maximum conversation senders kept in memory (LRU eviction beyond this).
pub(crate) const MAX_CONVERSATION_SENDERS: usize = 1000;
/// Maximum history messages to keep per sender.
pub(crate) const MAX_CHANNEL_HISTORY: usize = 50;
/// Minimum user-message length (in chars) for auto-save to memory.
/// Messages shorter than this (e.g. "ok", "thanks") are not stored,
/// reducing noise in memory recall.
pub(crate) const AUTOSAVE_MIN_MESSAGE_CHARS: usize = 20;

// System prompt functions live in `operant_runtime::agent::system_prompt`.
#[allow(unused_imports)]
pub use operant_runtime::agent::system_prompt::{
    BOOTSTRAP_MAX_CHARS, build_system_prompt, build_system_prompt_with_mode,
    build_system_prompt_with_mode_and_autonomy,
};

pub(crate) const DEFAULT_CHANNEL_INITIAL_BACKOFF_SECS: u64 = 2;
pub(crate) const DEFAULT_CHANNEL_MAX_BACKOFF_SECS: u64 = 60;
pub(crate) const MIN_CHANNEL_MESSAGE_TIMEOUT_SECS: u64 = 30;
/// Default timeout for processing a single channel message (LLM + tools).
/// Used as fallback when not configured in channels_config.message_timeout_secs.
#[cfg(test)]
pub(crate) const CHANNEL_MESSAGE_TIMEOUT_SECS: u64 = 300;
/// Cap timeout scaling so large max_tool_iterations values do not create unbounded waits.
pub(crate) const CHANNEL_MESSAGE_TIMEOUT_SCALE_CAP: u64 = 4;
pub(crate) const CHANNEL_PARALLELISM_PER_CHANNEL: usize = 4;
pub(crate) const CHANNEL_MIN_IN_FLIGHT_MESSAGES: usize = 8;
pub(crate) const CHANNEL_MAX_IN_FLIGHT_MESSAGES: usize = 64;
pub(crate) const CHANNEL_TYPING_REFRESH_INTERVAL_SECS: u64 = 4;
pub(crate) const CHANNEL_HEALTH_HEARTBEAT_SECS: u64 = 30;
pub(crate) const MODEL_CACHE_FILE: &str = "models_cache.json";
pub(crate) const MODEL_CACHE_PREVIEW_LIMIT: usize = 10;
pub(crate) const MEMORY_CONTEXT_MAX_ENTRIES: usize = 4;
pub(crate) const MEMORY_CONTEXT_ENTRY_MAX_CHARS: usize = 800;
pub(crate) const MEMORY_CONTEXT_MAX_CHARS: usize = 4_000;
pub(crate) const CHANNEL_HISTORY_COMPACT_KEEP_MESSAGES: usize = 12;
pub(crate) const CHANNEL_HISTORY_COMPACT_CONTENT_CHARS: usize = 600;
/// Proactive context-window budget in estimated characters (~4 chars/token).
/// When the total character count of conversation history exceeds this limit,
/// older turns are dropped before the request is sent to the provider,
/// preventing context-window-exceeded errors.  Set conservatively below
/// common context windows (128 k tokens ≈ 512 k chars) to leave room for
/// system prompt, memory context, and model output.
pub(crate) const PROACTIVE_CONTEXT_BUDGET_CHARS: usize = 400_000;
/// Guardrail for hook-modified outbound channel content.
pub(crate) const CHANNEL_HOOK_MAX_OUTBOUND_CHARS: usize = 20_000;

pub(crate) const SYSTEMD_STATUS_ARGS: [&str; 3] = ["--user", "is-active", "operant.service"];
pub(crate) const SYSTEMD_RESTART_ARGS: [&str; 3] = ["--user", "restart", "operant.service"];
pub(crate) const OPENRC_STATUS_ARGS: [&str; 2] = ["operant", "status"];
pub(crate) const OPENRC_RESTART_ARGS: [&str; 2] = ["operant", "restart"];
