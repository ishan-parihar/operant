// Vendored from jcode (crates/operant-logging/src/watchdog.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805; imports std-only
// (no re-rooting needed). See operant_app/mod.rs for scope.

//! Stall watchdog: detects when the process stops making progress and writes
//! actionable diagnostics to the log instead of leaving a silent freeze.
//!
//! Long-running sessions occasionally "just hang". Because the hang is silent,
//! the log ends abruptly with no clue about what the process was doing. This
//! module fixes that in three ways:
//!
//! 1. **Breadcrumbs.** Hot loops call [`beat`] with a short phase label. The
//!    most recent label plus its timestamp are kept in a lock-free slot.
//! 2. **Liveness heartbeat.** A monitor thread logs a periodic `watchdog.alive`
//!    event with RSS, thread count, and open-fd count, so a log that stops
//!    growing pins the freeze to a bounded time window and shows resource
//!    trends leading up to it.
//! 3. **Stall dumps.** When no beat arrives within the stall threshold, the
//!    monitor logs `watchdog.stall` including a per-thread state snapshot
//!    (name, run state, kernel wait channel) on Linux. That distinguishes a
//!    deadlock (threads in futex wait) from a spin (threads running) from a
//!    blocked syscall (threads in a network/IO wchan).
//!
//! The monitor thread is independent of the async runtime and the UI loop, so
//! it still reports when either is wedged.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Instant;

/// Cap on threads included in a stall dump so one dump cannot flood the log.
const MAX_THREADS_IN_DUMP: usize = 48;

static LAST_BEAT_MS: AtomicU64 = AtomicU64::new(0);
static BEAT_COUNT: AtomicU64 = AtomicU64::new(0);
static PHASE: AtomicUsize = AtomicUsize::new(0);
static PROCESS_START: OnceLock<Instant> = OnceLock::new();
/// Number of in-flight units of work. A process that is simply waiting for
/// user input is idle, not stalled, so stall reporting is gated on this.
static BUSY: AtomicUsize = AtomicUsize::new(0);

/// RAII guard marking a unit of work in flight. While any guard is alive the
/// process is expected to make progress, so missing beats mean a real hang
/// rather than an idle wait for input.
#[must_use = "the guard must stay alive for the duration of the work"]
pub struct WorkGuard(());

impl WorkGuard {
    fn new(phase: &'static str) -> Self {
        BUSY.fetch_add(1, Ordering::SeqCst);
        beat(phase);
        Self(())
    }
}

impl Drop for WorkGuard {
    fn drop(&mut self) {
        BUSY.fetch_sub(1, Ordering::SeqCst);
        beat("idle");
    }
}

/// Mark the start of a unit of work that is expected to make steady progress.
pub fn begin_work(phase: &'static str) -> WorkGuard {
    WorkGuard::new(phase)
}

/// Whether any work is currently in flight.
pub fn is_busy() -> bool {
    BUSY.load(Ordering::SeqCst) > 0
}

/// Phase labels are interned so [`beat`] stays allocation-free on hot paths.
/// The set of labels is small, fixed, and compile-time known in practice.
static PHASES: OnceLock<std::sync::Mutex<Vec<&'static str>>> = OnceLock::new();

fn phases() -> &'static std::sync::Mutex<Vec<&'static str>> {
    PHASES.get_or_init(|| std::sync::Mutex::new(vec!["startup"]))
}

fn process_start() -> Instant {
    *PROCESS_START.get_or_init(Instant::now)
}

fn now_ms() -> u64 {
    process_start().elapsed().as_millis() as u64
}

/// Record that the given phase is making progress. Cheap enough for per-frame
/// and per-event use: two relaxed atomic stores in the common case.
pub fn beat(phase: &'static str) {
    LAST_BEAT_MS.store(now_ms(), Ordering::Relaxed);
    BEAT_COUNT.fetch_add(1, Ordering::Relaxed);

    let current = PHASE.load(Ordering::Relaxed);
    let table = phases();
    if table
        .lock()
        .is_ok_and(|guard| guard.get(current).is_some_and(|name| *name == phase))
    {
        return;
    }
    let Ok(mut guard) = table.lock() else {
        return;
    };
    let index = match guard.iter().position(|name| *name == phase) {
        Some(index) => index,
        None => {
            guard.push(phase);
            guard.len() - 1
        }
    };
    PHASE.store(index, Ordering::Relaxed);
}

fn current_phase() -> &'static str {
    let index = PHASE.load(Ordering::Relaxed);
    phases()
        .lock()
        .ok()
        .and_then(|guard| guard.get(index).copied())
        .unwrap_or("unknown")
}

/// Snapshot of cheap process-level resource counters.
#[derive(Debug, Default, Clone, Copy)]
pub struct ResourceSnapshot {
    pub rss_mb: u64,
    pub threads: usize,
    pub open_fds: usize,
}

pub fn resource_snapshot() -> ResourceSnapshot {
    ResourceSnapshot {
        rss_mb: read_rss_mb(),
        threads: count_entries("/proc/self/task"),
        open_fds: count_entries("/proc/self/fd"),
    }
}

fn read_rss_mb() -> u64 {
    let Ok(statm) = std::fs::read_to_string("/proc/self/statm") else {
        return 0;
    };
    let Some(rss_pages) = statm.split_whitespace().nth(1) else {
        return 0;
    };
    let pages: u64 = rss_pages.parse().unwrap_or(0);
    pages.saturating_mul(4096) / (1024 * 1024)
}

fn count_entries(path: &str) -> usize {
    std::fs::read_dir(path)
        .map(|entries| entries.flatten().count())
        .unwrap_or(0)
}

/// Per-thread state, used to tell a deadlock apart from a spin or blocked IO.
pub fn thread_states() -> Vec<String> {
    let Ok(entries) = std::fs::read_dir("/proc/self/task") else {
        return Vec::new();
    };
    let mut states: Vec<String> = entries
        .flatten()
        .take(MAX_THREADS_IN_DUMP)
        .map(|entry| {
            let dir = entry.path();
            let tid = entry.file_name().to_string_lossy().to_string();
            let name = read_trimmed(dir.join("comm")).unwrap_or_else(|| "?".to_string());
            // The thread name comes from `comm`, and it may contain COLONS
            // (a Rust test thread is named after its module path:
            // "tui::app::tests"). The record is colon-separated, so a
            // colon in the name made it ambiguous — a parser that split on
            // ':' got the wrong number of fields, and the test that
            // checked the shape failed on any machine whose thread names
            // carried a module path. The name's colons become dots, which
            // keeps the record's four fields unambiguous.
            let name = name.replace(':', ".");
            let wchan = read_trimmed(dir.join("wchan")).unwrap_or_else(|| "?".to_string());
            let state = read_trimmed(dir.join("stat"))
                .and_then(|stat| {
                    // `stat` is "<pid> (<comm>) <state> ...": comm may contain
                    // spaces, so split after the final ')'.
                    let rest = stat.rsplit_once(')')?.1.trim().to_string();
                    rest.split_whitespace().next().map(str::to_string)
                })
                .unwrap_or_else(|| "?".to_string());
            format!("{tid}:{name}:{state}:{wchan}")
        })
        .collect();
    states.sort();
    states
}

fn read_trimmed(path: std::path::PathBuf) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beat_records_phase_and_advances_counter() {
        let before = BEAT_COUNT.load(Ordering::Relaxed);
        beat("test-phase");
        assert_eq!(current_phase(), "test-phase");
        assert!(BEAT_COUNT.load(Ordering::Relaxed) > before);
        beat("test-phase-2");
        assert_eq!(current_phase(), "test-phase-2");
    }

    #[test]
    fn work_guard_tracks_busy_state() {
        assert!(!is_busy(), "no work should be in flight at test start");
        {
            let _guard = begin_work("test-work");
            assert!(is_busy());
            assert_eq!(current_phase(), "test-work");
        }
        assert!(!is_busy(), "guard drop must clear busy state");
        assert_eq!(current_phase(), "idle");
    }

    #[test]
    fn resource_snapshot_reports_live_process_state() {
        let snapshot = resource_snapshot();
        if cfg!(target_os = "linux") {
            assert!(snapshot.threads > 0, "expected at least one thread");
            assert!(snapshot.open_fds > 0, "expected at least one open fd");
        }
    }

    #[test]
    fn thread_states_are_parsed_into_compact_records() {
        let states = thread_states();
        if cfg!(target_os = "linux") {
            assert!(!states.is_empty());
            for record in &states {
                // The record is "tid:name:state:wchan" — FOUR fields, and
                // the name's own colons are dots, so the split is
                // unambiguous whatever the thread is called.
                let fields: Vec<&str> = record.split(':').collect();
                assert_eq!(
                    fields.len(),
                    4,
                    "expected tid:name:state:wchan, got {record}"
                );
                assert!(
                    fields[0].chars().all(|c| c.is_ascii_digit()),
                    "the first field is the thread id, got {record}"
                );
                assert!(
                    !fields[2].is_empty() && fields[2].len() == 1,
                    "the third field is the single-character state, got {record}"
                );
            }
        }
    }
}
