//! Sub-agent delegation tool, including the bounded multi-agent **swarm**.
//!
//! A parent agent delegates focused analysis to isolated child agents without
//! changing its own ReAct loop. Three modes share one code path:
//!
//! * single — one child, synchronous;
//! * **swarm** (`tasks`) — N real worker sessions, each with its OWN agent
//!   loop, message history, and tool scope, fanned out behind the same
//!   semaphore + `buffer_unordered` pool the in-loop tool executor uses
//!   (`agent/stream.rs::execute_tools`);
//! * background — dispatched to a spawned task, polled by id.
//!
//! Recursive spawning is closed by `SpawnPermit`, which reserves capacity in
//! the **process-wide** live-worker count BEFORE a worker is built. Depth and
//! breadth are both refused up front; a checked-after-spawning guard is not a
//! guard. Because a worker is one model stream and one slice of the parent's
//! tool-call spend, a fan-out is a COST decision — see `describe_spawn_costs`
//! in the tool description and the reported defaults.

use std::error::Error as StdError;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use futures::FutureExt;
use futures::stream::{self, StreamExt};
use reqwest::Client;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use tokio::time::timeout;

use crate::agent::clients::openai::OpenAIModelClient;
use crate::agent::{AgentConfig, AgentEvent, ModelClient, OperantAgent};
use crate::client::{ClientConfig, Message, OpenAIClient};
use crate::database::Database;
use crate::schema::ToolSchema;
use crate::tools::async_delegation;
use crate::tools::delegation_output_schema::{
    MAX_SCHEMA_RETRIES, append_output_contract, build_retry_message, coerce_output_schema,
    validate_output,
};
use crate::tools::{OperantTool, ToolContext, ToolRegistry, ToolResult};

const TOOL_NAME: &str = "delegate_task";

/// Maximum concurrent children (default from Python implementation)
const DEFAULT_MAX_CONCURRENT_CHILDREN: usize = 3;
/// Default child timeout in seconds (10 minutes)
const DEFAULT_CHILD_TIMEOUT_SECONDS: u64 = 600;
/// Minimum spawn depth
const MIN_SPAWN_DEPTH: u32 = 1;
/// Maximum spawn depth cap
const MAX_SPAWN_DEPTH_CAP: u32 = 3;
/// Default max spawn depth.
///
/// MEASURED 2026-11-14 (audit finding 12): this literal was 1 with a comment
/// claiming "flat: parent -> child, no grandchildren", while the shipped config
/// default was 2 — so anyone reasoning from this constant was wrong about the
/// runtime, and a grandchild spawn was allowed by default nobody documented.
/// The constant now states the value the operator actually gets. It is only a
/// FALLBACK: `[delegation] max_spawn_depth` (default 2) always wins, and
/// `set_max_spawn_depth` is the one choke point the CLI sets.
const DEFAULT_MAX_SPAWN_DEPTH: u32 = 2;
/// Floor for a worker timeout, so a caller cannot pass `timeout_seconds: 0`
/// and get a worker that is born already-expired.
const MIN_WORKER_TIMEOUT_SECONDS: u64 = 30;

/// Default cap on LIVE sub-agent workers across the whole process.
///
/// This is the global breadth cap, and it is the number that makes the swarm
/// safe: it bounds concurrent model streams regardless of how many parents
/// delegate at once. `MAX_CONCURRENT_CHILDREN` only bounds one batch, so N
/// parents each fanning out M children would otherwise reach N*M live model
/// streams — an unbounded spend against one API budget.
pub const DEFAULT_MAX_GLOBAL_WORKERS: usize = 6;
/// Floor for the global breadth cap.
const MIN_GLOBAL_WORKERS: usize = 1;
/// Ceiling for the global breadth cap.
const MAX_GLOBAL_WORKERS_CAP: usize = 32;

type BoxedToolError = Box<dyn StdError + Send + Sync>;

/// Role of the sub-agent - determines capabilities
#[derive(Debug, Clone, Copy, PartialEq, Eq, JsonSchema, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SubAgentRole {
    /// Leaf agent - focused worker, cannot delegate further
    #[default]
    Leaf,
    /// Orchestrator - can spawn its own sub-agents
    Orchestrator,
}

/// A single task in batch mode
#[derive(Debug, Deserialize, JsonSchema)]
struct DelegationTask {
    /// The focused task instruction for the child agent
    goal: String,
    /// Optional context to pass to the sub-agent
    #[serde(default)]
    context: Option<String>,
    /// Optional toolsets to enable for this task
    #[serde(default)]
    toolsets: Option<Vec<String>>,
    /// Optional JSON Schema object the child's final answer must validate
    /// against (hermes delegation_output_schema.py parity).
    #[serde(default)]
    output_schema: Option<Value>,
}

/// Arguments for delegated sub-agent work.
#[derive(Debug, Deserialize, JsonSchema)]
struct SubAgentArgs {
    /// The focused task instruction for the child agent (single mode)
    #[serde(default, alias = "task")]
    goal: Option<String>,
    /// Optional context to pass to the sub-agent
    #[serde(default)]
    context: Option<String>,
    /// Optional toolsets to enable for the sub-agent
    #[serde(default)]
    toolsets: Option<Vec<String>>,
    /// Batch mode: array of tasks to run in parallel
    #[serde(default)]
    tasks: Option<Vec<DelegationTask>>,
    /// Role of the sub-agent: "leaf" (default) or "orchestrator"
    #[serde(default, alias = "agent_role")]
    role: Option<SubAgentRole>,
    /// Maximum iterations for the child agent
    #[serde(default)]
    max_iterations: Option<u32>,
    /// Timeout for child agent in seconds (default: 600)
    #[serde(default)]
    timeout_seconds: Option<u64>,
    /// Optional JSON Schema object the child's final answer must validate
    /// against. On failure the child gets exactly ONE bounded retry turn
    /// carrying the validation errors verbatim (hermes
    /// delegation_output_schema.py parity).
    #[serde(default)]
    output_schema: Option<Value>,
    /// Where the child should work. Defaults to the parent's working directory.
    ///
    /// MEASURED 2026-11-14: a child's working directory IS the process working
    /// directory — `agent::turn_context` reads `std::env::current_dir()` per turn
    /// and every file tool resolves relative paths through the process — so N swarm
    /// workers share one directory and a caller that passed a different path got
    /// silence: the child ran wherever the process happened to be. A path that
    /// differs from the process one is now refused with the reason, instead of
    /// being accepted and ignored.
    #[serde(default)]
    working_dir: Option<String>,
    /// Run the delegated task in the background and return a handle
    /// immediately (hermes async_delegation.py parity). Poll the result
    /// with `query`.
    #[serde(default)]
    background: Option<bool>,
    /// Poll the status/result of a previously dispatched background
    /// delegation by its `delegation_id`.
    #[serde(default)]
    query: Option<String>,
}

/// Runtime state for delegation
static MAX_SPAWN_DEPTH: AtomicU32 = AtomicU32::new(DEFAULT_MAX_SPAWN_DEPTH);
static ORCHESTRATOR_ENABLED: AtomicU32 = AtomicU32::new(1); // default true
static MAX_CONCURRENT_CHILDREN: AtomicU32 = AtomicU32::new(DEFAULT_MAX_CONCURRENT_CHILDREN as u32);

/// LIVE sub-agent workers across the WHOLE process.
///
/// Global on purpose: a per-parent or per-batch count cannot see the other
/// parents, so N parents each admitting M children yields N*M live model
/// streams. A single process-wide counter is the only place that count is
/// actually knowable.
static LIVE_WORKERS: AtomicUsize = AtomicUsize::new(0);
/// Configurable ceiling for `LIVE_WORKERS`.
static MAX_GLOBAL_WORKERS: AtomicUsize = AtomicUsize::new(DEFAULT_MAX_GLOBAL_WORKERS);

/// Monotonic counter behind the `sub-N` ids carried by
/// `AgentEvent::SubagentStarted/Stopped`. Children run headless, so the id is
/// the only thing that pairs a stop event back to its start.
static SUBAGENT_SEQ: AtomicU32 = AtomicU32::new(1);

/// Why a spawn was refused. Both variants are decided BEFORE any worker is
/// built — a refusal never costs a model stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpawnRefusal {
    /// The requested depth is past the configured delegation floor.
    Depth { requested: u32, max: u32 },
    /// The process-wide live-worker budget is exhausted.
    Breadth { live: usize, max: usize },
}

impl std::fmt::Display for SpawnRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Depth { requested, max } => write!(
                f,
                "Spawn refused: depth {requested} exceeds max_spawn_depth {max}. \
                 Workers cannot recurse past the configured delegation depth."
            ),
            // MEASURED 2026-11-14 (audit finding 5): this told the parent to
            // "wait for a worker to finish", but a refused SWARM has no handle
            // to poll — `query=` resolves background delegations only — so the
            // instruction could not be followed. The parent's real recourse is
            // to retry the call, which is what this now says.
            Self::Breadth { live, max } => write!(
                f,
                "Spawn refused: {live}/{max} sub-agent worker slots are live \
                 process-wide. Slots free as the running workers finish — retry this \
                 call, or lower the fan-out."
            ),
        }
    }
}

/// A reserved slice of the process-wide live-worker budget.
///
/// Reservation happens in `reserve`, which reads the live count and claims the
/// slots in ONE `fetch_update` — a plain `load`-then-`store` would let N
/// concurrent parents all observe `live < max` and all admit M children, which
/// is the N*M blow-up this guard exists to prevent.
///
/// Release lives in `Drop` and nowhere else, so the count is returned on
/// EVERY exit path: success, `?`, an early return, a panic unwind, and tokio
/// task cancellation/drop. A leaked count is the worst failure mode here — it
/// permanently shrinks capacity for the remaining life of the process, with no
/// error to point at — so there is exactly one release site and it cannot be
/// skipped by adding a new `return`.
#[derive(Debug)]
struct SpawnPermit {
    slots: usize,
}

impl SpawnPermit {
    /// Claim up to `want` worker slots for a child at `depth`.
    ///
    /// Partial admission is normal and visible: when only 2 of 5 requested
    /// slots are free, 2 are claimed and the caller reports the other 3 as
    /// refused rather than building workers that would immediately fail.
    /// `Err` only when NOTHING could be claimed.
    // `fetch_update is DEPRECATED: it is the pre-1.95 spelling, renamed to
    // `try_update`. The rename cannot be made — `try_update` is stable since
    // 1.95 and this workspace declares `rust-version = "1.89"`, so taking it
    // would break the MSRV to silence a deprecation lint. The method is a
    // plain rename with identical semantics.
    #[allow(
        deprecated,
        reason = "`try_update` needs 1.95; the workspace MSRV is 1.89 (rust-version)"
    )]
    fn reserve(want: usize, depth: u32) -> Result<Self, SpawnRefusal> {
        let max_depth = MAX_SPAWN_DEPTH.load(Ordering::Relaxed);
        if depth > max_depth {
            return Err(SpawnRefusal::Depth {
                requested: depth,
                max: max_depth,
            });
        }

        let want = want.max(1);
        let max = max_global_workers();
        // `fetch_update` hands back the PREVIOUS value, not the one the
        // closure chose, so the claim is staged in a Cell the closure fills.
        let claim = std::cell::Cell::new(0usize);
        // AcqRel/Acquire pair: the counter is a capacity ledger, not a
        // synchronisation primitive — no worker memory is published through
        // it. Relaxed ordering would be sufficient; AcqRel costs nothing at
        // this call rate and keeps the read-then-claim visibly paired.
        let reserved = LIVE_WORKERS.fetch_update(Ordering::AcqRel, Ordering::Acquire, |live| {
            let headroom = max.saturating_sub(live);
            let admitted = headroom.min(want);
            if admitted == 0 {
                return None;
            }
            claim.set(admitted);
            Some(live + admitted)
        });

        match reserved {
            Ok(_) => Ok(Self { slots: claim.get() }),
            Err(live) => Err(SpawnRefusal::Breadth { live, max }),
        }
    }

    /// How many workers this permit actually admits.
    fn slots(&self) -> usize {
        self.slots
    }
}

impl Drop for SpawnPermit {
    // `fetch_update` is DEPRECATED (renamed `try_update` in 1.95) — kept for
    // the workspace's 1.89 MSRV; see the note on `reserve`.
    #[allow(
        deprecated,
        reason = "`try_update` needs 1.95; the workspace MSRV is 1.89 (rust-version)"
    )]
    fn drop(&mut self) {
        // `checked_sub` inside fetch_update: a double-release bug saturates at
        // 0 instead of wrapping the counter to usize::MAX and wedging every
        // future spawn permanently.
        let _ = LIVE_WORKERS.fetch_update(Ordering::AcqRel, Ordering::Acquire, |live| {
            live.checked_sub(self.slots)
        });
    }
}

/// Live worker count across the process — exposed for the CLI status surfaces
/// and for tests that assert the ledger balances.
fn live_worker_count() -> usize {
    LIVE_WORKERS.load(Ordering::Relaxed)
}

fn max_global_workers() -> usize {
    MAX_GLOBAL_WORKERS.load(Ordering::Relaxed)
}

/// Whether `live` of `max` worker slots leaves no headroom.
///
/// Split from the caller so the DECISION is testable without the process-global
/// counters: this module's tests hold and release permits on parallel threads and
/// several of them set the global cap, so a test that drives the real counters
/// races them.
fn breadth_is_exhausted_at(live: usize, max: usize) -> bool {
    live >= max
}

/// Whether the process-wide worker budget has no headroom at all.
///
/// A peek for callers that must decide BEFORE they create something the child
/// would need a slot for (a background delegation record, a UI row). It is never
/// a substitute for [`SpawnPermit::reserve`], which is the atomic claim.
fn breadth_is_exhausted() -> bool {
    breadth_is_exhausted_at(live_worker_count(), max_global_workers())
}

fn next_subagent_id() -> String {
    format!("sub-{}", SUBAGENT_SEQ.fetch_add(1, Ordering::Relaxed))
}

/// One worker's full instruction set. A worker gets its own message history
/// and its own registry, but it is described by exactly this struct.
#[derive(Debug, Clone)]
struct WorkerSpec {
    goal: String,
    context: Option<String>,
    role: SubAgentRole,
    max_iterations: Option<u32>,
    /// Bounded wall-clock budget for this worker. Unbounded is a hang, and one
    /// wedged worker must not stall the swarm.
    timeout_secs: u64,
    output_schema: Option<Value>,
    /// The toolsets the caller asked for (NARROW-ONLY: the child's set is
    /// intersected with the parent's, never widened).
    toolsets: Option<Vec<String>>,
    /// Where the child should work, if the caller asked for somewhere other than
    /// the parent's working directory (refused — see `resolve_child_cwd`).
    working_dir: Option<String>,
    /// Stable id for this worker, also the id an operator surface shows.
    id: String,
    /// The other workers in this fan-out, as `(id, goal)` pairs, already clipped.
    ///
    /// MEASURED 2026-11-14 (audit finding 6): siblings could not see each other
    /// at all, and they all share ONE working directory, so two workers editing
    /// the same file had no way to notice — the collision surfaced only when the
    /// parent read two reports that disagreed. This is not a channel (a live one
    /// is a subsystem); it is the minimum that lets a worker NAME the conflict in
    /// the report the parent already reads.
    siblings: Vec<(String, String)>,
}

/// One child's finished turn: the text the parent reads, plus the session the
/// child actually ran in.
///
/// MEASURED 2026-11-14 (audit finding 4): a child mints its own session id
/// inside `agent::turn_context` (`sess_<uuid>`) and nothing carried it back, so
/// the truncation notice's "the full text is in the session store" named a store
/// with no key — the parent model could not ask for the tail and the operator
/// could not join a child session to the delegation that produced it. The id is
/// the join, so it travels with the answer instead of being re-derived.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ChildAnswer {
    text: String,
    session_id: Option<String>,
}

/// How a single worker ended. Every exit path — success, agent error,
/// timeout, panic, or spawn refusal — is one of these, so a worker can never
/// take the parent down: the failure is reported as data.
#[derive(Debug, Clone, PartialEq, Eq)]
enum WorkerStatus {
    Completed(ChildAnswer),
    Failed(String),
    TimedOut {
        after_secs: u64,
    },
    /// The worker future panicked. Contained via `catch_unwind` so the
    /// parent's task survives and the rest of the swarm still completes.
    Panicked,
    /// The spawn guard refused this worker; it was never built, so it cost
    /// no model stream.
    Refused(String),
}

/// A worker's goal paired with its outcome, so the parent can attribute each
/// result to the request that produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkerOutcome {
    /// The worker's id, so an operator surface can be told which row ended.
    id: String,
    goal: String,
    status: WorkerStatus,
}

/// A worker's execution, boxed so one `run` closure can build each worker's
/// distinct future without a second trait or a generic-per-call.
type WorkerRun = Pin<Box<dyn Future<Output = crate::error::Result<ChildAnswer>> + Send>>;

/// Fan out `workers` and collect every outcome, reusing the in-loop tool
/// pool's shape: a semaphore bounds how many workers hold a model stream at
/// once, and `buffer_unordered` yields each outcome the moment its worker
/// finishes rather than waiting on the slowest one. Outcomes are therefore in
/// COMPLETION order, not submission order — which is why every outcome
/// carries its own goal rather than a positional index.
///
/// Containment is per worker, not per swarm:
/// * each worker is wrapped in its own `timeout`, so a wedged worker is
///   dropped and the swarm still finishes;
/// * the worker's whole execution — including building its future — is
///   wrapped in `catch_unwind`, so a panicking worker becomes a
///   `WorkerStatus::Panicked` entry instead of unwinding the parent;
/// * an agent error becomes `WorkerStatus::Failed`.
///
/// None of these return `Err` — the swarm's only failure modes are the
/// caller's to interpret.
async fn run_swarm<F>(workers: Vec<WorkerSpec>, max_in_flight: usize, run: F) -> Vec<WorkerOutcome>
where
    F: Fn(WorkerSpec) -> WorkerRun,
{
    let in_flight = max_in_flight.max(1);

    // The bound is `buffer_unordered`'s, not a semaphore's: each future used to
    // acquire one of `in_flight` permits AND then be polled through
    // `buffer_unordered(in_flight)`, which is the same bound applied twice. The
    // second copy also carried an unreachable arm — the semaphore was never
    // closed, so "worker pool closed during shutdown" could not fire — and a
    // `collect::<Vec<_>>()` that only existed to hand `stream::iter` a Vec.
    let futures = workers.into_iter().map(|worker| {
        let run = &run;
        async move {
            let goal = worker.goal.clone();
            let id = worker.id.clone();
            let secs = worker.timeout_secs;

            // `run(worker)` is INSIDE the unwind boundary: building a worker's
            // future is as capable of panicking as running it.
            let status = match timeout(
                Duration::from_secs(secs),
                AssertUnwindSafe(async { run(worker).await }).catch_unwind(),
            )
            .await
            {
                Err(_) => WorkerStatus::TimedOut { after_secs: secs },
                Ok(Err(_panic)) => WorkerStatus::Panicked,
                Ok(Ok(Err(error))) => WorkerStatus::Failed(error.to_string()),
                Ok(Ok(Ok(answer))) => WorkerStatus::Completed(answer),
            };

            WorkerOutcome { id, goal, status }
        }
    });

    stream::iter(futures)
        .buffer_unordered(in_flight)
        .collect::<Vec<_>>()
        .await
}

/// Tool that delegates a focused task to an isolated child OperantAgent.
pub struct SubAgentTool {
    client_config: ClientConfig,
    http_client: Client,
    model: String,
    /// Parent's current depth (0 = root agent)
    parent_depth: u32,
    /// Parent's enabled toolsets
    parent_toolsets: Vec<String>,
    /// Parent's explicitly disabled tools — inherited so children never gain
    /// tools the parent lacks (hermes delegate_tool.py: "subagent must not
    /// gain tools the parent lacks").
    parent_disabled_tools: std::collections::HashSet<String>,
    /// Parent's explicitly disabled toolsets — inherited for the same reason.
    parent_disabled_toolsets: std::collections::HashSet<String>,
    /// The parent's tool-approval allowlist, inherited so a headless child can
    /// run what the operator has already approved. MEASURED 2026-11-14 (audit
    /// finding 2): a child is built from `AgentConfig::default()`, whose
    /// allowlist is an empty literal, while the PARENT gets one assigned after
    /// construction — so an operator's "allow always" reached the parent and not
    /// the child, and the child's gated call was refused with advice a headless
    /// agent cannot follow ("run interactively to approve it once").
    parent_approval_allowlist: std::collections::HashSet<String>,
    /// Where the parent persists "allow always" choices.
    parent_approval_allowlist_path: Option<std::path::PathBuf>,
    database: Arc<Database>,
    /// Optional event channel to forward child progress events to parent
    event_tx: Option<tokio::sync::mpsc::Sender<AgentEvent>>,
}

impl SubAgentTool {
    pub fn new(
        parent_client: &OpenAIClient,
        model: impl Into<String>,
        parent_depth: u32,
        parent_toolsets: Vec<String>,
        database: Arc<Database>,
        event_tx: Option<tokio::sync::mpsc::Sender<AgentEvent>>,
    ) -> Self {
        Self::with_parent_tool_policy(
            parent_client,
            model,
            parent_depth,
            parent_toolsets,
            std::collections::HashSet::new(),
            std::collections::HashSet::new(),
            database,
            event_tx,
        )
    }

    /// Construct with the parent's disabled tool/toolset policy so that
    /// spawned children inherit the exact same tool restrictions as the
    /// parent registry (hermes parity).
    #[expect(
        clippy::too_many_arguments,
        reason = "builder mirrors the SubAgentTool fields (hermes parity); callers construct it directly"
    )]
    pub fn with_parent_tool_policy(
        parent_client: &OpenAIClient,
        model: impl Into<String>,
        parent_depth: u32,
        parent_toolsets: Vec<String>,
        parent_disabled_tools: std::collections::HashSet<String>,
        parent_disabled_toolsets: std::collections::HashSet<String>,
        database: Arc<Database>,
        event_tx: Option<tokio::sync::mpsc::Sender<AgentEvent>>,
    ) -> Self {
        Self {
            client_config: parent_client.config_clone(),
            http_client: parent_client.http_client_clone(),
            model: model.into(),
            parent_depth,
            parent_toolsets,
            parent_disabled_tools,
            parent_disabled_toolsets,
            // Empty until `with_parent_approval` is called. A child that
            // inherits nothing keeps the pre-fix behaviour, not a widening: the
            // headless deny path still refuses anything not on this list.
            parent_approval_allowlist: std::collections::HashSet::new(),
            parent_approval_allowlist_path: None,
            database,
            event_tx,
        }
    }

    /// Inherit the parent's tool-approval allowlist and the file it persists to.
    ///
    /// A builder rather than two more constructor arguments: the policy is
    /// optional (a parent that never approved anything has an empty set), and
    /// every existing call site keeps compiling unchanged.
    pub fn with_parent_approval(
        mut self,
        allowlist: std::collections::HashSet<String>,
        path: Option<std::path::PathBuf>,
    ) -> Self {
        self.parent_approval_allowlist = allowlist;
        self.parent_approval_allowlist_path = path;
        self
    }

    /// Run a focused delegated task in an isolated child agent.
    ///
    /// This is the GUARDED entry point: it reserves a process-wide worker slot
    /// before the child is built, so a single delegation obeys the same
    /// depth + breadth limits as a swarm. Swarms call `call_unguarded` per
    /// worker instead, because `call_batch` already reserved the swarm's slots
    /// in one atomic claim — counting twice would double-charge the budget.
    #[expect(
        clippy::too_many_arguments,
        reason = "the worker's full instruction set is passed as arguments; the same fields as WorkerSpec"
    )]
    pub async fn call(
        &self,
        goal: impl Into<String>,
        context: Option<impl Into<String>>,
        role: SubAgentRole,
        max_iterations: Option<u32>,
        timeout_seconds: u64,
        output_schema: Option<Value>,
        toolsets: Option<Vec<String>>,
        working_dir: Option<String>,
    ) -> std::result::Result<String, BoxedToolError> {
        let child_depth = self.parent_depth + 1;
        // Depth AND process-wide breadth, both decided here — before
        // `ensure_supported_model`, before the client is built, before any
        // token is spent. The permit lives for the whole call INCLUDING the
        // output-schema retry loop, so one child is one live worker no matter
        // how many turns it takes.
        let _permit =
            SpawnPermit::reserve(1, child_depth).map_err(|refusal| refusal.to_string())?;

        self.call_unguarded(WorkerSpec {
            siblings: Vec::new(),
            id: next_subagent_id(),
            goal: goal.into(),
            context: context.map(Into::into),
            role,
            max_iterations,
            timeout_secs: timeout_seconds.max(MIN_WORKER_TIMEOUT_SECONDS),
            output_schema,
            toolsets,
            working_dir,
        })
        .await
        .map(|answer| answer.text)
    }

    /// The same guarded entry point, but it keeps the child's session id.
    ///
    /// The background path needs it: a delegation's row in the store has to be
    /// joinable to the session that ran it, or the operator can see that work
    /// happened without being able to open it (audit finding 10).
    #[expect(
        clippy::too_many_arguments,
        reason = "identical to `call`; one delegates to the other"
    )]
    pub async fn call_with_session(
        &self,
        goal: impl Into<String>,
        context: Option<impl Into<String>>,
        role: SubAgentRole,
        max_iterations: Option<u32>,
        timeout_seconds: u64,
        output_schema: Option<Value>,
        toolsets: Option<Vec<String>>,
        working_dir: Option<String>,
    ) -> std::result::Result<ChildAnswer, BoxedToolError> {
        let child_depth = self.parent_depth + 1;
        let _permit =
            SpawnPermit::reserve(1, child_depth).map_err(|refusal| refusal.to_string())?;
        self.call_unguarded(WorkerSpec {
            siblings: Vec::new(),
            id: next_subagent_id(),
            goal: goal.into(),
            context: context.map(Into::into),
            role,
            max_iterations,
            timeout_secs: timeout_seconds.max(MIN_WORKER_TIMEOUT_SECONDS),
            output_schema,
            toolsets,
            working_dir,
        })
        .await
    }

    /// Run one worker whose slot was ALREADY reserved by the caller.
    ///
    /// When `output_schema` is provided, the child's system prompt gets an
    /// OUTPUT CONTRACT block and the final answer is validated against the
    /// schema (hermes delegation_output_schema.py parity): on failure the
    /// child receives exactly ONE bounded retry turn carrying the validation
    /// errors verbatim, and a persistent failure still returns the answer
    /// (flagged) so the parent keeps the data.
    async fn call_unguarded(
        &self,
        worker: WorkerSpec,
    ) -> std::result::Result<ChildAnswer, BoxedToolError> {
        self.ensure_supported_model()?;

        let goal = worker.goal.trim();
        if goal.is_empty() {
            return Err("Sub-agent goal must not be empty".into());
        }

        let timeout_seconds = worker.timeout_secs;
        let max_iterations = worker.max_iterations;
        let role = worker.role;
        let output_schema = worker.output_schema;
        let toolsets = worker.toolsets.clone();

        // The caller already cleared the guard, but the depth floor still
        // decides the effective role: an orchestrator that is already at the
        // depth floor cannot itself spawn, so it is demoted to a leaf rather
        // than handed a tool that would only be refused.
        // Where this child will actually run. Refused up front when the caller
        // asked for somewhere else, so a swarm cannot silently pile N workers into
        // one directory under the belief that each got its own.
        let cwd = resolve_child_cwd(worker.working_dir.as_deref())?;

        let child_depth = self.parent_depth + 1;
        let max_depth = MAX_SPAWN_DEPTH.load(Ordering::Relaxed);

        // Determine effective role based on depth and orchestrator enabled
        let effective_role = if role == SubAgentRole::Orchestrator {
            let orchestrator_ok =
                ORCHESTRATOR_ENABLED.load(Ordering::Relaxed) == 1 && child_depth < max_depth;
            if orchestrator_ok {
                SubAgentRole::Orchestrator
            } else {
                SubAgentRole::Leaf
            }
        } else {
            SubAgentRole::Leaf
        };

        let schema = match coerce_output_schema(output_schema) {
            Ok(schema) => schema,
            Err(error) => return Err(error.into()),
        };
        let context: Option<String> = worker.context;
        // Taken before the loop: `worker` is partly moved above, and the prompt
        // is rebuilt on every schema retry, so the list has to outlive one turn.
        let worker_siblings: Vec<(String, String)> = worker.siblings.clone();

        let mut retry_errors: Vec<String> = Vec::new();
        let mut attempts: usize = 0;
        loop {
            // Build child system prompt based on role
            let mut system_prompt = build_child_system_prompt(
                goal,
                context.as_deref(),
                effective_role,
                child_depth,
                max_depth,
                &cwd,
                &worker_siblings,
            );
            // Append the full OUTPUT CONTRACT only on the first attempt — the
            // retry message explicitly tells the child NOT to re-read the
            // schema (hermes: "no schema re-paste", errors verbatim only).
            if let Some(schema) = schema.as_ref()
                && attempts == 0
            {
                system_prompt.push_str(&format!("\n\n{}", append_output_contract(None, schema)));
            }
            if !retry_errors.is_empty() {
                system_prompt.push_str(&format!("\n\n{}", build_retry_message(&retry_errors)));
            }

            let answer = self
                .run_child(
                    system_prompt,
                    goal.to_string(),
                    effective_role,
                    max_iterations,
                    timeout_seconds,
                    toolsets.clone(),
                )
                .await?;

            let Some(schema_ref) = schema.as_ref() else {
                return Ok(answer);
            };
            let (valid, errors, candidate) = validate_output(&answer.text, schema_ref);
            if valid {
                // Return the JSON the validator parsed, not the child's raw
                // text: a child that wrapped its object in a fence or a sentence
                // is contract-compliant, and the parent asked for the object.
                return Ok(ChildAnswer {
                    text: candidate.unwrap_or(answer.text),
                    session_id: answer.session_id,
                });
            }
            if attempts >= MAX_SCHEMA_RETRIES {
                // Persistent failure: keep the answer (the parent needs the
                // data) but flag the contract breach explicitly (hermes
                // returns the final text with the errors noted).
                return Ok(ChildAnswer {
                    text: format!(
                        "{}\n\n[SCHEMA VALIDATION FAILED after {MAX_SCHEMA_RETRIES} \
                         retr{} — errors: {}]",
                        answer.text,
                        if MAX_SCHEMA_RETRIES == 1 { "y" } else { "ies" },
                        errors.join("; ")
                    ),
                    session_id: answer.session_id,
                });
            }
            retry_errors = errors;
            attempts += 1;
        }
    }

    /// Best-effort send of a child-lifecycle event onto the parent channel.
    /// `try_send` so a full/slow parent channel can never stall a child.
    fn emit_subagent(&self, event: &AgentEvent) {
        if let Some(tx) = &self.event_tx {
            let _ = tx.try_send(event.clone());
        }
    }

    /// Run a single child agent to completion with the given system prompt.
    /// Shared by the sync, batch, and background delegation paths so depth
    /// limits, tool inheritance, and result shaping stay in one place.
    ///
    /// Returns the child's answer AND the session it ran in: the parent needs
    /// the id to read a truncated tail back, and nothing else carries it.
    ///
    /// Brackets the child run with `AgentEvent::SubagentStarted` /
    /// `SubagentStopped` so the TUI can show lifecycle for headless children
    /// (whose own events are deliberately not forwarded to the parent).
    #[allow(clippy::too_many_arguments)]
    async fn run_child(
        &self,
        system_prompt: String,
        goal: String,
        role: SubAgentRole,
        max_iterations: Option<u32>,
        timeout_seconds: u64,
        toolsets: Option<Vec<String>>,
    ) -> std::result::Result<ChildAnswer, BoxedToolError> {
        // Determine effective toolsets based on role, the parent's toolsets
        // and the subset the caller asked for.
        let child_toolsets = self.compute_child_toolsets(role, toolsets.as_deref());
        // A child whose toolset resolved to NOTHING still starts (the hermes
        // "all supplied toolsets were stripped" case is deliberate — see
        // `compute_child_toolsets`), but it must not quietly answer as if it had
        // read the repository. MEASURED 2026-11-14: the parent received an
        // ordinary successful tool result, indistinguishable from a real read,
        // and its "I had no tools" was indistinguishable from "I chose not to
        // act". Tell the child so, and tell the parent so.
        let zero_tools = child_toolsets.is_empty();
        let system_prompt = if zero_tools {
            format!(
                "{system_prompt}\n\nNOTE: this delegation grants you NO tools. You \
                 cannot read files, run commands or look anything up; answer from \
                 your own knowledge and say plainly in your answer that you had no \
                 tools."
            )
        } else {
            system_prompt
        };

        let raw_client = OpenAIClient::from_shared_http_client(
            self.client_config.clone(),
            self.http_client.clone(),
        );
        let client: Box<dyn ModelClient> = Box::new(OpenAIModelClient::new(raw_client));

        let max_iters: usize = max_iterations.unwrap_or_else(default_child_max_iterations) as usize;
        let config = AgentConfig {
            model: self.model.clone(),
            stream: false,
            system_prompt: Some(system_prompt),
            max_iterations: max_iters,
            // A child is HEADLESS: `permission_tx` is None, so the only way a
            // gated call can run is the allowlist. Inheriting the parent's is
            // what makes "allow always" mean the same thing for a delegation as
            // it does for the operator (audit finding 2). A child can never be
            // prompted, so it can never widen the set by persisting to it.
            approval_allowlist: self.parent_approval_allowlist.iter().cloned().collect(),
            approval_allowlist_path: self.parent_approval_allowlist_path.clone(),
            ..AgentConfig::default()
        };

        let registry = ToolRegistry::new(config.tool_timeout);
        // Register tools based on child toolsets (filtered)
        self.register_child_tools(&registry, &child_toolsets).await;

        // Child agents run HEADLESS: their events are NOT forwarded to the
        // parent channel. Forwarding interleaved the child's Content/Done/
        // IterationComplete into the parent's event stream — the gateway
        // runner treated a child's Done as turn-end mid-parent-turn,
        // breaking chronological rendering around delegations (R35). The
        // delegation result still returns to the parent as the tool result
        // text (hermes parity: delegation = one tool line + result).
        let agent = OperantAgent::new(config, client, registry, self.database.clone());

        // Run with timeout
        let timeout_duration = Duration::from_secs(timeout_seconds.max(30));
        let subagent_id = next_subagent_id();
        // The lifecycle pair is closed by a guard, not by the happy path: the
        // tool future races the interrupt flag (agent/stream.rs) and the outer
        // tool timeout, so a cancelled or timed-out parent drops this future
        // mid-await and a plain `emit` after the await never runs — the operator
        // is left with a subagent that started and never ended (MEASURED
        // 2026-11-14).
        let mut lifecycle = SubagentLifecycle::new(self, subagent_id.clone());
        self.emit_subagent(&AgentEvent::SubagentStarted {
            subagent_id: subagent_id.clone(),
            role: format!("{role:?}").to_lowercase(),
            depth: self.parent_depth + 1,
        });
        let result = timeout(timeout_duration, agent.run(goal)).await;

        let (status, summary) = match &result {
            Ok(Ok(message)) => ("completed", preview_of(&message.content, 200)),
            Ok(Err(error)) => ("failed", preview_of(&error.to_string(), 200)),
            Err(_) => ("timeout", format!("timed out after {timeout_seconds}s")),
        };
        lifecycle.finish(status, summary);

        // Same flag for the parent: the answer carries the banner, so a parent
        // that ignores the NOTE still cannot mistake a guess for a read.
        let answer = child_outcome(result, timeout_seconds)?;
        // Read AFTER the turn: the prologue mints the id on the first turn, so
        // before the run there is nothing to read. `run` takes `&self`, so the
        // agent is still here and the id is the one the store wrote under.
        let session_id = agent.session_id();
        Ok(ChildAnswer {
            text: flag_no_tools(zero_tools, answer),
            session_id,
        })
    }

    /// Fan out a **swarm**: run every task as a real worker session and
    /// collect one outcome per worker.
    ///
    /// The spawn guard is consulted FIRST, before a single worker is built.
    /// Capacity is claimed in one atomic reservation, so two parents fanning
    /// out at the same moment cannot both pass a check and collectively
    /// overshoot the process-wide budget — the N*M case. Workers beyond the
    /// reservation are reported as `Refused` rather than silently dropped, and
    /// they were never built, so a refused worker costs no model stream.
    async fn call_batch(
        &self,
        tasks: Vec<WorkerSpec>,
    ) -> std::result::Result<String, BoxedToolError> {
        if tasks.is_empty() {
            return Err("Swarm requires at least one task".into());
        }

        // ── Spawn guard, enforced BEFORE spawning ──
        let child_depth = self.parent_depth + 1;
        // Snapshot the budget BEFORE the reservation. `reserve` adds this
        // swarm's own slots to LIVE_WORKERS, so a shortfall reason built from a
        // later read tells the operator the budget was already full on entry —
        // MEASURED 2026-11-14: "only 3 of 5 fit (4 live of 4 allowed)" when one
        // slot had in fact been free. The same condition then read two different
        // ways in one turn, because a nested delegate_task reports the honest
        // pre-reservation count.
        let live_before = live_worker_count();
        let max_allowed = max_global_workers();
        let reservation = SpawnPermit::reserve(tasks.len(), child_depth);
        let admitted_slots = reservation.as_ref().map_or(0, SpawnPermit::slots);
        let refusal_text = reservation.as_ref().err().map(SpawnRefusal::to_string);

        let mut outcomes: Vec<WorkerOutcome> = Vec::with_capacity(tasks.len());
        let mut admitted: Vec<WorkerSpec> = Vec::with_capacity(admitted_slots);
        for worker in tasks {
            if admitted.len() < admitted_slots {
                admitted.push(worker);
            } else {
                // Report the shortfall instead of hiding it — a swarm that
                // quietly ran 2 of 5 requested tasks is a lie the parent
                // cannot detect.
                let reason = refusal_text.clone().unwrap_or_else(|| {
                    format!(
                        "Swarm breadth cap reached: only {admitted_slots} of the requested \
                         workers fit in the process-wide budget ({live_before} live of \
                         {max_allowed} allowed before this swarm)."
                    )
                });
                outcomes.push(WorkerOutcome {
                    id: worker.id,
                    goal: worker.goal,
                    status: WorkerStatus::Refused(reason),
                });
            }
        }

        // The reservation is held for the whole swarm, so slots are not
        // returned as individual workers finish. That is deliberately
        // conservative: over-reserving shrinks nothing that a concurrent
        // parent needed, whereas releasing early would let a long swarm
        // overshoot the budget it was admitted against.
        let _permit = reservation.map_err(|refusal| refusal.to_string())?;

        // Per-swarm in-flight cap, itself bounded by what the guard admitted —
        // the pool can never be wider than the reservation.
        let max_in_flight = (MAX_CONCURRENT_CHILDREN.load(Ordering::Relaxed) as usize)
            .clamp(1, 10)
            .min(admitted_slots.max(1));

        // An operator surface reads this while the swarm runs. MEASURED 2026-11-14
        // (audit finding 4): a foreground swarm recorded nothing anywhere — the
        // async registry holds only background delegations — so a running swarm was
        // invisible in the one pane an operator reads, and the first sign of it was
        // its result landing in the transcript.
        for worker in &admitted {
            crate::tools::foreground_swarm::begin(worker.id.clone(), preview_of(&worker.goal, 80));
        }

        let tool = self.clone_for_task();
        outcomes.extend(
            run_swarm(admitted, max_in_flight, move |worker| -> WorkerRun {
                let tool = tool.clone_for_task();
                Box::pin(async move {
                    tool.call_unguarded(worker)
                        .await
                        .map_err(|error| crate::error::Error::Agent(error.to_string()))
                })
            })
            .await,
        );

        // Close each row with the status the operator sees, not just the model's.
        for outcome in &outcomes {
            let (status, detail) = match &outcome.status {
                WorkerStatus::Completed(_) => ("completed", None),
                WorkerStatus::Failed(error) => ("failed", Some(error.clone())),
                WorkerStatus::TimedOut { after_secs } => {
                    ("timed_out", Some(format!("timed out after {after_secs}s")))
                }
                WorkerStatus::Panicked => ("panicked", Some("worker panicked".to_string())),
                WorkerStatus::Refused(reason) => ("refused", Some(reason.clone())),
            };
            crate::tools::foreground_swarm::finish(&outcome.id, status, detail);
        }

        let mut results: Vec<String> = Vec::new();
        let mut errors: Vec<String> = Vec::new();
        for outcome in &outcomes {
            match &outcome.status {
                WorkerStatus::Completed(answer) => results.push(answer.text.clone()),
                WorkerStatus::Failed(error) => errors.push(error.clone()),
                WorkerStatus::TimedOut { after_secs } => {
                    errors.push(format!("worker timed out after {after_secs}s"))
                }
                WorkerStatus::Panicked => errors.push("worker panicked".to_string()),
                WorkerStatus::Refused(reason) => errors.push(reason.clone()),
            }
        }

        if results.is_empty() && !errors.is_empty() {
            return Err(format!("All workers failed: {}", errors.join("; ")).into());
        }

        Ok(format_swarm_outcomes(&outcomes))
    }

    /// Dispatch a delegation in the background and return a handle immediately
    /// (hermes `async_delegation.py` parity). The child runs on a spawned
    /// tokio task; on completion the record transitions to completed/failed
    /// and an `AgentEvent::AsyncDelegation` is pushed onto the parent's event
    /// channel (when one exists) so the CLI/TUI can surface the outcome. The
    /// agent polls progress with `delegate_task(query="<id>")`.
    #[expect(
        clippy::too_many_arguments,
        reason = "the worker's full instruction set is passed as arguments; the same fields as WorkerSpec"
    )]
    async fn dispatch_background(
        &self,
        goal: String,
        context: Option<String>,
        role: SubAgentRole,
        max_iterations: Option<u32>,
        timeout_seconds: u64,
        output_schema: Option<Value>,
        toolsets: Option<Vec<String>>,
        working_dir: Option<String>,
        parent_session: Option<String>,
    ) -> std::result::Result<String, BoxedToolError> {
        // Refuse BEFORE the handle exists when there is no breadth headroom at
        // all. MEASURED 2026-11-14: with the process-wide cap already full, this
        // returned {"status":"dispatched"} for a child that could not reserve a
        // slot — the parent spent a turn on a handle for work that never
        // started, and learned of it only by polling. This is a PEEK, not the
        // claim (the atomic reservation still happens inside `call`); a child
        // that loses the race anyway is marked Failed, never Pending.
        if breadth_is_exhausted() {
            return Err(format!(
                "Spawn refused: {}/{} worker slots are already live. Slots free as \
                 the running workers finish — retry, or run this synchronously.",
                live_worker_count(),
                max_global_workers()
            )
            .into());
        }

        let Some(delegation_id) = async_delegation::try_create_record(
            &goal,
            &self.model,
            async_delegation::DEFAULT_MAX_ASYNC_CHILDREN,
            parent_session.as_deref(),
        ) else {
            return Err(format!(
                "Too many background delegations in flight (max {}); \
                 wait for one to complete or use synchronous delegation.",
                async_delegation::DEFAULT_MAX_ASYNC_CHILDREN
            )
            .into());
        };

        let tool = self.clone_for_task();
        let event_tx = self.event_tx.clone();
        let spawn_goal = goal.clone();
        let spawn_id = delegation_id.clone();
        tokio::spawn(async move {
            // `catch_unwind` is what makes a panic DATA here, exactly as it is
            // for a swarm worker (`run_swarm`, :392). Without it a panic
            // anywhere in the child — a bad unwrap in a tool, an arithmetic
            // overflow — dies before either mark_completed or mark_failed, so
            // the record stays Pending for the life of the process, the
            // pending count never drops, and after three of them every session
            // in the process is refused with "Too many background delegations
            // in flight" and no error naming the cause. The join handle is
            // dropped, so nobody else observes the panic either.
            let outcome = await_child(tool.call_with_session(
                spawn_goal,
                context,
                role,
                max_iterations,
                timeout_seconds,
                output_schema,
                toolsets,
                working_dir,
            ))
            .await;
            match outcome {
                Ok(answer) => {
                    // The session join first (audit finding 10): the row must
                    // name the session that ran it, or the operator can see
                    // that work happened without being able to open it.
                    if let Some(session_id) = answer.session_id.as_deref() {
                        async_delegation::mark_session(&spawn_id, session_id);
                    }
                    // Persist the record FIRST — the event send below is
                    // best-effort UI surfacing only (try_send so a full/slow
                    // parent channel can never stall the background task).
                    async_delegation::mark_completed(&spawn_id, &answer.text);
                    if let Some(tx) = event_tx {
                        let _ = tx.try_send(AgentEvent::AsyncDelegation {
                            delegation_id: spawn_id.clone(),
                            status: "completed".to_string(),
                            summary: preview_of(&answer.text, 200),
                        });
                    }
                }
                Err(error) => {
                    async_delegation::mark_failed(&spawn_id, &error.to_string());
                    if let Some(tx) = event_tx {
                        let _ = tx.try_send(AgentEvent::AsyncDelegation {
                            delegation_id: spawn_id.clone(),
                            status: "failed".to_string(),
                            summary: error.to_string(),
                        });
                    }
                }
            }
        });

        Ok(serde_json::json!({
            "delegation_id": delegation_id,
            "status": "dispatched",
            "model": self.model,
            "goal": preview_of(&goal, 80),
            "poll": format!("delegate_task query=\"{delegation_id}\""),
        })
        .to_string())
    }

    fn clone_for_task(&self) -> Self {
        Self {
            client_config: self.client_config.clone(),
            http_client: self.http_client.clone(),
            model: self.model.clone(),
            parent_depth: self.parent_depth,
            parent_toolsets: self.parent_toolsets.clone(),
            parent_disabled_tools: self.parent_disabled_tools.clone(),
            parent_disabled_toolsets: self.parent_disabled_toolsets.clone(),
            parent_approval_allowlist: self.parent_approval_allowlist.clone(),
            parent_approval_allowlist_path: self.parent_approval_allowlist_path.clone(),
            database: self.database.clone(),
            event_tx: self.event_tx.clone(),
        }
    }

    fn compute_child_toolsets(
        &self,
        role: SubAgentRole,
        requested: Option<&[String]>,
    ) -> Vec<String> {
        // Whether the parent supplied an explicit toolset list at all. hermes
        // semantics: an EMPTY parent list means "use defaults" (DEFAULT_TOOLSETS
        // fallback), but a NON-EMPTY parent list means "intersect/strip only"
        // — if every supplied toolset gets stripped, the child gets ZERO
        // tools, never a silent re-addition of tools the parent withheld.
        let parent_supplied = !self.parent_toolsets.is_empty();

        // Start with the parent's toolsets, with the parent's explicit
        // disabled toolsets removed (hermes: children never gain tools the
        // parent lacks). Most core tools live in the "builtin" toolset.
        let mut toolsets: Vec<String> = self
            .parent_toolsets
            .iter()
            .filter(|ts| !self.parent_disabled_toolsets.contains(ts.as_str()))
            .cloned()
            .collect();

        // Hermes fallback: when the parent supplies NO toolset list, children
        // get the default toolset (builtin core tools), not nothing. Without
        // this, children whose parent passes an empty toolset list (the CLI
        // registers the tool with `vec![]`) would receive ZERO tools. Guarded
        // by parent_supplied so a parent that explicitly supplied toolsets
        // (even ones that all got stripped) never gets builtin re-added, and
        // never re-adds a toolset the parent explicitly disabled.
        if toolsets.is_empty()
            && !parent_supplied
            && !self.parent_disabled_toolsets.contains("builtin")
        {
            toolsets.push("builtin".to_string());
        }

        // Remove toolsets that are blocked for ALL children. Mirrors hermes
        // delegate_tool.py `_strip_blocked_tools` + `DELEGATE_BLOCKED_TOOLS`:
        // children never get delegation/clarify/memory/code_execution from the
        // parent's toolset inheritance.
        let blocked_toolsets: [&str; 4] = ["delegation", "clarify", "memory", "code_execution"];
        toolsets.retain(|ts| !blocked_toolsets.contains(&ts.as_str()));

        // The caller asked for a SUBSET: narrow, never widen. A requested
        // toolset the parent does not have is simply not there — the
        // parent's policy is the ceiling, and the request can only lower
        // it. (Applied before the role's own grant below, so an
        // orchestrator keeps its delegation tool.)
        if let Some(requested) = requested {
            toolsets.retain(|ts| requested.iter().any(|r| r == ts));
        }

        // Orchestrators retain the delegation toolset: the delegate_task tool
        // itself is re-registered for orchestrator children in
        // register_child_tools (hermes `_blocked_toolsets_for_role` discards
        // "delegate_task" from the blocklist when role == "orchestrator").
        if role == SubAgentRole::Orchestrator {
            toolsets.push("delegation".to_string());
        }

        toolsets
    }

    async fn register_child_tools(&self, registry: &ToolRegistry, toolsets: &[String]) {
        use super::browser_tool::BrowserTool;
        use super::datetime_tool::{DateTimeTool, TimestampTool};
        use super::file_state::FileStateTool;
        use super::file_tools::{FileListTool, FileReadTool, FileSearchTool, FileWriteTool};
        use super::http_tool::HttpRequestTool;
        use super::patch_tool::PatchTool;
        use super::terminal_tool::TerminalTool;
        use super::vision_tool::VisionTool;
        use super::web_tools::{WebFetchTool, WebSearchTool};

        let toolset_set: std::collections::HashSet<&str> =
            toolsets.iter().map(String::as_str).collect();

        // Only register a tool when BOTH the child's computed toolset list
        // allows its toolset AND the parent did not explicitly disable it.
        // The child's toolset list was already filtered by the parent's
        // disabled toolsets + the hermes child-blocked list, so a tool whose
        // toolset is absent here is deliberately withheld from children.
        // Most tools default to toolset "builtin", which is always present
        // unless explicitly disabled by the parent.
        let builtin = "builtin";
        self.register_if_allowed(registry, &toolset_set, "terminal", builtin, TerminalTool)
            .await;
        self.register_if_allowed(registry, &toolset_set, "file_read", builtin, FileReadTool)
            .await;
        self.register_if_allowed(registry, &toolset_set, "file_write", builtin, FileWriteTool)
            .await;
        self.register_if_allowed(
            registry,
            &toolset_set,
            "file_search",
            builtin,
            FileSearchTool,
        )
        .await;
        self.register_if_allowed(registry, &toolset_set, "file_list", builtin, FileListTool)
            .await;
        self.register_if_allowed(registry, &toolset_set, "file_state", builtin, FileStateTool)
            .await;
        self.register_if_allowed(registry, &toolset_set, "web_search", builtin, WebSearchTool)
            .await;
        self.register_if_allowed(registry, &toolset_set, "web_fetch", builtin, WebFetchTool)
            .await;
        self.register_if_allowed(registry, &toolset_set, "browser", builtin, BrowserTool)
            .await;
        self.register_if_allowed(
            registry,
            &toolset_set,
            "http_request",
            builtin,
            HttpRequestTool,
        )
        .await;
        self.register_if_allowed(registry, &toolset_set, "patch", builtin, PatchTool)
            .await;
        self.register_if_allowed(registry, &toolset_set, "datetime", builtin, DateTimeTool)
            .await;
        self.register_if_allowed(registry, &toolset_set, "timestamp", builtin, TimestampTool)
            .await;
        self.register_if_allowed(
            registry,
            &toolset_set,
            "vision_analyze",
            builtin,
            VisionTool,
        )
        .await;

        // Recursive delegation: grant delegate_task ONLY to orchestrator
        // children (hermes: leaf children must never recursively delegate;
        // orchestrators retain the tool). The toolsets vec contains
        // "delegation" iff the effective role is orchestrator (see
        // compute_child_toolsets).
        if toolset_set.contains("delegation") {
            let raw_client = OpenAIClient::from_shared_http_client(
                self.client_config.clone(),
                self.http_client.clone(),
            );
            let child = SubAgentTool::with_parent_tool_policy(
                &raw_client,
                self.model.clone(),
                self.parent_depth + 1,
                self.parent_toolsets.clone(),
                self.parent_disabled_tools.clone(),
                self.parent_disabled_toolsets.clone(),
                self.database.clone(),
                self.event_tx.clone(),
            )
            // A grandchild is as headless as its parent: the allowlist has to
            // survive every hop or depth 2 loses what depth 1 had.
            .with_parent_approval(
                self.parent_approval_allowlist.clone(),
                self.parent_approval_allowlist_path.clone(),
            );
            let _ = registry.register(child).await;
        }
    }

    /// Register one child tool unless the parent disabled it or the child's
    /// computed toolset list does not include its toolset.
    async fn register_if_allowed<T: OperantTool + 'static>(
        &self,
        registry: &ToolRegistry,
        toolset_set: &std::collections::HashSet<&str>,
        name: &'static str,
        toolset: &'static str,
        tool: T,
    ) {
        if self.parent_disabled_tools.contains(name)
            || self.parent_disabled_toolsets.contains(toolset)
            || !toolset_set.contains(toolset)
        {
            return;
        }
        let _ = registry.register(tool).await;
    }

    fn ensure_supported_model(&self) -> std::result::Result<(), BoxedToolError> {
        if is_llama_model(&self.model) {
            return Err(format!(
                "Sub-agent model '{}' is rejected because Llama-family models are unsuitable for this tool-calling context",
                self.model
            )
            .into());
        }

        Ok(())
    }
}

/// Build the system prompt for a child agent based on role
fn build_child_system_prompt(
    goal: &str,
    context: Option<&str>,
    role: SubAgentRole,
    child_depth: u32,
    max_spawn_depth: u32,
    working_dir: &Path,
    siblings: &[(String, String)],
) -> String {
    let mut parts = vec![
        "You are a focused subagent working on a specific delegated task.".to_string(),
        format!("\nYOUR TASK:\n{}", goal),
        // The child used to be told nothing about where it runs, so a relative path
        // in its answer was a guess about the process cwd — and in a swarm every
        // sibling shares it.
        format!(
            "\nWORKING DIRECTORY:\n{}\nEvery relative path you use resolves there. \
             If your task needs a different directory, say so in your answer rather than \
             assuming one.",
            working_dir.display()
        ),
    ];

    // Siblings share this working directory, so the one thing a worker can do
    // about a collision is say so in its report. Absent for a single child, and
    // for an orchestrator's own children the list is empty (they fan out from
    // their own parent, not from this one).
    if !siblings.is_empty() {
        let listed: Vec<String> = siblings
            .iter()
            .map(|(id, goal)| format!("- {id}: {goal}"))
            .collect();
        parts.push(format!(
            "\nOTHER WORKERS IN THIS FAN-OUT (you share ONE working directory with them):\n{}\
             \nThey cannot see your work and you cannot see theirs. If your task touches a file \
             another worker's task also names, SAY SO in your answer — name the file and the \
             other worker's id — rather than editing it silently.",
            listed.join("\n")
        ));
    }

    if let Some(ctx) = context
        && !ctx.trim().is_empty()
    {
        parts.push(format!("\nCONTEXT:\n{}", ctx));
    }

    parts.push(
        "\nComplete this task using the tools available to you. ".to_string()
            + "When finished, provide a clear, concise summary of:\n"
            + "- What you did\n"
            + "- What you found or accomplished\n"
            + "- Any files you created or modified\n"
            + "- Any issues encountered\n\n"
            + "Be thorough but concise -- your response is returned to the "
            + "parent agent as a summary.",
    );

    // Add orchestrator-specific instructions
    if role == SubAgentRole::Orchestrator {
        let child_note = if child_depth + 1 >= max_spawn_depth {
            "Your own children MUST be leaves (cannot delegate further) because they would be at the depth floor.".to_string()
        } else {
            "Your own children can themselves be orchestrators or leaves, depending on the `role` you pass to delegate_task. Default is 'leaf'; pass role='orchestrator' explicitly when a child needs to further decompose its work.".to_string()
        };

        parts.push(
            "\n## Subagent Spawning (Orchestrator Role)\n".to_string()
                + "You have access to the `delegate_task` tool and CAN spawn "
                + "your own subagents to parallelize independent work.\n\n"
                + "WHEN to delegate:\n"
                + "- The goal decomposes into 2+ independent subtasks that can "
                + "run in parallel (e.g. research A and B simultaneously).\n"
                + "- A subtask is reasoning-heavy and would flood your context "
                + "with intermediate data.\n\n"
                + "WHEN NOT to delegate:\n"
                + "- Single-step mechanical work — do it directly.\n"
                + "- Trivial tasks you can execute in one or two tool calls.\n"
                + "- Re-delegating your entire assigned goal to one worker "
                + "(that's just pass-through with no value added).\n\n"
                + "Coordinate your workers' results and synthesize them before "
                + "reporting back to your parent. You are responsible for the "
                + "final summary, not your workers.\n\n"
                + &format!(
                    "NOTE: You are at depth {}. The delegation tree ",
                    child_depth
                )
                + &format!(
                    "is capped at max_spawn_depth={}. {}",
                    max_spawn_depth, child_note
                ),
        );
    }

    parts.join("\n")
}

/// How much of each worker's report is pushed into the parent's context.
///
/// MEASURED 2026-11-14 (audit finding 3): this was 4000, so a 6-worker swarm
/// pushed ~24 KB into the transcript permanently — about 6k tokens the parent
/// pays for whether or not it reads the text. Both references deliver a bounded
/// notification and let the parent PULL: codex returns `{agent_path, status}`
/// and jcode delivers out-of-band through the bus, never into history.
///
/// A PREVIEW rather than a status line, and deliberately not a dispatch-time
/// choice: the parent cannot know at fan-out time which workers it will need,
/// so a "summary vs full" switch forces a guess. A preview is enough to decide
/// which reports matter, and the full text is one call away — which is why
/// iter-2356 had to land first, so the session id in the heading is actually
/// readable. 600 chars is roughly the length of a verdict; only long reports
/// (a file, a transcript) get cut, and those are exactly the ones worth
/// pulling. jcode parity: a completion report is capped before it is injected.
const SWARM_REPORT_PREVIEW_CHARS: usize = 600;

/// How much of a sibling's goal a worker is told.
///
/// The list exists so a worker can NAME a collision, not so it can read every
/// sibling's brief: the full goals are already in the parent's request, and this
/// text is replicated into every child's prompt (audit finding 6).
const SIBLING_GOAL_CHARS: usize = 80;

/// Cap one worker's report to a preview, keeping its own text.
///
/// The notice names the session and the CALL that reads the rest. MEASURED
/// 2026-11-14 (audit finding 4): it used to say only "the full text is in the
/// session store", a pointer with no key — the child's session id was minted
/// inside `turn_context` and never carried back, so neither the parent model nor
/// the operator could ask for the tail. iter-2320 made the id travel; iter-2356
/// made it openable (`session_search(session_id=…)` reads a session tail). A
/// pointer that names the session but not the call still costs the reader a
/// hunt, so the notice says which tool to use.
fn cap_worker_report(answer: &str, session_id: Option<&str>) -> String {
    if answer.chars().count() <= SWARM_REPORT_PREVIEW_CHARS {
        return answer.to_string();
    }
    let mut out: String = answer.chars().take(SWARM_REPORT_PREVIEW_CHARS).collect();
    match session_id {
        Some(id) => out.push_str(&format!(
            "\n[preview: {} of {} chars — session_search(session_id=\"{id}\") reads the rest]",
            SWARM_REPORT_PREVIEW_CHARS,
            answer.chars().count()
        )),
        // No id read back: say so rather than naming a store the parent cannot
        // address. A child that never reached its prologue has no session.
        None => out.push_str(&format!(
            "\n[preview: {} of {} chars — this worker reported no session id, so the rest \
             is not addressable]",
            SWARM_REPORT_PREVIEW_CHARS,
            answer.chars().count()
        )),
    }
    out
}

/// Render swarm outcomes for the parent, one section per worker.
///
/// Each section is headed by the worker's own goal so the parent can attribute
/// a result to the request that produced it — a positional `Task 3` heading is
/// useless once some workers were refused or failed and the survivors shift.
/// The heading also carries the worker id and, for a completed worker, the
/// session it ran in: those two are the only handles the parent gets (audit
/// finding 4), so a section that omitted them could not be followed up.
/// Non-completed statuses are rendered inline, labelled, rather than dropped
/// into a separate error list that loses the goal association. Every worker is
/// reported, including the ones that only failed: a section that silently
/// disappears reads as a worker that never ran.
fn format_swarm_outcomes(outcomes: &[WorkerOutcome]) -> String {
    if outcomes.is_empty() {
        return "No results".to_string();
    }

    let mut output = String::from("## Swarm Results\n\n");
    for outcome in outcomes {
        let goal = preview_of(&outcome.goal, 80);
        // The session id exists only on a completed worker; a failed or refused
        // one never reached its prologue, so there is nothing to name.
        let session = match &outcome.status {
            WorkerStatus::Completed(answer) => answer.session_id.as_deref(),
            _ => None,
        };
        let heading = match session {
            Some(id) => format!("{} \u{00b7} {goal} (session {id})", outcome.id),
            None => format!("{} \u{00b7} {goal}", outcome.id),
        };
        match &outcome.status {
            WorkerStatus::Completed(answer) => {
                output.push_str(&format!(
                    "### {heading}\n{}\n\n",
                    cap_worker_report(&answer.text, answer.session_id.as_deref())
                ));
            }
            WorkerStatus::Failed(error) => {
                output.push_str(&format!("### {heading}\n[failed] {error}\n\n"));
            }
            WorkerStatus::TimedOut { after_secs } => {
                output.push_str(&format!(
                    "### {heading}\n[timed out after {after_secs}s]\n\n"
                ));
            }
            WorkerStatus::Panicked => {
                output.push_str(&format!("### {heading}\n[panicked]\n\n"));
            }
            WorkerStatus::Refused(reason) => {
                output.push_str(&format!("### {heading}\n[not spawned] {reason}\n\n"));
            }
        }
    }

    output.trim_end().to_string()
}

#[async_trait]
impl OperantTool for SubAgentTool {
    fn name(&self) -> &str {
        TOOL_NAME
    }

    fn description(&self) -> &str {
        "Delegate work to isolated sub-agents. Use this for deep analysis, \
        specialized coding investigation, architectural review, or other self-contained work. \
        Modes: single (goal + context), SWARM (tasks array — each entry becomes a real worker \
        session with its own conversation, tools and bounded timeout, all running in parallel), \
        and background (background=true returns a handle immediately — poll with query=\"<id>\"). \
        Optional output_schema enforces a structured JSON contract on the child's final answer \
        (exactly one bounded retry on validation failure). \
        The sub-agent has a fresh conversation and does not inherit parent memory. \
        Role 'leaf' (default) cannot delegate further; role 'orchestrator' can spawn its own \
        sub-agents. \
        SWARM COST: every worker is its own model stream and its own tool-call spend, so N \
        workers cost roughly N times one worker. Fan out only for genuinely independent \
        subtasks; a task you could do in one tool call should not get a worker. \
        SPAWN GUARD: depth and process-wide breadth are enforced BEFORE any worker starts. \
        Requests past the cap are refused or truncated with an explicit '[not spawned]' marker \
        rather than silently dropped, and each worker has a hard timeout so a wedged one cannot \
        stall the rest."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<SubAgentArgs>(
            TOOL_NAME,
            "Delegate a focused task to an isolated Operant sub-agent",
        )
    }

    async fn execute(&self, args: Value, context: ToolContext) -> ToolResult {
        let parsed = match parse_args(args) {
            Ok(parsed) => parsed,
            Err(error) => return ToolResult::error(TOOL_NAME, error),
        };
        // MEASURED 2026-11-14 (audit finding 2): this parameter was `_context`
        // and nothing read it, so a background delegation had no way to record
        // which session dispatched it and the hub could not place a worker under
        // its parent. The id is already here — `tool_context` puts it on both
        // channels (iter-2354) — so the record just has to carry it.
        let parent_session = context.resolved_session_id().map(str::to_string);

        // Four dispatch paths, one per method, so each can be read (and edited)
        // on its own. This function was 143 lines with all four inline, and the
        // background path alone took three separate fixes before the split.

        // Query mode: poll a previously dispatched background delegation.
        if let Some(query_id) = parsed.query.clone() {
            return self.poll_delegation(&query_id);
        }

        // Background mode: dispatch and return a handle immediately.
        if parsed.background.unwrap_or(false) {
            return self.dispatch_in_background(parsed, parent_session).await;
        }

        // Swarm mode: one real worker session per task, fanned out behind the
        // process-wide spawn guard.
        if parsed.tasks.is_some() {
            return self.run_a_swarm(parsed).await;
        }

        self.run_one(parsed).await
    }
}

// The four dispatch paths, on the inherent impl rather than the trait: they are
// `delegate_task`'s internal branching, not part of the `OperantTool` contract.
impl SubAgentTool {
    /// Query mode: poll a previously dispatched background delegation.
    fn poll_delegation(&self, query_id: &str) -> ToolResult {
        match async_delegation::get_record(query_id) {
            Some(record) => {
                let content = serde_json::to_value(&record).unwrap_or(Value::Null);
                ToolResult::success_with_name(TOOL_NAME, TOOL_NAME, content)
            }
            None => ToolResult::error(
                TOOL_NAME,
                format!(
                    "No background delegation found with id '{query_id}'. It either never \
                     existed or is gone: this process keeps the {} most recently FINISHED \
                     delegations, and a restart forgets all of them. Do not re-dispatch the \
                     work on this error alone — check the session store for the child's \
                     output first; if the record was pending when the process restarted, its \
                     child died with the process and the work must be re-run.",
                    async_delegation::MAX_RETAINED_TERMINAL
                ),
            ),
        }
    }

    /// Background mode: dispatch and return a handle immediately.
    async fn dispatch_in_background(
        &self,
        parsed: SubAgentArgs,
        parent_session: Option<String>,
    ) -> ToolResult {
        let Some(goal) = parsed.goal else {
            return ToolResult::error(
                TOOL_NAME,
                "Background delegation requires 'goal'".to_string(),
            );
        };
        if parsed.tasks.is_some() {
            return ToolResult::error(
                TOOL_NAME,
                "Background mode supports a single 'goal', not batch 'tasks'".to_string(),
            );
        }
        let role = parsed.role.unwrap_or(SubAgentRole::Leaf);
        let timeout = parsed
            .timeout_seconds
            .unwrap_or(DEFAULT_CHILD_TIMEOUT_SECONDS);
        match self
            .dispatch_background(
                goal,
                parsed.context,
                role,
                parsed.max_iterations,
                timeout,
                parsed.output_schema,
                parsed.toolsets,
                parsed.working_dir,
                parent_session,
            )
            .await
        {
            Ok(handle) => ToolResult::success_with_name(TOOL_NAME, TOOL_NAME, handle),
            Err(error) => ToolResult::error(TOOL_NAME, error.to_string()),
        }
    }

    /// Swarm mode: one real worker session per task, fanned out behind the
    /// process-wide spawn guard.
    async fn run_a_swarm(&self, parsed: SubAgentArgs) -> ToolResult {
        let Some(tasks) = parsed.tasks else {
            return ToolResult::error(TOOL_NAME, "Tasks array cannot be empty".to_string());
        };
        if tasks.is_empty() {
            return ToolResult::error(TOOL_NAME, "Tasks array cannot be empty".to_string());
        }
        let role = parsed.role.unwrap_or(SubAgentRole::Leaf);
        let timeout = parsed
            .timeout_seconds
            .unwrap_or(DEFAULT_CHILD_TIMEOUT_SECONDS);
        // Ids and goals first, so every worker can be told about the others
        // (audit finding 6). Clipped per sibling: the list is context the parent
        // pays for once per worker, so a 100-task fan-out must not put 100 full
        // goals into every child's prompt.
        let roster: Vec<(String, String)> = tasks
            .iter()
            .map(|task| {
                (
                    next_subagent_id(),
                    preview_of(task.goal.trim(), SIBLING_GOAL_CHARS),
                )
            })
            .collect();
        let workers: Vec<WorkerSpec> = tasks
            .into_iter()
            .zip(roster.iter().cloned())
            .map(|(task, (id, _))| WorkerSpec {
                id,
                goal: task.goal,
                context: task.context,
                role,
                max_iterations: parsed.max_iterations,
                timeout_secs: timeout,
                output_schema: task.output_schema,
                // A task's own request wins over the top-level one.
                toolsets: task.toolsets.or_else(|| parsed.toolsets.clone()),
                working_dir: parsed.working_dir.clone(),
                siblings: Vec::new(), // filled below, once every id exists
            })
            .collect();
        let workers: Vec<WorkerSpec> = workers
            .into_iter()
            .map(|worker| {
                let siblings = roster
                    .iter()
                    .filter(|(other, _)| *other != worker.id)
                    .cloned()
                    .collect();
                WorkerSpec { siblings, ..worker }
            })
            .collect();

        match self.call_batch(workers).await {
            Ok(content) => ToolResult {
                tool_call_id: TOOL_NAME.to_string(),
                name: TOOL_NAME.to_string(),
                success: true,
                content,
                error: None,
                timed_out: false,
            },
            Err(error) => ToolResult::error(TOOL_NAME, error.to_string()),
        }
    }

    /// Single mode: one child, synchronously.
    async fn run_one(&self, parsed: SubAgentArgs) -> ToolResult {
        let Some(goal) = parsed.goal else {
            return ToolResult::error(
                TOOL_NAME,
                "Either 'goal' or 'tasks' must be provided".to_string(),
            );
        };

        let role = parsed.role.unwrap_or(SubAgentRole::Leaf);
        let timeout = parsed
            .timeout_seconds
            .unwrap_or(DEFAULT_CHILD_TIMEOUT_SECONDS);

        match self
            .call(
                goal,
                parsed.context,
                role,
                parsed.max_iterations,
                timeout,
                parsed.output_schema,
                parsed.toolsets,
                parsed.working_dir,
            )
            .await
        {
            Ok(content) => ToolResult {
                tool_call_id: TOOL_NAME.to_string(),
                name: TOOL_NAME.to_string(),
                success: true,
                content,
                error: None,
                timed_out: false,
            },
            Err(error) => ToolResult::error(TOOL_NAME, error.to_string()),
        }
    }
}

fn parse_args(args: Value) -> Result<SubAgentArgs, String> {
    let mut parsed: SubAgentArgs = match args {
        Value::String(s) => serde_json::from_str(&s).map_err(|e| format!("Invalid JSON: {}", e))?,
        value => serde_json::from_value(value).map_err(|e| format!("Invalid arguments: {}", e))?,
    };

    if let Some(ref goal) = parsed.goal
        && goal.trim().is_empty()
    {
        return Err("goal must not be empty".to_string());
    }

    if let Some(ref tasks) = parsed.tasks {
        for (i, task) in tasks.iter().enumerate() {
            if task.goal.trim().is_empty() {
                return Err(format!("tasks[{}].goal must not be empty", i));
            }
        }
    }

    // One floor, applied at the boundary: every downstream read (single,
    // swarm, background) then agrees that a worker cannot be born already
    // expired, instead of each call site remembering to clamp.
    parsed.timeout_seconds = parsed
        .timeout_seconds
        .map(|seconds| seconds.max(MIN_WORKER_TIMEOUT_SECONDS));

    Ok(parsed)
}

fn is_llama_model(model: &str) -> bool {
    model.to_ascii_lowercase().contains("llama")
}

/// One-line preview with a hard character cap (for background handles and
/// completion summaries).
fn preview_of(text: &str, max_chars: usize) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out: String = flat.chars().take(max_chars).collect();
    if flat.chars().count() > max_chars {
        out.push('…');
    }
    out
}

/// Resolve the working directory a child will actually run in.
///
/// A child's cwd IS the process cwd (see `SubAgentArgs::working_dir`), so the only
/// honest answers are "the process one" or a refusal. Accepting a different path
/// and silently ignoring it is the failure this closes: the parent believes it
/// isolated the child and the child edits the shared tree.
fn resolve_child_cwd(requested: Option<&str>) -> std::result::Result<PathBuf, BoxedToolError> {
    let process_cwd =
        std::env::current_dir().map_err(|e| format!("cannot read the working directory: {e}"))?;
    let Some(raw) = requested.map(str::trim).filter(|r| !r.is_empty()) else {
        return Ok(process_cwd);
    };
    let wanted = std::path::Path::new(raw);
    let wanted_abs = if wanted.is_absolute() {
        wanted.to_path_buf()
    } else {
        process_cwd.join(wanted)
    };
    // Compare canonicalised where possible: a symlinked path to the same directory
    // is the same directory, and a typo'd one is a typo.
    let same = match (wanted_abs.canonicalize(), process_cwd.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => wanted_abs == process_cwd,
    };
    if same {
        return Ok(process_cwd);
    }
    let shown = wanted_abs.display();
    Err(format!(
        "a delegated child cannot change the working directory: every child of a parent \
         shares that parent's process working directory ({path}), because turn_context reads \
         std::env::current_dir() per turn and the file tools resolve relative paths through the \
         process. Requested: {shown}. Put the directory in the goal (e.g. \"cd <dir> && ...\") \
         or run the child in its own process; N swarm workers editing one tree is the exact \
         hazard this refuses.",
        path = process_cwd.display()
    )
    .into())
}

/// Prepend the "no tools were granted" banner to a child's answer.
///
/// A child whose toolset resolved to nothing is legal (a parent may narrow every
/// toolset away — see `compute_child_toolsets`), and until now its answer reached
/// the parent as an ordinary successful tool result: indistinguishable from a
/// child that read the repository. The banner is how the parent can tell. The
/// full text is in the session store; the parent needs the fact, not the file.
const NO_TOOLS_BANNER: &str =
    "[no tools were granted to this delegation — this answer came from the model alone]\n";

fn flag_no_tools(zero_tools: bool, answer: String) -> String {
    if zero_tools {
        format!("{NO_TOOLS_BANNER}{answer}")
    } else {
        answer
    }
}

/// Closes the `SubagentStarted` / `SubagentStopped` pair on EVERY exit.
///
/// `run_child` emits Started, awaits the child, then emitted Stopped — with no
/// guard, dropping the future at that await (the parent's interrupt flag, the
/// outer tool timeout, a turn teardown) skipped the stop entirely, so the TUI
/// printed "Subagent sub-7 started" and never printed an exit. The operator read
/// that as a subagent running forever.
struct SubagentLifecycle<'a> {
    tool: &'a SubAgentTool,
    id: String,
    finished: bool,
}

impl<'a> SubagentLifecycle<'a> {
    fn new(tool: &'a SubAgentTool, id: String) -> Self {
        Self {
            tool,
            id,
            finished: false,
        }
    }

    /// Record the real outcome. Calling this twice is a bug; the second call is
    /// ignored so a double-finish cannot emit two stops.
    fn finish(&mut self, status: &str, summary: String) {
        if self.finished {
            return;
        }
        self.finished = true;
        self.tool.emit_subagent(&AgentEvent::SubagentStopped {
            subagent_id: self.id.clone(),
            status: status.to_string(),
            summary,
        });
    }
}

impl Drop for SubagentLifecycle<'_> {
    fn drop(&mut self) {
        if !self.finished {
            self.tool.emit_subagent(&AgentEvent::SubagentStopped {
                subagent_id: self.id.clone(),
                status: "cancelled".to_string(),
                summary: "the parent's turn ended before this child finished".to_string(),
            });
        }
    }
}

/// Await a child turn, turning a panic into an ordinary failure.
///
/// The swarm path has contained panics since iter-1612 (`run_swarm`, :392);
/// the background path did not, so a panicking child skipped both
/// `mark_completed` and `mark_failed` and left its record `Pending` for the
/// life of the process (measured 2026-11-14: three of them exhaust
/// `DEFAULT_MAX_ASYNC_CHILDREN` for every session in the process, with no
/// error naming the cause). This is the one wrapper both paths can share.
async fn await_child<T>(
    child: impl std::future::Future<Output = std::result::Result<T, BoxedToolError>>,
) -> std::result::Result<T, BoxedToolError> {
    match AssertUnwindSafe(child).catch_unwind().await {
        Ok(inner) => inner,
        Err(_) => Err("the background child panicked before it produced an answer".into()),
    }
}

/// Shape a finished child turn into what the parent receives, for every
/// delegation path (sync, swarm worker, background).
///
/// The parent's tool result IS the child's final message. A child that ended
/// its turn with NO text has not answered the goal: a turn can stop itself
/// with no answer (the wall clock, the iteration budget or the grace gate can
/// each end it), and an empty final message then reached the parent as an
/// EMPTY tool result — indistinguishable from a successful call that returned
/// nothing, which is the failure a parent model cannot act on and cannot
/// report.
fn child_outcome(
    result: std::result::Result<crate::error::Result<Message>, tokio::time::error::Elapsed>,
    timeout_seconds: u64,
) -> std::result::Result<String, BoxedToolError> {
    match result {
        Ok(Ok(message)) => {
            if message.content.trim().is_empty() {
                return Err("The sub-agent finished its turn without producing any output".into());
            }
            Ok(message.content)
        }
        Ok(Err(error)) => Err(format!("Sub-agent error: {}", error).into()),
        Err(_) => Err(format!("Sub-agent timed out after {} seconds", timeout_seconds).into()),
    }
}

/// Set the maximum spawn depth (for testing/config)
pub fn set_max_spawn_depth(depth: u32) {
    let clamped = depth.clamp(MIN_SPAWN_DEPTH, MAX_SPAWN_DEPTH_CAP);
    report_clamp(
        "delegation.max_spawn_depth",
        depth as i64,
        clamped as i64,
        MIN_SPAWN_DEPTH as i64,
        MAX_SPAWN_DEPTH_CAP as i64,
    );
    MAX_SPAWN_DEPTH.store(clamped, Ordering::Relaxed);
}

/// Report a clamped operator setting instead of swallowing it.
///
/// MEASURED 2026-11-14 (audit finding 11): all three delegation setters clamped
/// silently, so `max_global_workers = 64` read as accepted and ran as 32 — the
/// operator tuned a number the process never used and nothing said so. A clamp
/// is the right behaviour (the ceiling bounds spend); a QUIET one is not.
fn report_clamp(key: &str, requested: i64, applied: i64, min: i64, max: i64) {
    if requested != applied {
        tracing::warn!(
            key,
            requested,
            applied,
            min,
            max,
            "setting is outside the supported range and was clamped"
        );
    }
}

/// Set whether orchestrator role is enabled
pub fn set_orchestrator_enabled(enabled: bool) {
    ORCHESTRATOR_ENABLED.store(if enabled { 1 } else { 0 }, Ordering::Relaxed);
}

/// Fallback iteration budget for a child when the caller sets none.
///
/// This used to be a literal 50 inside `run_child`, so the operator's configured
/// `delegation.max_iterations` was read by nobody: three different defaults
/// existed for one number (the README's 20 for the agent, the setup wizard's 90,
/// and this 50), and the one an operator tuned had no effect on delegated work.
static DEFAULT_CHILD_MAX_ITERATIONS: AtomicU32 = AtomicU32::new(50);

/// The child iteration budget when the caller does not set one.
fn default_child_max_iterations() -> u32 {
    DEFAULT_CHILD_MAX_ITERATIONS.load(Ordering::Relaxed)
}

/// Set the child iteration budget used when the caller sets none.
pub fn set_default_child_max_iterations(iterations: u32) {
    DEFAULT_CHILD_MAX_ITERATIONS.store(iterations.max(1), Ordering::Relaxed);
}

/// Set maximum concurrent children
pub fn set_max_concurrent_children(count: usize) {
    let clamped = count.clamp(1, 10);
    report_clamp(
        "delegation.max_concurrent_children",
        count as i64,
        clamped as i64,
        1,
        10,
    );
    MAX_CONCURRENT_CHILDREN.store(clamped as u32, Ordering::Relaxed);
}

/// Set the PROCESS-WIDE cap on live sub-agent workers.
///
/// This is the number that bounds total model streams. `max_concurrent_children`
/// only bounds a single batch, so on its own N parents fanning out M workers
/// each reach N*M live workers; this cap is what stops that.
pub fn set_max_global_workers(count: usize) {
    let clamped = count.clamp(MIN_GLOBAL_WORKERS, MAX_GLOBAL_WORKERS_CAP);
    report_clamp(
        "delegation.max_global_workers",
        count as i64,
        clamped as i64,
        MIN_GLOBAL_WORKERS as i64,
        MAX_GLOBAL_WORKERS_CAP as i64,
    );
    MAX_GLOBAL_WORKERS.store(clamped, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_args_accepts_object_argument() {
        let args = parse_args(serde_json::json!({
            "goal": "analyze this module"
        }))
        .unwrap();
        assert_eq!(args.goal, Some("analyze this module".to_string()));
    }

    /// A delegation that produced NOTHING used to reach the parent as an
    /// empty tool result — a successful-looking call with no output, which
    /// the parent model cannot act on and cannot distinguish from a child
    /// that simply had nothing to say.
    #[test]
    fn a_child_that_produced_no_output_is_a_failure_not_an_empty_result() {
        let error = child_outcome(Ok(Ok(Message::assistant("   \n "))), 60)
            .expect_err("an empty answer is a failure");
        assert!(
            error.to_string().contains("without producing any output"),
            "the error must name the cause: {error}"
        );
        assert_eq!(
            child_outcome(Ok(Ok(Message::assistant("done"))), 60).expect("a real answer passes"),
            "done"
        );
    }

    #[test]
    fn parse_args_accepts_raw_string_as_goal() {
        let args = parse_args(Value::String(r#"{"goal": "analyze this"}"#.to_string())).unwrap();
        assert_eq!(args.goal, Some("analyze this".to_string()));
    }

    #[test]
    fn parse_args_rejects_empty_goal() {
        let error = parse_args(serde_json::json!({ "goal": "  " })).unwrap_err();
        assert!(error.contains("goal") || error.contains("empty"));
    }

    #[test]
    fn parse_args_accepts_batch_tasks() {
        let args = parse_args(serde_json::json!({
            "tasks": [
                { "goal": "task 1" },
                { "goal": "task 2", "context": "some context" }
            ]
        }))
        .unwrap();
        assert_eq!(args.tasks.unwrap().len(), 2);
    }

    #[test]
    fn parse_args_accepts_role() {
        let args = parse_args(serde_json::json!({
            "goal": "test",
            "role": "orchestrator"
        }))
        .unwrap();
        assert_eq!(args.role, Some(SubAgentRole::Orchestrator));
    }

    #[test]
    fn parse_args_accepts_output_schema() {
        let args = parse_args(serde_json::json!({
            "goal": "summarize",
            "output_schema": { "type": "object", "required": ["summary"] }
        }))
        .unwrap();
        let schema = args.output_schema.unwrap();
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["required"][0], "summary");
    }

    #[test]
    fn parse_args_accepts_background_and_query() {
        let bg = parse_args(serde_json::json!({
            "goal": "deep dive",
            "background": true
        }))
        .unwrap();
        assert_eq!(bg.background, Some(true));

        let q = parse_args(serde_json::json!({ "query": "dlg-1-0" })).unwrap();
        assert_eq!(q.query.as_deref(), Some("dlg-1-0"));
    }

    #[test]
    fn parse_args_batch_task_carries_output_schema() {
        let args = parse_args(serde_json::json!({
            "tasks": [
                {
                    "goal": "task 1",
                    "output_schema": { "type": "object", "required": ["answer"] }
                }
            ]
        }))
        .unwrap();
        let task = args.tasks.unwrap().pop().unwrap();
        assert_eq!(task.output_schema.unwrap()["required"][0], "answer");
    }

    #[test]
    fn model_guard_rejects_llama_models() {
        assert!(is_llama_model("meta-llama/Llama-3.1-70B-Instruct"));
        assert!(is_llama_model("llama-3.2"));
    }

    #[test]
    fn model_guard_allows_non_llama_models() {
        assert!(!is_llama_model("gpt-4.1"));
        assert!(!is_llama_model("claude-3-5-sonnet"));
    }

    #[test]
    fn build_leaf_system_prompt_contains_goal() {
        let prompt = build_child_system_prompt(
            "Analyze this",
            Some("context"),
            SubAgentRole::Leaf,
            1,
            2,
            Path::new("/tmp"),
            &[],
        );
        assert!(prompt.contains("Analyze this"));
        assert!(prompt.contains("context"));
        assert!(!prompt.contains("Orchestrator Role"));
    }

    #[test]
    fn build_orchestrator_system_prompt_contains_delegation_info() {
        let prompt = build_child_system_prompt(
            "Analyze this",
            None,
            SubAgentRole::Orchestrator,
            1,
            2,
            Path::new("/tmp"),
            &[],
        );
        assert!(prompt.contains("Orchestrator Role"));
        assert!(prompt.contains("delegate_task"));
    }

    /// RT-291: `delegate_task`'s `toolsets` parameter must be honoured —
    /// and it may only NARROW the child's set.
    ///
    /// The parameter was in the args struct (top level and per task) and
    /// the schema described it ("Optional toolsets to enable for the
    /// sub-agent"), but the body never read it: the child's toolsets came
    /// from the parent's policy alone. The model was told it could scope
    /// the child and could not.
    #[test]
    fn the_requested_toolsets_narrow_the_child_set() {
        let tool = SubAgentTool::with_parent_tool_policy(
            &OpenAIClient::new(crate::client::ClientConfig::default()),
            "gpt-4.1",
            0,
            vec!["builtin".to_string(), "terminal".to_string()],
            std::collections::HashSet::new(),
            std::collections::HashSet::new(),
            Arc::new(Database::init(std::env::temp_dir().join("test_sub_ts_narrow.db")).unwrap()),
            None,
        );
        let all = tool.compute_child_toolsets(SubAgentRole::Leaf, None);
        assert!(all.contains(&"builtin".to_string()), "{all:?}");
        assert!(all.contains(&"terminal".to_string()), "{all:?}");

        // The request narrows: only the requested toolset survives.
        let narrowed =
            tool.compute_child_toolsets(SubAgentRole::Leaf, Some(&["terminal".to_string()]));
        assert_eq!(narrowed, vec!["terminal".to_string()], "{narrowed:?}");

        // An empty request means NO toolsets, not "everything".
        let none = tool.compute_child_toolsets(SubAgentRole::Leaf, Some(&[]));
        assert!(none.is_empty(), "{none:?}");

        // A toolset the parent does not have cannot be added by the request.
        let widened = tool.compute_child_toolsets(
            SubAgentRole::Leaf,
            Some(&["terminal".to_string(), "memory".to_string()]),
        );
        assert!(
            !widened.contains(&"memory".to_string()),
            "the request may never widen the child's set: {widened:?}"
        );
    }

    #[test]
    fn compute_child_toolsets_defaults_to_builtin_when_parent_passes_none() {
        // hermes fallback: a parent that supplies no toolset list (the CLI
        // registers the tool with vec![]) must not produce zero-tool children.
        let tool = SubAgentTool::with_parent_tool_policy(
            &OpenAIClient::new(crate::client::ClientConfig::default()),
            "gpt-4.1",
            0,
            vec![],
            std::collections::HashSet::new(),
            std::collections::HashSet::new(),
            Arc::new(Database::init(std::env::temp_dir().join("test_sub_ts.db")).unwrap()),
            None,
        );
        let ts = tool.compute_child_toolsets(SubAgentRole::Leaf, None);
        assert!(
            ts.contains(&"builtin".to_string()),
            "builtin fallback missing: {ts:?}"
        );
        assert!(!ts.contains(&"delegation".to_string()));
        assert!(!ts.contains(&"memory".to_string()));
        assert!(!ts.contains(&"clarify".to_string()));
        assert!(!ts.contains(&"code_execution".to_string()));
    }

    #[test]
    fn compute_child_toolsets_orchestrator_retains_delegation() {
        let tool = SubAgentTool::with_parent_tool_policy(
            &OpenAIClient::new(crate::client::ClientConfig::default()),
            "gpt-4.1",
            0,
            vec!["builtin".to_string()],
            std::collections::HashSet::new(),
            std::collections::HashSet::new(),
            Arc::new(Database::init(std::env::temp_dir().join("test_sub_orch.db")).unwrap()),
            None,
        );
        // Orchestrators retain the delegation toolset (hermes
        // `_blocked_toolsets_for_role` discards delegate_task from the
        // blocklist when role == "orchestrator").
        let ts = tool.compute_child_toolsets(SubAgentRole::Orchestrator, None);
        assert!(ts.contains(&"delegation".to_string()));
        // ...but the child-blocked toolsets are still stripped.
        assert!(!ts.contains(&"memory".to_string()));
        assert!(!ts.contains(&"clarify".to_string()));
    }

    #[test]
    fn compute_child_toolsets_honors_parent_disabled_toolsets() {
        let disabled_toolsets =
            std::collections::HashSet::from(["builtin".to_string(), "web".to_string()]);
        let tool = SubAgentTool::with_parent_tool_policy(
            &OpenAIClient::new(crate::client::ClientConfig::default()),
            "gpt-4.1",
            0,
            vec!["builtin".to_string(), "web".to_string()],
            std::collections::HashSet::new(),
            disabled_toolsets,
            Arc::new(Database::init(std::env::temp_dir().join("test_sub_disabled.db")).unwrap()),
            None,
        );
        let ts = tool.compute_child_toolsets(SubAgentRole::Leaf, None);
        // "web" is stripped AND the builtin fallback must NOT re-add builtin:
        // the parent supplied an explicit list and disabled builtin itself
        // (hermes: children never gain a toolset the parent disabled).
        assert!(!ts.contains(&"web".to_string()));
        assert!(!ts.contains(&"builtin".to_string()));
    }

    #[test]
    fn compute_child_toolsets_supplied_list_fully_stripped_yields_empty() {
        // Parent supplied an explicit toolset list, but every entry is
        // hermes-blocked for children. hermes semantics: non-empty parent
        // list → intersect/strip only — the child must get ZERO tools, NOT a
        // silent builtin re-addition.
        let tool = SubAgentTool::with_parent_tool_policy(
            &OpenAIClient::new(crate::client::ClientConfig::default()),
            "gpt-4.1",
            0,
            vec!["memory".to_string(), "delegation".to_string()],
            std::collections::HashSet::new(),
            std::collections::HashSet::new(),
            Arc::new(Database::init(std::env::temp_dir().join("test_sub_stripped.db")).unwrap()),
            None,
        );
        let ts = tool.compute_child_toolsets(SubAgentRole::Leaf, None);
        assert!(
            ts.is_empty(),
            "fully-stripped parent list must yield empty, got {ts:?}"
        );

        // Orchestrator role still retains delegation (its own sub-agents),
        // but never the blocked memory toolset.
        let orch_ts = tool.compute_child_toolsets(SubAgentRole::Orchestrator, None);
        assert_eq!(orch_ts, vec!["delegation".to_string()]);
    }

    #[tokio::test]
    async fn child_registry_grants_delegate_task_only_to_orchestrators() {
        use crate::tools::ToolRegistry;

        let client = OpenAIClient::new(crate::client::ClientConfig::default());
        let database =
            Arc::new(Database::init(std::env::temp_dir().join("test_child_reg.db")).unwrap());
        let tool = SubAgentTool::with_parent_tool_policy(
            &client,
            "gpt-4.1",
            0,
            vec![],
            std::collections::HashSet::new(),
            std::collections::HashSet::new(),
            database,
            None,
        );

        // Leaf child: core tools yes, delegate_task NO (hermes: leaf children
        // must never recursively delegate).
        let leaf_ts = tool.compute_child_toolsets(SubAgentRole::Leaf, None);
        let leaf_registry = ToolRegistry::new(Duration::from_secs(5));
        tool.register_child_tools(&leaf_registry, &leaf_ts).await;
        assert!(
            leaf_registry.contains("terminal").await,
            "leaf child lost its core tools"
        );
        assert!(
            !leaf_registry.contains(TOOL_NAME).await,
            "leaf child must never receive delegate_task"
        );

        // Orchestrator child: retains delegate_task for recursive delegation
        // (hermes `_blocked_toolsets_for_role` when role == "orchestrator").
        let orch_ts = tool.compute_child_toolsets(SubAgentRole::Orchestrator, None);
        let orch_registry = ToolRegistry::new(Duration::from_secs(5));
        tool.register_child_tools(&orch_registry, &orch_ts).await;
        assert!(
            orch_registry.contains(TOOL_NAME).await,
            "orchestrator child must retain delegate_task"
        );
    }

    #[tokio::test]
    async fn child_registry_honors_parent_disabled_tools() {
        use crate::tools::ToolRegistry;

        let client = OpenAIClient::new(crate::client::ClientConfig::default());
        let disabled_tools = std::collections::HashSet::from(["terminal".to_string()]);
        let database =
            Arc::new(Database::init(std::env::temp_dir().join("test_child_disabled.db")).unwrap());
        let tool = SubAgentTool::with_parent_tool_policy(
            &client,
            "gpt-4.1",
            0,
            vec![],
            disabled_tools,
            std::collections::HashSet::new(),
            database,
            None,
        );

        let ts = tool.compute_child_toolsets(SubAgentRole::Leaf, None);
        let registry = ToolRegistry::new(Duration::from_secs(5));
        tool.register_child_tools(&registry, &ts).await;
        // A tool the parent explicitly disabled must not leak into children.
        assert!(!registry.contains("terminal").await);
        assert!(registry.contains("file_read").await);
    }

    #[test]
    fn format_swarm_outcomes_handles_empty() {
        let result = format_swarm_outcomes(&[]);
        assert_eq!(result, "No results");
    }

    /// A child that is dropped mid-run must still get its Stopped event.
    ///
    /// MEASURED defect (2026-11-14): run_child emitted Started, awaited, then
    /// emitted Stopped, so dropping the future at the await (the parent's
    /// interrupt flag, the outer tool timeout, a turn teardown) left the operator
    /// with a subagent that started and never ended.
    #[tokio::test]
    async fn a_dropped_child_still_reports_a_stopped_event() {
        use crate::tools::ToolRegistry;

        let client = OpenAIClient::new(crate::client::ClientConfig::default());
        let database =
            Arc::new(Database::init(std::env::temp_dir().join("test_lifecycle.db")).unwrap());
        let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentEvent>(16);
        let tool = SubAgentTool::with_parent_tool_policy(
            &client,
            "gpt-4.1",
            0,
            vec![],
            std::collections::HashSet::new(),
            std::collections::HashSet::new(),
            database,
            Some(tx),
        );
        let _ = ToolRegistry::new(Duration::from_secs(5)); // the tool builds its own

        // Finish path: one stop, with the real status.
        let mut lifecycle = SubagentLifecycle::new(&tool, "sub-done".to_string());
        lifecycle.finish("completed", "all good".to_string());
        drop(lifecycle);
        match rx.try_recv() {
            Ok(AgentEvent::SubagentStopped {
                subagent_id,
                status,
                ..
            }) => {
                assert_eq!(subagent_id, "sub-done");
                assert_eq!(status, "completed");
            }
            other => panic!("expected one stop, got {other:?}"),
        }
        assert!(
            rx.try_recv().is_err(),
            "a finished child must not emit twice"
        );

        // Drop path: the guard closes the pair by itself.
        {
            let _lifecycle = SubagentLifecycle::new(&tool, "sub-dropped".to_string());
        }
        match rx.try_recv() {
            Ok(AgentEvent::SubagentStopped {
                subagent_id,
                status,
                ..
            }) => {
                assert_eq!(subagent_id, "sub-dropped");
                assert_eq!(status, "cancelled");
            }
            other => panic!("expected a cancelled stop, got {other:?}"),
        }
    }

    /// The configured child budget is the fallback, and it is a real control.
    #[test]
    fn the_child_iteration_budget_follows_the_configured_value() {
        let before = default_child_max_iterations();
        set_default_child_max_iterations(7);
        assert_eq!(default_child_max_iterations(), 7);
        // A zero is a ceiling of nothing, not a licence to spin: it floors at 1.
        set_default_child_max_iterations(0);
        assert_eq!(default_child_max_iterations(), 1);
        set_default_child_max_iterations(before);
    }

    /// A child runs where the process runs, and a request for somewhere else is
    /// refused with the reason rather than accepted and ignored.
    #[test]
    fn a_child_cwd_is_the_process_one_or_a_refusal() {
        let cwd = std::env::current_dir().expect("the test has a working directory");

        // Absent: the process directory, which is what every child got before.
        assert_eq!(resolve_child_cwd(None).expect("no request resolves"), cwd);

        // The same directory, spelled absolutely or relatively, is not a change.
        assert_eq!(
            resolve_child_cwd(Some(cwd.to_str().expect("utf-8"))).expect("same dir is fine"),
            cwd
        );
        assert_eq!(
            resolve_child_cwd(Some(".")).expect("dot is the process dir"),
            cwd
        );

        // Somewhere else: refused, and the refusal names both directories.
        let error = resolve_child_cwd(Some("/definitely/not/a/real/place"))
            .expect_err("a different directory must be refused");
        let text = error.to_string();
        assert!(
            text.contains("cannot change the working directory"),
            "{text}"
        );
        assert!(text.contains("/definitely/not/a/real/place"), "{text}");
        assert!(
            text.contains("shares that parent's process working directory"),
            "{text}"
        );
    }

    /// The prompt tells the child where it is, so a relative path in its answer is
    /// not a guess.
    #[test]
    fn the_child_prompt_states_the_working_directory() {
        let prompt = build_child_system_prompt(
            "Analyze this",
            None,
            SubAgentRole::Leaf,
            1,
            2,
            Path::new("/srv/work/thing"),
            &[],
        );
        assert!(prompt.contains("WORKING DIRECTORY"), "{prompt}");
        assert!(prompt.contains("/srv/work/thing"), "{prompt}");
    }

    /// The breadth peek is the decision a dispatch makes BEFORE it creates a
    /// record. Tested as a pure function: driving the process-global counters
    /// here would race the other tests in this module, which hold permits and set
    /// the cap on parallel threads.
    #[test]
    fn the_breadth_peek_is_live_against_max() {
        assert!(
            !breadth_is_exhausted_at(0, 6),
            "an idle process has headroom"
        );
        assert!(!breadth_is_exhausted_at(5, 6), "one free slot is headroom");
        assert!(
            breadth_is_exhausted_at(6, 6),
            "the last slot is not headroom"
        );
        assert!(
            breadth_is_exhausted_at(7, 6),
            "an over-cap count is not headroom"
        );
        assert!(breadth_is_exhausted_at(0, 0), "a zero cap admits nothing");
    }

    /// A zero-tool child's answer must be marked as such; an ordinary child's
    /// answer is passed through byte for byte.
    #[test]
    fn a_zero_tool_child_answer_carries_the_banner() {
        let flagged = flag_no_tools(true, "config.toml sets model = 'gpt-5.4'".to_string());
        assert!(flagged.starts_with(NO_TOOLS_BANNER), "{flagged}");
        assert!(
            flagged.contains("config.toml sets model = 'gpt-5.4'"),
            "{flagged}"
        );

        let ordinary = flag_no_tools(false, "all good".to_string());
        assert_eq!(ordinary, "all good");
    }

    /// MEASURED 2026-11-14 (audit finding 5): the breadth refusal told the
    /// parent to "wait for a worker to finish", but a refused swarm has no
    /// handle to poll — `query=` resolves background delegations only — so the
    /// instruction could not be followed. The recourse the parent actually has
    /// is to retry the call. This pins that the message names it, and that it
    /// no longer sends the model to a wait that does not exist.
    #[test]
    fn a_breadth_refusal_tells_the_parent_what_it_can_actually_do() {
        let message = SpawnRefusal::Breadth { live: 6, max: 6 }.to_string();
        assert!(
            message.contains("retry this call"),
            "the message must name the real recourse: {message}"
        );
        assert!(
            !message.contains("Wait for a worker"),
            "there is no wait to perform: a refused swarm has no handle to poll: {message}"
        );
        // The live/max pair is what makes the refusal actionable at all.
        assert!(message.contains("6/6"), "{message}");
    }

    /// A background child that panics must end as a FAILED record, not as a
    /// Pending one that holds a slot until the process exits.
    #[tokio::test]
    async fn a_panicking_background_child_becomes_an_ordinary_failure() {
        let panicked: std::result::Result<String, BoxedToolError> = await_child(async {
            panic!("child blew up mid-turn");
        })
        .await;
        let error = panicked.expect_err("a panic must not read as success");
        assert!(
            error.to_string().contains("panicked"),
            "the failure must name the panic, got: {error}"
        );

        // The success path is unchanged.
        let ok = await_child(async { Ok("the answer".to_string()) })
            .await
            .expect("a normal child still succeeds");
        assert_eq!(ok, "the answer");

        // An ordinary error is passed through verbatim, not rewritten.
        let failed = await_child(async { Err::<String, _>("child error: no model".into()) })
            .await
            .expect_err("an error stays an error");
        assert_eq!(failed.to_string(), "child error: no model");
    }

    /// A completed worker carrying text and no session id — the common case in
    /// tests that assert on rendering rather than on the session join.
    fn completed(text: impl Into<String>) -> WorkerStatus {
        WorkerStatus::Completed(ChildAnswer {
            text: text.into(),
            session_id: None,
        })
    }

    /// Audit finding 6: siblings share ONE working directory and could not see
    /// each other, so a collision surfaced only when the parent read two
    /// disagreeing reports. The prompt now names them.
    #[test]
    fn a_worker_is_told_which_siblings_share_its_directory() {
        let prompt = build_child_system_prompt(
            "edit the parser",
            None,
            SubAgentRole::Leaf,
            1,
            2,
            Path::new("/repo"),
            &[
                ("sub-1".to_string(), "edit the lexer".to_string()),
                ("sub-2".to_string(), "touch the same file".to_string()),
            ],
        );
        assert!(prompt.contains("sub-1"), "{prompt}");
        assert!(prompt.contains("edit the lexer"), "{prompt}");
        assert!(prompt.contains("share ONE working directory"), "{prompt}");
        assert!(
            prompt.contains("SAY SO in your answer"),
            "the worker must be told the report is where a conflict goes: {prompt}"
        );
    }

    /// A lone child has no siblings, and an empty list must not add a section:
    /// a single delegation's prompt is unchanged by this feature.
    #[test]
    fn a_lone_worker_gets_no_sibling_block() {
        let prompt = build_child_system_prompt(
            "solo",
            None,
            SubAgentRole::Leaf,
            1,
            2,
            Path::new("/repo"),
            &[],
        );
        assert!(!prompt.contains("OTHER WORKERS"), "{prompt}");
    }

    #[test]
    fn format_swarm_outcomes_labels_each_worker_by_its_goal() {
        let outcomes = vec![
            WorkerOutcome {
                id: "sub-a".to_string(),
                goal: "audit the parser".to_string(),
                status: completed("found two issues"),
            },
            WorkerOutcome {
                id: "sub-b".to_string(),
                goal: "review the pool".to_string(),
                status: WorkerStatus::Failed("provider 503".to_string()),
            },
        ];
        let result = format_swarm_outcomes(&outcomes);
        assert!(result.contains("audit the parser"), "{result}");
        assert!(result.contains("found two issues"), "{result}");
        assert!(result.contains("review the pool"), "{result}");
        assert!(result.contains("[failed] provider 503"), "{result}");
    }

    #[test]
    fn format_swarm_outcomes_marks_refusals_as_not_spawned() {
        let outcomes = vec![WorkerOutcome {
            id: "sub-c".to_string(),
            goal: "over budget".to_string(),
            status: WorkerStatus::Refused("Spawn refused: 6/6 slots live".to_string()),
        }];
        let result = format_swarm_outcomes(&outcomes);
        assert!(result.contains("[not spawned]"), "{result}");
    }

    /// A six-worker swarm of long answers must still report SIX workers.
    ///
    /// MEASURED defect (2026-11-14): the parent's read-back cap (4096 BYTES) cut
    /// the whole concatenation, so a swarm of six ~1.5 KB answers kept worker 1
    /// and part of worker 2 and workers 3-6 were paid for and never read.
    #[test]
    fn every_worker_in_a_long_swarm_is_reported_within_the_budget() {
        let outcomes: Vec<WorkerOutcome> = (0..6)
            .map(|i| WorkerOutcome {
                id: format!("sub-{i}"),
                goal: format!("task {i}"),
                status: completed("x".repeat(6_000)),
            })
            .collect();
        let result = format_swarm_outcomes(&outcomes);
        for i in 0..6 {
            assert!(
                result.contains(&format!("task {i}")),
                "worker {i} is missing"
            );
        }
        assert!(
            result.contains("preview:"),
            "the per-worker cap must be marked, not silent"
        );
        // 6 capped reports plus headings must fit the delegation read-back
        // budget (agent::MAX_DELEGATE_RESULT_LEN = 32 KiB).
        assert!(
            result.len() <= 32 * 1024,
            "swarm result is {} bytes, over the 32 KiB delegation budget",
            result.len()
        );
    }

    /// A short answer is passed through byte for byte — the cap is a ceiling,
    /// not a transformation.
    #[test]
    fn a_short_worker_report_is_not_marked_or_rewritten() {
        assert_eq!(cap_worker_report("all good", None), "all good");
        let long = "y".repeat(SWARM_REPORT_PREVIEW_CHARS + 1);
        let capped = cap_worker_report(&long, None);
        assert_eq!(
            capped,
            format!(
                "{}\n[preview: {SWARM_REPORT_PREVIEW_CHARS} of {} chars — this worker reported \
                 no session id, so the rest is not addressable]",
                "y".repeat(SWARM_REPORT_PREVIEW_CHARS),
                long.chars().count()
            )
        );
    }

    /// Audit finding 4: the truncation notice must name the session the tail is
    /// in. A pointer with no key is the defect — "the full text is in the
    /// session store" told the parent nothing it could act on.
    ///
    /// Audit finding 3: naming the session is not enough either. The id is only
    /// useful if the parent knows which tool opens it, so the notice carries the
    /// call. iter-2356 made that call work (a session id alone reads the tail);
    /// this pins that the notice points at it.
    #[test]
    fn a_truncated_report_names_the_session_and_the_call_that_reads_it() {
        let long = "z".repeat(SWARM_REPORT_PREVIEW_CHARS + 1);
        let capped = cap_worker_report(&long, Some("sess_abc123"));
        assert!(
            capped.contains("sess_abc123"),
            "the notice must carry the id: {capped}"
        );
        assert!(
            capped.contains("session_search(session_id=\"sess_abc123\")"),
            "the notice must name the call that reads the rest: {capped}"
        );
        assert!(
            !capped.contains("the full text is in the session store"),
            "the keyless pointer must be gone: {capped}"
        );
    }

    /// Audit finding 3: the pushed form is a PREVIEW, so a 6-worker swarm costs
    /// the parent a fraction of the ~24 KB it used to. MEASURED from the
    /// formatter rather than asserted on the constant: what the parent reads is
    /// the concatenation, so that is what this pins.
    #[test]
    fn a_swarm_pushes_previews_not_full_reports() {
        let outcomes: Vec<WorkerOutcome> = (0..6)
            .map(|i| WorkerOutcome {
                id: format!("sub-{i}"),
                goal: format!("task {i}"),
                status: WorkerStatus::Completed(ChildAnswer {
                    text: "q".repeat(4000),
                    session_id: Some(format!("sess_{i}")),
                }),
            })
            .collect();
        let result = format_swarm_outcomes(&outcomes);
        // Every worker still appears — the failure the old 4 KB read-back cap
        // caused was workers 3-6 vanishing, and a preview must not bring it back.
        for i in 0..6 {
            assert!(result.contains(&format!("task {i}")), "worker {i} is missing");
        }
        // And the pushed text is bounded by the preview width, not the report.
        assert!(
            result.len() < 6 * (SWARM_REPORT_PREVIEW_CHARS + 200),
            "swarm result is {} bytes, which is not a preview: it must stay near \
             6 x {SWARM_REPORT_PREVIEW_CHARS} plus headings",
            result.len()
        );
        assert!(
            result.contains("session_search(session_id="),
            "each cut report must say how to read the rest: {result}"
        );
    }

    /// The worker id and its session are the parent's only handles on a section
    /// (audit finding 4). A heading that drops either cannot be followed up.
    #[test]
    fn a_completed_section_carries_the_worker_id_and_its_session() {
        let outcomes = vec![WorkerOutcome {
            id: "sub-7".to_string(),
            goal: "audit the pool".to_string(),
            status: WorkerStatus::Completed(ChildAnswer {
                text: "done".to_string(),
                session_id: Some("sess_99".to_string()),
            }),
        }];
        let result = format_swarm_outcomes(&outcomes);
        assert!(result.contains("sub-7"), "{result}");
        assert!(result.contains("session sess_99"), "{result}");
    }

    // ── Spawn guard + swarm ──────────────────────────────────────────────
    //
    // The guard ledger (`LIVE_WORKERS`, `MAX_GLOBAL_WORKERS`,
    // `MAX_SPAWN_DEPTH`) is process-global, so every test that asserts on it
    // serialises. Tests 1 and 5 exercise `run_swarm` only and never touch the
    // ledger, so they stay parallel.

    fn guard_test_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn spec(goal: &str, timeout_secs: u64) -> WorkerSpec {
        WorkerSpec {
            siblings: Vec::new(),
            id: next_subagent_id(),
            working_dir: None,
            goal: goal.to_string(),
            context: None,
            role: SubAgentRole::Leaf,
            max_iterations: Some(1),
            timeout_secs,
            output_schema: None,
            toolsets: None,
        }
    }

    #[tokio::test]
    async fn swarm_should_spawn_bounded_workers_and_collect_results() {
        let goals = ["w1", "w2", "w3", "w4"];
        let in_flight_now = std::sync::Arc::new(AtomicUsize::new(0));
        let peak = std::sync::Arc::new(AtomicUsize::new(0));

        let workers: Vec<WorkerSpec> = goals.iter().map(|g| spec(g, 30)).collect();
        let outcomes = run_swarm(workers, 2, {
            let in_flight_now = in_flight_now.clone();
            let peak = peak.clone();
            move |worker| -> WorkerRun {
                let in_flight_now = in_flight_now.clone();
                let peak = peak.clone();
                Box::pin(async move {
                    let now = in_flight_now.fetch_add(1, Ordering::AcqRel) + 1;
                    peak.fetch_max(now, Ordering::AcqRel);
                    // Yield so overlap is observable rather than incidental.
                    tokio::task::yield_now().await;
                    let answer = format!("{} done", worker.goal);
                    in_flight_now.fetch_sub(1, Ordering::AcqRel);
                    Ok(ChildAnswer {
                        text: answer,
                        session_id: None,
                    })
                })
            }
        })
        .await;

        // One outcome per worker, no worker lost or duplicated.
        assert_eq!(outcomes.len(), goals.len());
        for goal in goals {
            let outcome = outcomes
                .iter()
                .find(|outcome| outcome.goal == goal)
                .unwrap_or_else(|| panic!("missing outcome for {goal}"));
            assert_eq!(outcome.status, completed(format!("{goal} done")));
        }
        // The pool really was bounded: never more than `max_in_flight` model
        // streams at once, and the budget was actually exercised.
        assert!(
            peak.load(Ordering::Relaxed) <= 2,
            "in-flight peak {} exceeded the pool bound of 2",
            peak.load(Ordering::Relaxed)
        );
        assert!(peak.load(Ordering::Relaxed) >= 1);
        assert_eq!(in_flight_now.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn spawn_guard_should_refuse_beyond_the_depth_cap() {
        let _guard = guard_test_lock();
        set_max_spawn_depth(1);

        // A child at depth 1 is inside the cap and reserves fine.
        let permit = SpawnPermit::reserve(1, 1);
        assert!(permit.is_ok(), "depth 1 must be inside a cap of 1");

        // Depth 2 is past the floor: refused, and refused BEFORE any slot is
        // claimed, so the ledger is untouched.
        let refused = SpawnPermit::reserve(1, 2);
        assert_eq!(
            refused.err(),
            Some(SpawnRefusal::Depth {
                requested: 2,
                max: 1
            })
        );
        assert_eq!(live_worker_count(), 1, "refusal must not claim a slot");
        drop(permit);
        assert_eq!(live_worker_count(), 0);
    }

    #[test]
    fn spawn_guard_should_refuse_beyond_the_global_breadth_cap() {
        let _guard = guard_test_lock();
        set_max_spawn_depth(3);
        set_max_global_workers(3);

        let a = SpawnPermit::reserve(1, 1);
        let b = SpawnPermit::reserve(1, 1);
        let c = SpawnPermit::reserve(1, 1);
        assert!(a.is_ok() && b.is_ok() && c.is_ok());
        assert_eq!(live_worker_count(), 3);

        // The 4th live worker is past the process-wide cap.
        assert_eq!(
            SpawnPermit::reserve(1, 1).err(),
            Some(SpawnRefusal::Breadth { live: 3, max: 3 })
        );
        assert_eq!(live_worker_count(), 3, "refusal must not claim a slot");

        // Partial admission: 5 requested with 3 live and cap 3 admits none.
        assert!(SpawnPermit::reserve(5, 1).is_err());

        drop(a);
        drop(b);
        drop(c);
        assert_eq!(live_worker_count(), 0);
    }

    #[test]
    fn spawn_guard_should_count_globally_not_per_parent() {
        let _guard = guard_test_lock();
        set_max_spawn_depth(3);
        set_max_global_workers(4);

        // The N*M case. Two "parents" each ask for 3 workers at the same time.
        // A per-parent count would let both through for 6 live workers; the
        // global ledger admits 4 and the second parent is refused.
        let parent_one = SpawnPermit::reserve(3, 1);
        let parent_two = SpawnPermit::reserve(3, 1);

        let one = parent_one.expect("first parent must be admitted");
        assert_eq!(one.slots(), 3);
        let two = parent_two.expect("second parent gets partial admission");
        assert_eq!(
            two.slots(),
            1,
            "only 1 of 3 slots remain, so parent two may run 1 worker"
        );
        assert_eq!(live_worker_count(), 4, "live workers must never exceed 4");

        // A third parent now has nothing left.
        assert!(SpawnPermit::reserve(1, 1).is_err());

        drop(one);
        drop(two);
        assert_eq!(live_worker_count(), 0);
    }

    #[tokio::test]
    async fn swarm_should_continue_after_a_worker_fails() {
        let outcomes = run_swarm(
            vec![
                spec("a", 30),
                spec("boom", 30),
                spec("c", 30),
                spec("d", 30),
            ],
            4,
            |worker| -> WorkerRun {
                Box::pin(async move {
                    if worker.goal == "boom" {
                        return Err(crate::error::Error::Agent("provider exploded".to_string()));
                    }
                    Ok(ChildAnswer {
                        text: format!("{} done", worker.goal),
                        session_id: None,
                    })
                })
            },
        )
        .await;

        // The swarm still reports every worker, and the failure is data.
        assert_eq!(outcomes.len(), 4);
        let failed = outcomes
            .iter()
            .find(|outcome| outcome.goal == "boom")
            .expect("the failing worker must still be reported");
        assert_eq!(
            failed.status,
            WorkerStatus::Failed("Agent error: provider exploded".to_string())
        );
        assert!(matches!(failed.status, WorkerStatus::Failed(_)));
        for goal in ["a", "c", "d"] {
            let survivor = outcomes
                .iter()
                .find(|outcome| outcome.goal == goal)
                .unwrap_or_else(|| panic!("{goal} must survive its sibling's failure"));
            assert_eq!(survivor.status, completed(format!("{goal} done")));
        }
    }

    #[tokio::test]
    async fn swarm_should_contain_a_panicking_worker() {
        let outcomes = run_swarm(
            vec![spec("boom", 30), spec("ok", 30)],
            2,
            |worker| -> WorkerRun {
                Box::pin(async move {
                    if worker.goal == "boom" {
                        panic!("worker blew up");
                    }
                    Ok(ChildAnswer {
                        text: "ok done".to_string(),
                        session_id: None,
                    })
                })
            },
        )
        .await;

        // A panicking worker is contained: it becomes a status, and the
        // healthy sibling still reports. The parent task survives to collect.
        let panicked = outcomes
            .iter()
            .find(|outcome| outcome.goal == "boom")
            .expect("the panicking worker must still be reported");
        assert_eq!(panicked.status, WorkerStatus::Panicked);
        let survivor = outcomes
            .iter()
            .find(|outcome| outcome.goal == "ok")
            .expect("sibling must still complete");
        assert_eq!(survivor.status, completed("ok done"));
    }

    #[tokio::test]
    async fn swarm_should_time_out_a_wedged_worker() {
        // A 1s budget (below the 30s floor a caller gets) proves the timeout
        // actually fires and that the rest of the swarm completes without it.
        let outcomes = run_swarm(
            vec![spec("wedged", 1), spec("ok", 1)],
            2,
            |worker| -> WorkerRun {
                Box::pin(async move {
                    if worker.goal == "wedged" {
                        // Never resolves on its own; only the deadline can end it.
                        std::future::pending::<()>().await;
                    }
                    Ok(ChildAnswer {
                        text: "ok done".to_string(),
                        session_id: None,
                    })
                })
            },
        )
        .await;

        let wedged = outcomes
            .iter()
            .find(|outcome| outcome.goal == "wedged")
            .expect("wedged worker must be reported");
        assert_eq!(wedged.status, WorkerStatus::TimedOut { after_secs: 1 });
        let survivor = outcomes
            .iter()
            .find(|outcome| outcome.goal == "ok")
            .expect("a wedged sibling must not stall the swarm");
        assert_eq!(survivor.status, completed("ok done"));
    }

    #[tokio::test]
    #[expect(
        clippy::await_holding_lock,
        reason = "the std lock is held deliberately: it serialises the process-global worker ledger across this test's awaits, which is exactly what an async-aware lock would not give us here"
    )]
    async fn spawn_guard_count_should_be_released_on_every_exit_path() {
        let _guard = guard_test_lock();
        set_max_spawn_depth(3);
        set_max_global_workers(4);
        let baseline = live_worker_count();
        assert_eq!(baseline, 0, "ledger must start balanced");

        // (a) Normal return — the permit simply goes out of scope.
        {
            let _permit = SpawnPermit::reserve(1, 1);
            assert_eq!(live_worker_count(), baseline + 1);
        }
        assert_eq!(live_worker_count(), baseline, "drop must release");

        // (b) Early error return: a worker built after reserving then fails
        // its pre-flight. The permit is released on the way out.
        fn failing_worker() -> Result<(), SpawnRefusal> {
            let _permit = SpawnPermit::reserve(1, 1)?;
            Err(SpawnRefusal::Depth {
                requested: 9,
                max: 1,
            })
        }
        assert!(failing_worker().is_err());
        assert_eq!(live_worker_count(), baseline, "error return must release");

        // (c) Panic unwind: the reservation was taken, then the worker panicked.
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _permit = SpawnPermit::reserve(2, 1);
            panic!("worker blew up after reserving");
        }));
        assert!(panicked.is_err(), "the panic must actually have unwound");
        assert_eq!(live_worker_count(), baseline, "unwind must release");

        // (d) Task cancellation: the owning task is aborted mid-flight, so no
        // code after the reservation runs. Drop still fires.
        let task = tokio::spawn(async {
            let _permit = SpawnPermit::reserve(1, 1);
            // Park forever so the abort lands with the permit held.
            std::future::pending::<()>().await;
        });
        // Give the task a chance to acquire the permit, then cancel it.
        tokio::task::yield_now().await;
        assert_eq!(live_worker_count(), baseline + 1, "task must hold its slot");
        task.abort();
        let joined = task.await;
        assert!(joined.is_err(), "the task must have been cancelled");
        assert_eq!(
            live_worker_count(),
            baseline,
            "cancellation must release, or capacity leaks for the process lifetime"
        );
    }

    #[test]
    fn parse_args_clamps_a_zero_timeout_to_the_worker_floor() {
        let args = parse_args(serde_json::json!({ "goal": "x", "timeout_seconds": 0 }))
            .expect("a zero timeout is clamped, not rejected");
        assert_eq!(args.timeout_seconds, Some(MIN_WORKER_TIMEOUT_SECONDS));
    }
}

#[cfg(test)]
mod approval_inheritance_tests {
    use super::*;

    fn tool_with_approval() -> SubAgentTool {
        let client = OpenAIClient::new(crate::client::ClientConfig::default());
        let db = Arc::new(
            Database::init(std::env::temp_dir().join("test_approval_inherit.db")).unwrap(),
        );
        SubAgentTool::with_parent_tool_policy(
            &client,
            "gpt-4.1",
            0,
            vec![],
            std::collections::HashSet::new(),
            std::collections::HashSet::new(),
            db,
            None,
        )
        .with_parent_approval(
            std::collections::HashSet::from(["terminal".to_string()]),
            Some(std::path::PathBuf::from("/tmp/approval_allowlist.json")),
        )
    }

    /// Audit finding 2: the allowlist must reach the child. A delegation that
    /// drops it re-derives the approval decision from nothing and has gated
    /// calls refused with advice a headless child cannot follow.
    #[test]
    fn a_swarm_worker_inherits_the_parents_approval_allowlist() {
        let parent = tool_with_approval();
        // The swarm path clones the tool per worker (`call_batch` ->
        // `clone_for_task`); a clone that dropped the set would lose the
        // allowlist for every worker while the single path kept it.
        let worker = parent.clone_for_task();
        assert_eq!(
            worker.parent_approval_allowlist, parent.parent_approval_allowlist,
            "a swarm worker must inherit the parent's allowlist"
        );
        assert_eq!(
            worker.parent_approval_allowlist_path, parent.parent_approval_allowlist_path,
            "and the file it persists to"
        );
    }

    /// A parent that approved nothing must not silently gain a set: the default
    /// is empty, which is the pre-fix behaviour rather than a widening.
    #[test]
    fn a_parent_with_no_allowlist_passes_an_empty_one() {
        let client = OpenAIClient::new(crate::client::ClientConfig::default());
        let db =
            Arc::new(Database::init(std::env::temp_dir().join("test_approval_empty.db")).unwrap());
        let tool = SubAgentTool::with_parent_tool_policy(
            &client,
            "gpt-4.1",
            0,
            vec![],
            std::collections::HashSet::new(),
            std::collections::HashSet::new(),
            db,
            None,
        );
        assert!(tool.parent_approval_allowlist.is_empty());
        assert!(tool.parent_approval_allowlist_path.is_none());
    }

    /// Audit finding 11: a clamp must be visible. The setters are the only place
    /// the ceiling is applied, so a silent one is an operator who tuned a number
    /// the process never used.
    #[test]
    fn an_out_of_range_setting_is_clamped_to_the_documented_ceiling() {
        let before = max_global_workers();
        set_max_global_workers(9_999);
        assert_eq!(
            max_global_workers(),
            MAX_GLOBAL_WORKERS_CAP,
            "the documented ceiling is {MAX_GLOBAL_WORKERS_CAP}"
        );
        set_max_global_workers(before);
    }
}
