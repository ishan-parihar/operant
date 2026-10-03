// Vendored from jcode (crates/jcode-overnight-core/src/lib.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805; partial —
// only the three wire structs the transcript renderer deserializes:
// OvernightTaskStatusCounts (:236), OvernightTaskCardSummary (:247),
// OvernightProgressCard (:259). [port-excision] the command parser, run
// lifecycle, and file formats are not ported.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct OvernightTaskStatusCounts {
    pub completed: usize,
    pub active: usize,
    pub blocked: usize,
    pub deferred: usize,
    pub failed: usize,
    pub skipped: usize,
    pub unknown: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct OvernightTaskCardSummary {
    pub total: usize,
    pub counts: OvernightTaskStatusCounts,
    pub validated: usize,
    pub high_risk: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OvernightProgressCard {
    pub run_id: String,
    pub status: String,
    pub phase: String,
    pub coordinator_session_id: String,
    pub coordinator_session_name: String,
    pub elapsed_label: String,
    pub target_duration_label: String,
    pub progress_percent: f32,
    pub target_wake_at: String,
    pub time_relation: String,
    pub last_activity_label: String,
    pub next_prompt_label: String,
    pub usage_risk: String,
    pub usage_confidence: String,
    pub usage_projection: String,
    pub resources_summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_event_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_event_summary: Option<String>,
    pub task_summary: OvernightTaskCardSummary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_task_title: Option<String>,
    pub review_path: String,
    pub log_path: String,
    pub run_dir: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
}
