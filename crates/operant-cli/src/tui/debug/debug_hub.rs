//! TuiDebugHub — centralized debug state.
//!
//! Holds the event bus plus aggregate debug counters. Published to from the
//! run loop and event handlers; read from the F12 debug overlay.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use parking_lot::Mutex;

use super::event_bus::{now_secs, TuiEvent, TuiEventBus};

/// Centralized debug state for the TUI. Cheap to clone (inner is Arc).
/// All fields are thread-safe (AtomicBool/AtomicU64/Mutex).
#[derive(Clone)]
pub struct TuiDebugHub {
    inner: Arc<Inner>,
}

struct Inner {
    event_bus: TuiEventBus,
    started_at: Instant,
    frame_count: AtomicU64,
    last_render_ms: AtomicU64,
    last_error: Mutex<Option<String>>,
    overlay_visible: AtomicBool,
    /// Path to dump the event log on exit (if set via env var).
    event_log_path: Mutex<Option<std::path::PathBuf>>,
}

impl TuiDebugHub {
    /// Create a new hub. `enabled` controls whether the event bus records.
    pub fn new(enabled: bool) -> Self {
        let event_log_path = std::env::var("OPERANT_TUI_EVENT_LOG")
            .ok()
            .map(std::path::PathBuf::from);

        Self {
            inner: Arc::new(Inner {
                event_bus: TuiEventBus::new(enabled),
                started_at: Instant::now(),
                frame_count: AtomicU64::new(0),
                last_render_ms: AtomicU64::new(0),
                last_error: Mutex::new(None),
                overlay_visible: AtomicBool::new(false),
                event_log_path: Mutex::new(event_log_path),
            }),
        }
    }

    /// Create from env var: enabled if `OPERANT_TUI_DEBUG=1`.
    pub fn new_from_env() -> Self {
        let enabled = std::env::var("OPERANT_TUI_DEBUG")
            .map(|v| v == "1" || v == "true")
            .unwrap_or(false);
        Self::new(enabled)
    }

    // ── Event bus access ─────────────────────────────────────────────

    pub fn event_bus(&self) -> &TuiEventBus {
        &self.inner.event_bus
    }

    pub fn publish(&self, event: TuiEvent) {
        self.inner.event_bus.publish(event);
    }

    // ── Frame tracking ───────────────────────────────────────────────

    /// Called from the run loop after each `terminal.draw`. Records frame
    /// count, render time, and publishes a FrameRendered event.
    pub fn record_frame(&self, render_ms: f64) {
        let frame = self.inner.frame_count.fetch_add(1, Ordering::Relaxed) + 1;
        self.inner
            .last_render_ms
            .store(render_ms as u64, Ordering::Relaxed);
        self.inner.event_bus.publish(TuiEvent::FrameRendered {
            frame,
            render_ms,
            at: now_secs(),
        });
    }

    pub fn frame_count(&self) -> u64 {
        self.inner.frame_count.load(Ordering::Relaxed)
    }

    pub fn last_render_ms(&self) -> u64 {
        self.inner.last_render_ms.load(Ordering::Relaxed)
    }

    pub fn uptime_secs(&self) -> f64 {
        self.inner.started_at.elapsed().as_secs_f64()
    }

    // ── Error tracking ───────────────────────────────────────────────

    pub fn record_error(&self, source: &str, message: &str) {
        let formatted = format!("[{source}] {message}");
        *self.inner.last_error.lock() = Some(formatted.clone());
        self.inner.event_bus.publish(TuiEvent::Error {
            source: source.to_string(),
            message: message.to_string(),
            at: now_secs(),
        });
    }

    pub fn last_error(&self) -> Option<String> {
        self.inner.last_error.lock().clone()
    }

    // ── Overlay toggle ───────────────────────────────────────────────

    pub fn toggle_overlay(&self) {
        let was = self.inner.overlay_visible.load(Ordering::Relaxed);
        self.inner.overlay_visible.store(!was, Ordering::Relaxed);
    }

    pub fn overlay_visible(&self) -> bool {
        self.inner.overlay_visible.load(Ordering::Relaxed)
    }

    // ── Exit dump ────────────────────────────────────────────────────

    /// Dump the event log to the path set by OPERANT_TUI_EVENT_LOG, if any.
    /// Call this on clean TUI exit.
    pub fn dump_on_exit(&self) {
        let path = self.inner.event_log_path.lock().clone();
        if let Some(path) = path {
            if let Err(e) = self.inner.event_bus.dump_to_file(&path) {
                eprintln!("[tui-debug] failed to dump event log to {path:?}: {e}");
            } else {
                eprintln!("[tui-debug] event log dumped to {path:?}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hub_records_frames() {
        let hub = TuiDebugHub::new(true);
        assert_eq!(hub.frame_count(), 0);
        hub.record_frame(5.0);
        hub.record_frame(3.0);
        assert_eq!(hub.frame_count(), 2);
        assert_eq!(hub.last_render_ms(), 3);
    }

    #[test]
    fn hub_records_errors() {
        let hub = TuiDebugHub::new(true);
        assert!(hub.last_error().is_none());
        hub.record_error("test", "something broke");
        assert_eq!(hub.last_error().unwrap(), "[test] something broke");
    }

    #[test]
    fn overlay_toggle() {
        let hub = TuiDebugHub::new(false);
        assert!(!hub.overlay_visible());
        hub.toggle_overlay();
        assert!(hub.overlay_visible());
        hub.toggle_overlay();
        assert!(!hub.overlay_visible());
    }

    #[test]
    fn new_from_env_respects_flag() {
        // Default: not set → disabled.
        let hub = TuiDebugHub::new_from_env();
        hub.record_frame(1.0);
        // Bus is disabled, so no events recorded.
        assert_eq!(hub.event_bus().len(), 0);
    }
}
