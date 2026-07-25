//! TuiEventBus — typed event ring buffer.
//!
//! Every event the TUI processes is published to this bus. The bus keeps a
//! ring buffer of the last 1,000 events. When the debug overlay is open,
//! it reads from this buffer to show the event trail. When
//! `OPERANT_TUI_EVENT_LOG` is set, the bus dumps to that file on exit.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

const EVENT_RING_CAPACITY: usize = 1_000;

/// Every event the TUI processes, in one tagged enum.
/// One ring buffer to rule them all — debug overlay, replay, and snapshot
/// all read from this.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TuiEvent {
    // ── Crossterm input ──────────────────────────────────────────────
    Key {
        code: String,
        modifiers: u8,
        at: f64,
    },
    #[allow(dead_code)] // Prepared for mouse event tracking
    Mouse {
        x: u16,
        y: u16,
        action: String,
        at: f64,
    },
    #[allow(dead_code)] // Prepared for resize event tracking
    Resize {
        width: u16,
        height: u16,
        at: f64,
    },
    #[allow(dead_code)] // Prepared for paste event tracking
    Paste {
        len: usize,
        at: f64,
    },

    // ── Async channels from agent ────────────────────────────────────
    AgentEvent {
        variant: String,
        summary: String,
        at: f64,
    },
    PermissionRequest {
        tool_name: String,
        at: f64,
    },
    UserQuestion {
        question_preview: String,
        at: f64,
    },
    ModelFetch {
        ok: bool,
        count: usize,
        at: f64,
    },
    SessionList {
        count: usize,
        at: f64,
    },
    SessionLoad {
        session_id: String,
        msg_count: usize,
        at: f64,
    },
    VoiceEvent {
        variant: String,
        at: f64,
    },

    // ── TUI-internal ─────────────────────────────────────────────────
    SlashCommand {
        name: String,
        args_preview: String,
        at: f64,
    },
    #[allow(dead_code)] // Prepared for overlay lifecycle tracking
    OverlayOpened {
        name: String,
        at: f64,
    },
    #[allow(dead_code)] // Prepared for overlay lifecycle tracking
    OverlayClosed {
        name: String,
        at: f64,
    },
    FrameRendered {
        frame: u64,
        render_ms: f64,
        at: f64,
    },
    Error {
        source: String,
        message: String,
        at: f64,
    },
}

impl TuiEvent {
    /// Human-readable one-line summary for the debug overlay.
    pub fn summary(&self) -> String {
        match self {
            Self::Key {
                code, modifiers, ..
            } => {
                let mods = if *modifiers != 0 {
                    format!("mod={modifiers} ")
                } else {
                    String::new()
                };
                format!("Key({mods}{code})")
            }
            Self::Mouse { x, y, action, .. } => format!("Mouse({action} @{x},{y})"),
            Self::Resize { width, height, .. } => format!("Resize({width}x{height})"),
            Self::Paste { len, .. } => format!("Paste({len} chars)"),
            Self::AgentEvent {
                variant, summary, ..
            } => {
                format!("AgentEvent({variant}: {summary})")
            }
            Self::PermissionRequest { tool_name, .. } => {
                format!("PermissionReq({tool_name})")
            }
            Self::UserQuestion {
                question_preview, ..
            } => {
                format!("UserQuestion({question_preview})")
            }
            Self::ModelFetch { ok, count, .. } => {
                format!("ModelFetch(ok={ok}, {count} models)")
            }
            Self::SessionList { count, .. } => format!("SessionList({count})"),
            Self::SessionLoad {
                session_id,
                msg_count,
                ..
            } => {
                format!("SessionLoad({session_id}: {msg_count} msgs)")
            }
            Self::VoiceEvent { variant, .. } => format!("VoiceEvent({variant})"),
            Self::SlashCommand {
                name, args_preview, ..
            } => {
                format!("Slash({name} {args_preview})")
            }
            Self::OverlayOpened { name, .. } => format!("OverlayOpen({name})"),
            Self::OverlayClosed { name, .. } => format!("OverlayClose({name})"),
            Self::FrameRendered {
                frame, render_ms, ..
            } => {
                format!("Frame(#{frame}, {render_ms:.1}ms)")
            }
            Self::Error {
                source, message, ..
            } => format!("Error({source}: {message})"),
        }
    }
}

/// Ring buffer of TUI events. Thread-safe via `Mutex<VecDeque>`.
/// When `enabled` is false, `publish` is a no-op (checked via AtomicBool).
pub struct TuiEventBus {
    ring: Mutex<VecDeque<TuiEvent>>,
    enabled: std::sync::atomic::AtomicBool,
}

impl Default for TuiEventBus {
    fn default() -> Self {
        Self::new(false)
    }
}

impl TuiEventBus {
    pub fn new(enabled: bool) -> Self {
        Self {
            ring: Mutex::new(VecDeque::with_capacity(EVENT_RING_CAPACITY)),
            enabled: std::sync::atomic::AtomicBool::new(enabled),
        }
    }

    /// Enable or disable event recording at runtime.
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled
            .store(enabled, std::sync::atomic::Ordering::Relaxed);
    }

    /// Publish an event to the ring buffer. No-op when disabled.
    /// Call this from every event handler — the AtomicBool check is
    /// branch-predicted to ~0 cost when disabled.
    pub fn publish(&self, event: TuiEvent) {
        if !self.enabled.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        let mut ring = match self.ring.lock() {
            Ok(g) => g,
            Err(_) => return, // poisoned — silently drop
        };
        if ring.len() >= EVENT_RING_CAPACITY {
            ring.pop_front();
        }
        ring.push_back(event);
    }

    /// Get the last N events as a Vec, oldest-first.
    pub fn recent(&self, last_n: usize) -> Vec<TuiEvent> {
        let ring = match self.ring.lock() {
            Ok(g) => g,
            Err(_) => return Vec::new(),
        };
        ring.iter().rev().take(last_n).rev().cloned().collect()
    }

    /// Snapshot the full ring as JSON (for replay / diffing).
    pub fn snapshot_json(&self) -> String {
        let ring = match self.ring.lock() {
            Ok(g) => g,
            Err(_) => return "[]".to_string(),
        };
        serde_json::to_string_pretty(&ring.iter().collect::<Vec<_>>())
            .unwrap_or_else(|_| "[]".to_string())
    }

    /// Dump the full ring to a file (for replay). Best-effort.
    pub fn dump_to_file(&self, path: &std::path::Path) -> std::io::Result<()> {
        let json = self.snapshot_json();
        std::fs::write(path, json)
    }

    /// Total events currently in the ring.
    #[allow(dead_code)] // Prepared for debug overlay stats
    pub fn len(&self) -> usize {
        self.ring.lock().map(|g| g.len()).unwrap_or(0)
    }

    #[allow(dead_code)] // Prepared for debug overlay stats
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Helper: current time as unix seconds (f64).
pub fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_buffer_evicts_oldest() {
        let bus = TuiEventBus::new(true);
        // Publish 1_100 events — should only keep last 1_000.
        for i in 0..1_100 {
            bus.publish(TuiEvent::FrameRendered {
                frame: i,
                render_ms: 1.0,
                at: i as f64,
            });
        }
        assert_eq!(bus.len(), 1_000);
        let recent = bus.recent(1_000);
        // First event in ring should be event #100 (0-indexed).
        match &recent[0] {
            TuiEvent::FrameRendered { frame, .. } => assert_eq!(*frame, 100),
            _ => panic!(),
        }
    }

    #[test]
    fn disabled_bus_drops_events() {
        let bus = TuiEventBus::new(false);
        bus.publish(TuiEvent::FrameRendered {
            frame: 1,
            render_ms: 1.0,
            at: 1.0,
        });
        assert!(bus.is_empty());
    }

    #[test]
    fn enabled_at_runtime() {
        let bus = TuiEventBus::new(false);
        bus.set_enabled(true);
        bus.publish(TuiEvent::FrameRendered {
            frame: 1,
            render_ms: 1.0,
            at: 1.0,
        });
        assert_eq!(bus.len(), 1);
    }

    #[test]
    fn snapshot_json_is_valid_array() {
        let bus = TuiEventBus::new(true);
        bus.publish(TuiEvent::Key {
            code: "Enter".into(),
            modifiers: 0,
            at: 1.0,
        });
        let json = bus.snapshot_json();
        assert!(json.starts_with('['));
        assert!(json.contains("key"));
    }
}
