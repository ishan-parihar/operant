// Vendored from jcode (crates/jcode-protocol/src/lib.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805; partial —
// only SwarmMemberStatus (:469-:515), the one `crate::protocol::*` symbol the
// renderers reference. See jcode_app/mod.rs for scope.
//! [port-excision] the rest of jcode-protocol (wire.rs, plan snapshots,
//! MemoryActivitySnapshot) is not ported; nothing the ported renderers read
//! reaches it.
use serde::{Deserialize, Serialize};

/// Swarm member status for lifecycle updates
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SwarmMemberStatus {
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub friendly_name: Option<String>,
    /// Lifecycle status (ready, running, completed, failed, stopped, etc.)
    pub status: String,
    /// Optional detail (task, error, etc.)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Stable label of the task/role this member was spawned or assigned for.
    /// Unlike `detail`, it is not overwritten by transient status updates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_label: Option<String>,
    /// Role: "agent" or "coordinator"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Whether this member is a headless spawned session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_headless: Option<bool>,
    /// Number of currently attached live client connections.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_attachments: Option<usize>,
    /// Seconds since the last status change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_age_secs: Option<u64>,
    /// Recent streamed output tail for live inline rendering (last few lines of
    /// the agent's in-progress assistant text). Only populated for swarm
    /// members when inline streaming taps are active.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tail: Option<String>,
    /// Session id this member reports back to (its spawner/parent in the swarm
    /// tree). Walking this chain reconstructs the spawn tree, which lets a
    /// client scope the inline gallery to the subtree it actually spawned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report_back_to_session_id: Option<String>,
    /// Todo/plan progress as (completed, total) for this member, when known.
    /// Surfaced on the inline swarm strip as a compact "C/T" counter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub todo_progress: Option<(u32, u32)>,
    /// Compact snapshot of this member's todo list (content + status), capped
    /// by the producer. Rendered in the focused inline swarm panel so the
    /// coordinator can see what each agent is working through.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub todo_items: Vec<SwarmTodoItem>,
    /// Ephemeral runtime metadata used by the live swarm card.
    #[serde(default, skip_serializing_if = "SwarmMemberRuntime::is_empty")]
    pub runtime: SwarmMemberRuntime,
}

// [port-decision] leaf ports verbatim from jcode-protocol/src/lib.rs:
// SwarmMemberRuntime (:519, + its is_empty impl :533), SwarmTodoItem (:546) and
// SwarmToolIntent (:557) — the swarm-status boundary structs SwarmMemberStatus
// fields reference (the sibling first-wave port dropped their definitions).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SwarmMemberRuntime {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Human-facing credential route, such as "OAuth" or "API key".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elapsed_secs: Option<u64>,
}

impl SwarmMemberRuntime {
    fn is_empty(&self) -> bool {
        self.model.is_none()
            && self.provider.is_none()
            && self.auth_method.is_none()
            && self.effort.is_none()
            && self.elapsed_secs.is_none()
    }
}

/// One compact todo entry crossing the swarm status boundary. Only the
/// display essentials travel; full todo metadata stays in the owning session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SwarmTodoItem {
    pub content: String,
    /// "pending", "in_progress", or "completed".
    pub status: String,
    /// The three most recent tool calls made while this todo was active.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_intents: Vec<SwarmToolIntent>,
}

/// Display-only tool activity nested beneath an active swarm todo.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SwarmToolIntent {
    /// Internal correlation key used by the server to update a running call.
    /// It is intentionally omitted from the wire payload.
    #[serde(default, skip_serializing)]
    pub tool_call_id: String,
    pub tool_name: String,
    pub intent: String,
    /// "running", "completed", or "error".
    pub status: String,
}
