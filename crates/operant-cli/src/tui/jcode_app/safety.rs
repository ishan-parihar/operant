// Vendored from jcode (crates/jcode-base/src/safety.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805 (batch-4
// sweep tail): only the file-backed permission-decision surface the TUI
// permissions overlay consumes — Urgency/PermissionRequest/PermissionResult/
// Decision plus record_permission_via_file and its queue/history helpers.
// The in-process SafetySystem (Mutex/OnceLock state machine) is NOT ported;
// nothing in the TUI layer reaches it.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

use super::storage;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Urgency {
    Low,
    Normal,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRequest {
    pub id: String,
    pub action: String,
    pub description: String,
    pub rationale: String,
    pub urgency: Urgency,
    pub wait: bool,
    pub created_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PermissionResult {
    Approved { message: Option<String> },
    Denied { reason: Option<String> },
    Queued { request_id: String },
    Timeout,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Decision {
    pub request_id: String,
    pub approved: bool,
    pub decided_at: DateTime<Utc>,
    pub decided_via: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

fn queue_path() -> Result<std::path::PathBuf> {
    Ok(storage::jcode_dir()?.join("safety").join("queue.json"))
}

fn history_path() -> Result<std::path::PathBuf> {
    Ok(storage::jcode_dir()?.join("safety").join("history.json"))
}

fn persist_queue(queue: &[PermissionRequest]) -> Result<()> {
    let path = queue_path()?;
    storage::write_json_fast(&path, queue)
}

fn persist_history(history: &[Decision]) -> Result<()> {
    let path = history_path()?;
    storage::write_json_fast(&path, history)
}

/// Record a permission decision by directly manipulating the queue/history JSON files.
/// Used by callers which don't have access to the live SafetySystem instance.
pub fn record_permission_via_file(
    request_id: &str,
    approved: bool,
    via: &str,
    message: Option<String>,
) -> Result<()> {
    let qp = queue_path()?;
    if let Some(parent) = qp.parent() {
        storage::ensure_dir(parent)?;
    }
    let mut queue: Vec<PermissionRequest> = if qp.exists() {
        storage::read_json(&qp).unwrap_or_default()
    } else {
        Vec::new()
    };
    queue.retain(|r| r.id != request_id);
    persist_queue(&queue)?;

    let hp = history_path()?;
    if let Some(parent) = hp.parent() {
        storage::ensure_dir(parent)?;
    }
    let mut history: Vec<Decision> = if hp.exists() {
        storage::read_json(&hp).unwrap_or_default()
    } else {
        Vec::new()
    };
    history.push(Decision {
        request_id: request_id.to_string(),
        approved,
        decided_at: Utc::now(),
        decided_via: via.to_string(),
        message,
    });
    persist_history(&history)?;

    Ok(())
}

// ---------------------------------------------------------------------------
// SafetySystem (jcode-base/src/safety.rs:150-290) — bounded set: the
// constructor + the two methods the permissions TUI entry consumes.
// request_permission / record_decision / log_action / generate_summary stay
// unported (server-side surfaces, unreached from the TUI layer).
// ---------------------------------------------------------------------------

pub struct SafetySystem {
    queue: Mutex<Vec<PermissionRequest>>,
    history: Mutex<Vec<Decision>>,
}

impl SafetySystem {
    /// Create a new SafetySystem, loading persisted queue/history from disk.
    pub fn new() -> Self {
        let queue: Vec<PermissionRequest> = queue_path()
            .ok()
            .and_then(|p| storage::read_json(&p).ok())
            .unwrap_or_default();

        let history: Vec<Decision> = history_path()
            .ok()
            .and_then(|p| storage::read_json(&p).ok())
            .unwrap_or_default();

        SafetySystem {
            queue: Mutex::new(queue),
            history: Mutex::new(history),
        }
    }

    /// Expire pending permission requests that can no longer be serviced
    /// because their originating session is no longer active.
    pub fn expire_dead_session_requests(&self, via: &str) -> Result<Vec<String>> {
        let mut expired: Vec<(String, String)> = Vec::new();

        if let Ok(mut q) = self.queue.lock() {
            let mut retained: Vec<PermissionRequest> = Vec::with_capacity(q.len());
            for req in q.drain(..) {
                if let Some(reason) = stale_request_reason(&req) {
                    expired.push((req.id.clone(), reason));
                } else {
                    retained.push(req);
                }
            }
            *q = retained;
            let _ = persist_queue(&q);
        }

        if expired.is_empty() {
            return Ok(Vec::new());
        }

        if let Ok(mut h) = self.history.lock() {
            for (request_id, reason) in &expired {
                h.push(Decision {
                    request_id: request_id.clone(),
                    approved: false,
                    decided_at: Utc::now(),
                    decided_via: via.to_string(),
                    message: Some(format!(
                        "Expired automatically: {}. Original agent is no longer active.",
                        reason
                    )),
                });
            }
            let _ = persist_history(&h);
        }

        Ok(expired.into_iter().map(|(id, _)| id).collect())
    }

    pub fn pending_requests(&self) -> Vec<PermissionRequest> {
        self.queue.lock().map(|q| q.clone()).unwrap_or_default()
    }
}

fn stale_request_reason(request: &PermissionRequest) -> Option<String> {
    let session_id = request_session_id(request)?;
    let mut session = match crate::tui::jcode_app::session::Session::load(&session_id) {
        Ok(s) => s,
        Err(_) => return Some(format!("owner session '{}' was not found", session_id)),
    };

    // Refresh crash status based on PID if needed.
    if session.detect_crash() {
        let _ = session.save();
    }

    if session.status == crate::tui::jcode_app::session::SessionStatus::Active {
        None
    } else {
        Some(format!(
            "owner session '{}' is {}",
            session_id,
            session.status.display()
        ))
    }
}

fn request_session_id(request: &PermissionRequest) -> Option<String> {
    let context = request.context.as_ref()?;

    context
        .get("session_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            context
                .get("requester")
                .and_then(|r| r.get("session_id"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
}
