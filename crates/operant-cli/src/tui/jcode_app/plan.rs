// Vendored from jcode (crates/jcode-plan/src/lib.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805; partial —
// only PlanItem is referenced by the ported tree (info_widget_todos'
// swarm_plan_todos + its tests). [port-excision] the rest of jcode-plan
// (MAX_PLAN_ITEMS, SwarmTaskProgress, VersionedPlan, bridge/dag/mermaid
// submodules) is not referenced and not ported. Re-rooted:
// `jcode_plan::` -> `crate::tui::jcode_app::plan::`. See jcode_app/mod.rs.
use serde::{Deserialize, Serialize};

/// A swarm plan item.
///
/// This is intentionally separate from session todos: plan data is shared at the
/// server/swarm level, while todos remain session-local.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlanItem {
    pub content: String,
    pub status: String,
    pub priority: String,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subsystem: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub file_scope: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocked_by: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assigned_to: Option<String>,
}
