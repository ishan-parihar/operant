// Vendored from jcode (crates/operant-memory-types/src/lib.rs + crates/operant-base/src/memory/activity.rs),
// MIT License, Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805;
// partial — the check_staleness closure the info widget reads. See
// operant_app/mod.rs for scope.
//! Included from operant-memory-types/src/lib.rs: MemoryActivity (:10, plus its
//! is_processing impl), StepStatus (:34), StepResult (:44), PipelineState (:51,
//! plus its impls), MemoryState (:108), MemoryEvent (:128). Included from
//! memory/activity.rs: MEMORY_ACTIVITY (:7), STALENESS_TIMEOUT_SECS (:13),
//! check_staleness (:123). [port-excision] activity.rs's remote-snapshot
//! plumbing and the rest of memory-types (graph, store) are not ported.
//! [port-excision] the render state the ported activity enums carry: MemoryState's
//! `Embedding` and `SidecarChecking`, StepStatus's `FoundRelevant` / `Extracting` /
//! `Maintaining` / `ToolAction`, and StepResult's `Error` and `Skipped` arms are
//! never constructed by a live path — the info widget reads `is_processing()` and
//! the staleness check, which need only `Idle` and `Running`. They are
//! `#[allow(dead_code)]` with the explicit arms kept, so the first live
//! construction is a visible decision point (the shape `operant_model/adapter.rs`
//! documents for its `ContentBlock` variants). Measured: the PRODUCTION profile
//! (`cargo clippy -p operant-cli --bin operant`) reports 0 for this file — the
//! warnings are test-profile only, which is what `--all-targets` adds.
use std::sync::Mutex;
use std::time::Instant;

/// Represents current memory system activity.
#[derive(Debug, Clone)]
pub struct MemoryActivity {
    /// Current state of the memory system.
    pub state: MemoryState,
    /// When the current state was entered, used for elapsed time display and staleness detection.
    pub state_since: Instant,
    /// Pipeline progress for the per-turn search, verify, inject, maintain flow.
    pub pipeline: Option<PipelineState>,
    /// Recent events, most recent first.
    pub recent_events: Vec<MemoryEvent>,
}

impl MemoryActivity {
    pub fn is_processing(&self) -> bool {
        !matches!(self.state, MemoryState::Idle)
            || self
                .pipeline
                .as_ref()
                .map(PipelineState::has_running_step)
                .unwrap_or(false)
    }
}

/// Status of a single pipeline step.
///
/// `Error` and `Skipped` carry the pipeline's failure/abstention outcomes. No
/// live path constructs them in this build (see the module head), so they are
/// `#[allow(dead_code)]` rather than deleted — the arms in `RenderResult`'s
/// sibling match stay explicit for the same reason.
#[cfg_attr(
    test,
    allow(dead_code, reason = "jcode-parity port: never constructed today")
)]
#[derive(Debug, Clone, PartialEq)]
pub enum StepStatus {
    Pending,
    Running,
    Done,
    Error,
    Skipped,
}

/// Result data for a completed pipeline step.
///
/// `Error` and `Skipped` are the two outcomes no live step produces (module head).
#[cfg_attr(
    test,
    allow(dead_code, reason = "jcode-parity port: never constructed today")
)]
#[derive(Debug, Clone)]
pub struct StepResult {
    pub summary: String,
    #[cfg_attr(
        test,
        allow(
            dead_code,
            reason = "jcode-parity port: no live step reports its latency"
        )
    )]
    pub latency_ms: u64,
}

/// Tracks the 4-step per-turn memory pipeline: search, verify, inject, maintain.
#[derive(Debug, Clone)]
pub struct PipelineState {
    pub search: StepStatus,
    pub search_result: Option<StepResult>,
    pub verify: StepStatus,
    pub verify_result: Option<StepResult>,
    pub verify_progress: Option<(usize, usize)>,
    pub inject: StepStatus,
    pub inject_result: Option<StepResult>,
    pub maintain: StepStatus,
    pub maintain_result: Option<StepResult>,
    pub started_at: Instant,
}

impl PipelineState {
    pub fn new() -> Self {
        Self {
            search: StepStatus::Pending,
            search_result: None,
            verify: StepStatus::Pending,
            verify_result: None,
            verify_progress: None,
            inject: StepStatus::Pending,
            inject_result: None,
            maintain: StepStatus::Pending,
            maintain_result: None,
            started_at: Instant::now(),
        }
    }

    pub fn is_complete(&self) -> bool {
        matches!(
            (&self.search, &self.verify, &self.inject, &self.maintain),
            (
                StepStatus::Done | StepStatus::Error | StepStatus::Skipped,
                StepStatus::Done | StepStatus::Error | StepStatus::Skipped,
                StepStatus::Done | StepStatus::Error | StepStatus::Skipped,
                StepStatus::Done | StepStatus::Error | StepStatus::Skipped,
            )
        )
    }

    pub fn has_running_step(&self) -> bool {
        matches!(self.search, StepStatus::Running)
            || matches!(self.verify, StepStatus::Running)
            || matches!(self.inject, StepStatus::Running)
            || matches!(self.maintain, StepStatus::Running)
    }
}

/// State of the memory sidecar.
///
/// `Embedding` and `SidecarChecking` are the two in-flight states the live panel
/// never enters — the staleness timer and `is_processing()` cover `Idle` and
/// `Running` (module head).
#[cfg_attr(
    test,
    allow(dead_code, reason = "jcode-parity port: never constructed today")
)]
#[derive(Debug, Clone, PartialEq, Default)]
pub enum MemoryState {
    /// Idle, no activity.
    #[default]
    Idle,
    /// Running embedding search.
    Embedding,
    /// Sidecar checking relevance.
    SidecarChecking { count: usize },
    /// Found relevant memories.
    FoundRelevant { count: usize },
    /// Extracting memories from conversation.
    Extracting { reason: String },
    /// Background maintenance or gardening of the memory graph.
    Maintaining { phase: String },
    /// Agent is actively using a memory tool.
    ToolAction { action: String, detail: String },
}

// [port-decision] leaf port: MemoryEventKind + InjectedMemoryItem verbatim from
// operant-memory-types/src/lib.rs:143-201,137-141 — referenced by MemoryEvent.kind
// (upstream reached them via the memory-types crate's glob).
#[cfg_attr(
    test,
    allow(
        dead_code,
        reason = "jcode-parity port: leaf type, read only by its own tests"
    )
)]
#[derive(Debug, Clone)]
pub struct InjectedMemoryItem {
    pub section: String,
    #[cfg_attr(
        test,
        allow(
            dead_code,
            reason = "jcode-parity port: the panel reads `section`, not the payload"
        )
    )]
    pub content: String,
}

/// The ported memory-event vocabulary.
///
/// `FoundRelevant`, `Extracting`, `Maintaining` and `ToolAction` are the four
/// variants no live path emits today (module head). Kept because the enum is a
/// wire vocabulary the port maps 1:1, not because something reads them.
#[cfg_attr(
    test,
    allow(
        dead_code,
        reason = "jcode-parity port: four variants never emitted today"
    )
)]
#[derive(Debug, Clone)]
pub enum MemoryEventKind {
    /// Embedding search started.
    EmbeddingStarted,
    /// Embedding search completed.
    EmbeddingComplete { latency_ms: u64, hits: usize },
    /// Sidecar started checking.
    SidecarStarted,
    /// Sidecar found memory relevant.
    SidecarRelevant { memory_preview: String },
    /// Sidecar found memory not relevant.
    SidecarNotRelevant,
    /// Sidecar call completed with latency.
    SidecarComplete { latency_ms: u64 },
    /// Memory was surfaced to main agent.
    MemorySurfaced { memory_preview: String },
    /// Memory payload was injected into model context.
    MemoryInjected {
        count: usize,
        prompt_chars: usize,
        age_ms: u64,
        preview: String,
        items: Vec<InjectedMemoryItem>,
    },
    /// Background maintenance started.
    MaintenanceStarted { verified: usize, rejected: usize },
    /// Background maintenance discovered or strengthened links.
    MaintenanceLinked { links: usize },
    /// Background maintenance adjusted confidence.
    MaintenanceConfidence { boosted: usize, decayed: usize },
    /// Background maintenance refined clusters.
    MaintenanceCluster { clusters: usize, members: usize },
    /// Background maintenance inferred or applied a shared tag.
    MaintenanceTagInferred { tag: String, applied: usize },
    /// Background maintenance detected a gap.
    MaintenanceGap { candidates: usize },
    /// Background maintenance completed.
    MaintenanceComplete { latency_ms: u64 },
    /// Extraction started.
    ExtractionStarted { reason: String },
    /// Extraction completed.
    ExtractionComplete { count: usize },
    /// Error occurred.
    Error { message: String },
    /// Agent stored a memory via tool.
    ToolRemembered {
        content: String,
        scope: String,
        category: String,
    },
    /// Agent recalled or searched memories via tool.
    ToolRecalled { query: String, count: usize },
    /// Agent forgot a memory via tool.
    ToolForgot { id: String },
    /// Agent tagged a memory via tool.
    ToolTagged { id: String, tags: String },
    /// Agent linked memories via tool.
    ToolLinked { from: String, to: String },
    /// Agent listed memories via tool.
    ToolListed { count: usize },
}

/// A memory system event.
#[derive(Debug, Clone)]
pub struct MemoryEvent {
    /// Type of event.
    pub kind: MemoryEventKind,
    /// When it happened.
    #[cfg_attr(
        test,
        allow(
            dead_code,
            reason = "jcode-parity port: no live consumer renders the event's wall clock"
        )
    )]
    pub timestamp: Instant,
    /// Optional details.
    #[cfg_attr(
        test,
        allow(
            dead_code,
            reason = "jcode-parity port: no live consumer renders the event's detail"
        )
    )]
    pub detail: Option<String>,
}

/// Global memory activity state - updated by sidecar, read by info widget
static MEMORY_ACTIVITY: Mutex<Option<MemoryActivity>> = Mutex::new(None);

/// Staleness timeout: auto-reset to Idle if state has been non-Idle for this long
const STALENESS_TIMEOUT_SECS: u64 = 10;

// [port-decision] dedup: memory/activity.rs defined STALENESS_TIMEOUT_SECS and
// get_activity twice across concatenated sources; kept the first (identical) copies.

pub fn check_staleness() -> bool {
    #[allow(clippy::collapsible_if)] // vendored-verbatim upstream nesting
    if let Ok(mut guard) = MEMORY_ACTIVITY.lock() {
        if let Some(activity) = guard.as_mut() {
            if !matches!(activity.state, MemoryState::Idle)
                && activity.state_since.elapsed().as_secs() >= STALENESS_TIMEOUT_SECS
            {
                // [port-decision] re-rooted `crate::logging` -> operant_app::logging (leftover from the app-crate root).
                crate::tui::operant_app::logging::info(&format!(
                    "Memory state stale ({:?} for {}s), auto-resetting to Idle",
                    activity.state,
                    activity.state_since.elapsed().as_secs()
                ));
                activity.state = MemoryState::Idle;
                activity.state_since = Instant::now();
                return true;
            }
        }
    }
    false
}
