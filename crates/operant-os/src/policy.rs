//! policy.json → live enforcement (boundary doc §4.3, operant phase C).
//!
//! Holon writes `runs/<id>/policy.json` at claim (its H3). This module is
//! operant's MUST-side: the document compiled at claim becomes the seat policy
//! the agent loop consults on every tool call, through the existing
//! [`SeatPolicySource`] seam — no second enforcement convention.
//!
//! Posture mapping (the adapter's two config files give way to this):
//!
//! | policy.json | SeatPolicy |
//! |---|---|
//! | `posture: "observe"` | mode Lockdown, allow = the read/reason tools an observer keeps, deny = the mutating tools |
//! | `posture: "act"` | mode Yolo, deny = the document's `deny` list |
//!
//! Paths (`roots`, `writes`) stay verified by the kernel's fingerprints — the
//! boundary doc gives the harness the tool-level fence and the kernel the
//! path-level one (a write outside the declared set fails the run at submit,
//! stale-plan cancellation catches the rest). Budgets are carried in the
//! document and consumed by the usage capture (phase D fills `result.usage`;
//! the cap comparison rides the same plumbing).
//!
//! Escalations surface to the model as refusals: an `Escalate` on an unattended
//! Holon run is queued + denied by the loop's F2 semantics (already wired); the
//! refused call is reported in `result.refusals[]` by the supervisor (phase E)
//! — the kernel treats it as a blocker, never a breach.

use std::path::Path;

use operant_core::org::seat_policy::{SeatMode, SeatPolicy};
use serde::Deserialize;

use crate::Error;

/// The tools an observer keeps: read, search, reason, and the Holon read
/// surface. Everything else in the builtin set mutates the machine, the org's
/// records, or reaches a human, and lands in the deny list.
const OBSERVE_ALLOW: &[&str] = &[
    "file_read",
    "file_list",
    "file_search",
    "file_state",
    "working_diff",
    "echo",
    "datetime",
    "timestamp",
    "todo",
    "clarify",
    "env_probe",
    // O-10: no mutating HTTP for observers — http_request supports POST/
    // PUT/DELETE/PATCH, and xai/openrouter are paid/mutating calls. An
    // observer keeps the read-only fetch tools only.
    "web_search",
    "web_fetch",
    "web_scrape",
    "web_extract",
    "web_crawl",
    // G1 live day (the first research body was denied aft_read): the AFT
    // READ suite is exactly what an observer mapping terrain wants —
    // IDE-grade navigation over a codebase, zero mutation. The AFT writers
    // (edit/apply_patch/ast_replace/undo/restore_checkpoint/checkpoint)
    // and aft_bash stay denied; the adapters' disabled_tools already
    // carried the writers, this adds the readers.
    "aft_read",
    "aft_outline",
    "aft_zoom",
    "aft_grep",
    "aft_glob",
    "aft_callers",
    "aft_inspect",
    "aft_ast_search",
    "aft_list_checkpoints",
    "aft_checkpoint_paths",
    "aft_status",
    "memory_search",
    "memory_recall",
    "skills_list",
    "skill_view",
    "session_search",
    "session_insights",
    "tool_search",
    "tool_describe",
    "vision_analyze",
    "video_analyze",
    "transcribe_audio",
    "osv_check",
    "browser_camofox_state",
    "debug_env",
    "debug_system",
    // Holon's read API is read-only by construction (operant_os::api)
    // The native Holon tools derive from the vendored list (O-05): every
    // read/submit tool is allowed; the mutating ones (memory_add,
    // msg_send/msg_request, claim, classify) are not — an observer reads
    // the organization and hands in a result, it does not act on it.
    // OBSERVE_HOLON_DENY is the exception list, so the table stays the
    // single source: a new read tool is allowed by default.
];

/// Native Holon tools an observer is DENIED (mutating; the complement of
/// the allow list above, which `observe_policy()` builds at runtime).
pub const OBSERVE_HOLON_DENY: &[&str] = &[
    "holon_memory_add",
    "holon_msg_send",
    "holon_msg_request",
    "holon_claim",
    "holon_classify",
    "holon_run_heartbeat",
];

/// `policy.json` as Holon compiles it (its execution/policy.py, version 1).
#[derive(Debug, Clone, Deserialize)]
pub struct PolicyDoc {
    pub version: u32,
    pub grade: Option<u32>,
    pub stage: String,
    pub posture: String,
    #[serde(default)]
    pub roots: Vec<String>,
    #[serde(default)]
    pub writes: Vec<String>,
    #[serde(default)]
    pub commands: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
    pub budget: PolicyBudget,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PolicyBudget {
    pub tokens: Option<u64>,
    pub cost: Option<f64>,
}

impl PolicyDoc {
    /// Load the policy a Holon kernel wrote for THIS run. A missing document
    /// is an error, not a permissive default: a Holon run always claims first.
    pub fn load(run_dir: &Path) -> crate::Result<Self> {
        let path = run_dir.join("policy.json");
        let raw = std::fs::read_to_string(&path).map_err(|_| Error::PolicyMissing(path))?;
        // version and posture gate the full parse: a document with an unknown
        // version names itself before its shape can fail a stricter parse
        let value: serde_json::Value = serde_json::from_str(&raw).map_err(Error::PolicyParse)?;
        if value.get("version").and_then(|v| v.as_u64()) != Some(1) {
            return Err(Error::PolicyVersion(
                value.get("version").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
            ));
        }
        match value.get("posture").and_then(|v| v.as_str()) {
            Some("observe") | Some("act") => {}
            _ => {
                return Err(Error::PolicyPosture(
                    value
                        .get("posture")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                ));
            }
        }
        let doc: Self = serde_json::from_value(value).map_err(Error::PolicyParse)?;
        Ok(doc)
    }

    /// Compile the document into the seat policy the loop consults.
    ///
    /// Observe: lockdown with the observer's allow list — a tool not on the
    /// list escalates (unattended: queued + denied, exactly the F2 semantics).
    /// Act: yolo with the document's deny list — the approved plan runs, the
    /// hardline blocklist still applies (it fires before any seat mode).
    pub fn seat_policy(&self) -> SeatPolicy {
        match self.posture.as_str() {
            "act" => SeatPolicy {
                mode: SeatMode::Yolo,
                allow: Vec::new(),
                deny: self.deny.clone(),
            },
            _ => SeatPolicy {
                mode: SeatMode::Lockdown,
                allow: OBSERVE_ALLOW
                    .iter()
                    .copied()
                    .chain(crate::tools::native_tool_names())
                    .filter(|n| !OBSERVE_HOLON_DENY.contains(n))
                    .map(|s| s.to_string())
                    .collect(),
                deny: self.deny.clone(),
            },
        }
    }
}
