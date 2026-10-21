// Vendored from jcode (crates/operant-base/src/prompt.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805; partial —
// see operant_app/mod.rs for scope.
//! Included: SWARM_EFFORT (:101), SWARM_DEEP_EFFORT (:110), is_swarm_effort
//! (:123), is_deep_swarm_effort (:129), swarm_root_reasoning_effort (:136),
//! ContextInfo (:285, plus the impl members the renderers call).
//! [port-excision] the rest of prompt.rs (directive text, onboarding) is not
//! ported.

/// Reasoning-effort sentinel that enables swarm orchestration. Providers
/// translate this to the configured root effort (maximum by default) when
/// building API requests, while the UI/session keep the literal `swarm` marker
/// so the agent knows to
/// inject [`SWARM_EFFORT`].
pub const SWARM_EFFORT: &str = "swarm";

/// Reasoning-effort sentinel for the **deep task graph** mode: configured root
/// reasoning AND the comprehensive DAG-first swarm workflow (decompose into a
/// validated task graph, critique/verify gates, typed artifact handoffs). Sits
/// one rung above [`SWARM_EFFORT`] on the effort ladder: `... xhigh`, `swarm`
/// (light fan-out), `swarm-deep` (deep task graph). Providers translate this to
/// the configured root effort, while the UI/session keep the literal marker so
/// the agent knows to inject [`SWARM_DEEP_EFFORT`].
pub const SWARM_DEEP_EFFORT: &str = "swarm-deep";

/// Returns true when `effort` is either swarm sentinel (light or deep),
/// case-insensitive. Providers resolve their configured root reasoning level.
pub fn is_swarm_effort(effort: &str) -> bool {
    let trimmed = effort.trim();
    trimmed.eq_ignore_ascii_case(SWARM_EFFORT) || trimmed.eq_ignore_ascii_case(SWARM_DEEP_EFFORT)
}

// [port-decision] dedup: prompt.rs defined the three swarm-effort consts and
// is_swarm_effort twice across concatenated sources; kept the first (identical)
// copies at the top of the file.

/// Returns true when `effort` is specifically the deep task-graph sentinel.
pub fn is_deep_swarm_effort(effort: &str) -> bool {
    effort.trim().eq_ignore_ascii_case(SWARM_DEEP_EFFORT)
}

/// Configured root reasoning level for an orchestration sentinel. Providers
/// translate this real level to their supported range while retaining the
/// sentinel in session state. Ordinary reasoning efforts are left untouched.
pub fn swarm_root_reasoning_effort(effort: &str) -> Option<&'static str> {
    if !is_swarm_effort(effort) {
        return None;
    }
    Some(
        crate::tui::operant_app::config_shim::config()
            .agents
            .root_effort_for_swarm(is_deep_swarm_effort(effort)),
    )
}

/// Information about what's loaded in the context window
///
/// A full accounting of the prompt's sections. The live context panel shows a
/// subset, so several of these counters have no reader in this build; they are
/// kept because the struct is the accounting surface a `/context` breakdown
/// reports against, and dropping a counter would make that report lie.
#[cfg_attr(
    test,
    allow(
        dead_code,
        reason = "jcode-parity port: the context accounting surface; the live panel reports a subset of the counters"
    )
)]
#[derive(Debug, Clone, Default)]
pub struct ContextInfo {
    // === Static (System Prompt) ===
    /// Base system prompt size (chars)
    pub system_prompt_chars: usize,
    /// Immutable session context size (chars), when persisted in transcript history.
    pub session_context_chars: usize,
    /// Whether project AGENTS.md was loaded
    pub has_project_agents_md: bool,
    /// Project AGENTS.md size (chars)
    pub project_agents_md_chars: usize,
    /// Whether global ~/AGENTS.md was loaded
    pub has_global_agents_md: bool,
    /// Global AGENTS.md size (chars)
    pub global_agents_md_chars: usize,
    /// Skills section size (chars)
    pub skills_chars: usize,
    /// Self-dev section size (chars)
    pub selfdev_chars: usize,
    /// Memory section size (chars)
    pub memory_chars: usize,
    /// Prompt overlay section size (chars)
    pub prompt_overlay_chars: usize,
    /// Preferred tools section size (chars)
    pub preferred_tools_chars: usize,
    // === Dynamic (Conversation) ===
    /// Tool definitions sent to API (chars)
    pub tool_defs_chars: usize,
    /// Number of tool definitions
    pub tool_defs_count: usize,
    /// User messages total size (chars)
    pub user_messages_chars: usize,
    /// Number of user messages
    pub user_messages_count: usize,
    /// Assistant messages total size (chars)
    pub assistant_messages_chars: usize,
    /// Number of assistant messages
    pub assistant_messages_count: usize,
    /// Tool calls size (chars)
    pub tool_calls_chars: usize,
    /// Number of tool calls
    pub tool_calls_count: usize,
    /// Tool results size (chars)
    pub tool_results_chars: usize,
    /// Number of tool results
    pub tool_results_count: usize,

    /// Total system prompt size (chars)
    pub total_chars: usize,
}

impl ContextInfo {
    /// Rough estimate of tokens (chars / 4 is a common approximation)
    pub fn estimated_tokens(&self) -> usize {
        self.total_chars / 4
    }
}
