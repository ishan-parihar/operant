//! Operant Agent orchestration loop with self-healing
//!
//! Implements the ReAct (Reason + Act) pattern for LLM-driven tool execution.
//! Includes the self-evolution pipeline: skill nudge counter, iteration budget,
//! turn finalizer, and background review daemon for autonomous skill/memory
//! improvement after each turn.

pub(crate) mod background_review;
pub mod chat_provider;
pub mod error_classifier;
pub mod insights;
pub mod iteration_budget;
pub mod learn_prompt;
pub mod learning_graph;
pub mod llm_compressor;
pub mod message_safety;
pub mod provider_registry;
pub mod runtime_key;
pub mod skill_bundle;
pub mod skill_preprocessing;
pub mod stream_retry_budget;
pub(crate) mod turn_context;
pub(crate) mod turn_finalizer;
pub use turn_finalizer::TurnExitReason;
pub mod turn_retry_state;
pub mod turn_rules;

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
    /// Maximum share of `context_window` that a single tool result may occupy
    /// before its bulk is withheld behind an explicit marker. Threaded from
    /// `BehaviorSettings::max_tool_result_share`; see
    /// `context_management::DEFAULT_MAX_TOOL_RESULT_SHARE`.
    pub max_tool_result_share: f64,
    /// Max self-healing attempts on tool errors
    pub max_healing_attempts: usize,
    /// Ordered list of fallback models for automatic failover on retryable errors.
    pub fallback_models: Vec<String>,
    /// Whether automatic fallback to fallback_models is enabled.
    pub fallback_on_errors: bool,
    /// Whether core's cross-iteration repetition circuit breaker (identical
    /// tool-call streak, R35) and its warning nudge are active. Channel
    /// facades thread Loop B's `PacingConfig::loop_detection_enabled`
    /// through here so operators keep the old loop's off switch. Default ON.
    pub loop_detection_enabled: bool,
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
            max_tool_result_share: settings.max_tool_result_share,
            max_healing_attempts: settings.max_healing_attempts,
            fallback_models: settings.fallback_models.clone(),
            fallback_on_errors: settings.fallback_on_errors,
            loop_detection_enabled: true,
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
    /// Agent finished with final response.
    ///
    /// `reason` is why the turn actually ended (BUGS.md S5): only
    /// [`TurnExitReason::TextResponse`] is a normal completion — a
    /// `GraceCall` or `CircuitBreaker` reason means the agent stopped early
    /// and `message` is a best-effort partial (possibly empty after the S1
    /// grace quality gate). Consumers that surface the message to an
    /// operator MUST check the reason (`gateway_runner` substitutes the
    /// stopped-early notice).
    Done {
        message: Message,
        reason: TurnExitReason,
    },
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
    /// Context compaction began. Emitted before the LLM compressor or the
    /// deterministic eviction pass runs so the CLI/TUI can mark the boundary
    /// instead of showing a silent stall.
    CompactionStarted {
        /// Estimated prompt tokens at the moment compaction was triggered.
        tokens_before: usize,
    },
    /// Context compaction finished. `tokens_after` can exceed
    /// `tokens_before` when a summarizer pads its own summary — the payload
    /// is a report, not a guarantee.
    CompactionCompleted {
        tokens_before: usize,
        tokens_after: usize,
        messages_before: usize,
        messages_after: usize,
    },
    /// A retry was scheduled after a classified failure. The loop re-issues
    /// the request immediately (no sleep), so `attempt`/`max_attempts` are
    /// the whole story the UI needs.
    RetryScheduled {
        /// 1-based retry attempt within the turn.
        attempt: usize,
        max_attempts: usize,
        /// Short machine reason: "context overflow", "credential rotated",
        /// "stream dropped".
        reason: String,
    },
    /// A fallback model served the request after the primary failed with a
    /// retryable error. Emitted by `FallbackModelClient` so the user sees
    /// which model actually answered.
    ModelFallback {
        from: String,
        to: String,
        /// Short classified reason (the `FailoverReason` display form).
        reason: String,
    },
    /// A child subagent started running.
    SubagentStarted {
        subagent_id: String,
        role: String,
        depth: u32,
    },
    /// A child subagent finished. `status` is "completed", "failed", or
    /// "timeout".
    SubagentStopped {
        subagent_id: String,
        status: String,
        summary: String,
    },
    /// The session todo list changed shape after a `todo` tool write.
    TodoUpdated {
        total: usize,
        completed: usize,
        in_progress: usize,
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
    /// Plan 016 G1 — composable provider runtime (harness).
    /// When set, the agent loop consults the harness for tool execution
    /// after the static registry. Read-only path: never mutates the
    /// harness from within `run()`. Constructed by the CLI when
    /// `config.harness.enabled = true`; left `None` (the dark-merge
    /// default) to keep the existing `ToolRegistry` path byte-identical.
    harness: Option<Arc<operant_harness::Harness>>,
    /// The conversation for whichever session is currently addressed by
    /// [`Self::session_id`].
    ///
    /// This is the hot copy: a session being actively worked keeps its turns
    /// here so a turn does not pay a rehydrate per message. It is scoped to
    /// one session at a time — when [`Self::set_session_id`] retargets the
    /// agent, this slot is released back to [`Self::sessions`] (which
    /// rehydrates it from disk on demand) rather than carried into the next
    /// session. See [`crate::session`] for why.
    conversation: Arc<RwLock<Vec<Message>>>,
    /// Durable per-session transcripts, load-on-demand with a bounded warm
    /// cache. Owns which transcript a given session id sees; the
    /// [`Database`] owns where it is written.
    sessions: Arc<crate::session::SessionStore>,
    event_tx: Option<mpsc::Sender<AgentEvent>>,
    permission_tx: Option<mpsc::Sender<ToolPermissionRequest>>,
    /// P1/P2 seat authority (permission-genome wave-2 slices E + F2). When
    /// `Some` AND the agent has a session id (the employee id), the
    /// tool-execution guard in `agent/stream.rs` consults
    /// [`crate::org::seat_authority::SeatAuthority::consult`] before the
    /// permission channel: the seat's verdict can run a tool with no prompt
    /// (allow entry, yolo mode, or a STANDING GRANT — the ledger is real
    /// since F2), escalate with the policy's own explanation (persisted to
    /// the `pending_requests` queue, deduplicated), or deny outright.
    /// `None` (the default) keeps the run path byte-identical — no decide()
    /// call at all.
    seat_authority: Option<Arc<crate::org::seat_authority::SeatAuthority>>,
    /// F2 unattended semantics: this agent runs without an interactive user
    /// (cron). A governed `Escalate` verdict is clamped to a same-run Deny
    /// after the ask is queued — an unattended run never waits on a prompt
    /// channel nobody drains, and the next run consults the grant the
    /// approver minted between ticks. The UNGOVERNED path (no policy row)
    /// ignores this flag entirely: it keeps today's channel behaviour
    /// byte-for-byte, including the dispatcher's no-active-channel
    /// auto-AllowSession arm.
    unattended: bool,
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
    /// Post-turn event seam. When set, one `TurnEnd` is emitted per
    /// completed turn at the turn-end chokepoint, after the memory
    /// `sync_turn` / `queue_prefetch` hooks, and `execute_tools` reports
    /// per-tool durations back to the bus. `None` (the default) means no
    /// seam is attached: the emit site is a single `None` check that
    /// constructs nothing and never touches the conversation history.
    turn_end_bus: Option<crate::turn_end::TurnEndBus>,
    /// Hook registry for lifecycle events (AgentStart, AgentEnd, etc.).
    /// When set, the agent emits events at key lifecycle points.
    hook_registry: Option<Arc<crate::gateway_pipeline::HookRegistry>>,
    /// Kernel-evolved prompt sections (audit C1). When the harness boot
    /// constructs a `PromptSlot` and registers `PromptSlotSeam` against
    /// it, every provider's `prompt.section` install lands here and is
    /// rendered into the frozen prefix below. `None` = no harness
    /// prompt evolution; the prompt is byte-identical to the
    /// pre-harness path.
    harness_prompt_slot: Option<Arc<crate::harness_slots::PromptSlot>>,
    /// /steer directive queue (iter-65). When the user sends a steer
    /// message during a multi-iteration tool-calling loop, it's queued
    /// here. The run() loop drains pending steers between iterations
    /// and injects them into the conversation so the model sees the
    /// user's real-time guidance without restarting the turn.
    steer_queue: Arc<tokio::sync::Mutex<Vec<String>>>,
    /// The ONE session id this agent persists into, for every
    /// `run()` and every persistence site (messages, metadata, tool
    /// context, compression state). Two namespaces used to live here —
    /// the build-time `with_persistent_session` id and a per-turn
    /// `sess_<uuid>` minted in the turn prologue — and a host that only
    /// knew the second (the Telegram gateway) orphaned every turn's
    /// trajectory under a throwaway id, leaving the reloadable
    /// `gw_<hash>` session with a text-only skeleton. One slot, one id.
    ///
    /// Interior-mutable so a long-lived multi-tenant host (the gateway
    /// serves many session keys from one agent) can retarget the agent
    /// between turns via [`Self::set_session_id`] without rebuilding
    /// it, exactly like [`Self::set_model`] retargets the model.
    session_id: Arc<std::sync::RwLock<Option<String>>>,
    /// Wave 2 (ORGANISM-ARCHITECTURE §2): the cast employee this agent
    /// executes as — THE seat the genome consults. `None` = no gateway
    /// employee binding (cron runs derive their employee id as the session
    /// id already; local runs stay session-keyed, byte-identical).
    seat_id: Arc<std::sync::RwLock<Option<String>>>,
    /// The bound employee's charter (org-layer system prompt), appended to
    /// the frozen prefix by [`Self::build_frozen_prefix`]. Set together with
    /// `seat_id` by the gateway turn so prompt-cache stability holds: the
    /// pair only changes when the conversation switches employee.
    charter: Arc<std::sync::RwLock<Option<String>>>,
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
    /// S2: per-tool consecutive-timeout breaker — TURN-LOCAL state, reset
    /// at the top of every `run()` (same discipline as `tool_guardrails`).
    /// `timeout_streaks` counts consecutive `ToolResult::timed_out`
    /// results per tool name; on the 2nd the run loop nudges the model, on
    /// the 3rd the tool lands in `masked_tools` for the rest of the turn:
    /// hidden from the request's schema list (`tools_for_turn`) and
    /// refused at dispatch (`execute_tools` preflight). The registry
    /// itself is untouched — other sessions and later turns are
    /// unaffected. Measured pathology: 11 `aft_bash` timeouts in one
    /// turn, each retried as an ordinary error (2026-10-03 §1 S2).
    timeout_streaks: std::sync::Mutex<std::collections::HashMap<String, u32>>,
    /// Tools masked for the rest of the current turn by the S2 breaker
    /// (see `timeout_streaks`).
    masked_tools: std::sync::Mutex<std::collections::HashSet<String>>,
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
    /// Client-side prompt-cache prefix tracker. Holds the recently-seen
    /// cacheable-prefix digests so the run loop can decide hit/miss
    /// eligibility for a request WITHOUT the provider reporting anything.
    /// See `clients::cache_monitor`. Cheap to clone; per-agent state.
    cache_tracker: crate::agent::clients::cache_monitor::PrefixTracker,
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
    /// F2: when this prompt is a seat-policy escalation, the queued ask it
    /// resolves — an approval mints the grant and resolves this row; a
    /// denial/timeout resolves it too. `None` is today's ungoverned prompt:
    /// nothing is minted or resolved, byte-identical to mainline.
    pub seat_escalation: Option<crate::org::seat_authority::SeatEscalation>,
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
            | "kernel_exec" // plan 015: cells run up to request_timeout_secs (120s default)
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

/// Tools that mutate the filesystem. Two of these in the same batch can
/// interleave (one reads a file the other is rewriting, or both write the
/// same path), so a batch with more than one mutation runs sequentially.
fn is_file_mutation_tool(name: &str) -> bool {
    matches!(
        name,
        "file_write"
            | "file_edit"
            | "patch"
            | "write_file"
            | "create_file"
            | "aft_write"
            | "aft_edit"
            | "aft_apply_patch"
    )
}

/// Whether this tool goes through the interactive permission gate. The gate
/// is already sequential (phase 1), but the *effect* of an approved call
/// still lands in the concurrent pool — so a batch containing one is
/// serialized too, matching the runtime tool loop's approval rule.
///
/// This is the same list the permission prompt uses, `file_read` included:
/// an approved read must not overlap a concurrent mutation, and sharing one
/// list means the gate and the predicate cannot drift apart.
fn is_permission_gated_tool(name: &str) -> bool {
    matches!(
        name,
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
    )
}

/// Decide whether a tool batch may use the concurrent pool or must run
/// sequentially.
///
/// Parallel is the default (independent reads / web fetches should overlap),
/// so this only vetoes batches that can interfere with each other:
///
/// * more than one filesystem mutation, or
/// * any approval/permission-gated call.
///
/// The predicate is deliberately a pure function of the batch's tool names so
/// it is directly unit-testable without an agent, registry, or network.
fn batch_allows_parallel_execution<'a>(names: impl Iterator<Item = &'a str>) -> bool {
    let mut mutations = 0usize;
    let mut gated = false;
    for name in names {
        if is_file_mutation_tool(name) {
            mutations += 1;
        }
        if is_permission_gated_tool(name) {
            gated = true;
        }
    }
    mutations <= 1 && !gated
}

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

/// Match a tool name against an allowlist pattern: exact match or a
/// `*`/`?` glob (hermes `_command_matches_permanent_allowlist` uses
/// `fnmatch`, the Python equivalent).
///
/// iter-636 (consolidation): delegate to the canonical matcher
/// (`context::lcm::glob_match`) — the same one the seat policy in
/// `org::seat_policy` enforces with. Three independent glob matchers
/// meant a pattern written with a character class (`content.[0-9]`) matched
/// in the org layer and silently NEVER matched in the two local subsets.
fn allowlist_pattern_matches(pattern: &str, name: &str) -> bool {
    crate::context::lcm::glob_match(pattern, name)
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
pub(crate) fn safe_truncate_str(s: &str, max_bytes: usize) -> &str {
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
pub use fallback::{ClassifiedError, Failover, FailoverDecision, FallbackModelClient};

mod pooled_client;
pub use pooled_client::PooledModelClient;

pub mod clients;

// Method-group impl blocks extracted from the former 4.1K-line impl OperantAgent.
mod builders;
mod compress;
// W1.4: the preflight compression surface the runtime reconciled facade
// drives (old runtime engine's knobs + ported behaviors b/c/e + todo fold).
pub use compress::{
    PreflightConfig, fast_trim_tool_results, next_probe_tier, parse_context_limit_from_error,
    reinject_todos, repair_tool_pairs,
};
mod events;
mod prompting;
mod run;
mod stream;

// iter-633: the per-turn seat-budget envelope + its task-local. The gateway
// runner is the only setter — see run.rs for why this is task-local and not
// a field on the agent (the shared InterruptFlag incident, 2026-10-05).
pub use run::{SEAT_BUDGET_ENVELOPE, SeatBudgetEnvelope};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::clients::openai::OpenAIModelClient;
    use crate::agent::model_client::ChatRequest;
    use crate::client::ChatResponse;
    use crate::client::OpenAIClient;
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
    fn wave2_charter_rides_the_frozen_prefix_and_none_keeps_it_byte_identical() {
        // Wave 2 (ORGANISM-ARCHITECTURE §2): the bound employee's charter is
        // appended to the frozen prefix inside an explicit marker so the
        // org role is visible to the model and cache-stable across turns.
        // A None charter must leave the prefix byte-identical to the
        // pre-Wave-2 output — the ungoverned/legacy path must not shift by
        // one byte (prompt-cache discipline).
        let db = Database::init(std::env::temp_dir().join("test_charter_prefix.sqlite")).unwrap();
        let agent = OperantAgent::new(
            AgentConfig::default(),
            Box::new(OpenAIModelClient::new(OpenAIClient::new(
                crate::client::ClientConfig::default(),
            ))),
            ToolRegistry::new(Duration::from_secs(1)),
            Arc::new(db),
        );
        let plain = agent.build_frozen_prefix();
        assert!(!plain.contains("<employee_charter>"));

        agent.set_charter(Some("You are hrmaster — workforce lifecycle.".to_string()));
        let with_charter = agent.build_frozen_prefix();
        assert!(with_charter.contains("<employee_charter>"));
        assert!(with_charter.contains("You are hrmaster — workforce lifecycle."));
        assert!(with_charter.contains("</employee_charter>"));
        // The charter APPENDS in this no-skills test; with skills present
        // it inserts before the skills block by design (byte-stable per
        // employee, which is the cache discipline that matters).
        assert!(with_charter.starts_with(&plain));

        // Clearing restores byte-identity (binding flipped back / unbound).
        agent.set_charter(None);
        assert_eq!(agent.build_frozen_prefix(), plain);
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
    fn frozen_prefix_includes_kernel_prompt_sections() {
        // C1 — a provider's `prompt.section` install must reach the live
        // agent's system prompt, and a provider unmount must remove it.
        use crate::agent::clients::openai::OpenAIModelClient;
        use crate::client::OpenAIClient;
        use crate::harness_slots::PromptSlot;

        let slot = Arc::new(PromptSlot::new());
        const SECTION_PAYLOAD: &str = "kernel-section payload";
        let db = Database::init(std::path::PathBuf::from("test_c1_slot.sqlite")).unwrap();
        let agent = OperantAgent::new(
            AgentConfig::default(),
            Box::new(OpenAIModelClient::new(OpenAIClient::new(
                crate::client::ClientConfig::default(),
            ))),
            ToolRegistry::new(Duration::from_secs(1)),
            Arc::new(db),
        )
        .with_harness_prompt_slot(Arc::clone(&slot));

        // Before any provider installs, the prompt is byte-identical to
        // the non-harness path.
        assert!(!agent.build_frozen_prefix().contains(SECTION_PAYLOAD));

        // A named fn pointer renders a fixed payload — a closure literal
        // would trip clippy::redundant_closure in this test.
        fn render() -> String {
            SECTION_PAYLOAD.to_string()
        }
        let render: Arc<dyn Fn() -> String + Send + Sync> = Arc::new(render);

        assert!(
            slot.install("ops/one", Arc::clone(&render)),
            "install must accept the section"
        );
        let prefix = agent.build_frozen_prefix();
        assert!(
            prefix.contains(SECTION_PAYLOAD),
            "installed section payload must render into the frozen prefix"
        );

        // Uninstall (the seam's undo) removes it from the next refresh.
        assert!(slot.remove("ops/one"));
        assert!(!agent.build_frozen_prefix().contains(SECTION_PAYLOAD));

        // The slot is bounded: a distinct id past the cap is refused.
        let small = PromptSlot::with_max_items(1);
        assert!(small.install("a", Arc::clone(&render)));
        assert!(!small.install("b", Arc::clone(&render)));
        // Replacing an existing id is always allowed.
        assert!(small.install("a", Arc::clone(&render)));
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

    #[tokio::test]
    async fn compaction_should_emit_started_and_completed_events() {
        use crate::agent::clients::openai::OpenAIModelClient;
        use crate::client::OpenAIClient;

        let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentEvent>(16);
        let db = Database::init(std::path::PathBuf::from("test_compaction_events.sqlite")).unwrap();
        let agent = OperantAgent::with_events(
            AgentConfig::default(),
            Box::new(OpenAIModelClient::new(OpenAIClient::new(
                crate::client::ClientConfig::default(),
            ))),
            ToolRegistry::new(Duration::from_secs(1)),
            Arc::new(db),
            tx,
        );

        // No llm_compressor attached → the deterministic eviction pass runs,
        // so the test needs no network.
        let messages = vec![Message::user("a".repeat(40_000))];
        agent.compress_context_overflow(messages).await;

        let mut seen_started = None;
        let mut seen_completed = None;
        while let Ok(event) = rx.try_recv() {
            match event {
                AgentEvent::CompactionStarted { tokens_before } => {
                    seen_started = Some(tokens_before)
                }
                AgentEvent::CompactionCompleted {
                    tokens_after,
                    messages_after,
                    ..
                } => seen_completed = Some((tokens_after, messages_after)),
                _ => {}
            }
        }
        assert!(seen_started.is_some(), "CompactionStarted must be emitted");
        assert!(
            seen_completed.is_some(),
            "CompactionCompleted must be emitted"
        );
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

        let (content, reasoning, tool_calls, _finish_reason, _usage) =
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

    // ── iter-633: mid-flight seat budget (Wave-4 §5) ──────────────────

    /// Mock that drives a MULTI-ITERATION turn without needing any tool:
    /// call 1 reports `finish_reason="length"` (a cut-off response), so the
    /// loop's truncation-continuation re-loops to a second iteration whose
    /// top is where the seat-budget boundary check runs. Call 1 burns 900
    /// tokens — far over the test's 500-token envelope.
    struct BudgetBurstClient {
        calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    impl BudgetBurstClient {
        fn chat_response(
            &self,
            text: &str,
            finish_reason: &str,
            usage: crate::client::Usage,
        ) -> crate::client::ChatResponse {
            use crate::client::{Choice, MessageDelta};
            crate::client::ChatResponse {
                id: format!(
                    "resp-{}",
                    self.calls.load(std::sync::atomic::Ordering::SeqCst)
                ),
                object: "chat.completion".to_string(),
                created: 0,
                model: "mock".to_string(),
                choices: vec![Choice {
                    index: 0,
                    message: MessageDelta {
                        role: Some(crate::client::Role::Assistant),
                        content: Some(text.to_string()),
                        reasoning_content: None,
                        tool_calls: None,
                    },
                    finish_reason: Some(finish_reason.to_string()),
                }],
                usage,
            }
        }
    }

    #[async_trait]
    impl ModelClient for BudgetBurstClient {
        fn provider_name(&self) -> &str {
            "mock-budget-burst"
        }

        async fn chat(&self, _request: ChatRequest) -> Result<crate::client::ChatResponse> {
            let call = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if call == 0 {
                // Iteration 1: a truncated response (re-loops) + a 900-token
                // burn that crosses the 500-token envelope.
                Ok(self.chat_response(
                    "partial answer cut off",
                    "length",
                    crate::client::Usage {
                        prompt_tokens: 800,
                        completion_tokens: 100,
                        total_tokens: 900,
                    },
                ))
            } else {
                // Iteration 2 (ungoverned) OR the grace call (governed).
                Ok(self.chat_response(
                    "second call answer",
                    "stop",
                    crate::client::Usage {
                        prompt_tokens: 40,
                        completion_tokens: 10,
                        total_tokens: 50,
                    },
                ))
            }
        }

        async fn chat_streaming(
            &self,
            _request: ChatRequest,
        ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
            Err(Error::Agent("streaming not used in budget test".into()))
        }
    }

    fn budget_test_agent(
        event_tx: tokio::sync::mpsc::Sender<AgentEvent>,
    ) -> (OperantAgent, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        let config = AgentConfig {
            stream: false,
            ..AgentConfig::default()
        };
        static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let db = Database::init(std::path::PathBuf::from(format!(
            "test_seat_budget_{}_{}.sqlite",
            std::process::id(),
            n
        )))
        .unwrap();
        // The mock and the test share one counter so the test can assert
        // the number of model calls without another channel.
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let agent = OperantAgent::with_events(
            config,
            Box::new(BudgetBurstClient {
                calls: calls.clone(),
            }),
            ToolRegistry::new(Duration::from_secs(1)),
            Arc::new(db),
            event_tx,
        );
        (agent, calls)
    }

    /// The envelope stops the turn AT the iteration boundary: iteration 1's
    /// 900 tokens cross the 500-token cap, so the second model call is the
    /// GRACE call (exit reason GraceCall) and the operator sees the 🛑
    /// budget notice — not a silently-continued turn.
    #[tokio::test]
    async fn seat_budget_envelope_stops_the_turn_at_the_iteration_boundary() {
        let (event_tx, mut event_rx) = tokio::sync::mpsc::channel::<AgentEvent>(16);
        let (agent, calls) = budget_test_agent(event_tx);
        let envelope = SeatBudgetEnvelope {
            cap_tokens: 500.0,
            used_at_turn_start: 0.0,
        };
        let response = SEAT_BUDGET_ENVELOPE
            .scope(envelope, agent.run("do the thing".to_string()))
            .await
            .expect("run under envelope");
        assert_eq!(response.content, "second call answer");
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "iteration 1 + one grace call — the boundary fired before a normal iteration 2"
        );
        let mut done_reason = None;
        let mut saw_budget_notice = false;
        while let Ok(event) = event_rx.try_recv() {
            match event {
                AgentEvent::Done { reason, .. } => done_reason = Some(reason),
                AgentEvent::Content { text }
                    if text.starts_with("🛑 Seat budget cap reached mid-turn") =>
                {
                    saw_budget_notice = true;
                }
                _ => {}
            }
        }
        assert!(
            saw_budget_notice,
            "the 🛑 budget meta-notice must be emitted so the operator sees WHY"
        );
        assert_eq!(
            done_reason,
            Some(TurnExitReason::GraceCall),
            "budget breach must exit via the grace path, not a silent full turn"
        );
    }

    /// No envelope (CLI/chat/TUI callers) = byte-identical legacy: the
    /// truncation continuation completes normally as a TextResponse with no
    /// budget notice.
    #[tokio::test]
    async fn without_envelope_the_turn_is_ungoverned_legacy() {
        let (event_tx, mut event_rx) = tokio::sync::mpsc::channel::<AgentEvent>(16);
        let (agent, calls) = budget_test_agent(event_tx);
        let response = agent
            .run("do the thing".to_string())
            .await
            .expect("ungoverned run");
        assert_eq!(response.content, "second call answer");
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "iteration 1 (truncated) + iteration 2 (final) — no grace call"
        );
        let mut done_reason = None;
        let mut saw_budget_notice = false;
        while let Ok(event) = event_rx.try_recv() {
            match event {
                AgentEvent::Done { reason, .. } => done_reason = Some(reason),
                AgentEvent::Content { text }
                    if text.starts_with("🛑 Seat budget cap reached mid-turn") =>
                {
                    saw_budget_notice = true;
                }
                _ => {}
            }
        }
        assert!(!saw_budget_notice, "no envelope: no budget notice");
        assert_eq!(
            done_reason,
            Some(TurnExitReason::TextResponse),
            "ungoverned turn ends as a normal text response"
        );
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
            max_tool_result_share: crate::context_management::DEFAULT_MAX_TOOL_RESULT_SHARE,
            max_healing_attempts: 1,
            fallback_models: Vec::new(),
            fallback_on_errors: false,
            loop_detection_enabled: true,
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
            max_tool_result_share: crate::context_management::DEFAULT_MAX_TOOL_RESULT_SHARE,
            max_healing_attempts: 1,
            fallback_models: Vec::new(),
            fallback_on_errors: false,
            loop_detection_enabled: true,
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
            max_tool_result_share: crate::context_management::DEFAULT_MAX_TOOL_RESULT_SHARE,
            max_healing_attempts: 1,
            fallback_models: Vec::new(),
            fallback_on_errors: false,
            loop_detection_enabled: true,
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
    fn parallel_batch_with_no_file_mutations_and_no_gate_stays_parallel() {
        // The old code ran this concurrently; the predicate must not change
        // that. `file_read` is absent here because it is permission-gated — a
        // gated batch serializes.
        assert!(batch_allows_parallel_execution(
            ["web_search", "web_search", "glob_search", "web_scrape"].into_iter()
        ));
    }

    #[test]
    fn two_file_mutations_serialize() {
        // Batch the 8-worker pool would have run concurrently — read-modify-
        // -write on the same path can interleave.
        assert!(!batch_allows_parallel_execution(
            ["aft_write", "aft_apply_patch", "web_search"].into_iter()
        ));
        assert!(!batch_allows_parallel_execution(
            ["file_write", "file_edit"].into_iter()
        ));
    }

    #[test]
    fn single_file_mutation_alongside_reads_stays_parallel() {
        // One mutation cannot race itself; the veto is only for >1.
        assert!(batch_allows_parallel_execution(
            ["write_file", "web_search", "glob_search"].into_iter()
        ));
    }

    #[test]
    fn any_approval_gated_call_serializes_the_batch() {
        assert!(!batch_allows_parallel_execution(
            ["web_search", "bash"].into_iter()
        ));
        assert!(!batch_allows_parallel_execution(
            ["process", "web_search"].into_iter()
        ));
        assert!(!batch_allows_parallel_execution(
            ["file_read", "file_read"].into_iter()
        ));
    }

    #[test]
    fn mutation_and_gate_classifiers_cover_their_tools() {
        for name in [
            "file_write",
            "file_edit",
            "patch",
            "write_file",
            "create_file",
            "aft_write",
            "aft_edit",
            "aft_apply_patch",
        ] {
            assert!(
                is_file_mutation_tool(name),
                "{name} should count as a mutation"
            );
        }
        for name in ["web_search", "file_read", "glob_search", "web_scrape"] {
            assert!(
                !is_file_mutation_tool(name),
                "{name} must not count as a mutation"
            );
        }
        for name in [
            "bash",
            "terminal",
            "execute_command",
            "code_execution",
            "file_read",
            "file_write",
            "file_edit",
            "patch",
            "process",
            "browser",
        ] {
            assert!(
                is_permission_gated_tool(name),
                "{name} should be permission-gated"
            );
        }
        assert!(!is_permission_gated_tool("web_search"));
        assert!(!is_permission_gated_tool("glob_search"));
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
        // iter-636 consolidation: character classes now behave as written
        // (the divergence this deleted — `content.[0-9]` matched in the org
        // seat policy and silently never matched here).
        assert!(allowlist_pattern_matches("content.[0-9]", "content.5"));
        assert!(!allowlist_pattern_matches("content.[0-9]", "content.x"));
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

        let (text, _reasoning, tcs, _extra, finish_reason, _usage) =
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
        let (_t, _r, _tcs, _e, finish_reason, _usage) = agent.process_stream(stream).await.unwrap();
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
