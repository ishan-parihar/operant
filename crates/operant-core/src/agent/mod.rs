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

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{RwLock, mpsc};

use crate::client::{Message, Role, ToolCall};
use crate::config::{BehaviorSettings, runtime_config};
use crate::database::Database;
use crate::memory::MemoryManager;
use crate::observer::Observer;
use crate::skills::SkillManager;
use crate::tools::{ToolRegistry, ToolResult};

use self::background_review::SelfEvolutionState;
use self::iteration_budget::IterationBudget;

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
    /// Set of tool-call IDs for which `AgentEvent::ToolStart` was already
    /// emitted during `process_stream` (streaming XML extraction). `execute_tools`
    /// checks this set and skips emitting duplicate `ToolStart` events — so the
    /// gateway runner sees exactly ONE `ToolStart` per tool call, arriving during
    /// streaming (not after the turn finishes), enabling chronological message
    /// splitting. Cleared at the start of each `run()`.
    stream_emitted_tool_starts: std::sync::Mutex<std::collections::HashSet<String>>,
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
    matches!(
        name,
        "delegate_task"
            | "aft_bash"
            | "aft_read"
            | "aft_write"
            | "aft_edit"
            | "aft_glob"
            | "aft_grep"
            | "aft_search"
            | "aft_ast_search"
            | "aft_outline"
            | "aft_zoom"
            | "aft_callers"
            | "aft_apply_patch"
    )
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

// Method-group impl blocks extracted from the former 4.1K-line impl OperantAgent.
mod builders;
mod compress;
mod events;
mod prompting;
mod run;
mod stream;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::model_client::ChatRequest;
    use crate::client::ChatResponse;
    use crate::error::{Error, Result};
    use async_trait::async_trait;
    use futures::stream::BoxStream;
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
