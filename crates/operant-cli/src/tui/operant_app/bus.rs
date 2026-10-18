// Vendored from jcode (crates/operant-batch-types/src/lib.rs + crates/operant-base/src/bus.rs),
// MIT License, Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805;
// partial — see operant_app/mod.rs for scope.
//! Included from operant-batch-types/src/lib.rs (whole, :1-:37): BatchSubcallState,
//! BatchSubcallProgress, BatchProgress. Included from operant-base/src/bus.rs:
//! UpdateStatus (:331), BusEvent (:405), Bus (:488), MODELS_UPDATED_DEBOUNCE (:496),
//! latest_update_status (:498), ModelsUpdatedPublishState (:503), impl Bus (:518).
//! [port-excision] BusEvent keeps only the variants the ported tree references
//! (UpdateStatus, MermaidRenderCompleted); the payload-carrying event variants
//! (ToolUpdated/TodoUpdated/... see upstream :405-:486) are dropped until a wave
//! ports their event types. publish_models_updated / new_isolated_for_tests /
//! reset_models_updated_publish_state_for_tests retained verbatim.

use crate::tui::operant_model::vendor_types::ToolCall;
use serde::{Deserialize, Serialize};

/// Progress update from a running batch tool call
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchSubcallState {
    Running,
    Succeeded,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BatchSubcallProgress {
    pub index: usize,
    pub tool_call: ToolCall,
    pub state: BatchSubcallState,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BatchProgress {
    pub session_id: String,
    /// Parent tool_call_id of the batch call
    pub tool_call_id: String,
    /// Total number of sub-calls in this batch
    pub total: usize,
    /// Number of sub-calls that have completed (success or error)
    pub completed: usize,
    /// Name of the sub-call that just completed
    pub last_completed: Option<String>,
    /// Sub-calls that are currently still running
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub running: Vec<ToolCall>,
    /// Ordered per-subcall progress state for richer UI rendering
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subcalls: Vec<BatchSubcallProgress>,
}

// --- crates/operant-base/src/bus.rs -------------------------------------------------

// [port-decision] dedup: bus.rs defined latest_update_status twice across
// concatenated sources; kept the first (identical) copy.

// [port-excision] upstream BusEvent (operant-base/src/bus.rs:405-:486) carried ~25
// payload variants (ToolUpdated, TodoUpdated, BatchProgress, ...); only
// UpdateStatus, ModelsUpdated (upstream bus.rs:464 — referenced by this file's
// own publish_models_updated debounce) and MermaidRenderCompleted are
// referenced by the ported tree.
