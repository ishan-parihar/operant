//! Operant Agent orchestration loop with self-healing
//!
//! Implements the ReAct (Reason + Act) pattern for LLM-driven tool execution.
//! Includes the self-evolution pipeline: skill nudge counter, iteration budget,
//! turn finalizer, and background review daemon for autonomous skill/memory
//! improvement after each turn.

pub(crate) mod background_review;
pub mod error_classifier;
pub mod insights;
pub mod iteration_budget;
pub mod learn_prompt;
pub mod learning_graph;
pub mod llm_compressor;
pub mod message_safety;
pub mod provider_registry;
pub mod skill_bundle;
pub mod skill_preprocessing;
pub(crate) mod turn_context;
pub(crate) mod turn_finalizer;
pub mod turn_retry_state;

use crate::turn_end_heuristics;

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use futures::stream::BoxStream;
use tokio::sync::{RwLock, mpsc};
use tokio::time::timeout;
use tracing::{debug, error, info, instrument, warn};

use crate::client::{ChatResponse, Message, Role, ToolCall, ToolCallFunction, Usage};
use crate::config::{BehaviorSettings, runtime_config};
use crate::context_files::{load_default_context_files, load_workspace_context};
use crate::database::Database;
use crate::distillation::distill_session_to_memory;
use crate::error::{Error, Result};
use crate::memory::MemoryManager;
use crate::observer::{Observer, ObserverEvent, ObserverMetric};
use crate::parser::{ToolCallParser, ToolCallStreamParser};
use crate::skills::SkillManager;
use crate::tools::{ToolContext, ToolRegistry, ToolResult};

use self::background_review::SelfEvolutionState;
use self::iteration_budget::IterationBudget;
use self::turn_finalizer::{
    PREFLIGHT_DECAY_CONSTANT, PREFLIGHT_DECAY_H50, PREFLIGHT_THRESHOLD_PERCENT, TurnDiagnostics,
    TurnExitReason, file_mutation_verifier_footer,
};

/// Skill-management principles injected into the frozen system prefix
/// whenever a skill manager is attached.
///
/// Mirrors hermes-agent's `agent/prompt_builder.py::SKILLS_GUIDANCE` (injected
/// whenever `skill_manage` is in the toolset). Without this block the agent
/// treats skills as a static read-only catalog — it never creates skills for
/// repeated workflows and never patches stale ones, which is the classic
/// "drift into non-alignment" with healthy skill-management behavior.
const SKILLS_GUIDANCE: &str = "

## Skill Management Principles
After completing a complex task (5+ tool calls), fixing a tricky error, or discovering a non-trivial workflow, save the approach as a skill with skill_manage so you can reuse it next time.
When using a skill and finding it outdated, incomplete, or wrong, patch it immediately with skill_manage(action='patch') — don't wait to be asked. Skills that aren't maintained become liabilities.

## Skill Safety Rule
1. **UNAVAILABLE** — If a skill's content is missing, truncated, or shows a stale placeholder (e.g. after context compression), the instructions are inaccessible — treat the skill as unloaded.
2. **RELOAD** — Before performing any action that depends on a skill, re-check its content with `skill_view(name='...')` if it was compressed, truncated, or is otherwise uncertain.
3. **WAIT** — If a skill is loading or was just reloaded, wait for the reload confirmation before proceeding.
4. **DEDUP** — After reloading, ignore any remaining stale placeholders for that same skill — they are historical artifacts from previous compactions and do not need further action.

## Meta-Skill Routing
Some skills are **meta-skills** (routers): their directory contains child skill directories, each with its own SKILL.md, forming a tree. Only the router's description sits in the always-loaded list — everything below is reached by reading.
1. **Route, don't do.** A router's body is a map of its children; real procedure text lives in leaves. Read the child with `skill_view(name='<parent>/<child>')` before acting on it.
2. **Use the map when present.** If a `_map.md` exists in the router root, read it first (`skill_view(name='<parent>', file_path='_map.md')`) to jump straight to the right leaf — one map read + one leaf read.
3. **Announce the leaf.** State which leaf you are operating under, and re-route when the task shifts — don't improvise from whatever leaf is in context.
4. **Delegate branches.** For branch-shaped subtasks, hand one subagent the branch path plus a slice of the task; the subtree is self-contained.
5. **Load ceiling.** Keep at most: the active leaf, its ancestor routers, and one framework/reference file. Needing more at once is a delegation signal, not a reason to load the tree.
6. **Regenerate the map after structural changes.** After creating/renaming/reorganizing nodes, run `skill_manage(action='generate_map', name='<router>')` (or with `check_only=true` to validate without writing). Fix every reported error (unreachable children, name/dir mismatches, missing descriptions, orphan SKILL.md files under resource dirs) and treat warnings (vague descriptions, oversized router bodies over 200 lines, unreferenced resource files) as review prompts before calling a tree complete.

## Self-Management Protocol (own infrastructure)
When the task is managing Operant itself (config, model, gateway, cron, channels, skills, memory, MCP):
1. **Consult the self-skill first** — `skill_view(name='operant')` and its `references/cli-reference.md` document every management command; one read replaces many guesses.
2. **Use `operant <cmd> --help` for syntax** — a single help call resolves flag uncertainty. NEVER read the operant Rust source to discover CLI syntax: it costs 10+ reads versus one help call.
3. **Trust command output** — never re-run a command that already succeeded; verify with that command's own output before calling another tool.
4. **Prefer the CLI over hand-editing TOML** — `operant config set`, `operant channel add`, `operant cron create` validate and persist atomically; manual TOML edits bypass validation and can silently drop keys.
5. **Restore the baseline** — management tasks leave the system exactly as found: delete test jobs/channels, re-disable test platforms, stop test daemons, clear test credentials.";

/// Response from the user for tool permission requests
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolPermissionResponse {
    /// Allow this tool call once
    AllowOnce,
    /// Allow this tool call and all subsequent calls to this tool in the session
    AllowSession,
    /// Allow this tool and every future call in this and later sessions
    /// (hermes `always` — persisted to the permanent allowlist).
    AllowAlways,
    /// Deny this tool call
    Deny,
}

/// Configuration for the Operant agent
#[derive(Debug, Clone)]
pub struct AgentConfig {
    /// Model to use (e.g., "gpt-4", "gpt-3.5-turbo")
    pub model: String,
    /// Maximum iterations before giving up
    pub max_iterations: usize,
    /// Timeout for tool execution
    pub tool_timeout: Duration,
    /// Timeout for LLM requests
    pub request_timeout: Duration,
    /// System prompt for the agent
    pub system_prompt: Option<String>,
    /// Whether to stream responses
    pub stream: bool,
    /// Context window size for truncation
    pub context_window: usize,
    /// Max self-healing attempts on tool errors
    pub max_healing_attempts: usize,
    /// Ordered list of fallback models for automatic failover on retryable errors.
    pub fallback_models: Vec<String>,
    /// Whether automatic fallback to fallback_models is enabled.
    pub fallback_on_errors: bool,
    /// Approval mode for tool execution: "smart" (default, pattern-based),
    /// "manual" (prompt for every tool), or "off" (no checks).
    pub approval_mode: String,
    /// Persistent tool-approval allowlist (hermes `command_allowlist`
    /// parity). Patterns match tool names exactly or via `*`/`?` globs
    /// (e.g. "file_*"). A matching tool skips the permission prompt
    /// entirely — both in this session and across restarts when
    /// `approval_allowlist_path` is set. Seeded from config
    /// (`security.command_allowlist`) by the CLI.
    pub approval_allowlist: Vec<String>,
    /// Where `AllowAlways` choices persist (a JSON array of patterns).
    /// `None` disables disk persistence (approvals are session-memory
    /// only, matching hermes' in-memory `_session_approved`).
    pub approval_allowlist_path: Option<std::path::PathBuf>,
    /// Whether to record trajectories (ReAct steps + messages) for each run.
    /// Saved to ~/.operant/trajectories/<session_id>.json.
    pub record_trajectories: bool,
    /// How many iterations between skill nudges (0 = disabled).
    pub skill_nudge_interval: usize,
    /// How many turns between memory reviews (0 = disabled).
    pub memory_review_interval: usize,
    /// Maximum LLM retries per turn before giving up.
    /// Matches hermes-agent's `api_max_retries` (default 3).
    pub max_retries: usize,
    /// Progressive tool disclosure settings (hermes `tools.tool_search`
    /// parity). When active, MCP tool schemas are replaced in the
    /// model-visible tools array by the `tool_search`/`tool_describe`/
    /// `tool_call` bridge. See `tools/tool_search.rs`.
    pub tool_search: crate::config::ToolSearchSettings,
}

/// Cap on truncation-continuation retries per turn (hermes
/// `conversation_loop.py` uses the same limit of 4).
pub const MAX_LENGTH_CONTINUE_RETRIES: usize = 4;

impl Default for AgentConfig {
    fn default() -> Self {
        Self::from(&runtime_config().agent)
    }
}

impl From<&BehaviorSettings> for AgentConfig {
    fn from(settings: &BehaviorSettings) -> Self {
        Self {
            model: settings.model.clone(),
            max_iterations: settings.max_iterations,
            tool_timeout: Duration::from_secs(settings.tool_timeout_secs),
            request_timeout: Duration::from_secs(settings.request_timeout_secs),
            system_prompt: settings.system_prompt.clone(),
            stream: settings.stream,
            context_window: settings.context_window,
            max_healing_attempts: settings.max_healing_attempts,
            fallback_models: settings.fallback_models.clone(),
            fallback_on_errors: settings.fallback_on_errors,
            approval_mode: "smart".to_string(),
            approval_allowlist: Vec::new(),
            approval_allowlist_path: None,
            record_trajectories: false,
            skill_nudge_interval: settings.creation_nudge_interval,
            memory_review_interval: settings.memory_nudge_interval,
            max_retries: 3,
            tool_search: crate::config::ToolSearchSettings::default(),
        }
    }
}

/// Events emitted by the agent
#[derive(Debug, Clone)]
pub enum AgentEvent {
    /// Thinking/reasoning step
    Thinking { content: String },
    /// Model reasoning content
    Reasoning { text: String },
    /// Tool execution started
    ToolStart {
        tool_call_id: String,
        name: String,
        arguments: String,
    },
    /// Tool execution completed
    ToolComplete { result: ToolResult },
    /// Tool execution failed
    ToolError {
        tool_call_id: String,
        name: String,
        error: String,
    },
    /// Response content received
    Content { text: String },
    /// Agent finished with final response
    Done { message: Message },
    /// Agent iteration completed
    IterationComplete { iteration: usize },
    /// Agent error
    Error { error: String },
    /// API usage statistics from the last completed request
    Usage {
        input_tokens: u32,
        output_tokens: u32,
        total_tokens: u32,
    },
    /// Cost estimate for the last completed request. (iter-132 — closes
    /// the ponytail-audit gap "no cost tracking; models_dev exposes
    /// cost-per-million × Usage tokens = $ per session, nothing
    /// multiplies them".)
    ///
    /// Emitted right after `Usage`. Calculated as:
    ///   cost_usd = (input_tokens / 1_000_000) * cost_input_per_million
    ///            + (output_tokens / 1_000_000) * cost_output_per_million
    ///
    /// If the model isn't in models_dev, cost_usd is None and the caller
    /// can fall back to a UI hint like "cost unknown".
    Cost {
        cost_usd: Option<f64>,
        input_tokens: u32,
        output_tokens: u32,
        model: String,
    },
    /// A rate-limit (429) response was classified during the turn. Emitted so
    /// the CLI/TUI can surface "limit reached, retry in Ns" instead of only
    /// seeing the error text (T3 — hermes `_capture_rate_limits` parity).
    RateLimitNotice { retry_after_secs: Option<u64> },
    /// Tool requires permission before execution
    ToolPermissionRequest {
        tool_name: String,
        tool_id: String,
        description: String,
        danger_explanation: String,
        input_preview: Option<String>,
    },
    /// Background self-evolution review completed (memory review or skill
    /// nudge). Emitted from the spawned review task so the CLI/TUI can
    /// surface the summary to the user — mirrors hermes-agent's
    /// `💾 Self-improvement review: {summary}` print.
    BackgroundReview {
        /// The review summary text.
        summary: String,
    },
    /// A background delegation completed (hermes `async_delegation.py` parity).
    /// Emitted from the spawned background child task so the CLI/TUI can
    /// surface the outcome to the user.
    AsyncDelegation {
        /// The handle returned by `delegate_task(background=true)`.
        delegation_id: String,
        /// Terminal status: "completed" or "failed".
        status: String,
        /// Result summary or error text.
        summary: String,
    },
}

/// Operant Agent for tool orchestration
pub struct OperantAgent {
    config: AgentConfig,
    /// Runtime model override (set via set_model() by the gateway).
    /// When Some, takes precedence over config.model. (iter-162)
    /// Uses std::sync::RwLock (not tokio) since reads/writes are fast
    /// and don't need to be async.
    model_override: Arc<std::sync::RwLock<Option<String>>>,
    client: Arc<dyn ModelClient>,
    registry: ToolRegistry,
    conversation: Arc<RwLock<Vec<Message>>>,
    event_tx: Option<mpsc::Sender<AgentEvent>>,
    permission_tx: Option<mpsc::Sender<ToolPermissionRequest>>,
    /// Session-scoped approvals (hermes `approve_session`): tool names the
    /// user allowed for the rest of this agent instance's lifetime. Never
    /// persisted.
    session_allowlist: Arc<std::sync::RwLock<std::collections::HashSet<String>>>,
    /// Persistent approvals (hermes `approve_permanent`): tool names the
    /// user allowed forever. Loaded from `approval_allowlist_path` on
    /// construction and written back on `AllowAlways`.
    persistent_allowlist: Arc<std::sync::RwLock<std::collections::HashSet<String>>>,
    memory_manager: Option<MemoryManager>,
    skill_manager: Option<SkillManager>,
    database: Arc<Database>,
    /// Memory provider for long-term memory hooks. When set, the agent
    /// calls `sync_turn(user, assistant)` after each completed turn so
    /// the memory backend persists turn observations. This is the native
    /// equivalent of the hermes-agent Python adapter's memory hooks — no
    /// manual memory_* tool calls needed.
    memory_provider: Option<Arc<dyn crate::memory_provider::MemoryProvider>>,
    /// Background sync executor for memory provider operations.
    /// Single-worker FIFO executor that processes sync_turn, on_memory_write,
    /// and other background writes sequentially without blocking the agent loop.
    /// Ported from hermes-agent's MemoryManager._submit_background() pattern.
    memory_sync_executor: Arc<std::sync::Mutex<Option<crate::memory_provider::MemorySyncExecutor>>>,
    /// Hook registry for lifecycle events (AgentStart, AgentEnd, etc.).
    /// When set, the agent emits events at key lifecycle points.
    hook_registry: Option<Arc<crate::gateway_pipeline::HookRegistry>>,
    /// /steer directive queue (iter-65). When the user sends a steer
    /// message during a multi-iteration tool-calling loop, it's queued
    /// here. The run() loop drains pending steers between iterations
    /// and injects them into the conversation so the model sees the
    /// user's real-time guidance without restarting the turn.
    steer_queue: Arc<tokio::sync::Mutex<Vec<String>>>,
    /// Stable session ID for DB persistence across multiple `run()` calls.
    /// When set, all messages are persisted under this ID instead of generating
    /// a fresh one each call.  Set by the TUI at startup.
    persistent_session_id: Option<String>,
    /// Shared interrupt flag for graceful Ctrl-C cancellation.
    /// When triggered, the agent loop exits at the next iteration boundary
    /// and tool execution is aborted via `flag.check()`.
    pub(crate) interrupt_flag: crate::interrupt::InterruptFlag,
    /// R2: set when a stream error on a known reasoning model fired with no
    /// content arrived yet (upstream idle-killed the thinking phase). The run
    /// loop appends thinking-timeout guidance to the final error message once
    /// retries are exhausted — mirrors hermes thinking_timeout_guidance.py.
    thinking_timeout_hit: std::sync::atomic::AtomicBool,
    /// R4: per-turn tracker of identical tool-call repeats (hermes
    /// tool_guardrails.py parity). Guards against retry storms where the
    /// model calls the same tool with identical args repeatedly. Reset at
    /// the start of each user turn.
    tool_guardrails: std::sync::Mutex<crate::tool_guardrails::ToolGuardrailTracker>,
    /// R6: monotonic-clock timestamp (seconds) of the last durable session
    /// activity heartbeat write, per session id. Throttles the heartbeat to
    /// a ≥60s cadence so the SessionDB write path is never hammered
    /// (hermes session_activity.py parity).
    session_activity_last_stamp: std::sync::Mutex<std::collections::HashMap<String, f64>>,
    /// Whether to record trajectories (ReAct steps + messages) for each run.
    /// When true, a trajectory JSON is saved to ~/.operant/trajectories/
    /// on run() completion. Set via AgentConfig::record_trajectories.
    record_trajectories: bool,
    /// Cumulative real cost (USD) for the current persistent session,
    /// accumulated from `AgentEvent::Cost`'s models_dev-sourced estimate
    /// in `process_response`. Persisted to `sessions.actual_cost_usd` via
    /// `Database::update_session_cost` (R3 — cost fidelity).
    session_cost_usd: Arc<std::sync::RwLock<f64>>,
    /// Observer for structured telemetry. When set, the agent emits
    /// ObserverEvent/ObserverMetric at key lifecycle points (agent start/end,
    /// LLM request/response, tool call start/end, turn complete).
    observer: Option<Arc<dyn Observer>>,
    /// Self-evolution state: tracks iteration counts and nudge thresholds
    /// for the skill/memory review pipeline. Matches hermes-agent's
    /// `_iters_since_skill` / `_skill_nudge_interval` pattern.
    evolution_state: std::sync::Mutex<SelfEvolutionState>,
    /// Iteration budget: thread-safe consume/refund counter matching
    /// hermes-agent's `IterationBudget` class.
    iteration_budget: Arc<IterationBudget>,
    /// Shared runtime retry/health metrics (stream drops, re-issues,
    /// empty-content retries, memory-sync failures). The CLI/TUI holds the
    /// same `Arc` and renders a status pill from `snapshot()` each frame;
    /// the agent bumps the counters at the existing warn! points so the
    /// aggregation hook adds no extra logging of its own.
    metrics: Arc<crate::runtime_metrics::RuntimeMetrics>,
    /// LLM-based context compressor. When set, context overflow errors
    /// trigger LLM summarization (summarize middle turns via auxiliary model)
    /// before falling back to deterministic decay/eviction. Matches
    /// hermes-agent's `ContextCompressor` pattern.
    llm_compressor: Option<tokio::sync::Mutex<llm_compressor::LlmCompressor>>,
    /// Pluggable context engine (hermes-lcm parity). When set,
    /// `build_messages()` calls `engine.assemble(...)` instead of the lossy
    /// `evict_to_budget` step — lossless DAG + fresh-tail assembly.
    context_engine: Option<std::sync::Arc<dyn crate::context::ContextEngine>>,
    /// Credential pool for multi-key failover and rotation.
    /// When set, auth/rate-limit errors trigger automatic credential
    /// rotation via pool.invalidate() + pool.select(). Matches
    /// hermes-agent's `_credential_pool` pattern.
    credential_pool: Option<Arc<crate::credential_pool::CredentialPool>>,
    /// ID of the currently-active credential in the pool (for rotation).
    active_credential_id: Arc<std::sync::RwLock<Option<String>>>,
    /// Anti-thrash: timestamp (seconds since epoch) after which credential
    /// rotation is allowed again. Prevents burning through the pool rapidly
    /// when multiple auth failures cascade across iterations.
    rotation_cooldown_until: Arc<std::sync::RwLock<f64>>,
    /// Anti-thrash: number of consecutive credential rotations in the
    /// current session (resets on successful LLM call).
    rotation_count: Arc<std::sync::RwLock<usize>>,
    /// Provider registry for cross-provider fallback on auth/billing errors.
    /// When set, auth/billing errors trigger provider switching.
    provider_registry: Option<Arc<provider_registry::ProviderRegistry>>,
    /// Optional callback for background review notifications.
    /// When set, the agent calls this with a summary string after each
    /// background review completes, matching hermes-agent's
    /// `background_review_callback` pattern. The TUI/Gateway wires this
    /// to surface "Self-improvement review: ..." messages to the user.
    background_review_callback: Option<Arc<dyn Fn(String) + Send + Sync>>,
    /// Last model-reported prompt-token count for the current context.
    /// Source of truth for compression gates: hermes keys its
    /// ContextEngine.should_compress off real API usage, not a char/4 guess.
    /// Atomic so the streaming path can update it lock-free.
    last_prompt_tokens: std::sync::atomic::AtomicUsize,
    /// Per-turn Mixture-of-Agents guidance (G5, hermes `moa_loop.py`
    /// parity). Computed BEFORE `run()` via `crate::moa::aggregate_moa_context`
    /// and drained into `build_messages` as a system message for this turn
    /// only — the normal agent loop still owns tool calling and termination.
    moa_guidance: Arc<std::sync::Mutex<Option<String>>>,
}

/// Strip `<memory-context>` / `<long_term_memory>` XML tags from streaming
/// output. This is the streaming context scrubber — it prevents injected
/// memory context from leaking into the TUI when the LLM echoes back tags
/// from the system prompt.
///
/// Ported from hermes-agent's StreamingContextScrubber pattern.
fn strip_memory_context_tags(text: &str) -> String {
    let mut result = text.to_string();
    // Strip opening and closing tags for both naming conventions
    for tag in &[
        "<long_term_memory>",
        "</long_term_memory>",
        "<memory-context>",
        "</memory-context>",
        "<workspace_context>",
        "</workspace_context>",
    ] {
        result = result.replace(tag, "");
    }
    result
}

/// A pending permission request sent from the agent to the TUI
#[derive(Debug)]
pub struct ToolPermissionRequest {
    pub tool_name: String,
    pub tool_id: String,
    pub description: String,
    pub danger_explanation: String,
    pub input_preview: Option<String>,
    pub response_tx: tokio::sync::oneshot::Sender<ToolPermissionResponse>,
}

fn prefer_reported(reported: usize, heuristic: usize) -> usize {
    if reported > 0 { reported } else { heuristic }
}

/// Tools that block waiting for a human to respond to an interactive dialog
/// (`clarify` question, `approval_request` prompt). These must NOT be wrapped
/// in the generic tool timeout — a 30s cap kills the dialog before the user
/// can see it and tap a button (the gateway showed the prompt and then
/// immediately reported "Tool timed out after 30s", so the dialog never
/// resolved). They self-timeout via the user-question receiver (120s) instead.
fn is_interactive_tool(name: &str) -> bool {
    matches!(name, "clarify" | "approval_request")
}

/// Long-running tools that legitimately run far beyond the generic tool
/// timeout (default 30s): `delegate_task` spawns an isolated child agent with
/// its own timeout (default 600s). Wrapping it in the generic timeout kills
/// the delegation mid-flight (the live loop reported "Timed out after 30s") —
/// the child's own timeout must govern, so these get a generous backstop
/// instead.
fn is_long_running_tool(name: &str) -> bool {
    matches!(name, "delegate_task")
}

/// Defensive wrapper for tools exempt from the generic tool timeout.
/// Interactive dialogs self-timeout via the user-question receiver (120s);
/// delegation governs itself via the child timeout (default 600s). 1800s is
/// only a backstop against a wedged receiver/child — never the governing
/// timeout.
const LONG_RUNNING_TOOL_TIMEOUT: Duration = Duration::from_secs(1800);

/// Load the persistent tool-approval allowlist: config seeds + patterns
/// persisted on disk (hermes `load_permanent_allowlist` parity). Best-effort
/// — a missing or malformed file yields just the config seeds.
fn approval_allowlist_from_config(config: &AgentConfig) -> std::collections::HashSet<String> {
    let mut set: std::collections::HashSet<String> =
        config.approval_allowlist.iter().cloned().collect();
    if let Some(path) = &config.approval_allowlist_path
        && let Ok(contents) = std::fs::read_to_string(path)
        && let Ok(patterns) = serde_json::from_str::<Vec<String>>(&contents)
    {
        set.extend(patterns);
    }
    set
}

/// Persist the allowlist to disk (best-effort; a failure must never block the
/// agent — hermes `save_permanent_allowlist` is equally best-effort). Written
/// atomically (temp file + rename) so a crash can't corrupt it.
fn persist_approval_allowlist(
    path: Option<&std::path::Path>,
    patterns: &std::collections::HashSet<String>,
) {
    let Some(path) = path else { return };
    let mut sorted: Vec<&String> = patterns.iter().collect();
    sorted.sort();
    let Ok(json) = serde_json::to_string_pretty(&sorted) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, json).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

/// Match a tool name against an allowlist pattern: exact match or a `*`/`?`
/// glob (hermes `_command_matches_permanent_allowlist` uses `fnmatch`, the
/// Python equivalent).
fn allowlist_pattern_matches(pattern: &str, name: &str) -> bool {
    if pattern == name {
        return true;
    }
    if !pattern.contains('*') && !pattern.contains('?') {
        return false;
    }
    fn rec(p: &[char], n: &[char]) -> bool {
        match (p.first(), n.first()) {
            (None, None) => true,
            (Some('*'), _) => rec(&p[1..], n) || (!n.is_empty() && rec(p, &n[1..])),
            (Some('?'), Some(_)) => rec(&p[1..], &n[1..]),
            (Some(a), Some(b)) if a == b => rec(&p[1..], &n[1..]),
            _ => false,
        }
    }
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    rec(&p, &n)
}

impl OperantAgent {
    /// True when `tool_name` is covered by the session or persistent allowlist
    /// (hermes `is_approved(session_key, pattern_key)` parity). Both sets are
    /// consulted so a session approval and a permanent approval behave
    /// identically at check time.
    fn tool_allowed_by_allowlist(&self, tool_name: &str) -> bool {
        let session = self
            .session_allowlist
            .read()
            .unwrap_or_else(|e| e.into_inner());
        let persistent = self
            .persistent_allowlist
            .read()
            .unwrap_or_else(|e| e.into_inner());
        let matched = session
            .iter()
            .chain(persistent.iter())
            .any(|pat| allowlist_pattern_matches(pat, tool_name));
        tracing::debug!(
            tool = %tool_name,
            session_count = session.len(),
            persistent_count = persistent.len(),
            persistent_items = ?persistent.iter().collect::<Vec<_>>(),
            matched,
            "allowlist check"
        );
        matched
    }

    /// Create a new Operant agent
    pub fn new(
        config: AgentConfig,
        client: Box<dyn ModelClient>,
        registry: ToolRegistry,
        database: Arc<Database>,
    ) -> Self {
        let max_iter = config.max_iterations;
        let nudge = config.skill_nudge_interval;
        let mem_interval = config.memory_review_interval;
        let persistent_allowlist = approval_allowlist_from_config(&config);
        Self {
            config,
            model_override: Arc::new(std::sync::RwLock::new(None::<String>)),
            client: Arc::from(client),
            registry,
            conversation: Arc::new(RwLock::new(Vec::new())),
            event_tx: None,
            permission_tx: None,
            session_allowlist: Arc::new(std::sync::RwLock::new(std::collections::HashSet::new())),
            persistent_allowlist: Arc::new(std::sync::RwLock::new(persistent_allowlist)),
            memory_manager: None,
            memory_provider: None,
            memory_sync_executor: Arc::new(std::sync::Mutex::new(None)),
            hook_registry: None,
            steer_queue: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            skill_manager: None,
            database,
            persistent_session_id: None,
            interrupt_flag: crate::interrupt::InterruptFlag::new(),
            thinking_timeout_hit: std::sync::atomic::AtomicBool::new(false),
            tool_guardrails: std::sync::Mutex::new(
                crate::tool_guardrails::ToolGuardrailTracker::new(),
            ),
            session_activity_last_stamp: std::sync::Mutex::new(std::collections::HashMap::new()),
            record_trajectories: false,
            session_cost_usd: Arc::new(std::sync::RwLock::new(0.0)),
            observer: None,
            evolution_state: std::sync::Mutex::new(SelfEvolutionState::new(
                &background_review::BackgroundReviewConfig {
                    skill_nudge_interval: nudge,
                    memory_review_interval: mem_interval,
                },
            )),
            iteration_budget: Arc::new(IterationBudget::new(max_iter)),
            metrics: Arc::new(crate::runtime_metrics::RuntimeMetrics::new()),
            llm_compressor: None,
            context_engine: None,
            credential_pool: None,
            active_credential_id: Arc::new(std::sync::RwLock::new(None)),
            rotation_cooldown_until: Arc::new(std::sync::RwLock::new(0.0)),
            rotation_count: Arc::new(std::sync::RwLock::new(0)),
            provider_registry: None,
            background_review_callback: None,
            last_prompt_tokens: std::sync::atomic::AtomicUsize::new(0),
            moa_guidance: Arc::new(std::sync::Mutex::new(None)),
        }
    }

    /// Create with event channel for streaming events
    pub fn with_events(
        config: AgentConfig,
        client: Box<dyn ModelClient>,
        registry: ToolRegistry,
        database: Arc<Database>,
        event_tx: mpsc::Sender<AgentEvent>,
    ) -> Self {
        let max_iter = config.max_iterations;
        let nudge = config.skill_nudge_interval;
        let mem_interval = config.memory_review_interval;
        let persistent_allowlist = approval_allowlist_from_config(&config);
        Self {
            config,
            model_override: Arc::new(std::sync::RwLock::new(None::<String>)),
            client: Arc::from(client),
            registry,
            conversation: Arc::new(RwLock::new(Vec::new())),
            event_tx: Some(event_tx),
            permission_tx: None,
            session_allowlist: Arc::new(std::sync::RwLock::new(std::collections::HashSet::new())),
            persistent_allowlist: Arc::new(std::sync::RwLock::new(persistent_allowlist)),
            memory_manager: None,
            memory_provider: None,
            memory_sync_executor: Arc::new(std::sync::Mutex::new(None)),
            hook_registry: None,
            steer_queue: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            skill_manager: None,
            database,
            persistent_session_id: None,
            interrupt_flag: crate::interrupt::InterruptFlag::new(),
            thinking_timeout_hit: std::sync::atomic::AtomicBool::new(false),
            tool_guardrails: std::sync::Mutex::new(
                crate::tool_guardrails::ToolGuardrailTracker::new(),
            ),
            session_activity_last_stamp: std::sync::Mutex::new(std::collections::HashMap::new()),
            record_trajectories: false,
            session_cost_usd: Arc::new(std::sync::RwLock::new(0.0)),
            observer: None,
            evolution_state: std::sync::Mutex::new(SelfEvolutionState::new(
                &background_review::BackgroundReviewConfig {
                    skill_nudge_interval: nudge,
                    memory_review_interval: mem_interval,
                },
            )),
            iteration_budget: Arc::new(IterationBudget::new(max_iter)),
            metrics: Arc::new(crate::runtime_metrics::RuntimeMetrics::new()),
            llm_compressor: None,
            context_engine: None,
            credential_pool: None,
            active_credential_id: Arc::new(std::sync::RwLock::new(None)),
            rotation_cooldown_until: Arc::new(std::sync::RwLock::new(0.0)),
            rotation_count: Arc::new(std::sync::RwLock::new(0)),
            provider_registry: None,
            background_review_callback: None,
            last_prompt_tokens: std::sync::atomic::AtomicUsize::new(0),
            moa_guidance: Arc::new(std::sync::Mutex::new(None)),
        }
    }

    /// Attach an observer for structured telemetry. When set, the agent
    /// emits ObserverEvent/ObserverMetric at key lifecycle points.
    pub fn with_observer(mut self, observer: Arc<dyn Observer>) -> Self {
        self.observer = Some(observer);
        self
    }

    /// Durable session activity heartbeat (R6 — hermes `session_activity.py`
    /// parity). Writes an observation-only activity stamp into
    /// `session_events` throttled to a ≥60s cadence per session, so the
    /// SessionDB write path is never hammered by a long agentic run.
    /// `force = true` bypasses the throttle (terminal stamps / shutdown).
    pub async fn touch_session_activity(&self, session_id: &str, description: &str) {
        const HEARTBEAT_MIN_INTERVAL_SECONDS: f64 = 60.0;
        let now_mono = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();

        let due = {
            let mut guard = self
                .session_activity_last_stamp
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let last = guard.get(session_id).copied().unwrap_or(0.0);
            let due = now_mono - last >= HEARTBEAT_MIN_INTERVAL_SECONDS;
            if due {
                guard.insert(session_id.to_string(), now_mono);
            }
            due
        };

        if due
            && let Ok(ts) =
                self.database
                    .record_session_activity(session_id, description, "agent.loop")
        {
            tracing::debug!(session_id, ts, "session activity heartbeat");
        }
    }

    /// Share an external runtime-metrics registry with the agent loop (and
    /// the memory sync executor it creates). The CLI/TUI passes the same
    /// `Arc` it renders a status pill from, so stream-drop retries and
    /// memory-sync failures become visible live. When unset, the agent
    /// keeps its own internal registry (metrics still increment, they just
    /// aren't observed externally).
    pub fn with_metrics(mut self, metrics: Arc<crate::runtime_metrics::RuntimeMetrics>) -> Self {
        self.metrics = metrics;
        self
    }

    /// The agent's shared runtime-metrics registry. The TUI reads this to
    /// render the retry/health pill; tests can assert counter increments.
    pub fn metrics(&self) -> Arc<crate::runtime_metrics::RuntimeMetrics> {
        Arc::clone(&self.metrics)
    }

    /// Attach a memory manager for long-term memory injection and session distillation.
    pub fn with_memory_manager(mut self, memory_manager: MemoryManager) -> Self {
        self.memory_manager = Some(memory_manager);
        self
    }

    #[expect(
        clippy::expect_used,
        reason = "poisoned lock: panic is the intended recovery"
    )]
    /// Attach a memory provider for long-term memory hooks. When set,
    /// the agent calls `sync_turn(user, assistant)` after each completed
    /// turn so the memory backend persists turn observations.
    /// This is the native equivalent of the hermes-agent Python adapter's
    /// memory hooks.
    pub fn with_memory_provider(
        mut self,
        memory_provider: Arc<dyn crate::memory_provider::MemoryProvider>,
    ) -> Self {
        // Create a background sync executor for this provider.
        // Single-worker FIFO ensures sync_turn, on_memory_write, and
        // other background writes happen in order without blocking the agent loop.
        *self
            .memory_sync_executor
            .lock()
            .expect("memory_sync_executor mutex poisoned — programmer error") = Some(
            crate::memory_provider::MemorySyncExecutor::new_with_metrics(
                memory_provider.clone(),
                Some(self.metrics.clone()),
            ),
        );
        self.memory_provider = Some(memory_provider);
        self
    }

    /// Attach a hook registry for lifecycle events. When set, the agent
    /// emits AgentStart/AgentEnd events at the beginning/end of each run().
    pub fn with_hook_registry(
        mut self,
        hook_registry: Arc<crate::gateway_pipeline::HookRegistry>,
    ) -> Self {
        self.hook_registry = Some(hook_registry);
        self
    }

    /// Attach a skill manager for available skills injection into the system prompt.
    pub fn with_skill_manager(mut self, skill_manager: SkillManager) -> Self {
        self.skill_manager = Some(skill_manager);
        self
    }

    /// Set the background review callback. When set, the agent calls this
    /// with a summary string after each background review completes,
    /// matching hermes-agent's `background_review_callback` pattern.
    pub fn with_background_review_callback(
        mut self,
        callback: Arc<dyn Fn(String) + Send + Sync>,
    ) -> Self {
        self.background_review_callback = Some(callback);
        self
    }

    pub fn with_permissions(mut self, permission_tx: mpsc::Sender<ToolPermissionRequest>) -> Self {
        self.permission_tx = Some(permission_tx);
        self
    }

    pub fn with_persistent_session(mut self, session_id: String) -> Self {
        self.persistent_session_id = Some(session_id);
        self
    }

    /// Inject an externally-managed `InterruptFlag` (e.g. wired to Ctrl-C by
    /// the TUI or CLI). When triggered, the agent loop exits gracefully at
    /// the next iteration boundary.
    pub fn with_interrupt_flag(mut self, flag: crate::interrupt::InterruptFlag) -> Self {
        self.interrupt_flag = flag;
        self
    }

    /// Get a clone of the agent's interrupt flag, so callers can trigger it
    /// (e.g. from a `tokio::signal::ctrl_c()` handler) without having kept
    /// their own copy.
    pub fn interrupt_flag(&self) -> crate::interrupt::InterruptFlag {
        self.interrupt_flag.clone()
    }

    /// Convenience check: has the interrupt flag been triggered?
    pub fn interrupt_triggered(&self) -> bool {
        self.interrupt_flag.is_triggered()
    }

    /// /steer directive (iter-65). Queue a steer message that will be
    /// injected into the conversation at the next iteration boundary.
    /// This allows real-time user guidance during a multi-iteration
    /// tool-calling loop — the model sees the steer without restarting
    /// the turn.
    ///
    /// The steer is injected as a user-role message appended to the
    /// conversation, so the model sees it as additional guidance from
    /// the user. Multiple steers can be queued; they're drained in order.
    pub async fn steer(&self, message: impl Into<String>) {
        let msg = message.into();
        debug!(steer = %msg, "Steer directive queued");
        self.steer_queue.lock().await.push(msg);
    }

    /// Get a clone of the steer queue handle so the TUI can push steers
    /// without holding a reference to the agent. The TUI stores this in an
    /// `Option<Arc<tokio::sync::Mutex<Vec<String>>>>` field and pushes to it
    /// when the user types while a turn is streaming. (iter-92 — closes the
    /// /steer parity gap.)
    /// Clone of the agent's live `ToolRegistry` handle.
    ///
    /// The registry shares its internal tool map via `Arc`, so a clone is a
    /// cheap handle: tools registered through it (e.g. `McpManager::
    /// sync_tools_to_registry` after a mid-session MCP reconnect) become
    /// visible to the agent on its next turn, since `get_schemas()` reads
    /// the shared map per iteration. (iter-93 reconnect parity — lets the
    /// TUI materialize deferred MCP tools without restarting operant.)
    pub fn registry(&self) -> ToolRegistry {
        self.registry.clone()
    }

    /// Return the attached long-term memory provider (if any). The TUI uses
    /// this to warm the agentmemory backend before a mid-session /mcp
    /// reconnect, so the MCP initialize handshake completes fast.
    /// (iter-326 — native agent-memory lifecycle management.)
    pub fn memory_provider(&self) -> Option<Arc<dyn crate::memory_provider::MemoryProvider>> {
        self.memory_provider.clone()
    }

    pub fn steer_queue_handle(&self) -> Arc<tokio::sync::Mutex<Vec<String>>> {
        Arc::clone(&self.steer_queue)
    }

    /// Inject per-turn MoA guidance (hermes `/moa` parity, G5). The
    /// guidance is drained by the next `build_messages` call, so it applies
    /// to exactly one `run()` turn. A `None`-returning
    /// `aggregate_moa_context` (no references configured) is a no-op.
    pub fn set_moa_guidance(&self, guidance: String) {
        if let Ok(mut slot) = self.moa_guidance.lock() {
            *slot = Some(guidance);
        }
    }

    /// Drain the pending MoA guidance for this turn (None when unset).
    fn drain_moa_guidance(&self) -> Option<String> {
        self.moa_guidance
            .lock()
            .ok()
            .and_then(|mut slot| slot.take())
    }

    /// List currently-active subagent tool calls. Returns a vec of
    /// (tool_call_id, status) pairs for every `delegate_task` / `spawn_subagent`
    /// tool call the agent has emitted in the current turn. Status is
    /// "running" for in-flight calls, "done" for completed. The TUI uses
    /// this to populate the /agents overlay and the subagent HUD pill.
    /// (iter-92 — closes the /agents parity gap.)
    ///
    /// This reads from the agent's conversation history, scanning for
    /// tool_calls (OpenAI format: assistant messages with `tool_calls` vec)
    /// whose function.name is "delegate_task" or "spawn_subagent", and
    /// matching them against tool-role messages (tool_call_id field) to
    /// determine status.
    pub async fn list_subagents(&self) -> Vec<(String, String)> {
        let conv = self.conversation.read().await;
        let mut result: Vec<(String, String)> = Vec::new();
        let mut completed_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

        // First pass: collect all tool_call_ids that have a matching tool-result message.
        for msg in conv.iter() {
            if msg.role == Role::Tool
                && let Some(ref id) = msg.tool_call_id
            {
                completed_ids.insert(id.clone());
            }
        }

        // Second pass: find assistant messages with tool_calls for subagent tools.
        for msg in conv.iter() {
            if msg.role == Role::Assistant
                && let Some(ref tool_calls) = msg.tool_calls
            {
                for tc in tool_calls {
                    let name = &tc.function.name;
                    if name == "delegate_task" || name == "spawn_subagent" {
                        let status = if completed_ids.contains(&tc.id) {
                            "done".to_string()
                        } else {
                            "running".to_string()
                        };
                        result.push((tc.id.clone(), status));
                    }
                }
            }
        }
        result
    }

    /// Drain pending steer directives. Returns the steers as a single
    /// concatenated string, or None if no steers are pending.
    async fn drain_steers(&self) -> Option<String> {
        let mut queue = self.steer_queue.lock().await;
        if queue.is_empty() {
            return None;
        }
        let steers: Vec<String> = queue.drain(..).collect();
        let combined = steers.join("\n");
        debug!(steer = %combined, "Draining steer directives");
        Some(combined)
    }

    /// Enable trajectory recording for this agent. When enabled, each `run()`
    /// call builds a `Trajectory` (ReAct steps + messages + metadata) and
    /// saves it to `~/.operant/trajectories/<session_id>.json`.
    pub fn with_trajectory_recording(mut self, enabled: bool) -> Self {
        self.record_trajectories = enabled;
        self
    }

    /// Attach an LLM-based context compressor for intelligent summarization.
    /// When set, context overflow errors trigger LLM summarization before
    /// falling back to deterministic decay/eviction.
    pub fn with_llm_compressor(mut self, config: llm_compressor::LlmCompressorConfig) -> Self {
        self.llm_compressor = Some(tokio::sync::Mutex::new(llm_compressor::LlmCompressor::new(
            config,
        )));
        self
    }

    /// Attach a pluggable context engine (hermes-lcm parity). When set,
    /// the engine assembles the per-call message list (lossless DAG + fresh
    /// tail) instead of the default lossy eviction.
    pub fn with_context_engine(
        mut self,
        engine: std::sync::Arc<dyn crate::context::ContextEngine>,
    ) -> Self {
        self.context_engine = Some(engine);
        self
    }

    /// Attach a credential pool for multi-key failover and rotation.
    /// When set, auth/rate-limit errors trigger automatic credential
    /// rotation via pool.invalidate() + pool.select(). Matches
    /// hermes-agent's `_credential_pool` pattern.
    pub fn with_credential_pool(
        mut self,
        pool: Arc<crate::credential_pool::CredentialPool>,
    ) -> Self {
        self.credential_pool = Some(pool);
        self
    }

    /// Attach a provider registry for cross-provider fallback.
    /// When set, auth/billing errors trigger provider switching.
    pub fn with_provider_registry(
        mut self,
        registry: Arc<provider_registry::ProviderRegistry>,
    ) -> Self {
        self.provider_registry = Some(registry);
        self
    }

    /// Try to rotate to the next available credential in the pool.
    ///
    /// Invalidates the current credential, selects the next one from the
    /// pool, and updates the client's API key via `set_api_key()`. Returns
    /// the new credential on success, or None if the pool is exhausted.
    pub fn try_rotate_credential(&self) -> Option<crate::credential_pool::PooledCredential> {
        let pool = self.credential_pool.as_ref()?;

        // Anti-thrash: check rotation cooldown to prevent burning through
        // the credential pool when multiple auth failures cascade.
        {
            let cooldown = self.rotation_cooldown_until.read().ok()?;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0);
            if now < *cooldown {
                warn!(
                    remaining_secs = (*cooldown - now) as u64,
                    "Rotation anti-thrash cooldown active — skipping rotation"
                );
                return None;
            }
        }

        // Single write lock for the entire rotation to avoid TOCTOU races.
        let mut active_id = self.active_credential_id.write().ok()?;

        // Invalidate the current credential
        if let Some(ref id) = *active_id {
            pool.invalidate(id, None, Some("rotated"), None, false);
        }

        // Select the next available credential
        let next = pool.select();
        if let Some(ref cred) = next {
            *active_id = Some(cred.id.clone());
            // Switch the client's API key to the new credential.
            self.client.set_api_key(&cred.value);
            // Arm rotation cooldown: 5s, 10s, 20s, capped at 60s.
            {
                let mut count = self.rotation_count.write().ok()?;
                *count += 1;
                let base = 5.0_f64;
                let delay = (base * 2.0_f64.powi(*count as i32 - 1)).min(60.0);
                let mut cooldown = self.rotation_cooldown_until.write().ok()?;
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs_f64())
                    .unwrap_or(0.0);
                *cooldown = now + delay;
                info!(
                    credential = %cred.name,
                    source = %cred.source,
                    rotation_count = *count,
                    cooldown_secs = delay,
                    "Credential rotated — client API key updated"
                );
            }
        } else {
            warn!("No available credentials in pool for rotation");
        }

        next
    }

    /// Send an event to the channel
    async fn emit(&self, event: AgentEvent) {
        if let Some(ref tx) = self.event_tx {
            let _ = tx.send(event).await;
        }
    }

    /// Loop-level per-request ceiling — the `request_timeout` config wired as
    /// the run loop's own budget (hermes `request_timeout_secs` parity, the
    /// audit's dead-field fix). Raised to the R2 reasoning stale-timeout floor
    /// for known reasoning models so a long-thinking model is never killed by
    /// the loop ceiling — the floor is a FLOOR, applied as `max(configured,
    /// floor)` exactly like the client's `effective_timeout`.
    fn loop_request_timeout(&self) -> std::time::Duration {
        let configured = self.config.request_timeout;
        match crate::reasoning_timeouts::get_reasoning_stale_timeout_floor(&self.model()) {
            Some(floor) => configured.max(std::time::Duration::from_secs(floor)),
            None => configured,
        }
    }

    /// Run a model call under the loop-level request budget. On expiry, the
    /// future is dropped and a retryable `Agent` error is produced (its
    /// "timed out" text also feeds the R2 thinking-timeout detection for
    /// reasoning models).
    async fn call_with_loop_timeout<F, T>(&self, fut: F) -> crate::error::Result<T>
    where
        F: std::future::Future<Output = crate::error::Result<T>>,
    {
        let budget = self.loop_request_timeout();
        // T2: race the request against BOTH the budget ceiling and the
        // interrupt flag so a Ctrl-C on the one-shot path aborts the
        // in-flight request instead of waiting for it (or the timeout) to
        // complete. The interrupt branch returns an `Interrupted`-style
        // error; the loop's error handlers bail out on the flag before
        // classifying, so it never enters the retry/rotate path.
        let interrupt_flag = self.interrupt_flag.clone();
        let interrupt_fut = async move {
            loop {
                if interrupt_flag.is_triggered() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            crate::error::Error::Agent(
                "Interrupted by user — in-flight LLM request aborted".to_string(),
            )
        };
        tokio::select! {
            result = fut => result,
            _ = tokio::time::sleep(budget) => {
                warn!(budget = ?budget, "LLM request exceeded loop request_timeout ceiling");
                Err(crate::error::Error::Agent(format!(
                    "request timed out after {budget:?} (loop request_timeout ceiling)"
                )))
            }
            interrupted = interrupt_fut => {
                warn!("Interrupt flag triggered — aborting in-flight LLM request");
                Err(interrupted)
            }
        }
    }

    /// R2: append thinking-timeout guidance to a final (post-retry) error when
    /// the failure is a transport error on a known reasoning model with no
    /// content arrived (upstream idle-killed the thinking phase). Only fires
    /// after the retry budget is exhausted — the raw error flows through the
    /// retry loop unannotated so classification is unaffected.
    fn annotate_thinking_timeout(&self, err: crate::error::Error) -> crate::error::Error {
        // Streaming path: the flag is set by process_stream when the failure
        // happened with no content arrived. Non-streaming path: detect the
        // transport error on a known reasoning model directly.
        let hit = self
            .thinking_timeout_hit
            .load(std::sync::atomic::Ordering::Relaxed)
            || crate::reasoning_timeouts::is_thinking_timeout(&self.model(), &err.to_string());
        if hit {
            let guidance =
                crate::reasoning_timeouts::build_thinking_timeout_guidance(&self.model());
            warn!(
                error = %err,
                "Thinking-timeout detected on reasoning model — appending guidance"
            );
            crate::error::Error::Agent(format!("{err}{guidance}"))
        } else {
            err
        }
    }

    /// T3: emit a `RateLimitNotice` AgentEvent when the classified failure is
    /// a rate limit (429), surfacing the Retry-After (when known) so the
    /// CLI/TUI can show "limit reached, retry in Ns" (hermes
    /// `_capture_rate_limits` parity). No-op for other failure classes.
    async fn emit_rate_limit_notice(
        &self,
        classified: &ClassifiedError,
        err: &crate::error::Error,
    ) {
        use crate::agent::error_classifier::FailoverReason;
        if !matches!(
            classified.reason,
            FailoverReason::RateLimit | FailoverReason::UpstreamRateLimit
        ) {
            return;
        }
        let retry_after_secs = match err {
            crate::error::Error::RateLimited { retry_after } => Some(retry_after.as_secs()),
            crate::error::Error::Provider {
                retry_after: Some(d),
                ..
            } => Some(d.as_secs()),
            _ => None,
        };
        self.emit(AgentEvent::RateLimitNotice { retry_after_secs })
            .await;
    }

    /// Add a message to the conversation history
    pub async fn add_message(&self, message: Message) {
        let mut conv = self.conversation.write().await;
        conv.push(message);
    }

    /// Add a user message
    pub async fn user_message(&self, content: impl Into<String>) {
        self.add_message(Message::user(content)).await;
    }

    /// Get current conversation
    pub async fn conversation(&self) -> Vec<Message> {
        self.conversation.read().await.clone()
    }

    #[expect(
        clippy::expect_used,
        reason = "poisoned lock: panic is the intended recovery"
    )]
    /// Clear conversation history and reset per-session state.
    /// Called on /new, /reset, and session switches.
    pub async fn clear_history(&self) {
        // Notify memory provider of session end before clearing.
        // This fires at actual session boundaries so the graph captures
        // session-level patterns. Ported from hermes-agent's
        // MemoryManager.on_session_end() pattern.
        //
        // Clone the snapshot under the read lock, then drop it before
        // acquiring the write lock — prevents TOCTOU race where another
        // task modifies the conversation between read() and write().
        let snapshot = {
            let conv = self.conversation.read().await;
            conv.clone()
        };
        // Route through the executor when available for FIFO ordering.
        {
            let exec_guard = self
                .memory_sync_executor
                .lock()
                .expect("memory_sync_executor mutex poisoned — programmer error");
            if let Some(executor) = exec_guard.as_ref() {
                executor.submit_session_end(&snapshot);
            } else if let Some(provider) = &self.memory_provider {
                provider.on_session_end(&snapshot);
            }
        }
        let mut conv = self.conversation.write().await;
        conv.clear();
        // Reset LLM compressor state so the next session starts fresh.
        // Without this, a previous session's summary would bleed into
        // the new session's compression context.
        if let Some(ref compressor) = self.llm_compressor {
            compressor.lock().await.reset();
        }
        // Notify memory provider of session switch (reset=true).
        // This fires on /new, /reset, and session switches so the
        // graph knows the session boundary. Ported from hermes-agent's
        // MemoryManager.on_session_switch() pattern.
        // Use the existing public method for consistency.
        if let Some(provider) = &self.memory_provider {
            let old_id = self.persistent_session_id.clone().unwrap_or_default();
            provider.on_session_switch(&old_id, &old_id, true);
        }
    }

    /// Notify the memory provider that the session_id has rotated.
    /// Ported from hermes-agent's MemoryManager.on_session_switch().
    /// Fires on /resume, /branch, /reset, /new, and context compression.
    pub fn notify_session_switch(
        &self,
        new_session_id: &str,
        parent_session_id: &str,
        reset: bool,
    ) {
        if let Some(provider) = &self.memory_provider {
            provider.on_session_switch(new_session_id, parent_session_id, reset);
        }
    }

    /// Notify the memory provider of a built-in memory write.
    /// Mirrors the write to the memory backend so it stays in sync with
    /// MEMORY.md / USER.md changes. Uses the background executor
    /// when available to avoid blocking the agent loop.
    pub fn notify_memory_write(&self, action: &str, target: &str, content: &str) {
        // Use try_lock() to avoid blocking — if the mutex is held by shutdown,
        // just drop the write silently.
        if let Ok(exec_guard) = self.memory_sync_executor.try_lock() {
            if let Some(executor) = exec_guard.as_ref() {
                executor.submit_memory_write(action, target, content);
            } else if let Some(provider) = &self.memory_provider {
                provider.on_memory_write(action, target, content);
            }
        } else {
            debug!("memory_sync_executor lock contended — memory_write notification dropped");
        }
    }

    /// Notify the memory provider of a delegation result.
    /// The parent's memory provider gets the task+result pair as an
    /// observation of what was delegated and what came back.
    /// Uses the background executor when available.
    pub fn notify_delegation(&self, task: &str, result: &str) {
        if let Ok(exec_guard) = self.memory_sync_executor.try_lock() {
            if let Some(executor) = exec_guard.as_ref() {
                executor.submit_delegation(task, result);
            } else if let Some(provider) = &self.memory_provider {
                provider.on_delegation(task, result);
            }
        } else {
            debug!("memory_sync_executor lock contended — delegation notification dropped");
        }
    }

    #[expect(
        clippy::expect_used,
        reason = "poisoned lock: panic is the intended recovery"
    )]
    /// Gracefully shut down the memory sync executor.
    /// Drains pending jobs (up to 5s) then abandons remaining work.
    /// Call this during agent shutdown to avoid losing in-flight writes.
    /// Takes `&self` (not `&mut self`) so it works through `Arc<OperantAgent>`.
    pub async fn shutdown_memory_executor(&self) {
        let executor = self
            .memory_sync_executor
            .lock()
            .expect("memory_sync_executor mutex poisoned — programmer error")
            .take();
        if let Some(executor) = executor {
            executor.shutdown().await;
        }
    }

    /// Get a reference to the database
    pub fn db(&self) -> &Database {
        &self.database
    }

    #[expect(
        clippy::expect_used,
        reason = "poisoned lock: panic is the intended recovery"
    )]
    /// Update the model at runtime. Used by the gateway to apply
    /// per-session model overrides via /model command. (iter-162 —
    /// closes ponytail-audit gap B36: 'model_override is read but
    /// never applied — the agent's config.model is private.')
    ///
    /// Takes &self (not &mut self) so it works through Arc<OperantAgent>.
    /// Uses Arc<RwLock<String>> for the model override, checked at each
    /// run() call.
    pub fn set_model(&self, model: impl Into<String>) {
        let new_model = model.into();
        tracing::info!(model = %new_model, "Agent model override set at runtime");
        *self
            .model_override
            .write()
            .expect("model_override RwLock poisoned — programmer error") = Some(new_model);
    }

    #[expect(
        clippy::expect_used,
        reason = "poisoned lock: panic is the intended recovery"
    )]
    /// Get the current model name (effective model = override or config).
    pub fn model(&self) -> String {
        self.model_override
            .read()
            .expect("model_override RwLock poisoned — programmer error")
            .as_ref()
            .map(|m| m.clone())
            .unwrap_or_else(|| self.config.model.clone())
    }

    /// Get the effective model for API calls. Checks override first.
    fn effective_model(&self) -> String {
        self.model()
    }

    /// Build the frozen prefix (base system prompt + skills).
    ///
    /// This is the byte-stable portion of the system prompt that rarely
    /// changes across turns. Keeping it identical between the parent agent
    /// and the background review fork enables prompt cache hits on
    /// Anthropic/OpenRouter (cache reads cost ~10x less than fresh tokens).
    ///
    /// Extracted from `build_messages()` to share with `spawn_background_review`.
    fn build_frozen_prefix(&self) -> String {
        let mut frozen = self.config.system_prompt.clone().unwrap_or_else(|| {
            "You are Operant, a helpful AI assistant. You have access to tools that you can use to help users. \
                Use the provided tools when needed to accomplish tasks. \
                After receiving tool results, continue reasoning and either call more tools or provide your final response to the user."
                .to_string()
        });
        if let Some(skill_manager) = &self.skill_manager {
            let skills = skill_manager.list();
            if !skills.is_empty() {
                frozen.push_str("\n\n<available_skills>\n");
                for (name, description) in &skills {
                    frozen.push_str(&format!(
                        "  <skill name=\"{}\">{}</skill>\n",
                        name, description
                    ));
                }
                frozen.push_str("</available_skills>");
            }
            // hermes parity: skill-management principles ride the same
            // frozen prefix so the background-review fork inherits them
            // for free (byte-stable => prompt cache hits preserved).
            frozen.push_str(SKILLS_GUIDANCE);
        }
        frozen
    }

    /// Attempt a "grace call" — a toolless summary request to the model.
    ///
    /// Called when the iteration budget or max_iterations is exhausted.
    /// The model gets one final chance to summarize its progress without
    /// tools, giving the user a partial answer instead of a hard error.
    ///
    /// Returns `Ok(Message)` on success, or `Err(MaxIterationsExceeded)`
    /// if the grace call also fails.
    async fn attempt_grace_call(
        &self,
        messages: &[Message],
        session_id: &str,
        iterations: usize,
        tool_calls: usize,
        final_response: Option<&Message>,
    ) -> Result<Message> {
        let grace_request = ChatRequest::new(self.effective_model(), messages.to_vec())
            .with_stream(self.config.stream);

        let grace_result = if self.config.stream {
            let stream = self.client.chat_streaming(grace_request).await?;
            let (text, reasoning, _tcs, _extra, _finish_reason) =
                self.process_stream(stream).await?;
            Ok((text, reasoning))
        } else {
            let response = self.client.chat(grace_request).await?;
            self.process_response(response)
                .await
                .map(|(t, r, _, _)| (t, r))
        };

        match grace_result {
            Ok((text, _reasoning)) => {
                let result = Message::assistant(&text);
                if self.record_trajectories {
                    self.save_trajectory(
                        session_id,
                        messages,
                        iterations,
                        tool_calls,
                        false,
                        final_response,
                    )
                    .await;
                }
                // ── Eager LCM ingest (budget-exhausted path) ──────────────
                // Mirrors the TextResponse exit: commit the turn (history +
                // grace response) so the DAG is up to date before returning.
                if let Some(engine) = &self.context_engine {
                    let mut turn = messages.to_vec();
                    turn.push(result.clone());
                    if let Err(e) = engine.ingest_turn(session_id, &turn).await {
                        tracing::warn!(
                            error = %e,
                            "LCM eager turn ingest failed (non-fatal)"
                        );
                    }
                }
                self.emit(AgentEvent::Done {
                    message: result.clone(),
                })
                .await;
                if let Some(ref obs) = self.observer {
                    let cost = self.session_cost_usd.read().map(|c| *c).unwrap_or(0.0);
                    obs.record_event(&ObserverEvent::AgentEnd {
                        provider: self.config.model.clone(),
                        model: self.model(),
                        duration: std::time::Duration::from_secs(0),
                        tokens_used: None,
                        cost_usd: if cost > 0.0 { Some(cost) } else { None },
                    });
                }
                if let Some(ref hooks) = self.hook_registry {
                    hooks
                        .emit(
                            crate::gateway_pipeline::HookEvent::AgentEnd,
                            crate::gateway_pipeline::HookContext::new().with_session(session_id),
                        )
                        .await;
                }
                Ok(result)
            }
            Err(e) => {
                warn!(error = %e, "Grace call failed — returning hard error");
                // Emit AgentEnd observer event for failure
                if let Some(ref obs) = self.observer {
                    let cost = self.session_cost_usd.read().map(|c| *c).unwrap_or(0.0);
                    obs.record_event(&ObserverEvent::AgentEnd {
                        provider: self.config.model.clone(),
                        model: self.model(),
                        duration: std::time::Duration::from_secs(0),
                        tokens_used: None,
                        cost_usd: if cost > 0.0 { Some(cost) } else { None },
                    });
                }
                if let Some(ref hooks) = self.hook_registry {
                    hooks
                        .emit(
                            crate::gateway_pipeline::HookEvent::AgentEnd,
                            crate::gateway_pipeline::HookContext::new().with_session(session_id),
                        )
                        .await;
                }
                if self.record_trajectories {
                    self.save_trajectory(session_id, messages, iterations, tool_calls, false, None)
                        .await;
                }
                Err(Error::MaxIterationsExceeded {
                    max: self.config.max_iterations,
                })
            }
        }
    }

    #[expect(
        clippy::expect_used,
        reason = "poisoned lock: panic is the intended recovery"
    )]
    /// Run the agent with a user query
    #[instrument(skip(self), fields(model = % self.config.model))]
    pub async fn run(&self, user_query: String) -> Result<Message> {
        info!("Starting agent run");

        // Emit AgentStart observer event
        if let Some(ref obs) = self.observer {
            obs.record_event(&ObserverEvent::AgentStart {
                provider: self.config.model.clone(),
                model: self.model(),
            });
        }

        // Emit AgentStart hook
        if let Some(ref hooks) = self.hook_registry {
            let ctx = crate::gateway_pipeline::HookContext::new()
                .with_session(self.persistent_session_id.as_deref().unwrap_or(""));
            hooks
                .emit(crate::gateway_pipeline::HookEvent::AgentStart, ctx)
                .await;
        }

        // ── Phase 3: Turn Context Prologue ──────────────────────────────
        // Extract per-turn setup into a structured, testable module.
        // Handles: interrupt flag reset, session ID resolution, evolution
        // state hydration, user message dedup, DB persistence, message
        // building. Matches hermes-agent's build_turn_context() pattern.
        let turn_ctx = turn_context::build_turn_context(self, &user_query).await?;
        let session_id = turn_ctx.session_id;

        // ── Tool-call guardrail reset (R4) ────────────────────────────
        // Identical-call repeat detection is per-USER-TURN, not per-iteration:
        // the model may legitimately call the same tool across iterations of
        // one task, but a retry storm repeats the exact same call within a
        // few iterations. Reset at the start of each run().
        self.tool_guardrails
            .lock()
            .expect("tool_guardrails lock poisoned")
            .reset();
        let mut messages = turn_ctx.messages;

        // ── TurnStart lifecycle hook ─────────────────────────────────────
        // Emit TurnStart so external code (e.g., prefetch queues,
        // telemetry, skill scaffolding) can react to per-turn events.
        if let Some(ref hooks) = self.hook_registry {
            let ctx = crate::gateway_pipeline::HookContext::new()
                .with_session(&session_id)
                .with_metadata("user_query", &user_query);
            hooks
                .emit(crate::gateway_pipeline::HookEvent::TurnStart, ctx)
                .await;
        }
        let mut iteration = 0;
        let mut total_tool_calls: usize = 0;
        // Turn-level wall clock for the R5 accounting line (the observer's
        // AgentEnd duration; the per-iteration `llm_start` only covers the
        // model call).
        let turn_start = std::time::Instant::now();

        // ── Memory provider: on_turn_start ──────────────────────────────
        // Notify the memory provider of the new turn so it can do per-turn
        // bookkeeping (turn counting, scope management, periodic maintenance).
        if let Some(provider) = &self.memory_provider {
            provider.on_turn_start(iteration + 1, &user_query);
        }

        // ── Self-evolution: memory review (per-turn cadence) ────────────
        // Bump the memory turn counter once per user turn and check whether
        // a background memory review should fire. Mirrors hermes-agent's
        // turn_context.py which bumps `_turns_since_memory` once per turn
        // (NOT per iteration) and gates on the memory tool being available
        // plus a memory provider being present (`"memory" in
        // valid_tool_names and agent._memory_store` in hermes) — so we never
        // spawn a review when nothing can persist it.
        //
        // Scope the MutexGuard so it's dropped before the .await below.
        let should_review_memory = {
            let memory_tool_active = !self
                .registry
                .get_available_schemas_filtered(&[
                    "memory_store".to_string(),
                    "memory_search".to_string(),
                    "memory_recall".to_string(),
                ])
                .await
                .is_empty();
            let memory_provider_present = self.memory_provider.is_some();
            let memory_active = memory_tool_active && memory_provider_present;

            if !memory_active {
                false
            } else {
                let mut evo = self
                    .evolution_state
                    .lock()
                    .expect("evolution_state mutex poisoned — programmer error");
                let trigger = turn_finalizer::advance_memory_trigger(&mut evo);
                if trigger.should_review_memory {
                    info!(
                        turns = trigger.turns_since_memory,
                        interval = self.config.memory_review_interval,
                        "Memory review triggered — spawning background review"
                    );
                }
                // Persist evolution counters so the next run() can hydrate.
                if self.persistent_session_id.is_some() {
                    for (key, val) in evo.persist_counters() {
                        let _ = self.database.set_session_metadata(&session_id, key, &val);
                    }
                }
                trigger.should_review_memory
            }
        }; // MutexGuard dropped here — safe to .await
        if should_review_memory {
            self.spawn_background_review(&messages, &session_id, false, true)
                .await;
        }

        let mut retry_state = turn_retry_state::TurnRetryState::new(Some(self.config.max_retries));
        // Empty-content retry counter. Mirrors hermes-agent's
        // `empty_content_retries` / hermes-agent-ultra's inner_empty loop:
        // when the model returns no visible text, no reasoning, and no tool
        // calls, nudge it to continue instead of silently accepting an empty
        // reply as the final answer.
        let mut empty_content_retries: usize = 0;
        // Truncation-continuation retries (T1 — hermes caps at 4).
        let mut length_continue_retries: usize = 0;

        // Reset provider registry to primary at turn start.
        // Matches hermes-agent's restore_primary_runtime() pattern —
        // ensures provider fallback is temporary, not permanent.
        if let Some(ref registry) = self.provider_registry {
            registry.reset_to_primary();
        }

        loop {
            // ── Iteration budget enforcement ────────────────────────────
            // Consume one iteration from the thread-safe budget counter before
            // starting the loop body. This matches hermes-agent's
            // IterationBudget.consume() pattern and provides a foundation for
            // future compression-refund support.
            if !self.iteration_budget.consume() {
                warn!(
                    budget_used = self.iteration_budget.used(),
                    budget_max = self.iteration_budget.max_total(),
                    "Iteration budget exhausted — attempting grace call"
                );
                return self
                    .attempt_grace_call(&messages, &session_id, iteration, total_tool_calls, None)
                    .await;
            }

            iteration += 1;
            debug!(iteration, "Agent iteration");

            // ── Graceful interrupt check (Ctrl-C) ──
            // If the interrupt flag has been triggered (e.g. by a Ctrl-C
            // signal handler in the TUI/CLI), exit the loop cleanly instead
            // of starting another LLM round-trip + tool execution cycle.
            if self.interrupt_flag.is_triggered() {
                // ── Turn diagnostics (interrupt exit) ────────────────────
                let diag = TurnDiagnostics {
                    exit_reason: TurnExitReason::Interrupted,
                    model: self.model(),
                    api_calls: iteration,
                    max_iterations: self.config.max_iterations,
                    budget_used: self.iteration_budget.used(),
                    budget_max: self.iteration_budget.max_total(),
                    tool_turns: total_tool_calls,
                    response_len: 0,
                    session_id: session_id.clone(),
                };
                warn!("{}", diag.log_message());
                if self.record_trajectories {
                    self.save_trajectory(
                        &session_id,
                        &messages,
                        iteration,
                        total_tool_calls,
                        false,
                        None,
                    )
                    .await;
                }
                self.emit(AgentEvent::Error {
                    error: "Interrupted by user".to_string(),
                })
                .await;
                let _ = message_safety::close_interrupted_tool_sequence(&mut messages, None);
                return Err(Error::Agent("Interrupted by user".to_string()));
            }

            if iteration > self.config.max_iterations {
                // ── Turn diagnostics (budget exhaustion) ─────────────────
                let diag = TurnDiagnostics {
                    exit_reason: TurnExitReason::BudgetExhausted,
                    model: self.model(),
                    api_calls: iteration,
                    max_iterations: self.config.max_iterations,
                    budget_used: self.iteration_budget.used(),
                    budget_max: self.iteration_budget.max_total(),
                    tool_turns: total_tool_calls,
                    response_len: 0,
                    session_id: session_id.clone(),
                };
                warn!("{}", diag.log_message());
                // ── Grace call (iter-57) ────────────────────────────────
                // When max_iterations is exceeded, hermes-agent makes one
                // extra "grace call" with tools stripped, asking the model
                // to summarize what it has so far. This gives the user a
                // partial answer instead of a hard error.
                return self
                    .attempt_grace_call(&messages, &session_id, iteration, total_tool_calls, None)
                    .await;
            }

            // Log iteration progress (not as a Thinking event — that pollutes
            // the TUI's thinking display with debug text. Use tracing instead.)
            // (iter-120 — user-reported bug: "Iteration 1/90: Requesting LLM
            // response..." was appearing in the thinking block.)
            tracing::debug!(
                iteration,
                max = self.config.max_iterations,
                "Requesting LLM response"
            );

            // Get tool schemas
            let tools = self
                .registry
                .get_schemas_for_request(&self.config.tool_search, self.config.context_window)
                .await;

            let request = ChatRequest::new(self.effective_model(), messages.clone())
                .with_tools(tools)
                .with_stream(self.config.stream);

            // Emit LlmRequest observer event
            if let Some(ref obs) = self.observer {
                obs.record_event(&ObserverEvent::LlmRequest {
                    provider: self.config.model.clone(),
                    model: self.model(),
                    messages_count: messages.len(),
                });
            }

            let llm_start = std::time::Instant::now();
            let mut stream_extra_content = None;
            let response = if request.stream {
                // The run loop's own per-request budget (request_timeout,
                // raised to the R2 reasoning floor) — the client's transport
                // timeout is the wire-level guard, this is the loop ceiling.
                let mut stream = match self
                    .call_with_loop_timeout(self.client.chat_streaming(request))
                    .await
                {
                    Ok(s) => s,
                    Err(e) => {
                        // T2: bail out on interrupt instead of classifying.
                        if self.interrupt_flag.is_triggered() {
                            return Err(e);
                        }
                        // ── Context overflow auto-compression (iter-63) ───────
                        // When the provider returns a context_overflow error,
                        // compress the conversation using context_management
                        // and retry once. This prevents hard failures on long
                        // sessions that exceed the context window.
                        let classified = FallbackModelClient::classify_error(&e);
                        self.emit_rate_limit_notice(&classified, &e).await;
                        if classified.should_compress && !retry_state.compress_attempted {
                            retry_state.compress_attempted = true;
                            warn!(reason = %classified.reason, "Context overflow detected — compressing and retrying");
                            // Try LLM summarization first (intelligent compression),
                            // fall back to deterministic decay/eviction.
                            messages = self.compress_context_overflow(messages).await;
                            // Refund the iteration since the original LLM call
                            // was wasted on a context overflow — the retry gets
                            // a fresh consume() on the next loop iteration.
                            self.iteration_budget.refund();
                            retry_state.consume_retry();
                            // Rebuild request with compressed messages
                            let tools = self
                                .registry
                                .get_schemas_for_request(
                                    &self.config.tool_search,
                                    self.config.context_window,
                                )
                                .await;
                            let retry_request =
                                ChatRequest::new(self.effective_model(), messages.clone())
                                    .with_tools(tools)
                                    .with_stream(self.config.stream);
                            self.client.chat_streaming(retry_request).await?
                        } else if classified.should_rotate_credential
                            && !retry_state.rotate_attempted
                        {
                            // Credential rotation: invalidate current key,
                            // select next from pool, update client, and retry.
                            retry_state.rotate_attempted = true;
                            warn!(
                                reason = %classified.reason,
                                retry = retry_state.retry_count,
                                max = retry_state.max_retries,
                                "Auth/rate-limit error — rotating credential and retrying"
                            );
                            if self.try_rotate_credential().is_some() {
                                self.iteration_budget.refund();
                                retry_state.consume_retry();
                                let tools = self
                                    .registry
                                    .get_schemas_for_request(
                                        &self.config.tool_search,
                                        self.config.context_window,
                                    )
                                    .await;
                                let retry_request =
                                    ChatRequest::new(self.effective_model(), messages.clone())
                                        .with_tools(tools)
                                        .with_stream(self.config.stream);
                                self.client.chat_streaming(retry_request).await?
                            } else {
                                warn!("No more credentials to rotate — returning original error");
                                return Err(e);
                            }
                        } else {
                            return Err(e);
                        }
                    }
                };
                // ── Mid-stream drop recovery (hermes parity) ─────────────
                // Providers that close the SSE connection before the full
                // body arrives surface a transport error (reqwest's "error
                // decoding response body") from the stream. hermes-agent
                // explicitly retries these drops (_log_stream_retry /
                // _emit_stream_drop) instead of aborting the turn; we mirror
                // that here with the turn's existing retry budget.
                //
                // Rotate-classified mid-stream errors (429/401 chunks) are
                // retried too: the pooled client has already benched the
                // failed key via its stream wrapper, so the re-issued
                // request rotates to the next available key — rotation
                // fires on the retry (hermes wraps the whole stream
                // lifecycle with mark_exhausted_and_rotate, not just
                // connection establishment).
                let processed = loop {
                    // T2: the whole stream consumption runs under the loop
                    // budget ceiling (with the R2 reasoning floor) and the
                    // interrupt flag, so a Ctrl-C mid-stream aborts the
                    // request immediately instead of waiting for the turn
                    // to finish.
                    match self
                        .call_with_loop_timeout(self.process_stream(stream))
                        .await
                    {
                        Ok(processed) => break processed,
                        Err(e) => {
                            // T2: never classify/retry an interrupt-aborted
                            // request — propagate the interrupted error up.
                            if self.interrupt_flag.is_triggered() {
                                return Err(e);
                            }
                            let classified = FallbackModelClient::classify_error(&e);
                            let retryable = classified.retryable
                                && !classified.should_compress
                                && (classified.should_rotate_credential
                                    || !retry_state.rotate_attempted);
                            if retryable && retry_state.consume_retry() {
                                self.iteration_budget.refund();
                                // Aggregation hook: bump the shared retry
                                // counters so the TUI status pill can show
                                // stream-drop activity live. (The warn! below
                                // is the log side of the same event.)
                                self.metrics.record_stream_drop();
                                self.metrics.record_stream_retry();
                                warn!(
                                    error = %e,
                                    retry = retry_state.retry_count,
                                    max = retry_state.max_retries,
                                    "Stream dropped mid-read — re-issuing LLM request"
                                );
                                let tools = self
                                    .registry
                                    .get_schemas_for_request(
                                        &self.config.tool_search,
                                        self.config.context_window,
                                    )
                                    .await;
                                let retry_request =
                                    ChatRequest::new(self.effective_model(), messages.clone())
                                        .with_tools(tools)
                                        .with_stream(self.config.stream);
                                stream = self
                                    .call_with_loop_timeout(
                                        self.client.chat_streaming(retry_request),
                                    )
                                    .await?;
                            } else {
                                return Err(self.annotate_thinking_timeout(e));
                            }
                        }
                    }
                };
                let (text, reasoning, tcs, extra, finish_reason) = processed;
                stream_extra_content = extra;
                Ok((text, reasoning, tcs, finish_reason))
            } else {
                let response = match self.call_with_loop_timeout(self.client.chat(request)).await {
                    Ok(r) => r,
                    Err(e) => {
                        // T2: bail out on interrupt instead of classifying.
                        if self.interrupt_flag.is_triggered() {
                            return Err(e);
                        }
                        let classified = FallbackModelClient::classify_error(&e);
                        self.emit_rate_limit_notice(&classified, &e).await;
                        if classified.should_compress && !retry_state.compress_attempted {
                            retry_state.compress_attempted = true;
                            warn!(reason = %classified.reason, "Context overflow detected — compressing and retrying");
                            // Try LLM summarization first (intelligent compression),
                            // fall back to deterministic decay/eviction.
                            messages = self.compress_context_overflow(messages).await;
                            // Refund the iteration since the original LLM call
                            // was wasted on a context overflow.
                            self.iteration_budget.refund();
                            retry_state.consume_retry();
                            let tools = self
                                .registry
                                .get_schemas_for_request(
                                    &self.config.tool_search,
                                    self.config.context_window,
                                )
                                .await;
                            let retry_request =
                                ChatRequest::new(self.effective_model(), messages.clone())
                                    .with_tools(tools)
                                    .with_stream(self.config.stream);
                            self.call_with_loop_timeout(self.client.chat(retry_request))
                                .await?
                        } else if classified.should_rotate_credential
                            && !retry_state.rotate_attempted
                        {
                            // Credential rotation: same as streaming path.
                            retry_state.rotate_attempted = true;
                            warn!(
                                reason = %classified.reason,
                                retry = retry_state.retry_count,
                                max = retry_state.max_retries,
                                "Auth/rate-limit error — rotating credential and retrying"
                            );
                            if self.try_rotate_credential().is_some() {
                                self.iteration_budget.refund();
                                retry_state.consume_retry();
                                let tools = self
                                    .registry
                                    .get_schemas_for_request(
                                        &self.config.tool_search,
                                        self.config.context_window,
                                    )
                                    .await;
                                let retry_request =
                                    ChatRequest::new(self.effective_model(), messages.clone())
                                        .with_tools(tools)
                                        .with_stream(self.config.stream);
                                self.call_with_loop_timeout(self.client.chat(retry_request))
                                    .await?
                            } else {
                                warn!("No more credentials to rotate — returning original error");
                                return Err(self.annotate_thinking_timeout(e));
                            }
                        } else {
                            return Err(self.annotate_thinking_timeout(e));
                        }
                    }
                };
                self.process_response(response).await
            };

            // Emit LlmResponse observer event with timing
            let llm_duration = llm_start.elapsed();
            if let Some(ref obs) = self.observer {
                obs.record_event(&ObserverEvent::LlmResponse {
                    provider: self.config.model.clone(),
                    model: self.model(),
                    duration: llm_duration,
                    success: response.is_ok(),
                    error_message: response.as_ref().err().map(|e| e.to_string()),
                    input_tokens: None,
                    output_tokens: None,
                });
                obs.record_metric(&ObserverMetric::RequestLatency(llm_duration));
            }

            // Collect tool names before the match so they're accessible
            // in the self-evolution check after the match block.
            #[allow(unused_assignments)]
            let mut tool_names: Vec<String> = Vec::new();

            match response {
                Ok((response_text, reasoning_text, tool_calls, finish_reason)) => {
                    // Reset retry state on successful LLM response.
                    retry_state.reset_on_success();

                    // ── Truncation continuation (T1 — hermes parity) ──────
                    // When the provider reports a cut-off response
                    // (finish_reason="length", or a suspicious stop on
                    // Ollama-GLM), don't surface the partial answer as
                    // final: append a continuation prompt and re-loop,
                    // bounded by MAX_LENGTH_CONTINUE_RETRIES (hermes uses
                    // the same cap) and the iteration budget.
                    if tool_calls.is_empty()
                        && length_continue_retries < MAX_LENGTH_CONTINUE_RETRIES
                    {
                        let truncated = finish_reason.as_deref() == Some("length")
                            || turn_end_heuristics::should_treat_stop_as_truncated(
                                &self.config.model,
                                finish_reason.as_deref(),
                                &response_text,
                                messages.iter().any(|m| m.role == Role::Tool),
                                false,
                            );
                        if truncated {
                            // Thinking-exhausted: the model burned the whole
                            // output budget on reasoning with nothing visible
                            // left — continuation retries are pointless, give
                            // a targeted error (hermes conversation_loop.py
                            // thinking-exhausted detection).
                            if turn_end_heuristics::thinking_exhausted(&response_text) {
                                return Err(Error::Agent(
                                    "Model used all output tokens on reasoning with none left \
                                     for the response. Try lowering reasoning effort or \
                                     increasing max_tokens."
                                        .to_string(),
                                ));
                            }
                            length_continue_retries += 1;
                            self.metrics.record_truncation_continuation();
                            warn!(
                                finish_reason = ?finish_reason,
                                "Response truncated — requesting continuation ({}/{})",
                                length_continue_retries,
                                MAX_LENGTH_CONTINUE_RETRIES
                            );
                            self.emit(AgentEvent::Content {
                                text: format!(
                                    "↻ Response truncated — requesting continuation ({}/{})",
                                    length_continue_retries, MAX_LENGTH_CONTINUE_RETRIES
                                ),
                            })
                            .await;
                            let continue_msg =
                                Message::user(turn_end_heuristics::continuation_prompt());
                            messages.push(continue_msg.clone());
                            self.add_message(continue_msg).await;
                            // Refund the consumed iteration — the LLM call
                            // was wasted on a truncated turn; the continuation
                            // is the same logical turn.
                            self.iteration_budget.refund();
                            continue;
                        }
                    }

                    // ── Empty-content recovery (hermes parity) ─────────────
                    // If the model produced no visible text, no reasoning, and
                    // no tool calls, it has emitted an empty turn (free-tier
                    // providers do this intermittently). Rather than surfacing
                    // an empty reply as the final answer, retry up to
                    // max_retries times with the empty assistant turn appended
                    // to the conversation, exactly like hermes-agent's
                    // conversation_loop.py empty-retry loop and
                    // hermes-agent-ultra's methods_run_stream.rs inner_empty
                    // loop.
                    let has_visible_text = !response_text.trim().is_empty();
                    let has_reasoning = !reasoning_text.trim().is_empty();
                    if tool_calls.is_empty()
                        && !has_visible_text
                        && !has_reasoning
                        && empty_content_retries < self.config.max_retries
                    {
                        empty_content_retries += 1;
                        self.metrics.record_empty_content_retry();
                        warn!(
                            "Empty assistant response — retrying ({}/{})",
                            empty_content_retries, self.config.max_retries
                        );
                        self.emit(AgentEvent::Content {
                            text: format!(
                                "Empty assistant response — retrying ({}/{})",
                                empty_content_retries, self.config.max_retries
                            ),
                        })
                        .await;
                        // Append the empty assistant turn so the model sees its
                        // own empty reply and is nudged to actually respond.
                        messages.push(Message::assistant(""));
                        self.add_message(Message::assistant("")).await;
                        // Refund the consumed iteration — the LLM call was
                        // wasted on an empty turn.
                        self.iteration_budget.refund();
                        continue;
                    }
                    // Add assistant message to conversation
                    // When tool calls are present, any text before them is typically
                    // model thinking/planning that shouldn't be shown to the user.
                    let effective_text = if !tool_calls.is_empty() {
                        String::new()
                    } else {
                        response_text.clone()
                    };
                    let mut assistant_msg = Message::assistant(&effective_text);
                    if !reasoning_text.is_empty() {
                        assistant_msg = assistant_msg.with_reasoning(reasoning_text);
                    }
                    if !tool_calls.is_empty() {
                        assistant_msg = assistant_msg.with_tool_calls(tool_calls.clone());
                    }
                    // Attach provider-specific extra content (e.g. Gemini thought_signature)
                    if let Some(ref extra) = stream_extra_content
                        && !extra.is_null()
                    {
                        assistant_msg = assistant_msg.with_extra_content(extra.clone());
                    }

                    messages.push(assistant_msg.clone());
                    self.add_message(assistant_msg.clone()).await;

                    // Persist assistant message — use save_message_full when
                    // the message has tool_calls so they're not lost on reload.
                    // Previously save_message (4-arg) dropped tool_calls, which
                    // meant reloaded sessions lost the assistant's tool-call
                    // context (the tool results became orphaned).
                    let timestamp = chrono::Utc::now().to_rfc3339();
                    if assistant_msg.tool_calls.is_some() {
                        let tool_calls_json = assistant_msg
                            .tool_calls
                            .as_ref()
                            .and_then(|tcs| serde_json::to_string(tcs).ok());
                        let msg_data = crate::database::MessageData {
                            id: 0,
                            session_id: session_id.clone(),
                            role: "assistant".to_string(),
                            content: Some(effective_text.clone()),
                            tool_call_id: None,
                            tool_calls: tool_calls_json,
                            tool_name: None,
                            timestamp,
                            token_count: None,
                            // T1: persist the provider's real finish reason
                            // (previously hardcoded to "tool_calls").
                            finish_reason: finish_reason.clone(),
                            reasoning: assistant_msg.reasoning.clone(),
                            reasoning_content: None,
                            reasoning_details: None,
                            codex_reasoning_items: None,
                            codex_message_items: None,
                            platform_message_id: None,
                            observed: None,
                            active: 1,
                        };
                        if let Err(e) = self.database.save_message_full(&msg_data) {
                            tracing::warn!(error = %e, "failed to persist assistant message");
                        }
                    } else {
                        if let Err(e) = self.database.save_message(
                            &session_id,
                            "assistant",
                            &effective_text,
                            &timestamp,
                        ) {
                            tracing::warn!(error = %e, "failed to persist assistant message");
                        }
                    }
                    self.database
                        .save_session(
                            &session_id,
                            None,
                            "agent",
                            &chrono::Utc::now().to_rfc3339(),
                            &chrono::Utc::now().to_rfc3339(),
                        )
                        .ok();
                    if let Ok(total) = self.session_cost_usd.read() {
                        self.database.update_session_cost(&session_id, *total).ok();
                    }

                    // If no tool calls, we're done
                    if tool_calls.is_empty() {
                        let result = assistant_msg.clone();
                        self.spawn_session_distillation(messages.clone());

                        // Save trajectory if recording is enabled.
                        if self.record_trajectories {
                            self.save_trajectory(
                                &session_id,
                                &messages,
                                iteration,
                                total_tool_calls,
                                true,
                                Some(&result),
                            )
                            .await;
                        }

                        // ── Turn diagnostics (final response) ───────────────────
                        // Log structured diagnostics at turn completion, matching
                        // hermes-agent's turn-exit diagnostic log pattern.
                        {
                            let diag = TurnDiagnostics {
                                exit_reason: TurnExitReason::TextResponse,
                                model: self.model(),
                                api_calls: iteration,
                                max_iterations: self.config.max_iterations,
                                budget_used: self.iteration_budget.used(),
                                budget_max: self.iteration_budget.max_total(),
                                tool_turns: total_tool_calls,
                                response_len: result.content.len(),
                                session_id: session_id.clone(),
                            };
                            info!("{}", diag.log_message());
                        }

                        // ── TurnEnd lifecycle hook ───────────────────────────────
                        // Emit TurnEnd with iteration and tool call counts.
                        if let Some(ref hooks) = self.hook_registry {
                            let ctx = crate::gateway_pipeline::HookContext::new()
                                .with_session(&session_id)
                                .with_metadata("iterations", iteration.to_string())
                                .with_metadata("tool_calls", total_tool_calls.to_string());
                            hooks
                                .emit(crate::gateway_pipeline::HookEvent::TurnEnd, ctx)
                                .await;
                        }

                        self.emit(AgentEvent::Done {
                            message: assistant_msg,
                        })
                        .await;

                        // R6 — durable session activity heartbeat (hermes
                        // session_activity.py parity): stamp the session as
                        // active so gateway/session liveness views see work
                        // even when the session never sends a message.
                        self.touch_session_activity(&session_id, "turn complete")
                            .await;

                        // Memory provider: sync_turn + queue_prefetch hooks.
                        // sync_turn persists the completed turn to graph memory
                        // (entity extraction + auto-wiring). queue_prefetch
                        // queues background recall for the next turn.
                        // This is the native equivalent of the hermes-agent
                        // MemoryManager.sync_all() + queue_prefetch_all() pattern.
                        //
                        // Uses the MemorySyncExecutor for ordered, non-blocking
                        // background writes. Falls back to direct spawn when the
                        // executor isn't available (e.g. no memory provider).
                        if let Ok(exec_guard) = self.memory_sync_executor.try_lock() {
                            if let Some(executor) = exec_guard.as_ref() {
                                executor.submit_sync_turn(&user_query, &result.content);
                            } else if let Some(provider) = &self.memory_provider {
                                let user_text = user_query.clone();
                                let assistant_text = result.content.clone();
                                let provider_clone = provider.clone();
                                tokio::spawn(async move {
                                    if let Err(e) =
                                        provider_clone.sync_turn(&user_text, &assistant_text).await
                                    {
                                        tracing::warn!(error = %e, "Memory provider sync_turn hook failed");
                                    }
                                });
                            }
                        }
                        // Memory provider: queue background recall for the
                        // NEXT turn (hermes `queue_prefetch_all` call-site
                        // parity). The authoritative search runs in prefetch()
                        // at the top of the next turn; queue_prefetch is a
                        // non-blocking hook the provider can use to warm its
                        // backend — a slow provider can never block the
                        // turn-completion path.
                        if let Some(provider) = &self.memory_provider {
                            provider.queue_prefetch(&user_query);
                        }

                        // Emit AgentEnd hook
                        if let Some(ref hooks) = self.hook_registry {
                            hooks
                                .emit(
                                    crate::gateway_pipeline::HookEvent::AgentEnd,
                                    crate::gateway_pipeline::HookContext::new()
                                        .with_session(&session_id),
                                )
                                .await;
                        }

                        // Emit AgentEnd observer event (R5 turn-summary feed —
                        // the observer prints the per-turn accounting line;
                        // the grace/budget-exhausted path already emits this).
                        if let Some(ref obs) = self.observer {
                            let cost = self.session_cost_usd.read().map(|c| *c).unwrap_or(0.0);
                            obs.record_event(&ObserverEvent::AgentEnd {
                                provider: self.config.model.clone(),
                                model: self.model(),
                                duration: turn_start.elapsed(),
                                tokens_used: None,
                                cost_usd: if cost > 0.0 { Some(cost) } else { None },
                            });
                        }

                        // ── Eager LCM ingest (hermes context_engine parity) ──
                        // Commit the COMPLETED turn into the lossless DAG NOW
                        // (not only at the next build_messages), so the final
                        // assistant response is immediately recallable by the
                        // following turn via lcm_recall. Idempotent by
                        // (session, position, content_hash) — safe to run.
                        if let Some(engine) = &self.context_engine {
                            // Borrow — `messages` is not used after this point.
                            if let Err(e) = engine.ingest_turn(&session_id, &messages).await {
                                tracing::warn!(
                                    error = %e,
                                    "LCM eager turn ingest failed (non-fatal)"
                                );
                            }
                        }

                        return Ok(result);
                    }

                    total_tool_calls += tool_calls.len();

                    // Collect tool names before execute_tools() consumes
                    // the Vec, so the self-evolution check can detect
                    // skill_manage calls without holding a reference to the
                    // moved tool_calls.
                    tool_names = tool_calls
                        .iter()
                        .map(|tc| tc.function.name.clone())
                        .collect();

                    // Build a lookup map from tool_call_id → arguments so
                    // we can extract file paths / task descriptions when
                    // mirroring memory writes and delegation results.
                    let call_args: std::collections::HashMap<String, String> = tool_calls
                        .iter()
                        .map(|tc| (tc.id.clone(), tc.function.arguments.clone()))
                        .collect();

                    // ── Progressive LCM ingest (hermes context_engine parity) ──
                    // Commit the accumulated conversation (including the
                    // assistant message just pushed above) into the lossless
                    // DAG BEFORE executing tools, so a same-iteration tool
                    // call like `lcm_recall` can find statements the model
                    // just made. Idempotent by (session, position,
                    // content_hash) — safe to run every iteration.
                    if let Some(engine) = &self.context_engine
                        && let Err(e) = engine.ingest_turn(&session_id, &messages).await
                    {
                        tracing::warn!(
                            error = %e,
                            "LCM progressive ingest failed (non-fatal)"
                        );
                    }

                    // Execute tools and add results
                    let tool_results = self.execute_tools(tool_calls).await?;

                    // Add tool results to messages and persist them (truncated)
                    for result in tool_results {
                        // Secret redaction (hermes `redact.py` parity): tool
                        // output can carry env assignments, API keys, JWT
                        // tokens, connection strings, etc. from terminal
                        // output or file reads. Redact before the text is
                        // pushed to the LLM-bound message list, persisted to
                        // the session DB, or written to the trajectory.
                        let content = if result.success {
                            crate::redaction::redact_sensitive_text_if_enabled(
                                &truncate_tool_result(&result.name, &result.content),
                            )
                        } else {
                            crate::redaction::redact_sensitive_text_if_enabled(
                                result.error.as_deref().unwrap_or("Error"),
                            )
                        };

                        // ── Memory write mirroring (hermes parity) ────────
                        // When a built-in memory tool writes an entry
                        // (write_file to MEMORY.md/USER.md, patch, create_file),
                        // mirror the write to the memory provider so the graph
                        // stays in sync. Ported from hermes-agent's
                        // MemoryManager.notify_memory_tool_write() pattern.
                        // Only fires for memory-related file paths, not all writes.
                        if result.success
                            && (result.name == "write_file"
                                || result.name == "patch"
                                || result.name == "create_file")
                            && let Some(args_str) = call_args.get(&result.tool_call_id)
                            && let Ok(args_val) =
                                serde_json::from_str::<serde_json::Value>(args_str)
                        {
                            let path = args_val
                                .get("path")
                                .or_else(|| args_val.get("file_path"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            // Only mirror writes to memory-related files
                            let is_memory = path.ends_with("MEMORY.md")
                                || path.ends_with("USER.md")
                                || path.contains("/MEMORY.")
                                || path.contains("/USER.");
                            if is_memory {
                                self.notify_memory_write(&result.name, path, &result.content);
                            }
                        }

                        // ── Delegation observation (hermes parity) ────────
                        // When a subagent tool completes (delegate_task,
                        // spawn_subagent), notify the memory provider so the
                        // parent's graph captures delegated work. Ported from
                        // hermes-agent's MemoryManager.on_delegation() pattern.
                        if result.success
                            && (result.name == "delegate_task" || result.name == "spawn_subagent")
                        {
                            let task_desc = call_args
                                .get(&result.tool_call_id)
                                .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
                                .and_then(|v| {
                                    v.get("task")
                                        .or_else(|| v.get("prompt"))
                                        .and_then(|t| t.as_str())
                                        .map(String::from)
                                })
                                .unwrap_or_else(|| result.name.clone());
                            self.notify_delegation(&task_desc, &result.content);
                        }

                        // Persist tool result (truncated)
                        if let Err(e) = self.database.save_message(
                            &session_id,
                            "tool",
                            &content,
                            &chrono::Utc::now().to_rfc3339(),
                        ) {
                            tracing::warn!(error = %e, "failed to persist tool result");
                        }
                        self.database
                            .save_session(
                                &session_id,
                                None,
                                "agent",
                                &chrono::Utc::now().to_rfc3339(),
                                &chrono::Utc::now().to_rfc3339(),
                            )
                            .ok();
                        if let Ok(total) = self.session_cost_usd.read() {
                            self.database.update_session_cost(&session_id, *total).ok();
                        }
                        // Emit ToolCall observer event
                        if let Some(ref obs) = self.observer {
                            obs.record_event(&ObserverEvent::ToolCall {
                                tool: result.name.clone(),
                                duration: Duration::from_millis(0),
                                success: result.success,
                            });
                        }

                        if result.success {
                            self.emit(AgentEvent::ToolComplete {
                                result: result.clone(),
                            })
                            .await;
                        } else {
                            self.emit(AgentEvent::ToolError {
                                tool_call_id: result.tool_call_id.clone(),
                                name: result.name.clone(),
                                error: result.error.clone().unwrap_or_default(),
                            })
                            .await;
                        }

                        messages.push(Message::tool(&result.tool_call_id, &content));
                        self.add_message(Message::tool(&result.tool_call_id, &content))
                            .await;
                    }

                    // ── File mutation advisory footer ──────────────────────────
                    // After all tool results are processed, scan for failed file
                    // mutations (write_file, patch, create_file) and log an advisory.
                    // The footer is logged for observability — the model will see the
                    // tool results with error messages on the next iteration anyway.
                    // Matches hermes-agent's _format_file_mutation_failure_footer pattern.
                    if let Some(footer) = file_mutation_verifier_footer(&messages) {
                        tracing::warn!(footer = %footer, "File mutation advisory");
                    }
                }
                Err(e) => {
                    // ── Turn diagnostics (error exit) ─────────────────────
                    {
                        let diag = TurnDiagnostics {
                            exit_reason: TurnExitReason::Error,
                            model: self.model(),
                            api_calls: iteration,
                            max_iterations: self.config.max_iterations,
                            budget_used: self.iteration_budget.used(),
                            budget_max: self.iteration_budget.max_total(),
                            tool_turns: total_tool_calls,
                            response_len: 0,
                            session_id: session_id.clone(),
                        };
                        warn!("{}", diag.log_message());
                    }
                    error!(error = %e, "Error processing stream");
                    self.emit(AgentEvent::Error {
                        error: e.user_message(),
                    })
                    .await;
                    if self.record_trajectories {
                        self.save_trajectory(
                            &session_id,
                            &messages,
                            iteration,
                            total_tool_calls,
                            false,
                            None,
                        )
                        .await;
                    }
                    // Emit AgentEnd observer event on error
                    if let Some(ref obs) = self.observer {
                        let cost = self.session_cost_usd.read().map(|c| *c).unwrap_or(0.0);
                        obs.record_event(&ObserverEvent::AgentEnd {
                            provider: self.config.model.clone(),
                            model: self.model(),
                            duration: llm_duration,
                            tokens_used: None,
                            cost_usd: if cost > 0.0 { Some(cost) } else { None },
                        });
                    }
                    return Err(e);
                }
            }

            self.emit(AgentEvent::IterationComplete { iteration }).await;

            // Emit TurnComplete observer event
            if let Some(ref obs) = self.observer {
                obs.record_event(&ObserverEvent::TurnComplete);
            }

            // ── Self-evolution: skill nudge (per-iteration cadence) ──
            // After each completed iteration, bump the skill counter and check
            // if a skill-review should fire. Mirrors hermes-agent's
            // turn_finalizer.py logic where _iters_since_skill is checked after
            // the tool-calling loop — bumped per *iteration*, NOT per turn.
            // (Memory review is on a separate per-turn cadence handled at the
            // turn boundary above.)
            //
            // When skill_manage is called, the skill counter resets immediately
            // so the nudge window restarts from zero.
            //
            // Scope the MutexGuard so it's dropped before the .await below.
            // A std::sync::MutexGuard held across an await point makes the
            // future !Send, which breaks tokio::spawn.
            let should_review_skills = {
                let skill_manage_called = tool_names.iter().any(|n| n == "skill_manage");
                let mut evo = self
                    .evolution_state
                    .lock()
                    .expect("evolution_state mutex poisoned — programmer error");

                let trigger = turn_finalizer::advance_skill_trigger(&mut evo, skill_manage_called);

                if trigger.should_review_skills {
                    info!(
                        iters = trigger.iters_since_skill,
                        interval = self.config.skill_nudge_interval,
                        "Skill nudge triggered — spawning background review"
                    );
                }

                // ── Persist evolution counters to session metadata ──
                // After bumping, persist so the next run() can hydrate.
                if self.persistent_session_id.is_some() {
                    for (key, val) in evo.persist_counters() {
                        let _ = self.database.set_session_metadata(&session_id, key, &val);
                    }
                }

                trigger.should_review_skills
            }; // MutexGuard dropped here — safe to .await
            if should_review_skills {
                self.spawn_background_review(&messages, &session_id, true, false)
                    .await;
            }

            // ── /steer directive drain (iter-65) ──────────────────────────
            // Between iterations, check if the user queued any steer
            // directives. If so, inject them as a user-role message so
            // the model sees the real-time guidance on the next iteration.
            // This mirrors hermes-agent's /steer drain which injects into
            // the last tool-role message to preserve role alternation.
            if let Some(steer_text) = self.drain_steers().await {
                info!(steer = %steer_text, "Injecting steer directive");
                let steer_msg = Message::user(format!(
                    "[STEER] {}\n\nPlease adjust your approach based on this guidance.",
                    steer_text
                ));
                messages.push(steer_msg.clone());
                self.add_message(steer_msg).await;
            }
        }
    }

    /// Build messages including system prompt.
    ///
    /// Applies context management (decay + eviction) to fit within the
    /// context window budget. When the estimated token count exceeds
    /// 80% of the budget, aggressive preflight compression fires to
    /// prevent wasted LLM calls that would fail with
    /// context_length_exceeded.
    ///
    /// Ported from hermes-agent's `turn_context.py` preflight compression
    /// pattern: estimate → check threshold → compress → fit within budget.
    async fn build_messages(&self, session_id: &str) -> Result<Vec<Message>> {
        let mut messages = Vec::new();

        // ── Prompt-cache stability (iter-39) ─────────────────────────────
        // Split the system prompt into TWO messages:
        //   1. FROZEN PREFIX: base system prompt + skills. These rarely
        //      change across turns, so keeping them byte-stable lets
        //      Anthropic's prompt cache hit (cache reads cost ~10x less
        //      than fresh prompt tokens).
        //   2. VOLATILE SUFFIX: memory context + workspace context. These
        //      change each turn (memory grows, workspace files change),
        //      so they go in a separate message AFTER the frozen prefix.
        //      The frozen prefix stays cache-stable; only the volatile
        //      suffix + conversation history are re-processed each turn.
        //
        // This is a simplified version of magic-context's m[0]/m[1]
        // cache layout. The full m[0]/m[1] scheme uses HARD/SOFT/SOFT+
        // pass taxonomy + byte-identical replay; this implementation
        // just splits into frozen vs volatile, which captures ~80% of
        // the cache benefit with ~10% of the complexity.

        // Build the frozen prefix (base system prompt + skills + memory
        // provider status line). Uses the shared helper to avoid
        // duplicating the prefix logic with spawn_background_review's
        // cache parity path.
        let mut frozen_prefix = self.build_frozen_prefix();
        if let Some(provider) = &self.memory_provider {
            let block = provider.system_prompt_block().await;
            if !block.trim().is_empty() {
                frozen_prefix.push_str("\n\n");
                frozen_prefix.push_str(block.trim());
            }
        }
        messages.push(Message::system(frozen_prefix));

        // Volatile suffix: memory context + workspace context. These
        // change each turn, so they're a separate message that doesn't
        // bust the frozen prefix's cache entry.
        let mut volatile_suffix = String::new();
        if let Some(memory_manager) = &self.memory_manager {
            let memory_context = memory_manager.build_memory_context(2048).await;
            let memory_context = memory_context.trim();
            if !memory_context.is_empty() {
                volatile_suffix.push_str("\n\n<long_term_memory>\n");
                volatile_suffix.push_str(memory_context);
                volatile_suffix.push_str("\n</long_term_memory>");
            }
        }

        // Memory provider: per-turn semantic recall (prefetch).
        // Runs with an 8s timeout — matches hermes-agent's prefetch
        // timeout pattern. Results land under <memory_context> tags
        // (distinct from the file-backed <long_term_memory> block).
        if let Some(provider) = &self.memory_provider {
            let last_user = {
                let conv = self.conversation.read().await;
                conv.iter()
                    .rev()
                    .find(|m| m.role == Role::User)
                    .map(|m| m.content.clone())
            };
            if let Some(query) = last_user {
                let provider_context =
                    tokio::time::timeout(Duration::from_secs(8), provider.prefetch(&query))
                        .await
                        .unwrap_or_default();
                let provider_context = provider_context.trim();
                if !provider_context.is_empty() {
                    volatile_suffix.push_str("\n\n<memory_context>\n");
                    volatile_suffix.push_str(provider_context);
                    volatile_suffix.push_str("\n</memory_context>");
                }
            }
        }

        let context_files = self.load_context_file_prompt();
        if !context_files.trim().is_empty() {
            volatile_suffix.push_str("\n\n<workspace_context>\n");
            volatile_suffix.push_str(context_files.trim());
            volatile_suffix.push_str("\n</workspace_context>");
        }

        if !volatile_suffix.trim().is_empty() {
            messages.push(Message::system(volatile_suffix.trim().to_string()));
        }

        // ── MoA guidance injection (G5, hermes moa_loop.py parity) ─────
        // Per-turn Mixture-of-Agents guidance computed before run(); injected
        // as a system message after the volatile suffix so the acting loop
        // sees it for every iteration of this turn. Drained — it never leaks
        // into the next turn (a plain turn with no MoA is byte-identical to
        // before, preserving prompt-cache stability).
        if let Some(guidance) = self.drain_moa_guidance() {
            messages.push(Message::system(guidance));
        }

        // Add conversation history
        let conv = self.conversation.read().await;
        messages.extend(conv.clone());
        drop(conv);

        // Apply context management: decay-render old messages + evict
        // if over budget. Without this, any long-running session would
        // eventually exceed the context window and 400-error.
        //
        // Preflight compression (proactive): estimate tokens before the
        // LLM call. If the estimated count exceeds 80% of the context
        // window, apply aggressive decay to compress older messages.
        // This prevents wasted LLM calls that would fail with
        // context_length_exceeded. Ported from hermes-agent's
        // turn_context.py preflight compression pattern.
        let budget = self.config.context_window;
        let reserve = 4096; // tokens reserved for the model's response
        let effective_budget = budget.saturating_sub(reserve);

        let estimated_tokens = self.estimate_current_tokens(&messages);
        let preflight_threshold = budget * PREFLIGHT_THRESHOLD_PERCENT as usize / 100;
        if estimated_tokens > preflight_threshold {
            info!(
                estimated = estimated_tokens,
                threshold = preflight_threshold,
                budget,
                "Preflight compression: estimated tokens exceed threshold"
            );
            // Memory provider: on_pre_compress hook.
            // Extract insights from messages about to be compressed and
            // prepend them as a user context block so the downstream
            // compression/decay preserves what the memory provider still
            // considers important. Mirrors hermes-agent's plugin behavior
            // (insert `[agentmemory context before compaction]` at index 0).
            if let Some(provider) = &self.memory_provider {
                let insights = provider.on_pre_compress(&messages);
                if !insights.is_empty() {
                    tracing::debug!(
                        insights_len = insights.len(),
                        "Memory provider pre-compress insights captured"
                    );
                    messages.insert(
                        0,
                        crate::client::Message::user(format!(
                            "[memory context before compaction]\n{insights}"
                        )),
                    );
                }
            }
            messages = crate::context_management::decay_render(
                messages,
                PREFLIGHT_DECAY_H50,
                PREFLIGHT_DECAY_CONSTANT,
            );
        }

        // Context engine hook (hermes-lcm parity): when a lossless engine is
        // attached, it assembles the final list (D0 fresh tail kept verbatim,
        // older context compacted into the DAG and recallable) INSTEAD of the
        // lossy eviction below.
        //
        // The session key is the one resolved by turn_context for this run
        // (NOT a `"default"` fallback) so DAG ingestion uses the SAME key as
        // the loop's progressive/eager ingest — otherwise the same turn is
        // stored twice under two session keys (wasted storage + scoped recall
        // misses).
        if let Some(engine) = &self.context_engine {
            // `assemble` consumes `messages`; on failure fall back to lossy
            // eviction over the raw conversation history (rare error path).
            match engine
                .assemble(session_id, messages, effective_budget)
                .await
            {
                Ok(assembled) => messages = assembled,
                Err(e) => {
                    warn!(error = %e, engine = engine.name(),
                          "context engine assemble failed — falling back to lossy eviction");
                    let history = self.conversation.read().await;
                    messages = crate::context_management::evict_to_budget(
                        history.clone(),
                        effective_budget,
                    );
                }
            }
        } else {
            // Standard eviction: remove oldest messages within tiers until
            // the total fits within the effective budget.
            messages = crate::context_management::evict_to_budget(messages, effective_budget);
        }

        let seq_repairs = message_safety::repair_message_sequence(&mut messages);
        if seq_repairs > 0 {
            info!(
                repairs = seq_repairs,
                "Repaired message sequence violations"
            );
        }

        // Drop thinking-only assistant messages and merge consecutive user
        // messages. Needed for Anthropic models that emit reasoning as
        // separate empty-content assistant messages.
        messages = message_safety::drop_thinking_only_and_merge_users(&messages);

        // Sanitize tool calls for strict API providers (Gemini, Claude
        // strict mode) that enforce stricter name/argument validation.
        let tool_sans = message_safety::sanitize_tool_calls_for_strict_api(&mut messages);
        if tool_sans > 0 {
            debug!(
                sanitizations = tool_sans,
                "Sanitized tool calls for strict API"
            );
        }

        Ok(messages)
    }

    fn load_context_file_prompt(&self) -> String {
        let mut blocks = Vec::new();

        let global_context = load_default_context_files();
        if !global_context.trim().is_empty() {
            blocks.push(global_context);
        }

        match std::env::current_dir() {
            Ok(cwd) => {
                if let Some(workspace_context) = load_workspace_context(&cwd) {
                    blocks.push(workspace_context);
                }
            }
            Err(error) => {
                warn!(error = %error, "Could not determine current directory for context files")
            }
        }

        blocks.join("\n\n")
    }

    /// Save a trajectory (ReAct steps + messages + metadata) for this run.
    ///
    /// Writes to `~/.operant/trajectories/<session_id>-<timestamp>.json`.
    /// Each trajectory captures: session ID, model, iteration count, tool
    /// call count, success status, full message history, and per-step
    /// thought/action/observation where extractable.
    async fn save_trajectory(
        &self,
        session_id: &str,
        messages: &[Message],
        iterations: usize,
        tool_calls: usize,
        success: bool,
        _final_response: Option<&Message>,
    ) {
        use crate::trajectory::{Trajectory, TrajectoryStep};

        let mut trajectory = Trajectory::new(
            format!("{}_{}", session_id, chrono::Utc::now().timestamp()),
            session_id,
            &self.config.model,
        );
        trajectory.iterations = iterations;
        trajectory.tool_calls = tool_calls;
        trajectory.success = success;

        // Build per-step records from the message history.
        // Each assistant message with tool calls → a reasoning step.
        // Each tool result → an observation step.
        // The final assistant message (no tool calls) → a response step.
        let mut step_idx = 0usize;
        for msg in messages {
            match msg.role.as_str() {
                "assistant" => {
                    let mut step = TrajectoryStep {
                        step: step_idx,
                        thought: Some(msg.content.clone()),
                        action: None,
                        action_args: None,
                        observation: None,
                        response: None,
                        success: true,
                    };
                    if let Some(tool_calls) = msg.tool_calls.as_ref() {
                        if let Some(first) = tool_calls.first() {
                            step.action = Some(first.function.name.clone());
                            step.action_args = Some(first.function.arguments.clone());
                        }
                    } else {
                        // No tool calls → this is a response step
                        step.response = Some(msg.content.clone());
                    }
                    trajectory.add_step(step);
                    step_idx += 1;
                }
                "tool" => {
                    // Attach as observation to the last step
                    if let Some(last) = trajectory.steps.last_mut() {
                        last.observation = Some(msg.content.clone());
                    }
                }
                _ => {}
            }
            trajectory.add_message(msg.clone());
        }

        // The final_response is the last assistant message; it's already
        // captured in the messages loop above, so no extra step needed.

        trajectory.calculate_tokens();

        // Write to ~/.operant/trajectories/
        let trajectories_dir = crate::platform::operant_home().join("trajectories");
        if let Err(e) = std::fs::create_dir_all(&trajectories_dir) {
            warn!(error = %e, "Failed to create trajectories dir");
            return;
        }
        let path = trajectories_dir.join(format!("{}.json", trajectory.id));
        match trajectory.to_json() {
            Ok(json) => {
                if let Err(e) = std::fs::write(&path, json) {
                    warn!(error = %e, path = ?path, "Failed to write trajectory");
                } else {
                    info!(path = %path.display(), "Trajectory saved");
                }
            }
            Err(e) => {
                warn!(error = %e, "Failed to serialize trajectory");
            }
        }
    }

    fn spawn_session_distillation(&self, history: Vec<Message>) {
        let Some(memory_manager) = self.memory_manager.clone() else {
            return;
        };

        let client = self.client.clone();
        let model = self.config.model.clone();
        tokio::spawn(async move {
            if let Err(error) =
                distill_session_to_memory(client, model, memory_manager, history).await
            {
                warn!(error = %error, "Session distillation failed");
            }
        });
    }

    /// Spawn a background review daemon — a lightweight tokio task that
    /// replays the conversation snapshot through the LLM with a review
    /// prompt. Matches hermes-agent's `spawn_background_review_thread`
    /// pattern.
    ///
    /// The review agent:
    /// 1. Receives the conversation snapshot + a review prompt.
    /// 2. Gets tool schemas so it can call skill_manage / memory tools.
    /// 3. Writes go straight to stores; main conversation is untouched.
    /// 4. Results are logged but don't block the main loop.
    ///
    /// ## Tool Whitelist
    ///
    /// The review agent only gets memory and skill tools — never the full
    /// tool registry. This prevents the review from accidentally executing
    /// dangerous tools (terminal, file_write, etc.) and matches hermes-agent's
    /// `set_thread_tool_whitelist` pattern.
    ///
    /// ## Prompt Cache Reuse
    ///
    /// When running on the same model (not routed), the review agent shares
    /// the parent's warm cached system prompt so the outbound HTTP request
    /// hits the same Anthropic/OpenRouter prefix cache (~26% cost reduction).
    /// When routed to a different model, a compact digest replay minimizes
    /// cold-written tokens.
    ///
    /// ## Persistence Isolation
    ///
    /// The review agent does NOT write to the user's session database.
    /// All DB writes are skipped — the review only writes to memory and
    /// skill stores via its tools.
    async fn spawn_background_review(
        &self,
        messages: &[Message],
        session_id: &str,
        review_skills: bool,
        review_memory: bool,
    ) {
        use self::background_review::{build_review_prompt, digest_history};

        let prompt = build_review_prompt(review_memory, review_skills);
        let client = self.client.clone();
        let model = self.config.model.clone();
        let session_id = session_id.to_string();

        // ── Resolve auxiliary model for background review ──────────────
        // Check if the user configured an auxiliary model for background
        // reviews. If so, route the review to that model instead of the
        // main model. Different model = cold cache → use digest replay.
        let cfg = runtime_config();
        let (review_model, is_routed) = if let Some(aux) = cfg.auxiliary_models.memory.as_ref() {
            if let Some(ref aux_model) = aux.model {
                if aux_model != &model {
                    (aux_model.clone(), true)
                } else {
                    (model.clone(), false)
                }
            } else {
                (model.clone(), false)
            }
        } else {
            (model.clone(), false)
        };

        // ── Snapshot the conversation ─────────────────────────────────
        // Limit to last 40 messages to keep token usage reasonable.
        let start = messages.len().saturating_sub(40);
        let snapshot: Vec<Message> = messages[start..].to_vec();

        // ── Tool whitelist: only memory + skill tools ─────────────────
        // The review agent should ONLY have access to memory and skill
        // management tools. Never terminal, file_write, browser, etc.
        // This matches hermes-agent's `set_thread_tool_whitelist` pattern.
        let review_tool_names: Vec<String> = vec![
            "memory_store".to_string(),
            "memory_search".to_string(),
            "memory_recall".to_string(),
            "skill_manage".to_string(),
            "skill_view".to_string(),
        ];
        let tools = self
            .registry
            .get_available_schemas_filtered(&review_tool_names)
            .await;

        // ── Cache-aware replay selection ──────────────────────────────
        // Same model → full replay (warm cache reads, cheapest).
        // Different model → digest replay (cold cache, minimize tokens).
        let review_history = if is_routed {
            debug!(
                routed_model = %review_model,
                "Review routed to auxiliary model — using digest replay"
            );
            digest_history(&snapshot, 24)
        } else {
            snapshot.clone()
        };

        let registry_for_review = self.registry.clone();
        let callback = self.background_review_callback.clone();
        let event_tx = self.event_tx.clone();

        // ── Prompt cache parity (Phase 2) ─────────────────────────
        // When the review runs on the SAME model (not routed), share the
        // parent's frozen prefix (system prompt + skills) so the outbound
        // HTTP request hits the same provider prefix cache. The review
        // instructions go in a USER message, not the system message, so
        // the system prompt bytes stay byte-identical to the parent's.
        // Matches hermes-agent's `_cached_system_prompt` pinning pattern.
        //
        // When routed to a different model, the cache key differs anyway,
        // so no benefit to sharing — use None.
        let parent_frozen_prefix: Option<String> = if !is_routed {
            Some(self.build_frozen_prefix())
        } else {
            None
        };

        tokio::spawn(async move {
            debug!(
                session_id = %session_id,
                review_model = %review_model,
                is_routed,
                review_skills,
                review_memory,
                "Background review daemon started"
            );

            // ── Write origin context (Phase 2) ────────────────────
            // Set the write origin to "background_review" so the
            // skills_tool write guards know this is a review session.
            // This prevents the review agent from modifying protected
            // (bundled) or hub-installed skills. Matches hermes-agent's
            // _memory_write_origin = "background_review" pattern.
            let _origin_token = crate::write_origin::set_write_origin("background_review");
            crate::tools::skills_tool::reset_review_read_marks();

            // ── Prompt cache parity (Phase 2) ─────────────────────
            // When parent_frozen_prefix is available (same model, not routed),
            // use the parent's EXACT system prompt bytes so the outbound HTTP
            // request hits the same provider prefix cache. The review-specific
            // instructions go in a USER message, not the system message — this
            // ensures the system prompt bytes stay byte-identical.
            // Matches hermes-agent's `_cached_system_prompt` pinning pattern.
            let (system_prompt_str, review_harness) = if let Some(ref frozen) = parent_frozen_prefix
            {
                (
                    frozen.clone(),
                    format!(
                        "[Background review context]\n\nYou are a background review agent. Your job is to evaluate the \
conversation above and update skills and/or memory as needed. \
You have access to memory_store, memory_search, memory_recall, \
skill_manage, and skill_view tools only — do not attempt other tools. \
Be ACTIVE — most sessions produce at least one update. \
If nothing needs updating, say 'Nothing to save.' and stop.\n\n{}",
                        prompt
                    ),
                )
            } else {
                (
                    "You are Operant, a helpful AI assistant.".to_string(),
                    format!(
                        "You are a background review agent. Your job is to evaluate the \
conversation above and update skills and/or memory as needed. \
You have access to memory_store, memory_search, memory_recall, \
skill_manage, and skill_view tools only — do not attempt other tools. \
Be ACTIVE — most sessions produce at least one update. \
If nothing needs updating, say 'Nothing to save.' and stop.\n\n{}",
                        prompt
                    ),
                )
            };

            // Build messages: identical system prompt + review harness as user msg + snapshot
            let mut review_messages = Vec::new();
            review_messages.push(Message::system(&system_prompt_str));
            review_messages.push(Message::user(&review_harness));
            review_messages.extend(review_history);

            // ── Multi-turn tool execution loop ────────────────────────
            // Run up to MAX_REVIEW_ITERATIONS iterations to allow the review
            // agent to execute tools and see their results. This matches
            // hermes-agent's forked AIAgent.run_conversation() pattern.
            const MAX_REVIEW_ITERATIONS: usize = 5;
            let mut actions_taken: Vec<String> = Vec::new();

            for review_iter in 0..MAX_REVIEW_ITERATIONS {
                debug!(
                    iteration = review_iter + 1,
                    max = MAX_REVIEW_ITERATIONS,
                    session_id = %session_id,
                    "Background review iteration"
                );

                // Create the review chat request (non-streaming for background)
                let request = ChatRequest::new(review_model.clone(), review_messages.clone())
                    .with_tools(tools.clone())
                    .with_stream(false);

                let response = match client.chat(request).await {
                    Ok(r) => r,
                    Err(e) => {
                        warn!(
                            error = %e,
                            session_id = %session_id,
                            iteration = review_iter + 1,
                            "Background review agent failed at LLM call"
                        );
                        break;
                    }
                };

                // Extract assistant message from response
                let assistant_msg = match response.choices.first() {
                    Some(choice) => choice.message.clone(),
                    None => {
                        warn!("Background review: no choices in response");
                        break;
                    }
                };

                // Check if the model wants to stop (no tool calls)
                let tool_calls_deltas = assistant_msg.tool_calls.clone().unwrap_or_default();
                let content = assistant_msg.content.clone().unwrap_or_default();

                // If the model says "Nothing to save" or has no tool calls, we're done
                if content.contains("Nothing to save") || tool_calls_deltas.is_empty() {
                    if content.contains("Nothing to save") {
                        debug!("Background review: nothing to save");
                    } else if !content.is_empty() {
                        // Model provided a summary without tool calls
                        let preview: String = content.chars().take(200).collect();
                        info!(
                            session_id = %session_id,
                            response_preview = %preview,
                            "Background review completed with summary"
                        );
                    }
                    break;
                }

                // Add assistant message to review conversation
                let mut assistant_message = Message::assistant(&content);
                if !tool_calls_deltas.is_empty() {
                    // Convert ToolCallDelta to ToolCall for the message
                    let tool_calls: Vec<ToolCall> = tool_calls_deltas
                        .iter()
                        .filter_map(|delta| {
                            let function = delta.function.as_ref()?;
                            let id = delta.id.clone().unwrap_or_else(|| {
                                format!("bg-review-{}-{}", review_iter, delta.index)
                            });
                            Some(ToolCall {
                                id,
                                function: ToolCallFunction {
                                    name: function.name.clone(),
                                    arguments: function.arguments.clone(),
                                },
                            })
                        })
                        .collect();
                    assistant_message = assistant_message.with_tool_calls(tool_calls);
                }
                review_messages.push(assistant_message);

                // ── Execute whitelisted tools ─────────────────────────
                // Only execute tools that are in our whitelist. This matches
                // hermes-agent's set_thread_tool_whitelist pattern.
                for tool_call_delta in &tool_calls_deltas {
                    // Extract function info from the delta
                    let function = match &tool_call_delta.function {
                        Some(f) => f,
                        None => continue,
                    };
                    let tool_name = &function.name;
                    let args_str = &function.arguments;
                    let tool_id = tool_call_delta.id.as_deref().unwrap_or("unknown");

                    // Check if tool is in whitelist
                    if !review_tool_names.contains(tool_name) {
                        warn!(
                            tool = %tool_name,
                            "Background review attempted non-whitelisted tool"
                        );
                        let error_result = serde_json::json!({
                            "success": false,
                            "error": format!("Tool '{}' is not allowed in background review. Only memory and skill tools are permitted.", tool_name)
                        });
                        review_messages.push(Message::tool(tool_id, error_result.to_string()));
                        continue;
                    }

                    debug!(
                        tool = %tool_name,
                        args = %args_str,
                        "Background review executing tool"
                    );

                    // Parse arguments
                    let args: serde_json::Value = serde_json::from_str(args_str)
                        .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

                    // Execute the tool using the registry
                    let tool_result = registry_for_review
                        .execute(tool_name, tool_id, args, ToolContext::default())
                        .await;

                    match tool_result {
                        Ok(result) => {
                            let result_str = if result.success {
                                result.content.clone()
                            } else {
                                format!(
                                    "{{\"success\": false, \"error\": \"{}\"}}",
                                    result.error.unwrap_or_else(|| "Unknown error".to_string())
                                )
                            };

                            // Track actions taken for summary
                            if result.success {
                                let action_summary = format!(
                                    "{}: {}",
                                    tool_name,
                                    result_str.chars().take(100).collect::<String>()
                                );
                                actions_taken.push(action_summary);
                            }

                            review_messages.push(Message::tool(tool_id, &result_str));
                        }
                        Err(e) => {
                            warn!(
                                tool = %tool_name,
                                error = %e,
                                "Background review tool execution failed"
                            );
                            let error_result = serde_json::json!({
                                "success": false,
                                "error": format!("Tool execution failed: {}", e)
                            });
                            review_messages.push(Message::tool(tool_id, error_result.to_string()));
                        }
                    }
                }
            }

            // ── Summarize actions taken ──────────────────────────────
            // Surface a compact summary to the user via tracing AND callback.
            // Matches hermes-agent's _safe_print + background_review_callback pattern.
            if !actions_taken.is_empty() {
                let summary = actions_taken.join(" · ");
                let notification = format!("💾 Self-improvement review: {}", summary);
                info!(
                    session_id = %session_id,
                    actions = %summary,
                    action_count = actions_taken.len(),
                    "Background review completed with updates"
                );
                // Deliver via callback (TUI/Gateway wired via with_background_review_callback)
                // AND via AgentEvent so TUI/CLI surfaces it without needing the callback wired.
                if let Some(ref cb) = callback {
                    cb(notification.clone());
                }
                if let Some(ref tx) = event_tx {
                    let _ = tx
                        .send(AgentEvent::BackgroundReview {
                            summary: notification,
                        })
                        .await;
                }
            } else {
                debug!(
                    session_id = %session_id,
                    "Background review completed — no actions taken"
                );
            }

            debug!(session_id = %session_id, "Background review daemon finished");
        });
    }

    /// Compress context on overflow: try LLM summarization first, fall back
    /// to deterministic decay/eviction. Matches hermes-agent's compression
    /// pipeline: LLM compressor → fallback to manage_context.
    /// Compress an overflowing conversation, then fold the active todo list
    /// back into the compressed history so the model keeps its plan across
    /// compactions (hermes conversation_compression.py:
    /// `todo_snapshot = agent._todo_store.format_for_injection()`).
    async fn compress_context_overflow(&self, messages: Vec<Message>) -> Vec<Message> {
        let compressed = self.compress_context_overflow_inner(messages).await;
        self.reinject_todos_after_compression(compressed)
    }

    /// hermes parity: fold the active todo list back into the compressed
    /// history after compression. Any prior snapshot row is stripped first so
    /// repeated compactions refresh rather than accumulate (#26981 analog).
    fn reinject_todos_after_compression(&self, mut messages: Vec<Message>) -> Vec<Message> {
        let session_id = self.persistent_session_id.as_deref().unwrap_or("default");
        // The todo tool defaults to "default" when the model omits sessionId;
        // on gateway paths a persistent session id may be set while the model
        // still writes under the default key — look up both, preferring the
        // one that actually holds active todos.
        let snapshot =
            crate::tools::todo_tool::todo_injection_for_session(session_id).or_else(|| {
                if session_id != "default" {
                    crate::tools::todo_tool::todo_injection_for_session("default")
                } else {
                    None
                }
            });
        let Some(snapshot) = snapshot else {
            return messages;
        };

        messages.retain(|m| !crate::tools::todo_tool::is_todo_injection_row(&m.content));

        // Fold into a trailing REAL user message so compression never
        // introduces a synthetic user/user pair (hermes
        // conversation_compression.py); otherwise append as a new user turn.
        if let Some(tail) = messages.last_mut().filter(|m| m.role == Role::User) {
            tail.content.push_str("\n\n");
            tail.content.push_str(&snapshot);
            return messages;
        }
        messages.push(Message::user(snapshot));
        messages
    }

    async fn compress_context_overflow_inner(&self, messages: Vec<Message>) -> Vec<Message> {
        if let Some(ref compressor) = self.llm_compressor {
            // Bind database persistence on first compression attempt.
            // This ensures cooldown state survives process restarts —
            // matching hermes-agent's ContextCompressor cooldown persistence.
            // bind_persistence is idempotent and loads existing cooldown from DB.
            {
                let mut guard = compressor.lock().await;
                if guard.session_id().is_none()
                    && let Some(session_id) = self.persistent_session_id.as_ref()
                {
                    guard.bind_persistence(Arc::clone(&self.database), session_id.clone());
                }
            }

            // Check whether LLM compression is warranted (cheap, no await)
            {
                let guard = compressor.lock().await;
                if !guard.should_compress(self.estimate_current_tokens(&messages)) {
                    // Under threshold — deterministic fallback
                    let budget = self.config.context_window;
                    return crate::context_management::manage_context(messages, budget, 4096);
                }
                // Anti-thrash: skip LLM compression if in cooldown after recent failure.
                if guard.is_in_cooldown() {
                    warn!("LLM compression in anti-thrash cooldown — using deterministic fallback");
                    let budget = self.config.context_window;
                    return crate::context_management::manage_context(messages, budget, 4096);
                }
            }
            info!("Attempting LLM-based context compression");
            // Lock again for the async compress call (tokio::sync::Mutex is await-safe)
            let mut guard = compressor.lock().await;
            match guard.compress(messages.clone(), &self.client).await {
                Ok(result) => {
                    info!(
                        tokens_before = result.tokens_before,
                        tokens_after = result.tokens_after,
                        turns_summarized = result.turns_summarized,
                        "LLM compression succeeded"
                    );
                    drop(guard);
                    return result.messages;
                }
                Err(e) => {
                    warn!(error = %e, "LLM compression failed — falling back to deterministic");
                }
            }
            drop(guard);
        }
        // Deterministic fallback: decay + eviction
        let budget = self.config.context_window;
        crate::context_management::manage_context(messages, budget, 4096)
    }

    /// Access the underlying model client (useful for tools needing direct
    /// access to the concrete provider client).
    pub fn client(&self) -> &Arc<dyn ModelClient> {
        &self.client
    }

    /// Token estimate for the compression gate. Prefers the model-reported
    /// prompt-token count from the last request (source of truth, matching
    /// hermes context_engine), falling back to the char/4 heuristic when no
    /// request has completed yet this session.
    fn estimate_current_tokens(&self, messages: &[Message]) -> usize {
        prefer_reported(
            self.last_prompt_tokens
                .load(std::sync::atomic::Ordering::Relaxed),
            crate::context_management::estimate_total_tokens(messages),
        )
    }

    /// Emit `AgentEvent::Usage`/`AgentEvent::Cost` for a completed request
    /// and accumulate the session-level cost total. Shared by
    /// `process_response` (non-streaming) and `process_stream` (streaming,
    /// iter-247) now that both paths can produce a `Usage`.
    async fn emit_usage_and_cost(&self, usage: &Usage) {
        self.last_prompt_tokens.store(
            usage.prompt_tokens.try_into().unwrap_or(0),
            std::sync::atomic::Ordering::Relaxed,
        );
        self.emit(AgentEvent::Usage {
            input_tokens: usage.prompt_tokens,
            output_tokens: usage.completion_tokens,
            total_tokens: usage.total_tokens,
        })
        .await;

        // iter-132: emit a Cost event right after Usage. Look up the
        // model in models_dev to get cost-per-million, then multiply by
        // token counts. If the model isn't in the catalog, emit
        // cost_usd=None so the caller can show "cost unknown".
        //
        // We split the model name on '/' (provider/model format) to get
        // the provider and model parts. If there's no '/', we use the
        // whole string as the model and "" as the provider.
        let (provider, model_name) = match self.config.model.split_once('/') {
            Some((p, m)) => (p.to_string(), m.to_string()),
            None => (String::new(), self.config.model.clone()),
        };
        let cost_usd = crate::models_dev::get_model_capabilities(&provider, &model_name)
            .await
            .and_then(|caps| {
                let input_cost = caps
                    .cost_input_per_million
                    .map(|c| (usage.prompt_tokens as f64 / 1_000_000.0) * c);
                let output_cost = caps
                    .cost_output_per_million
                    .map(|c| (usage.completion_tokens as f64 / 1_000_000.0) * c);
                input_cost.zip(output_cost).map(|(i, o)| i + o)
            });
        self.emit(AgentEvent::Cost {
            cost_usd,
            input_tokens: usage.prompt_tokens,
            output_tokens: usage.completion_tokens,
            model: self.config.model.clone(),
        })
        .await;

        if let Some(cost) = cost_usd
            && let Ok(mut total) = self.session_cost_usd.write()
        {
            *total += cost;
        }
    }

    /// Process streaming response with early tool detection
    async fn process_stream(
        &self,
        mut stream: BoxStream<'static, Result<StreamChunk>>,
    ) -> Result<(
        String,
        String,
        Vec<ToolCall>,
        Option<serde_json::Value>,
        Option<String>,
    )> {
        let mut accumulated_extra: Option<serde_json::Value> = None;
        let mut parser = ToolCallStreamParser::new().on_tool_call(|tc| {
            let tc_id = tc.id.clone();
            debug!(tool_call_id = %tc_id, name = %tc.function.name, "Early tool call detected");
        });
        let mut content_router = ThinkBlockRouter::default();
        let mut tool_call_router = ToolCallContentRouter::default();
        let mut accumulated_text = String::new();
        let mut accumulated_reasoning = String::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();
        // Streaming usage arrives split across chunks (Anthropic reports
        // input_tokens on message_start and output_tokens on message_delta;
        // OpenAI-compatible providers report both together on one trailing
        // chunk when stream_options.include_usage is set). Track whichever
        // halves have arrived and only treat usage as complete once both
        // are known.
        let mut usage_prompt_tokens: Option<u32> = None;
        let mut usage_completion_tokens: Option<u32> = None;
        // Capture the original stream error so we can surface it (instead of
        // the generic "Stream processing failed" string) and decide whether
        // to flush partials after the loop.
        let mut stream_error: Option<Error> = None;
        // Provider-reported finish reason from the terminal chunk(s) (T1 —
        // truncation detection). Last non-None wins.
        let mut finish_reason: Option<String> = None;

        while let Some(chunk_result) = stream.next().await {
            match chunk_result {
                Ok(chunk) => {
                    if let Some(u) = chunk.usage {
                        if u.prompt_tokens > 0 {
                            usage_prompt_tokens = Some(u.prompt_tokens);
                        }
                        if u.completion_tokens > 0 {
                            usage_completion_tokens = Some(u.completion_tokens);
                        }
                    }

                    // Process reasoning from StreamChunk.
                    // If the provider sends reasoning natively (via
                    // reasoning_content), use that and DON'T also extract
                    // reasoning from content text — otherwise the same
                    // reasoning appears twice. (iter-123 — fixes duplicate
                    // thinking bug.)
                    let has_native_reasoning = chunk.reasoning.is_some();
                    if let Some(reasoning) = chunk.reasoning {
                        // Preserve \n (reasoning may double as message content
                        // when the final answer is empty); only CR is stripped.
                        let reasoning = reasoning.replace('\r', "");
                        let reasoning = strip_reasoning_tags(&reasoning);
                        if !reasoning.is_empty() {
                            accumulated_reasoning.push_str(&reasoning);
                            self.emit(AgentEvent::Reasoning { text: reasoning }).await;
                        }
                    }

                    // Capture provider-specific extra content (e.g. Gemini thought_signature)
                    if let Some(ref extra) = chunk.extra_content
                        && !extra.is_null()
                    {
                        accumulated_extra = Some(extra.clone());
                    }

                    // Process content from StreamChunk
                    // Sanitize provider streaming text: strip carriage returns
                    // (they corrupt terminal display by moving the cursor back
                    // to column 0). Newlines are PRESERVED — they carry the
                    // markdown structure (headers, tables, code fences,
                    // blockquotes) that the Telegram/Discord renderers depend
                    // on. (iter-263 replaced every \n with a space to mask a
                    // provider mid-word newline quirk; that collapsed ALL
                    // gateway responses into single-line blobs and broke every
                    // markdown layout.)
                    if let Some(text) = chunk.content {
                        let text = text.replace('\r', "");
                        let (content_delta, reasoning_delta) = content_router.feed(&text);

                        if !content_delta.is_empty() {
                            let chunk_tool_calls = parser.process_chunk(&content_delta);
                            for tc in chunk_tool_calls {
                                if !tool_calls.iter().any(|existing| existing.id == tc.id) {
                                    tool_calls.push(tc);
                                }
                            }

                            let visible_text = tool_call_router.feed(&content_delta);
                            if !visible_text.is_empty() {
                                let scrubbed = strip_memory_context_tags(&visible_text);
                                if !scrubbed.is_empty() {
                                    accumulated_text.push_str(&scrubbed);
                                    self.emit(AgentEvent::Content { text: scrubbed }).await;
                                }
                            }
                        }

                        // Only emit reasoning from content_router if the
                        // provider didn't already send it natively. This
                        // prevents duplicate thinking. (iter-123)
                        if !has_native_reasoning && !reasoning_delta.is_empty() {
                            accumulated_reasoning.push_str(&reasoning_delta);
                            self.emit(AgentEvent::Reasoning {
                                text: reasoning_delta,
                            })
                            .await;
                        }
                    }

                    // Merge native provider tool-call deltas
                    if let Some(chunk_tool_calls) = chunk.tool_calls {
                        for tc in chunk_tool_calls {
                            merge_stream_tool_call(&mut tool_calls, tc);
                        }
                    }

                    // Capture the provider finish reason (T1).
                    if let Some(fr) = &chunk.finish_reason {
                        finish_reason = Some(fr.clone());
                    }
                }
                Err(e) => {
                    error!(error = %e, "Stream error");
                    // Capture the original error so we can surface it after
                    // flushing partials. Previously the error was swallowed
                    // and replaced with a generic "Stream processing failed"
                    // string, making debugging impossible.
                    stream_error = Some(e);
                    break;
                }
            }
        }

        // Flush any partial content/tool_calls still buffered in the routers
        // and parser. This runs on both success AND error paths so partial
        // tool calls (e.g. a tool_use block that started but didn't finish
        // before the stream broke) are still extracted and returned to the
        // caller. Previously the error path `break`ed before this flush,
        // dropping all partials.
        let (remaining_content, remaining_reasoning) = content_router.finish();
        if !remaining_content.is_empty() {
            let remaining_calls = parser.process_chunk(&remaining_content);
            for tc in remaining_calls {
                merge_stream_tool_call(&mut tool_calls, tc);
            }
            let visible = tool_call_router.feed(&remaining_content);
            if !visible.is_empty() {
                accumulated_text.push_str(&visible);
                // Emit the flushed partial so the TUI sees it even if we're
                // about to return Err — otherwise content streamed right
                // before the error would be silently lost.
                self.emit(AgentEvent::Content {
                    text: strip_memory_context_tags(&visible),
                })
                .await;
            }
        }
        let tail = tool_call_router.finish();
        if !tail.is_empty() {
            let scrubbed_tail = strip_memory_context_tags(&tail);
            accumulated_text.push_str(&scrubbed_tail);
            self.emit(AgentEvent::Content {
                text: scrubbed_tail,
            })
            .await;
        }
        if !remaining_reasoning.is_empty() {
            accumulated_reasoning.push_str(&remaining_reasoning);
            self.emit(AgentEvent::Reasoning {
                text: remaining_reasoning,
            })
            .await;
        } // Also try to extract any remaining tool calls from accumulated text.
        // On the error path we don't want a parser failure to mask the
        // original stream error, so fall back to an empty vec.
        // Normalize CR/CRLF before final processing. Newlines are preserved
        // (markdown structure) — only carriage returns are stripped, matching
        // the per-chunk sanitization above.
        accumulated_text = accumulated_text.replace('\r', "");
        accumulated_reasoning = accumulated_reasoning.replace('\r', "");
        let mut remaining_parser = ToolCallParser::new();
        let remaining_calls = if stream_error.is_some() {
            remaining_parser
                .parse(&accumulated_text)
                .unwrap_or_default()
        } else {
            remaining_parser.parse(&accumulated_text)?
        };

        // Merge tool calls, avoiding duplicates
        for tc in remaining_calls {
            merge_stream_tool_call(&mut tool_calls, tc);
        }

        // ── Validate tool call arguments before returning (iter-261) ──
        // Truncated streaming can leave tool_calls with incomplete JSON
        // arguments (e.g. `{"query": "te` from a cut-off SSE stream).
        // Repair what we can; discard tool calls whose arguments are
        // irreparably broken so execute_tools doesn't surface a raw
        // "Invalid JSON" error to the user.
        tool_calls.retain(|tc| {
            let args = tc.function.arguments.trim();
            if args.is_empty() || args == "{}" {
                return true; // Empty args are valid (tool uses defaults)
            }
            match serde_json::from_str::<serde_json::Value>(args) {
                Ok(_) => true,
                Err(_) => {
                    let repaired =
                        message_safety::repair_tool_call_arguments(args, &tc.function.name);
                    match serde_json::from_str::<serde_json::Value>(&repaired) {
                        Ok(_) => {
                            debug!(
                                tool = %tc.function.name,
                                original_len = args.len(),
                                "Tool arguments auto-repaired in process_stream"
                            );
                            true // repaired — keep it (repair happens again in execute_tools)
                        }
                        Err(e) => {
                            warn!(
                                tool = %tc.function.name,
                                error = %e,
                                args_preview = %safe_truncate_str(args, 80),
                                "Discarding tool call with irreparable arguments"
                            );
                            false // irreparable — drop this tool call
                        }
                    }
                }
            }
        });

        if let Some(err) = stream_error {
            // Surface the ORIGINAL stream error (e.g. reqwest's "error
            // decoding response body" when a provider closes the SSE
            // connection mid-body) rather than wrapping it in a generic
            // "Stream processing failed" Agent error. The raw variant
            // (`Error::Network`) classifies as retryable, which the run()
            // loop uses to re-issue the request (hermes-parity stream-drop
            // recovery). Trajectory saving is handled by the caller (run())
            // when this error propagates up.
            //
            // R2: a transport failure on a known reasoning model with NO
            // content arrived yet is a thinking-timeout (upstream idle-killed
            // the thinking phase). Record it so the run loop can annotate the
            // final error with guidance once retries are exhausted.
            if crate::reasoning_timeouts::is_thinking_timeout(&self.model(), &err.to_string())
                && accumulated_text.is_empty()
                && accumulated_reasoning.is_empty()
                && tool_calls.is_empty()
            {
                self.thinking_timeout_hit
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                self.emit(AgentEvent::Content {
                    text: "⚠ The model's thinking phase may have exceeded the upstream idle timeout — retrying."
                        .to_string(),
                })
                .await;
            }
            return Err(err);
        }

        // iter-247: emit Usage/Cost for streaming the same way
        // process_response does for non-streaming, now that both usage
        // halves are available. If the provider never sent usage data (or
        // only sent one half), silently skip rather than reporting
        // incomplete numbers.
        if let (Some(prompt_tokens), Some(completion_tokens)) =
            (usage_prompt_tokens, usage_completion_tokens)
        {
            let usage = Usage {
                prompt_tokens,
                completion_tokens,
                total_tokens: prompt_tokens + completion_tokens,
            };
            self.emit_usage_and_cost(&usage).await;
        }

        Ok((
            accumulated_text,
            accumulated_reasoning,
            tool_calls,
            accumulated_extra,
            finish_reason,
        ))
    }

    async fn process_response(
        &self,
        response: ChatResponse,
    ) -> Result<(String, String, Vec<ToolCall>, Option<String>)> {
        let mut choice = response
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| Error::ParseResponse("response had no choices".to_string()))?;
        // Provider-reported finish reason (T1 — truncation detection).
        let finish_reason = choice.finish_reason.take();

        let message = choice.message;
        let raw_content = message.content.unwrap_or_default();
        let content = strip_tool_call_markup(&raw_content);
        let reasoning = message
            .reasoning_content
            .map(|value| strip_reasoning_tags(&value))
            .unwrap_or_default();
        let mut tool_calls = extract_tool_calls_from_choice(message.tool_calls);
        let mut xml_parser = ToolCallParser::new();
        if let Ok(xml_tool_calls) = xml_parser.parse(&raw_content) {
            for tool_call in xml_tool_calls {
                merge_stream_tool_call(&mut tool_calls, tool_call);
            }
        }

        if !content.is_empty() {
            self.emit(AgentEvent::Content {
                text: strip_memory_context_tags(&content),
            })
            .await;
        }
        if !reasoning.is_empty() {
            self.emit(AgentEvent::Reasoning {
                text: reasoning.clone(),
            })
            .await;
        }

        self.emit_usage_and_cost(&response.usage).await;

        Ok((content, reasoning, tool_calls, finish_reason))
    }

    #[expect(
        clippy::expect_used,
        reason = "invariant guaranteed by surrounding validation"
    )]
    /// Execute tools and handle self-healing
    async fn execute_tools(&self, tool_calls: Vec<ToolCall>) -> Result<Vec<ToolResult>> {
        // ── Concurrent tool execution (iter-56) ──────────────────────────
        // Previously this was a sequential for-loop. Now it's two phases:
        //
        // Phase 1 (sequential): Pre-flight checks — interrupt flag, arg
        //   parsing, tool validation, approval gate, permission prompts.
        //   These MUST be sequential because permission prompts are
        //   interactive (the user sees one dialog at a time).
        //
        // Phase 2 (concurrent): Execute all approved tools concurrently
        //   using FuturesUnordered with a semaphore (max 8, matching
        //   hermes's _MAX_TOOL_WORKERS). Independent tool calls (e.g.
        //   4 web searches) now run in parallel instead of serially.
        //
        // Results are collected in the SAME ORDER as the input tool_calls
        // (the model expects results in the same order as the calls).

        use futures::stream::{self, StreamExt};
        use std::sync::Arc as StdArc;
        use tokio::sync::Semaphore;

        // ── Phase 1: Pre-flight (sequential) ────────────────────────────
        let mut pending: Vec<(usize, ToolCall, serde_json::Value)> = Vec::new();
        let mut early_results: Vec<Option<ToolResult>> = vec![None; tool_calls.len()];
        // T6: within-batch dedupe (hermes `_deduplicate_tool_calls`) — only
        // the first occurrence of each (tool, arguments) pair in a single
        // assistant message executes; exact duplicates are skipped with a
        // synthetic result so result ordering is preserved and degenerate
        // batches don't double-mutate.
        let mut seen_tool_calls: std::collections::HashSet<(String, String)> =
            std::collections::HashSet::new();

        for (idx, tool_call) in tool_calls.into_iter().enumerate() {
            // Check interrupt flag
            if self.interrupt_flag.is_triggered() {
                early_results[idx] = Some(ToolResult::error(
                    &tool_call.id,
                    "Skipped: interrupted by user (Ctrl-C)".to_string(),
                ));
                continue;
            }

            // T6: skip exact duplicates within this batch.
            let dup_key = (
                tool_call.function.name.clone(),
                tool_call.function.arguments.trim().to_string(),
            );
            if !seen_tool_calls.insert(dup_key) {
                warn!(
                    tool = %tool_call.function.name,
                    "Duplicate tool call with identical arguments — skipped (T6 within-batch dedupe)"
                );
                early_results[idx] = Some(ToolResult::error(
                    &tool_call.id,
                    format!(
                        "Duplicate tool call '{}' with identical arguments skipped — already \
                         invoked once in this batch.",
                        tool_call.function.name
                    ),
                ));
                continue;
            }

            let name = tool_call.function.name.clone();
            let raw_args = tool_call.function.arguments.clone();
            let trimmed = raw_args.trim();
            let args_str = if trimmed.is_empty() {
                "{}".to_string()
            } else {
                raw_args
            };

            debug!(tool = %name, args = %args_str, "Executing tool");

            // Emit ToolCallStart observer event
            if let Some(ref obs) = self.observer {
                obs.record_event(&ObserverEvent::ToolCallStart {
                    tool: name.clone(),
                    arguments: Some(args_str.clone()),
                });
            }

            self.emit(AgentEvent::ToolStart {
                tool_call_id: tool_call.id.clone(),
                name: name.clone(),
                arguments: args_str.clone(),
            })
            .await;

            // Parse arguments — with auto-repair for common truncation issues.
            // (iter-123 — fixes "Invalid JSON: EOF while parsing" errors
            // caused by streaming tool-call argument fragmentation.)
            let mut args: serde_json::Value = match serde_json::from_str(&args_str) {
                Ok(a) => a,
                Err(e) => {
                    // Try to repair common truncation issues:
                    // 1. Missing closing brace — append }
                    // 2. Missing closing bracket — append ]
                    // 3. Truncated string value — append "
                    let repaired = message_safety::repair_tool_call_arguments(&args_str, &name);
                    if let Ok(a) = serde_json::from_str(&repaired) {
                        debug!(tool = %name, "Tool arguments auto-repaired");
                        a
                    } else {
                        let preview = safe_truncate_str(&args_str, 120);
                        warn!(
                            tool = %name,
                            error = %e,
                            args_preview = %preview,
                            args_len = args_str.len(),
                            "Failed to parse tool arguments (truncated by provider?)"
                        );
                        early_results[idx] = Some(ToolResult::error(
                            &tool_call.id,
                            format!(
                                "Tool '{}' received truncated arguments from the model (length {}). \
                                 The model's response was likely cut off — please retry your request.",
                                name,
                                args_str.len()
                            ),
                        ));
                        continue;
                    }
                }
            };

            // ── Tool-call guardrails (R4 — hermes tool_guardrails.py) ──
            // Detect retry storms: the model calling the same tool with
            // identical args repeatedly within one turn. Side-effecting tools
            // are skipped on the 3rd identical call; no-effect tools (cheap,
            // read-only) get a warning then skip on the 4th. The synthetic
            // result tells the model to stop repeating, saving round-trips and
            // preventing repeated mutations.
            {
                use crate::tool_guardrails::GuardrailDecision;
                let decision = {
                    let mut g = self
                        .tool_guardrails
                        .lock()
                        .expect("tool_guardrails lock poisoned");
                    g.observe(&name, &args_str)
                };
                match decision {
                    GuardrailDecision::Allow => {}
                    GuardrailDecision::Warn => {
                        let count = self
                            .tool_guardrails
                            .lock()
                            .expect("tool_guardrails lock poisoned")
                            .count_of(&name, &args_str);
                        warn!(
                            tool = %name,
                            count,
                            "Repeated identical tool call — warning model"
                        );
                        self.emit(AgentEvent::Content {
                            text: format!(
                                "⚠ Tool '{name}' has been called with identical arguments {count} times this turn."
                            ),
                        })
                        .await;
                    }
                    GuardrailDecision::Skip => {
                        let count = self
                            .tool_guardrails
                            .lock()
                            .expect("tool_guardrails lock poisoned")
                            .count_of(&name, &args_str);
                        warn!(
                            tool = %name,
                            count,
                            "Repeated identical tool call — skipping duplicate"
                        );
                        self.metrics.record_guardrail_skip();
                        early_results[idx] = Some(ToolResult::error(
                            &tool_call.id,
                            crate::tool_guardrails::build_skip_message(&name, count),
                        ));
                        continue;
                    }
                }
            }

            // Validate tool exists
            if !self.registry.contains(&name).await {
                error!(tool = %name, "Tool not found");
                early_results[idx] = Some(ToolResult::error(
                    &tool_call.id,
                    format!("Tool '{}' not found", name),
                ));
                continue;
            }

            // ── Centralized argument validation (iter-262) ───────────
            // Validate required fields BEFORE calling tool.execute().
            // Without this, each tool's serde_json::from_value() would fail
            // independently with opaque "missing field 'query'" errors.
            // Centralized validation gives a clear, consistent error message
            // and prevents truncated tool calls from reaching the tool impl.
            if let Some(tool) = self.registry.get(&name).await {
                let schema = tool.schema();
                schema.sanitize_args(&mut args);
                if let Err(e) = schema.validate_args(&args) {
                    warn!(tool = %name, error = %e, "Tool argument validation failed");
                    // Use the schema validation error message directly — it
                    // already includes the field name (e.g. "Missing required
                    // field: query"). Avoid duplicating the tool name.
                    early_results[idx] = Some(ToolResult::error(&tool_call.id, e.to_string()));
                    continue;
                }
            }

            // Smart approval gate
            if self.config.approval_mode != "off" {
                let approval_result = crate::approval::check_tool_approval(
                    &name,
                    &args,
                    Some(&self.config.approval_mode),
                );
                match approval_result.verdict.as_str() {
                    "blocked" => {
                        warn!(tool = %name, "Tool call blocked by approval guard");
                        early_results[idx] = Some(ToolResult::error(
                            &tool_call.id,
                            format!(
                                "Blocked by security policy: {}",
                                approval_result
                                    .reason
                                    .unwrap_or_else(|| "blocked".to_string())
                            ),
                        ));
                        continue;
                    }
                    "requires_approval" => {
                        warn!(tool = %name, "Tool call flagged — will prompt user");
                    }
                    _ => {}
                }
            }

            // Permission guard for dangerous tools (interactive — sequential)
            if let Some(ref permission_tx) = self.permission_tx {
                // hermes parity: a tool covered by the session or permanent
                // allowlist (`command_allowlist` / `always`) never prompts —
                // it runs immediately. Checked before the hardcoded
                // dangerous-tool list so allowlisted tools bypass the gate
                // (hermes `_command_matches_permanent_allowlist` fires before
                // detection, with only the hardline floor above it).
                if self.tool_allowed_by_allowlist(&name) {
                    // allowed by allowlist — no prompt
                } else {
                    let needs_permission = matches!(
                        name.as_str(),
                        "bash"
                            | "terminal"
                            | "execute_command"
                            | "code_execution"
                            | "file_read"
                            | "file_write"
                            | "file_edit"
                            | "patch"
                            | "process"
                            | "browser"
                    );
                    if needs_permission {
                        let (resp_tx, resp_rx) = tokio::sync::oneshot::channel();
                        let description = format!("Execute {} tool", name);
                        let danger = match name.as_str() {
                            "bash" | "terminal" | "execute_command" => {
                                "This runs a shell command on your system".to_string()
                            }
                            "code_execution" => {
                                "This runs code on your system with the operant process's permissions (not sandboxed)".to_string()
                            }
                            "file_read" => "This reads a file from your system".to_string(),
                            "file_write" => "This writes content to a file".to_string(),
                            "file_edit" | "patch" => "This modifies an existing file".to_string(),
                            "process" => "This manages background processes".to_string(),
                            "browser" => "This opens and interacts with a browser".to_string(),
                            _ => "This tool may modify your system".to_string(),
                        };
                        let input_preview = Some(args_str.clone());
                        let _ = permission_tx
                            .send(ToolPermissionRequest {
                                tool_name: name.clone(),
                                tool_id: tool_call.id.clone(),
                                description,
                                danger_explanation: danger,
                                input_preview,
                                response_tx: resp_tx,
                            })
                            .await;
                        let response = tokio::select! {
                            r = resp_rx => r.unwrap_or(ToolPermissionResponse::Deny),
                            _ = tokio::time::sleep(Duration::from_secs(120)) => ToolPermissionResponse::Deny,
                        };
                        match response {
                            ToolPermissionResponse::AllowOnce => {}
                            ToolPermissionResponse::AllowSession => {
                                // hermes `approve_session`: remember the tool
                                // for the rest of this agent instance so it
                                // never prompts again this session.
                                self.session_allowlist
                                    .write()
                                    .unwrap_or_else(|e| e.into_inner())
                                    .insert(name.clone());
                            }
                            ToolPermissionResponse::AllowAlways => {
                                // hermes `approve_permanent` +
                                // `save_permanent_allowlist`: remember forever
                                // and persist to disk so later sessions honor
                                // the choice too.
                                self.session_allowlist
                                    .write()
                                    .unwrap_or_else(|e| e.into_inner())
                                    .insert(name.clone());
                                let patterns = {
                                    let mut guard = self
                                        .persistent_allowlist
                                        .write()
                                        .unwrap_or_else(|e| e.into_inner());
                                    guard.insert(name.clone());
                                    guard.clone()
                                };
                                persist_approval_allowlist(
                                    self.config.approval_allowlist_path.as_deref(),
                                    &patterns,
                                );
                            }
                            ToolPermissionResponse::Deny => {
                                early_results[idx] = Some(ToolResult::error(
                                    &tool_call.id,
                                    "Permission denied by user".to_string(),
                                ));
                                continue;
                            }
                        }
                    }
                }
            }

            // Tool passed all pre-flight checks — queue for concurrent execution
            pending.push((idx, tool_call, args));
        }

        // ── Phase 2: Concurrent execution ───────────────────────────────
        // Use a semaphore to limit concurrency to 8 (matching hermes).
        // If only 1 tool is pending, skip the overhead and execute directly.
        if pending.is_empty() {
            // All tools were handled in pre-flight (errors/blocked/denied)
            // (iter-141 — fixed A20/A21: was .unwrap() which panics if a
            // future was cancelled. Use flatten() to gracefully skip None.)
            let results = early_results.into_iter().flatten().collect();
            return Ok(results);
        }

        if pending.len() == 1 {
            // Single tool — no concurrency overhead
            let (idx, tool_call, args) = pending
                .into_iter()
                .next()
                .expect("pending non-empty in single-tool branch");
            let name = tool_call.function.name.clone();
            let tool_future =
                self.registry
                    .execute(&name, &tool_call.id, args, ToolContext::default());
            // Interactive tools (clarify / approval_request) block waiting
            // for a human — the generic tool timeout (30s) would kill the
            // dialog before the user can respond. Long-running tools
            // (delegate_task) spawn a child agent with its own timeout. Both
            // get a generous defensive wrapper instead: the user-question
            // receiver resolves dialogs on their own (120s timeout reply),
            // and the child timeout governs delegation — the wrapper is only
            // a backstop against a wedged receiver/child.
            let result = if is_interactive_tool(&name) || is_long_running_tool(&name) {
                timeout(LONG_RUNNING_TOOL_TIMEOUT, tool_future).await
            } else {
                timeout(self.config.tool_timeout, tool_future).await
            };
            early_results[idx] = Some(match result {
                Ok(Ok(r)) => r,
                Ok(Err(e)) => ToolResult::error(&tool_call.id, e.to_string()),
                Err(_) => ToolResult::error(
                    &tool_call.id,
                    format!("Tool timed out after {:?}", self.config.tool_timeout),
                ),
            });
        } else {
            // Multiple tools — execute concurrently with semaphore
            let semaphore = StdArc::new(Semaphore::new(8));
            let tool_timeout = self.config.tool_timeout;

            let futures: Vec<_> = pending
                .into_iter()
                .map(|(idx, tool_call, args)| {
                    let sem = semaphore.clone();
                    let registry = &self.registry;
                    let interrupt_flag = &self.interrupt_flag;
                    async move {
                        // Acquire semaphore permit (limits to 8 concurrent)
                        // (iter-141 — fixed A20: was .unwrap() which panics
                        // if the semaphore closes during shutdown. Use
                        // ok() + early return on failure.)
                        let _permit = match sem.acquire().await {
                            Ok(p) => p,
                            Err(_) => {
                                return (
                                    idx,
                                    ToolResult::error(
                                        &tool_call.id,
                                        "Skipped: semaphore closed during shutdown".to_string(),
                                    ),
                                );
                            }
                        };

                        // Check interrupt flag before execution
                        if interrupt_flag.is_triggered() {
                            return (
                                idx,
                                ToolResult::error(
                                    &tool_call.id,
                                    "Skipped: interrupted".to_string(),
                                ),
                            );
                        }

                        let name = tool_call.function.name.clone();
                        let exec =
                            registry.execute(&name, &tool_call.id, args, ToolContext::default());
                        // Interactive tools exempt from the generic tool
                        // timeout (see is_interactive_tool); long-running
                        // tools like delegate_task carry their own child
                        // timeout. Both get the generous backstop — the
                        // user-question receiver resolves dialogs on their
                        // own 120s timeout and the child timeout governs
                        // delegation.
                        let result = if is_interactive_tool(&name) || is_long_running_tool(&name) {
                            timeout(LONG_RUNNING_TOOL_TIMEOUT, exec).await
                        } else {
                            timeout(tool_timeout, exec).await
                        };

                        (
                            idx,
                            match result {
                                Ok(Ok(r)) => r,
                                Ok(Err(e)) => ToolResult::error(&tool_call.id, e.to_string()),
                                Err(_) => ToolResult::error(
                                    &tool_call.id,
                                    format!("Tool timed out after {:?}", tool_timeout),
                                ),
                            },
                        )
                    }
                })
                .collect();

            // Execute all futures concurrently and collect results
            let results = stream::iter(futures)
                .buffer_unordered(8)
                .collect::<Vec<_>>()
                .await;

            // Place results in the correct position
            for (idx, result) in results {
                early_results[idx] = Some(result);
            }
        }

        // Collect results in original order
        // (iter-141 — fixed A20/A21: was .unwrap() which panics if a
        // future was cancelled. Use flatten() to gracefully skip None.)
        let results = early_results.into_iter().flatten().collect();
        Ok(results)
    }

    /// Run agent and handle self-healing on tool errors
    pub async fn run_with_healing(&self, user_query: String) -> Result<Message> {
        let mut iteration = 0;
        let max_healing_attempts = self.config.max_healing_attempts;

        loop {
            iteration += 1;

            match self.run(user_query.clone()).await {
                Ok(response) => return Ok(response),
                Err(e) if e.is_self_healing() && iteration <= max_healing_attempts => {
                    warn!(iteration, error = %e, "Self-healing: re-prompting LLM");

                    // Add error context as a system message
                    let error_msg = format!(
                        "Note: The previous attempt encountered an error: {}. \
                        Please correct your approach and try again.",
                        e.user_message()
                    );

                    self.add_message(Message::system(&error_msg)).await;
                }
                Err(e) => {
                    error!(error = %e, "Agent run failed");
                    return Err(e);
                }
            }
        }
    }
}

#[derive(Debug, Default)]
struct ThinkBlockRouter {
    pending: String,
    inside_reasoning: bool,
}

impl ThinkBlockRouter {
    fn feed(&mut self, chunk: &str) -> (String, String) {
        self.pending.push_str(chunk);
        self.drain_ready()
    }

    fn finish(&mut self) -> (String, String) {
        let (mut content, mut reasoning) = self.drain_ready();
        if !self.pending.is_empty() {
            if self.inside_reasoning {
                reasoning.push_str(&self.pending);
                if content.trim().is_empty() {
                    content.push_str(&self.pending);
                }
            } else {
                content.push_str(&self.pending);
            }
            self.pending.clear();
        }
        (content, reasoning)
    }

    fn drain_ready(&mut self) -> (String, String) {
        const MAX_TAG_LEN: usize = 23;
        let mut content = String::new();
        let mut reasoning = String::new();

        loop {
            let lowered = self.pending.to_ascii_lowercase();
            let tag = if self.inside_reasoning {
                find_first_tag(&lowered, CLOSE_REASONING_TAGS)
            } else {
                find_first_tag(&lowered, OPEN_REASONING_TAGS)
            };

            if let Some((index, marker)) = tag {
                let segment = self.pending[..index].to_string();
                if self.inside_reasoning {
                    reasoning.push_str(&segment);
                } else {
                    content.push_str(&segment);
                }
                self.pending.drain(..index + marker.len());
                self.inside_reasoning = !self.inside_reasoning;
                continue;
            }

            let keep = self.pending.len().min(MAX_TAG_LEN.saturating_sub(1));
            let flush_len =
                floor_char_boundary(&self.pending, self.pending.len().saturating_sub(keep));
            if flush_len == 0 {
                break;
            }

            let segment = self.pending[..flush_len].to_string();
            if self.inside_reasoning {
                reasoning.push_str(&segment);
            } else {
                content.push_str(&segment);
            }
            self.pending.drain(..flush_len);
        }

        (content, reasoning)
    }
}

const OPEN_REASONING_TAGS: &[&str] = &[
    "<think>",
    "<thinking>",
    "<reasoning>",
    "<thought>",
    "<reasoning_scratchpad>",
];

const CLOSE_REASONING_TAGS: &[&str] = &[
    "</think>",
    "</thinking>",
    "</reasoning>",
    "</thought>",
    "</reasoning_scratchpad>",
];

fn find_first_tag<'a>(haystack: &str, tags: &'a [&'a str]) -> Option<(usize, &'a str)> {
    tags.iter()
        .filter_map(|tag| haystack.find(tag).map(|index| (index, *tag)))
        .min_by_key(|(index, _)| *index)
}

fn floor_char_boundary(text: &str, index: usize) -> usize {
    let mut boundary = index.min(text.len());
    while boundary > 0 && !text.is_char_boundary(boundary) {
        boundary -= 1;
    }
    boundary
}

/// Truncate tool results that are too large for context (e.g. base64 audio).
/// Keeps a JSON summary with metadata but strips the bulk data.
const MAX_TOOL_RESULT_LEN: usize = 4096;

fn truncate_tool_result(tool_name: &str, content: &str) -> String {
    if content.len() <= MAX_TOOL_RESULT_LEN {
        return content.to_string();
    }
    // Try to parse as JSON and strip large fields
    if let Ok(mut val) = serde_json::from_str::<serde_json::Value>(content)
        && let Some(obj) = val.as_object_mut()
    {
        // Remove known large fields
        let had_audio = obj.remove("audio").is_some();
        let had_data = obj.remove("data").is_some();
        if had_audio {
            obj.insert(
                "audio".to_string(),
                serde_json::json!("[audio data delivered to user]"),
            );
        }
        if had_data {
            obj.insert(
                "data".to_string(),
                serde_json::json!("[large data truncated]"),
            );
        }
        let mut serialized = serde_json::to_string(&*obj).unwrap_or_default();
        if serialized.len() <= MAX_TOOL_RESULT_LEN {
            return serialized;
        }
        // Still too long — truncate the largest string fields in place so the
        // JSON stays valid and the trailing metadata keys survive. This is the
        // skill_view parity fix: serde_json's default BTreeMap ordering puts
        // bulky fields like "content" FIRST, so a naive head-truncate would
        // drop the model-visible metadata (name, description, tags,
        // supporting_files) that comes after it.
        let mut string_fields: Vec<(String, usize)> = obj
            .iter()
            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.len())))
            .collect();
        string_fields.sort_by_key(|(_, len)| std::cmp::Reverse(*len));
        for (key, _len) in string_fields {
            if serialized.len() <= MAX_TOOL_RESULT_LEN {
                break;
            }
            let Some(current) = obj.get(&key).and_then(|v| v.as_str()) else {
                continue;
            };
            // Budget for this field = the whole serialized JSON minus the
            // other fields, minus room for the truncation marker.
            let other_len = serialized.len() - current.len();
            let budget = MAX_TOOL_RESULT_LEN.saturating_sub(other_len);
            if budget <= 48 {
                // Can't even fit a stub — drop the field entirely.
                obj.remove(&key);
            } else {
                let kept = safe_truncate_str(current, budget - 32);
                obj.insert(
                    key.clone(),
                    serde_json::json!(format!("{}... [truncated]", kept)),
                );
            }
            serialized = serde_json::to_string(&*obj).unwrap_or_default();
        }
        if serialized.len() <= MAX_TOOL_RESULT_LEN {
            return serialized;
        }
        // Final fallback: hard head-truncate (char-boundary-safe).
        return format!(
            "{}... [truncated, tool: {}]",
            safe_truncate_str(&serialized, MAX_TOOL_RESULT_LEN),
            tool_name
        );
    }
    // Fallback: hard truncate (char-boundary-safe to avoid panic on CJK/emoji)
    format!(
        "{}... [truncated, tool: {}]",
        safe_truncate_str(content, MAX_TOOL_RESULT_LEN),
        tool_name
    )
}

/// Truncate a string to at most `max_bytes` bytes, ending at a UTF-8 char
/// boundary. Without this, `&s[..N]` panics if N falls in the middle of a
/// multi-byte character (common with CJK text or emoji in tool output).
fn safe_truncate_str(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn strip_reasoning_tags(text: &str) -> String {
    let mut cleaned = text.to_string();
    for tag in OPEN_REASONING_TAGS
        .iter()
        .chain(CLOSE_REASONING_TAGS.iter())
    {
        cleaned = cleaned.replace(tag, "");
        cleaned = cleaned.replace(&tag.to_uppercase(), "");
    }
    cleaned
}

fn extract_tool_calls_from_choice(
    deltas: Option<Vec<crate::client::ToolCallDelta>>,
) -> Vec<ToolCall> {
    deltas
        .unwrap_or_default()
        .into_iter()
        .filter_map(|delta| {
            let function = delta.function?;
            Some(ToolCall {
                id: delta
                    .id
                    .unwrap_or_else(|| format!("call_choice_{}_{}", delta.index, function.name)),
                function,
            })
        })
        .collect()
}
pub(crate) fn merge_stream_tool_call(tool_calls: &mut Vec<ToolCall>, tool_call: ToolCall) {
    if let Some(existing) = tool_calls
        .iter_mut()
        .find(|existing| existing.id == tool_call.id)
    {
        if existing.function.name.is_empty() {
            existing.function.name = tool_call.function.name;
        }
        if !tool_call.function.arguments.is_empty() {
            existing
                .function
                .arguments
                .push_str(&tool_call.function.arguments);
        }
    } else {
        tool_calls.push(tool_call);
    }
}

#[derive(Default)]
struct ToolCallContentRouter {
    pending: String,
    inside_tool_call: bool,
}

impl ToolCallContentRouter {
    fn feed(&mut self, chunk: &str) -> String {
        self.pending.push_str(chunk);
        self.drain_ready(false)
    }

    fn finish(&mut self) -> String {
        self.drain_ready(true)
    }

    fn drain_ready(&mut self, flush_all: bool) -> String {
        const OPEN: &str = "<tool_call";
        const CLOSE: &str = "</tool_call";
        let mut content = String::new();

        loop {
            if self.inside_tool_call {
                if let Some(index) = find_ascii_case_insensitive(&self.pending, CLOSE) {
                    let close_end = self.pending[index..]
                        .find('>')
                        .map(|offset| index + offset + 1);
                    if let Some(close_end) = close_end {
                        self.pending.drain(..close_end);
                        self.inside_tool_call = false;
                        continue;
                    }
                }

                if flush_all {
                    self.pending.clear();
                }
                break;
            }

            if let Some(index) = find_ascii_case_insensitive(&self.pending, OPEN) {
                content.push_str(&self.pending[..index]);
                if let Some(open_end) = self.pending[index..]
                    .find('>')
                    .map(|offset| index + offset + 1)
                {
                    self.pending.drain(..open_end);
                    self.inside_tool_call = true;
                    continue;
                }

                self.pending.drain(..index);
                break;
            }

            let keep = if flush_all {
                0
            } else {
                longest_suffix_prefix_match_case_insensitive(&self.pending, OPEN)
            };
            let flush_len = self.pending.len().saturating_sub(keep);
            if flush_len == 0 {
                break;
            }

            content.push_str(&self.pending[..flush_len]);
            self.pending.drain(..flush_len);
            break;
        }

        content
    }
}

fn longest_suffix_prefix_match(value: &str, marker: &str) -> usize {
    let max = value.len().min(marker.len().saturating_sub(1));
    for len in (1..=max).rev() {
        if value.ends_with(&marker[..len]) {
            return len;
        }
    }
    0
}

fn longest_suffix_prefix_match_case_insensitive(value: &str, marker: &str) -> usize {
    let lowered = value.to_ascii_lowercase();
    longest_suffix_prefix_match(&lowered, marker)
}

fn find_ascii_case_insensitive(value: &str, marker: &str) -> Option<usize> {
    value.to_ascii_lowercase().find(marker)
}

fn strip_tool_call_markup(content: &str) -> String {
    let mut router = ToolCallContentRouter::default();
    let mut visible = router.feed(content);
    visible.push_str(&router.finish());
    visible
}

mod model_client;
pub use model_client::{ChatRequest, ModelClient, StreamChunk};

mod fallback;
pub use fallback::{ClassifiedError, FallbackModelClient};

mod pooled_client;
pub use pooled_client::PooledModelClient;

pub mod clients;

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use serial_test::serial;

    #[test]
    fn estimate_current_tokens_prefers_reported_usage() {
        // heuristic is used when the model has not reported usage yet
        assert_eq!(prefer_reported(0, 120), 120);
        // real reported prompt-token count wins over the heuristic
        assert_eq!(prefer_reported(5_000, 120), 5_000);
    }

    #[test]
    fn truncate_tool_result_preserves_skill_view_metadata() {
        // skill_view returns {name, description, content, tags,
        // supporting_files, path}. serde_json's BTreeMap ordering puts the
        // bulky "content" field first; a head-truncate would drop the
        // metadata. The JSON-aware truncation must keep metadata intact and
        // only bound the large content field.
        let big_content = "x".repeat(10_000);
        let result = serde_json::json!({
            "name": "arxiv",
            "description": "Search arXiv papers",
            "content": big_content,
            "tags": ["Research", "Academic"],
            "supporting_files": ["scripts/search.sh"],
            "path": "/tmp/skills/skills/arxiv/SKILL.md"
        });
        let truncated = truncate_tool_result("skill_view", &result.to_string());
        assert!(truncated.len() <= MAX_TOOL_RESULT_LEN + 128);
        // Metadata survives and remains machine-readable.
        assert!(truncated.contains("\"name\":\"arxiv\""));
        assert!(truncated.contains("Search arXiv papers"));
        assert!(truncated.contains("Research"));
        assert!(truncated.contains("scripts/search.sh"));
        // The content field is bounded, not lost entirely.
        assert!(truncated.contains("[truncated]"));
        assert!(truncated.contains("xxx"));
    }

    #[test]
    fn truncate_tool_result_keeps_small_results_untouched() {
        let small = serde_json::json!({"name": "arxiv", "content": "short"}).to_string();
        assert_eq!(truncate_tool_result("skill_view", &small), small);
    }

    #[test]
    fn truncate_tool_result_falls_back_for_non_json() {
        let big = "y".repeat(10_000);
        let truncated = truncate_tool_result("terminal", &big);
        assert!(truncated.len() <= MAX_TOOL_RESULT_LEN + 64);
        assert!(truncated.contains("[truncated, tool: terminal]"));
    }

    #[test]
    fn frozen_prefix_injects_skill_guidance_when_skill_manager_attached() {
        use crate::agent::clients::openai::OpenAIModelClient;
        use crate::client::OpenAIClient;

        let dir = tempfile::TempDir::new().unwrap();
        let skills_dir = dir.path().join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        let skill_dir = skills_dir.join("demo-skill");
        std::fs::create_dir(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: demo-skill\ndescription: A demo skill\n---\n\n# Demo\n\nInstructions.\n",
        )
        .unwrap();
        let mut skill_manager = SkillManager::new(skills_dir);
        skill_manager.load_all().unwrap();

        let db = Database::init(std::path::PathBuf::from("test_guidance.sqlite")).unwrap();
        let agent = OperantAgent::new(
            AgentConfig::default(),
            Box::new(OpenAIModelClient::new(OpenAIClient::new(
                crate::client::ClientConfig::default(),
            ))),
            ToolRegistry::new(Duration::from_secs(1)),
            Arc::new(db),
        )
        .with_skill_manager(skill_manager);

        let prefix = agent.build_frozen_prefix();
        // Skills index still listed (progressive disclosure tier 1).
        assert!(prefix.contains("<available_skills>"));
        assert!(prefix.contains("demo-skill"));
        // hermes SKILLS_GUIDANCE parity: the principles must ride along.
        assert!(prefix.contains("## Skill Management Principles"));
        assert!(prefix.contains("skill_manage"));
        assert!(prefix.contains("Skills that aren't maintained become liabilities"));
        // meta-skill parity: the routing contract rides the same prefix.
        assert!(prefix.contains("## Meta-Skill Routing"));
        assert!(prefix.contains("Route, don't do"));
        assert!(prefix.contains("skill_view(name='<parent>/<child>')"));
        assert!(prefix.contains("Regenerate the map after structural changes"));
        assert!(prefix.contains("check_only=true"));
        assert!(prefix.contains("## Skill Safety Rule"));
        assert!(prefix.contains("skill_view"));
    }

    #[test]
    fn frozen_prefix_omits_guidance_without_skill_manager() {
        use crate::agent::clients::openai::OpenAIModelClient;
        use crate::client::OpenAIClient;

        let db = Database::init(std::path::PathBuf::from("test_guidance_none.sqlite")).unwrap();
        let agent = OperantAgent::new(
            AgentConfig::default(),
            Box::new(OpenAIModelClient::new(OpenAIClient::new(
                crate::client::ClientConfig::default(),
            ))),
            ToolRegistry::new(Duration::from_secs(1)),
            Arc::new(db),
        );

        let prefix = agent.build_frozen_prefix();
        assert!(!prefix.contains("## Skill Management Principles"));
        assert!(!prefix.contains("Skill Safety Rule"));
    }

    #[tokio::test]
    async fn build_messages_injects_moa_guidance_once_and_drains() {
        use crate::agent::clients::openai::OpenAIModelClient;
        use crate::client::OpenAIClient;

        let db = Database::init(std::path::PathBuf::from("test_moa_guidance.sqlite")).unwrap();
        let agent = OperantAgent::new(
            AgentConfig::default(),
            Box::new(OpenAIModelClient::new(OpenAIClient::new(
                crate::client::ClientConfig::default(),
            ))),
            ToolRegistry::new(Duration::from_secs(1)),
            Arc::new(db),
        );
        agent.user_message("hi").await;

        // G5: no guidance set → byte-identical to a plain turn (no MoA msg).
        let msgs = agent.build_messages("moa-test").await.unwrap();
        assert!(
            !msgs
                .iter()
                .any(|m| m.role == Role::System && m.content.contains("Mixture of Agents")),
            "no MoA message when guidance unset"
        );

        agent.set_moa_guidance("[Mixture of Agents context — test guidance]".to_string());
        let msgs = agent.build_messages("moa-test").await.unwrap();
        let injected: Vec<_> = msgs
            .iter()
            .filter(|m| m.role == Role::System && m.content.contains("Mixture of Agents"))
            .collect();
        assert_eq!(injected.len(), 1, "guidance injected exactly once");

        // Drained — the next turn is byte-identical again (no leak).
        let msgs = agent.build_messages("moa-test").await.unwrap();
        assert!(
            !msgs
                .iter()
                .any(|m| m.role == Role::System && m.content.contains("Mixture of Agents")),
            "guidance drained after one turn"
        );
    }

    #[serial]
    #[tokio::test]
    async fn build_messages_injects_long_term_memory() {
        use crate::agent::clients::openai::OpenAIModelClient;
        use crate::client::OpenAIClient;

        let memory_manager = MemoryManager::new();
        memory_manager
            .store(
                crate::memory::MemoryBlock::new("fact1", "fact", "User prefers concise answers")
                    .importance(80),
            )
            .await;

        let db = Database::init(std::path::PathBuf::from("test_db.sqlite")).unwrap();
        let agent = OperantAgent::new(
            AgentConfig::default(),
            Box::new(OpenAIModelClient::new(OpenAIClient::new(
                crate::client::ClientConfig::default(),
            ))),
            ToolRegistry::new(Duration::from_secs(1)),
            Arc::new(db),
        )
        .with_memory_manager(memory_manager);

        let messages = agent.build_messages("test-session").await.unwrap();
        // iter-39: the system prompt is now split into a frozen prefix
        // (base prompt + skills) and a volatile suffix (memory + workspace
        // context). Long-term memory lands in the second system message,
        // not the first. Concatenate all system message content to check.
        let system: String = messages
            .iter()
            .filter(|m| m.role == crate::client::Role::System)
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(system.contains("<long_term_memory>"));
        assert!(system.contains("[fact] User prefers concise answers"));
        assert!(system.contains("</long_term_memory>"));
    }
    #[serial]
    #[tokio::test]
    async fn lcm_engine_injects_auto_recall_evidence_into_build_messages() {
        // P3 end-to-end: with the LCM engine attached (context_engine=lcm),
        // build_messages() auto-recalls relevant prior DAG content and injects
        // it as a system evidence block — no manual lcm_recall needed.
        use crate::agent::clients::openai::OpenAIModelClient;
        use crate::client::OpenAIClient;
        use crate::context::{ContextEngine, LcmContextEngine};

        let dir =
            std::env::temp_dir().join(format!("operant_lcm_agent_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let engine = LcmContextEngine::new(crate::context::LcmConfig {
            db_path: dir.join("lcm.db"),
            tail_tokens: 12_000,
            auto_recall: true,
            auto_recall_limit: 3,
            auto_recall_max_chars: 4_000,
            rollups_inject: true,
            ignore_session_patterns: Vec::new(),
            readonly_sessions: Vec::new(),
        })
        .unwrap();

        // Seed the DAG with a prior-turn fact (session-scoped).
        engine
            .ingest_turn(
                "agent-lcm-test",
                &[crate::client::Message::user(
                    "the launch date for project Phoenix is September 14th, 2027",
                )],
            )
            .await
            .unwrap();

        let db = Database::init(std::path::PathBuf::from("test_db_lcm.sqlite")).unwrap();
        let agent = OperantAgent::new(
            AgentConfig::default(),
            Box::new(OpenAIModelClient::new(OpenAIClient::new(
                crate::client::ClientConfig::default(),
            ))),
            ToolRegistry::new(Duration::from_secs(1)),
            Arc::new(db),
        )
        .with_persistent_session("agent-lcm-test".to_string())
        .with_context_engine(Arc::new(engine));

        // A fresh turn asking about the fact — the agent has NOT seen the
        // prior turn in its own conversation, only the DAG knows it.
        // (Mirrors turn_context: the user query is added to the conversation
        // before build_messages runs, so auto-recall has a query to use.)
        agent
            .add_message(crate::client::Message::user(
                "What is the launch date for project Phoenix?",
            ))
            .await;
        let messages = agent.build_messages("agent-lcm-test").await.unwrap();
        let system: String = messages
            .iter()
            .filter(|m| m.role == crate::client::Role::System)
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            system.contains("LCM recalled evidence"),
            "auto-recall evidence block must be injected, got system: {system:?}"
        );
        assert!(
            system.contains("September 14th, 2027"),
            "evidence must carry the recalled fact, got system: {system:?}"
        );
    }

    #[serial]
    #[tokio::test]
    async fn lcm_engine_injects_stored_rollup_into_build_messages() {
        // P1 end-to-end: with the LCM engine attached and a stored rollup in
        // lcm_rollups, an over-budget build_messages() must inject the rollup
        // summary block into the assembled context (auto-recall OFF so the
        // rollup is the ONLY path the fact can reach the model).
        use crate::agent::clients::openai::OpenAIModelClient;
        use crate::client::OpenAIClient;
        use crate::context::{ContextEngine, LcmConfig, LcmContextEngine, rollup};

        let dir =
            std::env::temp_dir().join(format!("operant_lcm_rollup_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let engine = LcmContextEngine::new(LcmConfig {
            db_path: dir.join("lcm.db"),
            // Tiny tail so the context always overflows and compacts.
            tail_tokens: 10,
            auto_recall: false,
            auto_recall_limit: 3,
            auto_recall_max_chars: 4_000,
            rollups_inject: true,
            ignore_session_patterns: Vec::new(),
            readonly_sessions: Vec::new(),
        })
        .unwrap();

        // Seed the DAG with a prior-turn fact, then build a real stored
        // rollup over it (echo summarizer → summary contains the fact).
        engine
            .ingest_turn(
                "agent-rollup-test",
                &[
                    crate::client::Message::user(
                        "the deploy freeze window is every Friday after 3pm UTC",
                    ),
                    crate::client::Message::assistant("noted: freeze starts Friday 15:00 UTC"),
                ],
            )
            .await
            .unwrap();
        let echo = |t: String| async move { Ok(format!("ROLLUP[{t}]")) };
        rollup::build_rollup(
            &engine,
            "agent-rollup-test",
            rollup::RollupPeriod::Day,
            None,
            echo,
        )
        .await
        .unwrap()
        .expect("rollup built");

        // Small context window forces compaction: effective budget is
        // context_window - 4096 (response reserve), and the frozen prefix +
        // workspace context alone far exceeds that, so assemble() must compact
        // and inject the stored rollup.
        let agent_cfg = AgentConfig {
            context_window: 8_000,
            ..AgentConfig::default()
        };
        let db = Database::init(std::path::PathBuf::from("test_db_lcm_rollup.sqlite")).unwrap();
        let agent = OperantAgent::new(
            agent_cfg,
            Box::new(OpenAIModelClient::new(OpenAIClient::new(
                crate::client::ClientConfig::default(),
            ))),
            ToolRegistry::new(Duration::from_secs(1)),
            Arc::new(db),
        )
        .with_persistent_session("agent-rollup-test".to_string())
        .with_context_engine(Arc::new(engine));

        // A fresh user turn — the conversation is small but the frozen system
        // prefix plus the turn pushes it over the 10-token tail budget, so
        // compaction fires and the stored rollup is injected.
        agent
            .add_message(crate::client::Message::user(
                "When does the deploy freeze start?",
            ))
            .await;
        let messages = agent.build_messages("agent-rollup-test").await.unwrap();
        let system: String = messages
            .iter()
            .filter(|m| m.role == crate::client::Role::System)
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            system.contains("LCM rollups of earlier context"),
            "rollup block must be injected, got system: {system:?}"
        );
        assert!(
            system.contains("deploy freeze window is every Friday"),
            "rollup summary must carry the stored fact, got system: {system:?}"
        );
        // The D0 fresh tail must never be starved by the injected rollup:
        // the user's own turn is the freshest content and must survive.
        let all_content: String = messages
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            all_content.contains("When does the deploy freeze start?"),
            "freshest user turn must survive compaction, got: {all_content:?}"
        );
    }

    #[test]
    fn think_router_splits_inline_think_blocks() {
        let mut router = ThinkBlockRouter::default();
        let (content_a, reasoning_a) = router.feed("Hello<think>plan");
        let (content_b, reasoning_b) = router.feed(" more</think> world");
        let (content_c, reasoning_c) = router.finish();

        assert_eq!(content_a, "Hello");
        assert_eq!(reasoning_a, "");
        assert_eq!(content_b, "");
        assert_eq!(reasoning_b, "plan more");
        assert_eq!(content_c, " world");
        assert_eq!(reasoning_c, "");
    }

    #[test]
    fn strip_reasoning_tags_removes_supported_markers() {
        assert_eq!(
            strip_reasoning_tags(
                "<think>abc</think><REASONING_SCRATCHPAD>def</REASONING_SCRATCHPAD>"
            ),
            "abcdef"
        );
    }

    #[test]
    fn think_router_does_not_split_multibyte_characters() {
        let mut router = ThinkBlockRouter::default();
        let (_content, _reasoning) = router.feed("Halo! 🧑‍💻 Senang bertemu");
        let (_content, _reasoning) = router.finish();
    }

    #[test]
    fn think_router_falls_back_to_content_for_unclosed_reasoning() {
        let mut router = ThinkBlockRouter::default();
        let (content, reasoning) = router.feed("<think>Visible answer");
        let (rest_content, rest_reasoning) = router.finish();

        assert_eq!(content, "");
        assert_eq!(reasoning, "");
        assert_eq!(rest_content, "Visible answer");
        assert_eq!(rest_reasoning, "Visible answer");
    }

    #[test]
    fn tool_call_router_hides_xml_from_visible_content() {
        let mut router = ToolCallContentRouter::default();

        let first = router.feed("Before <tool_call>{\"name\":\"datetime\"}");
        let second = router.feed("{\"arguments\":{}}</tool_call> after");
        let rest = router.finish();

        assert_eq!(first, "Before ");
        assert_eq!(second, " after");
        assert_eq!(rest, "");
    }

    #[test]
    fn tool_call_router_keeps_plain_text_streaming() {
        let mut router = ToolCallContentRouter::default();

        let first = router.feed("Halo ");
        let second = router.feed("operant!");
        let rest = router.finish();

        assert_eq!(first, "Halo ");
        assert_eq!(second, "operant!");
        assert_eq!(rest, "");
    }

    #[test]
    fn extract_tool_calls_from_choice_handles_non_streaming_calls() {
        let tool_calls = extract_tool_calls_from_choice(Some(vec![crate::client::ToolCallDelta {
            index: 0,
            id: Some("call_1".to_string()),
            call_type: Some("function".to_string()),
            function: Some(crate::client::ToolCallFunction {
                name: "datetime".to_string(),
                arguments: "{\"timezone\":\"UTC\"}".to_string(),
            }),
        }]));

        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].id, "call_1");
        assert_eq!(tool_calls[0].function.name, "datetime");
    }

    #[test]
    fn extract_tool_calls_from_choice_ignores_empty_entries() {
        let tool_calls = extract_tool_calls_from_choice(Some(vec![crate::client::ToolCallDelta {
            index: 0,
            id: None,
            call_type: None,
            function: None,
        }]));

        assert!(tool_calls.is_empty());
    }

    #[test]
    fn merge_stream_tool_call_appends_incremental_arguments() {
        let mut tool_calls = vec![ToolCall {
            id: "call_0_datetime".to_string(),
            function: crate::client::ToolCallFunction {
                name: "datetime".to_string(),
                arguments: "{\"format\":".to_string(),
            },
        }];

        merge_stream_tool_call(
            &mut tool_calls,
            ToolCall {
                id: "call_0_datetime".to_string(),
                function: crate::client::ToolCallFunction {
                    name: "datetime".to_string(),
                    arguments: "\"%Y-%m-%d\"}".to_string(),
                },
            },
        );

        assert_eq!(tool_calls.len(), 1);
        assert_eq!(
            tool_calls[0].function.arguments,
            "{\"format\":\"%Y-%m-%d\"}"
        );
    }

    #[test]
    fn tool_call_router_hides_split_tool_call_open_tag() {
        let mut router = ToolCallContentRouter::default();

        let first = router.feed("Before <tool_ca");
        let second = router.feed("ll>{\"name\":\"datetime\"}</tool_call> after");
        let rest = router.finish();

        assert_eq!(first, "Before ");
        assert_eq!(second, " after");
        assert_eq!(rest, "");
    }

    #[serial]
    #[tokio::test]
    async fn process_response_parses_xml_tool_calls_in_non_stream_mode() {
        use crate::agent::clients::openai::OpenAIModelClient;
        use crate::client::OpenAIClient;

        let db = Database::init(std::path::PathBuf::from("test_db_resp.sqlite")).unwrap();
        let agent = OperantAgent::new(
            AgentConfig::default(),
            Box::new(OpenAIModelClient::new(OpenAIClient::new(
                crate::client::ClientConfig::default(),
            ))),
            ToolRegistry::new(Duration::from_secs(1)),
            Arc::new(db),
        );

        let response = ChatResponse {
            id: "resp_1".to_string(),
            object: "chat.completion".to_string(),
            created: 0,
            model: "demo".to_string(),
            choices: vec![crate::client::Choice {
                index: 0,
                message: crate::client::MessageDelta {
                    role: Some(crate::client::Role::Assistant),
                    content: Some(
                        "<tool_call>{\"name\":\"datetime\",\"arguments\":\"{}\"}</tool_call>"
                            .to_string(),
                    ),
                    reasoning_content: Some("need tool".to_string()),
                    tool_calls: None,
                },
                finish_reason: Some("stop".to_string()),
            }],
            usage: crate::client::Usage {
                prompt_tokens: 1,
                completion_tokens: 1,
                total_tokens: 2,
            },
        };

        let (content, reasoning, tool_calls, _finish_reason) =
            agent.process_response(response).await.unwrap();

        assert_eq!(content, "");
        assert_eq!(reasoning, "need tool");
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].function.name, "datetime");
    }

    // ── iter-330: mid-stream drop recovery (hermes parity) ─────────────

    /// Mock client whose first streaming attempt dies mid-read with a
    /// transport error (like a provider closing the SSE connection before
    /// the body completes) and whose second attempt succeeds. Used to verify
    /// the run() loop re-issues the request instead of aborting the turn.
    struct DropThenOkClient {
        calls: std::sync::atomic::AtomicUsize,
    }

    #[async_trait]
    impl ModelClient for DropThenOkClient {
        fn provider_name(&self) -> &str {
            "mock-drop-then-ok"
        }

        async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse> {
            Err(Error::Agent("non-streaming not used in drop test".into()))
        }

        async fn chat_streaming(
            &self,
            _request: ChatRequest,
        ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
            let call = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if call == 0 {
                // First call: a stream that immediately yields a transport
                // error (reqwest "error decoding response body" analogue).
                let err = reqwest::Client::new()
                    .get("http://127.0.0.1:9/")
                    .send()
                    .await
                    .unwrap_err();
                let stream = futures::stream::once(async move { Err(Error::Network(err.into())) });
                Ok(Box::pin(stream))
            } else {
                // Second call: a valid stream with final content.
                let stream = futures::stream::iter(vec![Ok(StreamChunk::new(
                    Some("retried answer".to_string()),
                    None,
                    None,
                ))]);
                Ok(Box::pin(stream))
            }
        }
    }

    #[tokio::test]
    async fn run_retries_mid_stream_drop_and_succeeds() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db = Database::init(temp_dir.path().join("drop_test.sqlite")).unwrap();

        let config = AgentConfig {
            model: "demo".to_string(),
            max_iterations: 3,
            tool_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(10),
            system_prompt: Some("You are a test agent.".to_string()),
            stream: true,
            context_window: 8000,
            max_healing_attempts: 1,
            fallback_models: Vec::new(),
            fallback_on_errors: false,
            approval_mode: "off".to_string(),
            approval_allowlist: Vec::new(),
            approval_allowlist_path: None,
            record_trajectories: false,
            skill_nudge_interval: 0,
            memory_review_interval: 0,
            max_retries: 3,
            tool_search: Default::default(),
        };
        let agent = OperantAgent::new(
            config,
            Box::new(DropThenOkClient {
                calls: std::sync::atomic::AtomicUsize::new(0),
            }),
            ToolRegistry::new(Duration::from_secs(1)),
            Arc::new(db),
        );

        let result = agent
            .run("hello".to_string())
            .await
            .expect("run() should retry the mid-stream drop and return the retried answer");
        assert_eq!(result.content, "retried answer");
    }

    /// The retry-metrics aggregation hook must bump the shared counters at
    /// the same points the loop logs its stream-drop warnings — this is what
    /// the TUI status pill renders. Guards the runtime_metrics wiring end to
    /// end through a real run() (drop once, retry, succeed).
    #[tokio::test]
    async fn run_records_stream_retry_metrics() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db = Database::init(temp_dir.path().join("metrics_test.sqlite")).unwrap();

        let config = AgentConfig {
            model: "demo".to_string(),
            max_iterations: 3,
            tool_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(10),
            system_prompt: Some("You are a test agent.".to_string()),
            stream: true,
            context_window: 8000,
            max_healing_attempts: 1,
            fallback_models: Vec::new(),
            fallback_on_errors: false,
            approval_mode: "off".to_string(),
            approval_allowlist: Vec::new(),
            approval_allowlist_path: None,
            record_trajectories: false,
            skill_nudge_interval: 0,
            memory_review_interval: 0,
            max_retries: 3,
            tool_search: Default::default(),
        };
        let agent = OperantAgent::new(
            config,
            Box::new(DropThenOkClient {
                calls: std::sync::atomic::AtomicUsize::new(0),
            }),
            ToolRegistry::new(Duration::from_secs(1)),
            Arc::new(db),
        );

        let result = agent
            .run("hello".to_string())
            .await
            .expect("run() should survive the drop and succeed");
        assert_eq!(result.content, "retried answer");

        // The aggregation hook must have recorded exactly one drop + one
        // re-issue (the second chat_streaming call succeeded cleanly).
        let snap = agent.metrics().snapshot();
        assert_eq!(snap.stream_drops, 1, "one mid-stream drop recorded");
        assert_eq!(snap.stream_retries, 1, "one re-issue recorded");
        assert!(snap.has_any());
        assert!(snap.last_stream_retry_at > 0, "retry timestamp set");
        // Memory and empty-content counters stay untouched on this path.
        assert_eq!(snap.memory_sync_failures, 0);
        assert_eq!(snap.empty_content_retries, 0);
    }

    /// Mock client whose first streaming attempt dies mid-read with a
    /// rotate-classified error (a 429 chunk after the connection was
    /// established) and whose subsequent attempts succeed. Used to verify
    /// that the run() loop's mid-stream recovery re-issues the request and
    /// that the pooled client rotates to the next key on the retry.
    struct DropRotateThenOkClient {
        calls: std::sync::atomic::AtomicUsize,
    }

    #[async_trait]
    impl ModelClient for DropRotateThenOkClient {
        fn provider_name(&self) -> &str {
            "mock-drop-rotate-then-ok"
        }

        async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse> {
            Err(Error::Agent("non-streaming not used in drop test".into()))
        }

        async fn chat_streaming(
            &self,
            _request: ChatRequest,
        ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
            let call = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if call == 0 {
                // First call: a stream that yields a mid-stream 429 chunk.
                let stream = futures::stream::iter(vec![Err(Error::RateLimited {
                    retry_after: Duration::from_secs(5),
                })]);
                Ok(Box::pin(stream))
            } else {
                // Subsequent calls: a valid stream with final content.
                let stream = futures::stream::iter(vec![Ok(StreamChunk::new(
                    Some("rotated answer".to_string()),
                    None,
                    None,
                ))]);
                Ok(Box::pin(stream))
            }
        }
    }

    #[tokio::test]
    async fn run_retries_mid_stream_rotate_and_rotates_credential() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db = Database::init(temp_dir.path().join("rotate_drop_test.sqlite")).unwrap();

        // Two-key pool shared by the pooled client AND the agent: the
        // mid-stream 429 benches k1 (via the pooled stream wrapper), so the
        // re-issued request must rotate to k2.
        let pool = std::sync::Arc::new(crate::credential_pool::CredentialPool::new("demo"));
        pool.add(crate::credential_pool::PooledCredential::new(
            "k1",
            crate::credential_pool::AuthType::ApiKey,
            "key-1",
            "test",
        ));
        pool.add(crate::credential_pool::PooledCredential::new(
            "k2",
            crate::credential_pool::AuthType::ApiKey,
            "key-2",
            "test",
        ));

        let config = AgentConfig {
            model: "demo".to_string(),
            max_iterations: 3,
            tool_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(10),
            system_prompt: Some("You are a test agent.".to_string()),
            stream: true,
            context_window: 8000,
            max_healing_attempts: 1,
            fallback_models: Vec::new(),
            fallback_on_errors: false,
            approval_mode: "off".to_string(),
            approval_allowlist: Vec::new(),
            approval_allowlist_path: None,
            record_trajectories: false,
            skill_nudge_interval: 0,
            memory_review_interval: 0,
            max_retries: 3,
            tool_search: Default::default(),
        };
        let client: Box<dyn ModelClient> = Box::new(PooledModelClient::new(
            std::sync::Arc::new(DropRotateThenOkClient {
                calls: std::sync::atomic::AtomicUsize::new(0),
            }),
            pool.clone(),
        ));
        let agent = OperantAgent::new(
            config,
            client,
            ToolRegistry::new(Duration::from_secs(1)),
            Arc::new(db),
        )
        .with_credential_pool(pool.clone());

        let result = agent
            .run("hello".to_string())
            .await
            .expect("run() should retry the mid-stream 429 and succeed on the rotated key");
        assert_eq!(result.content, "rotated answer");
        // k1 is benched (mid-stream 429), k2 carried the retry.
        let available: Vec<String> = pool
            .list()
            .into_iter()
            .filter(|c| c.is_available())
            .map(|c| c.name)
            .collect();
        assert_eq!(
            available,
            vec!["k2".to_string()],
            "rotation fired on the mid-stream retry"
        );
    }

    #[test]
    fn tool_permission_response_variants() {
        let allow_once = ToolPermissionResponse::AllowOnce;
        let allow_session = ToolPermissionResponse::AllowSession;
        let always = ToolPermissionResponse::AllowAlways;
        let deny = ToolPermissionResponse::Deny;

        assert_eq!(allow_once, ToolPermissionResponse::AllowOnce);
        assert_eq!(allow_session, ToolPermissionResponse::AllowSession);
        assert_eq!(always, ToolPermissionResponse::AllowAlways);
        assert_eq!(deny, ToolPermissionResponse::Deny);
        assert_ne!(allow_once, deny);
        assert_ne!(always, allow_session);
    }

    #[test]
    fn interactive_tools_exempt_from_generic_timeout() {
        // clarify / approval_request block waiting for a human — they must
        // never be wrapped in the generic tool timeout (the gateway showed
        // "Tool timed out after 30s" and killed the dialog before the user
        // could tap). Everything else keeps the hard timeout.
        assert!(is_interactive_tool("clarify"));
        assert!(is_interactive_tool("approval_request"));
        // Long-running tools (delegate_task spawns a child agent with its own
        // 600s default timeout) must not be killed by the 30s generic tool
        // timeout — the live loop reported "Timed out after 30s" on
        // delegation before the exemption existed.
        assert!(is_long_running_tool("delegate_task"));
        assert!(!is_long_running_tool("terminal"));
        assert!(!is_long_running_tool("web_search"));
        assert!(!is_interactive_tool("bash"));
        assert!(!is_interactive_tool("code_execution"));
        assert!(!is_interactive_tool("web_search"));
    }

    #[test]
    fn allowlist_pattern_matching() {
        // Exact match.
        assert!(allowlist_pattern_matches(
            "code_execution",
            "code_execution"
        ));
        // `*` glob — prefix and suffix wildcards.
        assert!(allowlist_pattern_matches("file_*", "file_write"));
        assert!(allowlist_pattern_matches("file_*", "file_read"));
        assert!(!allowlist_pattern_matches("file_*", "browser"));
        assert!(allowlist_pattern_matches("*_search", "memory_search"));
        assert!(!allowlist_pattern_matches("*_search", "memory_store"));
        // `?` single-character wildcard.
        assert!(allowlist_pattern_matches("bash?", "bashx"));
        assert!(!allowlist_pattern_matches("bash?", "bash"));
        // A plain pattern (no glob chars) must not substring-match.
        assert!(!allowlist_pattern_matches("file", "file_write"));
        // Mid-pattern star.
        assert!(allowlist_pattern_matches(
            "mcp_*_tool",
            "mcp_management_tool"
        ));
    }

    #[test]
    fn persistent_allowlist_round_trips_to_disk() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("approval_allowlist.json");

        let config = AgentConfig {
            approval_allowlist_path: Some(path.clone()),
            approval_allowlist: vec!["code_execution".to_string(), "file_*".to_string()],
            ..AgentConfig::default()
        };

        // Config seeds + persisted patterns both load on construction.
        let loaded = approval_allowlist_from_config(&config);
        assert!(loaded.contains("code_execution"));
        assert!(loaded.contains("file_*"));

        // The AllowAlways path: extend the set with a new tool and persist.
        let mut patterns = loaded;
        patterns.insert("browser".to_string());
        persist_approval_allowlist(Some(&path), &patterns);

        // A fresh config (same path, no seeds) must reload the persisted
        // pattern — proving the "always allow" choice survives restarts.
        let fresh = AgentConfig {
            approval_allowlist_path: Some(path.clone()),
            ..AgentConfig::default()
        };
        let reloaded = approval_allowlist_from_config(&fresh);
        assert!(
            reloaded.contains("browser"),
            "persisted pattern must reload"
        );
        assert!(reloaded.contains("code_execution"));
    }

    #[test]
    fn agent_event_tool_permission_request_variant() {
        let event = AgentEvent::ToolPermissionRequest {
            tool_name: "terminal".to_string(),
            tool_id: "call_1".to_string(),
            description: "Execute terminal tool".to_string(),
            danger_explanation: "This runs a shell command".to_string(),
            input_preview: Some("ls -la".to_string()),
        };

        match event {
            AgentEvent::ToolPermissionRequest {
                tool_name, tool_id, ..
            } => {
                assert_eq!(tool_name, "terminal");
                assert_eq!(tool_id, "call_1");
            }
            _ => panic!("Expected ToolPermissionRequest variant"),
        }
    }

    // ── R2 loop-budget helpers (request_timeout ceiling + reasoning floor) ──

    fn test_agent_with_request_timeout(secs: u64) -> OperantAgent {
        use crate::agent::clients::openai::OpenAIModelClient;
        use crate::client::OpenAIClient;
        static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let config = AgentConfig {
            request_timeout: Duration::from_secs(secs),
            ..AgentConfig::default()
        };
        // Unique per call — parallel tests must never share the SQLite file.
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let db = Database::init(std::path::PathBuf::from(format!(
            "test_loop_timeout_{}_{n}.sqlite",
            std::process::id()
        )))
        .unwrap();
        OperantAgent::new(
            config,
            Box::new(OpenAIModelClient::new(OpenAIClient::new(
                crate::client::ClientConfig::default(),
            ))),
            ToolRegistry::new(Duration::from_secs(1)),
            Arc::new(db),
        )
    }

    #[test]
    fn loop_request_timeout_uses_configured_budget() {
        let mut agent = test_agent_with_request_timeout(30);
        // Demo model has no reasoning floor → the loop budget is exactly the
        // configured request_timeout.
        agent.config.model = "demo".to_string();
        assert_eq!(agent.loop_request_timeout(), Duration::from_secs(30));
    }

    #[test]
    fn loop_request_timeout_raised_to_reasoning_floor() {
        let mut agent = test_agent_with_request_timeout(30);
        // A known reasoning model with a 300s floor must raise the loop
        // budget above the configured 30s — the floor is a FLOOR, applied as
        // max(configured, floor) so long-thinking models are never killed by
        // the loop ceiling.
        agent.config.model = "openai/o3-mini".to_string();
        assert_eq!(agent.loop_request_timeout(), Duration::from_secs(300));
    }

    #[test]
    fn loop_request_timeout_never_lowers_configured_budget() {
        let mut agent = test_agent_with_request_timeout(600);
        // Configured 600s stays 600s even for a reasoning model with a 300s
        // floor (max wins, never min).
        agent.config.model = "openai/o3-mini".to_string();
        assert_eq!(agent.loop_request_timeout(), Duration::from_secs(600));
    }

    #[tokio::test]
    async fn call_with_loop_timeout_returns_result_on_time() {
        let agent = test_agent_with_request_timeout(5);
        let out = agent
            .call_with_loop_timeout(async { Ok::<_, crate::error::Error>("fast".to_string()) })
            .await
            .unwrap();
        assert_eq!(out, "fast");
    }

    #[tokio::test]
    async fn call_with_loop_timeout_expires_and_errors() {
        let agent = test_agent_with_request_timeout(1);
        let err = agent
            .call_with_loop_timeout(async {
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                Ok::<_, crate::error::Error>("too slow".to_string())
            })
            .await
            .unwrap_err();
        // The expired call surfaces as a retryable Agent error with the
        // budget in the message (its "timed out" text also feeds the R2
        // thinking-timeout detection for reasoning models).
        let msg = err.to_string();
        assert!(msg.contains("timed out"), "got: {msg}");
        assert!(msg.contains("loop request_timeout ceiling"), "got: {msg}");
    }

    #[tokio::test]
    async fn call_with_loop_timeout_propagates_underlying_error() {
        let agent = test_agent_with_request_timeout(5);
        let err = agent
            .call_with_loop_timeout(async {
                Err::<(), crate::error::Error>(crate::error::Error::Agent("boom".to_string()))
            })
            .await
            .unwrap_err();
        assert!(err.to_string().contains("boom"));
    }

    // ── T2: interrupt aborts the in-flight request ─────────────────────────

    #[tokio::test]
    async fn call_with_loop_timeout_aborts_on_interrupt() {
        let agent = test_agent_with_request_timeout(30);
        agent.interrupt_flag.trigger();
        let err = agent
            .call_with_loop_timeout(async {
                // Long future — must be aborted by the interrupt branch, not
                // by the 30s budget.
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                Ok::<_, crate::error::Error>("too slow".to_string())
            })
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("Interrupted"), "got: {msg}");
        assert!(msg.contains("aborted"), "got: {msg}");
    }

    #[tokio::test]
    async fn call_with_loop_timeout_untouched_when_flag_clear() {
        let agent = test_agent_with_request_timeout(5);
        let out = agent
            .call_with_loop_timeout(async { Ok::<_, crate::error::Error>("ok".to_string()) })
            .await
            .unwrap();
        assert_eq!(out, "ok");
    }

    // ── T1: finish_reason plumbing through process_stream ──────────────────

    #[tokio::test]
    async fn process_stream_captures_finish_reason() {
        let agent = test_agent_with_request_timeout(5);
        use crate::agent::model_client::StreamChunk;

        let chunks: Vec<std::result::Result<StreamChunk, crate::error::Error>> = vec![
            Ok(StreamChunk::new(
                Some("partial answer ".to_string()),
                None,
                None,
            )),
            Ok(StreamChunk {
                content: Some("cut off".to_string()),
                reasoning: None,
                tool_calls: None,
                extra_content: None,
                usage: None,
                finish_reason: Some("length".to_string()),
            }),
        ];
        let stream: BoxStream<'static, Result<StreamChunk>> =
            Box::pin(futures::stream::iter(chunks));

        let (text, _reasoning, tcs, _extra, finish_reason) =
            agent.process_stream(stream).await.unwrap();
        assert_eq!(text, "partial answer cut off");
        assert!(tcs.is_empty());
        assert_eq!(finish_reason.as_deref(), Some("length"));
    }

    #[tokio::test]
    async fn process_stream_finish_reason_none_when_absent() {
        let agent = test_agent_with_request_timeout(5);
        use crate::agent::model_client::StreamChunk;

        let chunks: Vec<std::result::Result<StreamChunk, crate::error::Error>> =
            vec![Ok(StreamChunk::new(Some("hello".to_string()), None, None))];
        let stream: BoxStream<'static, Result<StreamChunk>> =
            Box::pin(futures::stream::iter(chunks));
        let (_t, _r, _tcs, _e, finish_reason) = agent.process_stream(stream).await.unwrap();
        assert!(finish_reason.is_none());
    }

    // ── T6: within-batch tool-call dedupe ──────────────────────────────────

    #[tokio::test]
    async fn execute_tools_dedupes_identical_batch_calls() {
        use crate::tools::debug_helpers::EchoTool;

        let config = AgentConfig::default();
        let registry = ToolRegistry::new(Duration::from_secs(1));
        registry.register(EchoTool).await.unwrap();
        let db = Database::init(std::path::PathBuf::from(format!(
            "test_db_dedupe_{}.sqlite",
            std::process::id()
        )))
        .unwrap();
        let agent = OperantAgent::new(
            config,
            Box::new(crate::agent::clients::openai::OpenAIModelClient::new(
                crate::client::OpenAIClient::new(crate::client::ClientConfig::default()),
            )),
            registry,
            Arc::new(db),
        );

        let mk = |id: &str, args: &str| ToolCall {
            id: id.to_string(),
            function: crate::client::ToolCallFunction {
                name: "echo".to_string(),
                arguments: args.to_string(),
            },
        };
        // Three calls: two identical + one different. The duplicate must be
        // skipped with a synthetic result; ordering preserved.
        let results = agent
            .execute_tools(vec![
                mk("c1", r#"{"message":"hi"}"#),
                mk("c2", r#"{"message":"hi"}"#),
                mk("c3", r#"{"message":"bye"}"#),
            ])
            .await
            .unwrap();
        assert_eq!(results.len(), 3);
        assert!(
            results[0].success,
            "first occurrence executes: {:?}",
            results[0]
        );
        let dup_err = results[1].error.as_deref().unwrap_or_default();
        assert!(
            dup_err.contains("Duplicate tool call"),
            "duplicate must be skipped: {:?}",
            results[1]
        );
        assert!(
            results[2].success,
            "different args still execute: {:?}",
            results[2]
        );
    }

    #[tokio::test]
    async fn execute_tools_does_not_dedupe_different_args() {
        use crate::tools::debug_helpers::EchoTool;

        let config = AgentConfig::default();
        let registry = ToolRegistry::new(Duration::from_secs(1));
        registry.register(EchoTool).await.unwrap();
        let db = Database::init(std::path::PathBuf::from(format!(
            "test_db_nodedupe_{}.sqlite",
            std::process::id()
        )))
        .unwrap();
        let agent = OperantAgent::new(
            config,
            Box::new(crate::agent::clients::openai::OpenAIModelClient::new(
                crate::client::OpenAIClient::new(crate::client::ClientConfig::default()),
            )),
            registry,
            Arc::new(db),
        );
        let mk = |id: &str, args: &str| ToolCall {
            id: id.to_string(),
            function: crate::client::ToolCallFunction {
                name: "echo".to_string(),
                arguments: args.to_string(),
            },
        };
        let results = agent
            .execute_tools(vec![
                mk("c1", r#"{"message":"a"}"#),
                mk("c2", r#"{"message":"b"}"#),
            ])
            .await
            .unwrap();
        assert_eq!(results.len(), 2);
        assert!(results[0].success && results[1].success);
    }
}
