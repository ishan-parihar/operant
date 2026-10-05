// Vendored from jcode (crates/jcode-batch-types/src/lib.rs + crates/jcode-base/src/bus.rs),
// MIT License, Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805;
// partial — see jcode_app/mod.rs for scope.
//! Included from jcode-batch-types/src/lib.rs (whole, :1-:37): BatchSubcallState,
//! BatchSubcallProgress, BatchProgress. Included from jcode-base/src/bus.rs:
//! UpdateStatus (:331), BusEvent (:405), Bus (:488), MODELS_UPDATED_DEBOUNCE (:496),
//! latest_update_status (:498), ModelsUpdatedPublishState (:503), impl Bus (:518).
//! [port-excision] BusEvent keeps only the variants the ported tree references
//! (UpdateStatus, MermaidRenderCompleted); the payload-carrying event variants
//! (ToolUpdated/TodoUpdated/... see upstream :405-:486) are dropped until a wave
//! ports their event types. publish_models_updated / new_isolated_for_tests /
//! reset_models_updated_publish_state_for_tests retained verbatim.
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::broadcast;

use crate::tui::jcode_model::vendor_types::ToolCall;
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

// --- crates/jcode-base/src/bus.rs -------------------------------------------------

#[derive(Clone, Debug)]
pub enum UpdateStatus {
    Checking,
    Available {
        current: String,
        latest: String,
    },
    Downloading {
        version: String,
        /// Bytes downloaded so far (0 before the transfer starts).
        downloaded: u64,
        /// Total asset size when known, for progress-bar rendering.
        total: Option<u64>,
    },
    Installing {
        version: String,
    },
    Installed {
        version: String,
    },
    UpToDate,
    /// Automatic checks are not applicable, e.g. a local untracked checkout.
    Skipped {
        reason: String,
    },
    Error(String),
}

pub struct Bus {
    sender: broadcast::Sender<BusEvent>,
    /// Debounce state for [`Bus::publish_models_updated`]. Per-instance (not a
    /// global static) so tests can exercise coalescing on a private bus
    /// without racing other tests that publish to the global bus.
    models_updated_state: std::sync::Arc<Mutex<ModelsUpdatedPublishState>>,
}

const MODELS_UPDATED_DEBOUNCE: Duration = Duration::from_millis(750);

fn latest_update_status() -> &'static Mutex<Option<UpdateStatus>> {
    static STATE: OnceLock<Mutex<Option<UpdateStatus>>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(None))
}

// [port-decision] dedup: bus.rs defined latest_update_status twice across
// concatenated sources; kept the first (identical) copy.

#[derive(Default)]
struct ModelsUpdatedPublishState {
    last_published_at: Option<Instant>,
    publish_pending: bool,
}

impl Bus {
    pub fn global() -> &'static Bus {
        static INSTANCE: OnceLock<Bus> = OnceLock::new();
        INSTANCE.get_or_init(Bus::new)
    }

    /// A standalone bus with its own subscribers and debounce state. Used by
    /// tests; production code shares [`Bus::global`].
    #[cfg(test)] // [port-decision] upstream cfg(test-support) trimmed to
    // plain test: the feature does not exist in operant and cfg value
    // `test-support` trips unexpected_cfgs; the isolated-bus ctor is
    // test-only either way.
    pub fn new_isolated_for_tests() -> std::sync::Arc<Bus> {
        std::sync::Arc::new(Bus::new())
    }

    fn new() -> Bus {
        let (sender, _) = broadcast::channel(256);
        Bus {
            sender,
            models_updated_state: std::sync::Arc::new(Mutex::new(
                ModelsUpdatedPublishState::default(),
            )),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<BusEvent> {
        self.sender.subscribe()
    }

    pub fn publish(&self, event: BusEvent) {
        if let BusEvent::UpdateStatus(status) = &event {
            let mut latest = latest_update_status()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            *latest = Some(status.clone());
        }
        let _ = self.sender.send(event);
    }

    pub fn latest_update_status(&self) -> Option<UpdateStatus> {
        latest_update_status()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub fn publish_models_updated(&self) {
        // A models-updated publish means some provider catalog changed
        // out-of-band. Invalidate memoized route catalogs so the next render
        // rebuilds from the new cache instead of serving a stale memo.
        crate::tui::jcode_app::provider::catalog_scheduler::bump_catalog_generation();

        let delay = {
            let now = Instant::now();
            let mut state = self
                .models_updated_state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match state.last_published_at {
                None => {
                    state.last_published_at = Some(now);
                    None
                }
                Some(last) => {
                    let elapsed = now.saturating_duration_since(last);
                    if elapsed >= MODELS_UPDATED_DEBOUNCE {
                        state.last_published_at = Some(now);
                        None
                    } else if state.publish_pending {
                        return;
                    } else {
                        state.publish_pending = true;
                        Some(MODELS_UPDATED_DEBOUNCE - elapsed)
                    }
                }
            }
        };

        if let Some(delay) = delay {
            let Ok(handle) = tokio::runtime::Handle::try_current() else {
                let mut state = self
                    .models_updated_state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                state.publish_pending = false;
                state.last_published_at = Some(Instant::now());
                drop(state);
                self.publish(BusEvent::ModelsUpdated);
                return;
            };
            // The delayed publish must flow through *this* bus instance (its
            // sender and its debounce state), not Bus::global(): an isolated
            // test bus would otherwise leak its coalesced event onto the
            // global bus and clear the wrong pending flag.
            let state = std::sync::Arc::clone(&self.models_updated_state);
            let sender = self.sender.clone();
            handle.spawn(async move {
                tokio::time::sleep(delay).await;
                let mut state = state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                state.publish_pending = false;
                state.last_published_at = Some(Instant::now());
                drop(state);
                let _ = sender.send(BusEvent::ModelsUpdated);
            });
            return;
        }

        self.publish(BusEvent::ModelsUpdated);
    }
}

#[derive(Clone, Debug)]
pub enum BusEvent {
    UpdateStatus(UpdateStatus),
    /// Provider's available models list may have changed
    ModelsUpdated,
    /// Deferred Mermaid rendering completed and cached content may now be visible
    MermaidRenderCompleted,
}

// [port-excision] upstream BusEvent (jcode-base/src/bus.rs:405-:486) carried ~25
// payload variants (ToolUpdated, TodoUpdated, BatchProgress, ...); only
// UpdateStatus, ModelsUpdated (upstream bus.rs:464 — referenced by this file's
// own publish_models_updated debounce) and MermaidRenderCompleted are
// referenced by the ported tree.
