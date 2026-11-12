// Vendored from jcode (crates/operant-logging/src/watchdog.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805; imports std-only
// (no re-rooting needed). See operant_app/mod.rs for scope.
//
// The port carried the bookkeeping primitives (`beat`, `begin_work`, `is_busy`,
// `resource_snapshot`, `thread_states`) but NOT the monitor thread, the
// thresholds, or the two log events the doc header below promises — so
// nothing outside this module's own tests ever read them, and a frozen TUI
// left the log exactly as silent as the module exists to fix (RT-467). The
// "operant-local" section further down is the missing half, wired to the TUI
// by `frame()` (the render path) and `start_monitor()` (the app's constructor).
// The one edit to the ported body itself: `beat`'s two stores now go through
// `touch_beat`, so the frame path can record liveness without claiming a
// phase label. Every other ported line is unchanged.

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
    touch_beat();

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
#[cfg_attr(
    test,
    allow(
        dead_code,
        reason = "jcode-parity port: the RSS half of the watchdog snapshot; the panel renders threads and fds today, and the monitor thread logs all three"
    )
)]
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

// ---- operant-local: the monitor half the port did not carry (RT-467) ----

/// How often the monitor thread writes the liveness heartbeat.
const ALIVE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

/// How long work may go without a beat before the monitor calls it a stall.
///
/// The TUI paints a frame roughly every 33ms while a turn streams and at
/// worst every 5s while idle (the idle poll timeout), so 15s is three idle
/// gaps of slack: long enough that no healthy gap between frames reaches it,
/// short enough that a freeze is in the log while the operator is still
/// looking at the frozen screen.
const STALL_THRESHOLD: std::time::Duration = std::time::Duration::from_secs(15);

/// How often the monitor thread reads the beat clock.
const MONITOR_TICK: std::time::Duration = std::time::Duration::from_secs(1);

/// What the monitor reports on one tick.
///
/// The decision is a pure function of the clock (see [`decide`]) so it can be
/// pinned without sleeping for the threshold to elapse.
#[derive(Debug, PartialEq, Eq)]
enum Tick {
    /// Nothing to report.
    Quiet,
    /// The liveness heartbeat is due.
    Alive,
    /// Work is in flight and no beat arrived within the stall threshold.
    Stall { silent_ms: u64 },
}

/// Decide what the monitor reports at `now_ms`.
///
/// `last_beat_ms` is the elapsed-time stamp of the most recent beat, `busy`
/// says whether work is in flight, and `next_alive_ms` is when the next
/// liveness heartbeat is due. A stall outranks the heartbeat: the freeze is
/// the more urgent fact.
fn decide(now_ms: u64, last_beat_ms: u64, busy: bool, next_alive_ms: u64) -> Tick {
    let silent_ms = now_ms.saturating_sub(last_beat_ms);
    if busy && silent_ms >= STALL_THRESHOLD.as_millis() as u64 {
        return Tick::Stall { silent_ms };
    }
    if now_ms >= next_alive_ms {
        return Tick::Alive;
    }
    Tick::Quiet
}

/// The liveness heartbeat: the process is alive, and here is what it is
/// costing. It is written on its own interval whether or not anything else
/// happens, so a log that stops growing is bounded by one interval.
fn report_alive(snapshot: ResourceSnapshot) {
    tracing::info!(
        target: "watchdog",
        rss_mb = snapshot.rss_mb,
        threads = snapshot.threads,
        open_fds = snapshot.open_fds,
        beats = BEAT_COUNT.load(Ordering::Relaxed),
        "watchdog.alive"
    );
}

/// The stall report: the freeze, and what every thread was doing.
///
/// The per-thread records are what tell a deadlock (threads in futex wait)
/// from a spin (threads running) from a blocked syscall (threads in an IO
/// wait channel).
fn report_stall(silent_ms: u64, phase: &str, states: &[String]) {
    tracing::warn!(target: "watchdog", silent_ms, phase, "watchdog.stall");
    for state in states {
        tracing::warn!(target: "watchdog", "{state}");
    }
}

/// The monitor thread: reads the beat clock once a second, forever.
///
/// It is deliberately independent of the async runtime and the UI loop — a
/// wedged UI thread is exactly the case it has to survive.
fn monitor_loop() {
    let mut next_alive_ms = now_ms() + ALIVE_INTERVAL.as_millis() as u64;
    let mut next_stall_ms = 0u64;
    loop {
        std::thread::sleep(MONITOR_TICK);
        let now = now_ms();
        let last_beat_ms = LAST_BEAT_MS.load(Ordering::Relaxed);
        match decide(now, last_beat_ms, is_busy(), next_alive_ms) {
            Tick::Quiet => {}
            Tick::Alive => {
                next_alive_ms = now + ALIVE_INTERVAL.as_millis() as u64;
                report_alive(resource_snapshot());
            }
            Tick::Stall { silent_ms } => {
                // One report per heartbeat interval: a freeze is a standing
                // condition, and a line every second would bury the log.
                if now < next_stall_ms {
                    continue;
                }
                next_stall_ms = now + ALIVE_INTERVAL.as_millis() as u64;
                report_stall(silent_ms, current_phase(), &thread_states());
            }
        }
    }
}

/// Set once the monitor thread has been started.
static MONITOR_STARTED: OnceLock<()> = OnceLock::new();

/// Start the monitor thread. The first call wins; later calls are no-ops.
pub fn start_monitor() {
    if MONITOR_STARTED.set(()).is_err() {
        return;
    }
    if let Err(error) = std::thread::Builder::new()
        .name("watchdog".to_string())
        .spawn(monitor_loop)
    {
        tracing::warn!(target: "watchdog", %error, "watchdog monitor thread did not start");
    }
}

/// The guard for the work currently in flight, if any.
static WORK: std::sync::Mutex<Option<WorkGuard>> = std::sync::Mutex::new(None);

/// Record that the process is alive, without claiming a phase label.
///
/// The phase is the breadcrumb of WHAT the process was doing, and the work
/// guard already owns it ("turn" while a turn is in flight). The frame path
/// runs on every frame; if it also claimed the phase, the label would say
/// "render" for every stall and the work that was in flight would be lost.
fn touch_beat() {
    LAST_BEAT_MS.store(now_ms(), Ordering::Relaxed);
    BEAT_COUNT.fetch_add(1, Ordering::Relaxed);
}

/// Record whether work is in flight.
///
/// The busy flag is pushed on every frame rather than held by a guard at the
/// turn's call sites because a turn ends by several paths (stream end,
/// cancel, error) and the frame is the one place that always runs
/// afterwards. The guard still owns the accounting: dropping it clears the
/// busy count.
fn set_work_in_flight(in_flight: bool) {
    let Ok(mut slot) = WORK.lock() else {
        return;
    };
    match (in_flight, slot.is_some()) {
        (true, false) => *slot = Some(begin_work("turn")),
        (false, true) => *slot = None,
        _ => {}
    }
}

/// The TUI's liveness signal: a frame was painted, and `work_in_flight` says
/// whether a turn is running.
///
/// A freeze stops the frames, so the frame path is where the watchdog has to
/// be told the UI is alive. This is the only entry point the TUI needs.
pub fn frame(work_in_flight: bool) {
    touch_beat();
    set_work_in_flight(work_in_flight);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The busy count and the phase label are process-global, so the tests
    /// that assert on them take this lock and run one at a time. (The TUI's
    /// render path writes the same globals in production; no test paints a
    /// streaming frame today, so it cannot interleave with them here.)
    static GLOBAL_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// A subscriber that collects the formatted events into a shared buffer.
    #[derive(Clone, Default)]
    struct Capture(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for Capture {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
        type Writer = Capture;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    /// Capture what the monitor writes, as the operator would read it.
    fn captured(write: impl FnOnce()) -> String {
        let capture = Capture::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(capture.clone())
            .without_time()
            .with_ansi(false)
            .finish();
        tracing::subscriber::with_default(subscriber, write);
        let bytes = capture.0.lock().unwrap_or_else(|e| e.into_inner()).clone();
        String::from_utf8(bytes).unwrap_or_default()
    }

    /// The two log events are the whole point of the module: the row that
    /// recorded the gap (RT-467) says neither `watchdog.alive` nor
    /// `watchdog.stall` was written by ANY code path. These assertions are
    /// on the formatted lines an operator reads, not on the internals.
    #[test]
    fn the_stall_report_names_the_freeze_and_the_threads() {
        let states = vec!["12345:worker:S:futex_wait".to_string()];
        let text = captured(|| report_stall(15_003, "turn", &states));
        assert!(
            text.contains("watchdog.stall"),
            "the stall event must reach the log: {text}"
        );
        assert!(
            text.contains("silent_ms=15003"),
            "the stall report must say how long the silence was: {text}"
        );
        assert!(
            text.contains("phase=\"turn\"") || text.contains("phase=turn"),
            "the stall report must name what was in flight: {text}"
        );
        assert!(
            text.contains("12345:worker:S:futex_wait"),
            "the per-thread state snapshot must reach the log: {text}"
        );
    }

    /// The heartbeat is what bounds "the log stopped growing": it carries
    /// the resource counters the module's doc header promises.
    #[test]
    fn the_heartbeat_reports_the_process_resources() {
        let snapshot = ResourceSnapshot {
            rss_mb: 42,
            threads: 7,
            open_fds: 11,
        };
        let text = captured(|| report_alive(snapshot));
        assert!(
            text.contains("watchdog.alive"),
            "the heartbeat event must reach the log: {text}"
        );
        assert!(text.contains("rss_mb=42"), "{text}");
        assert!(text.contains("threads=7"), "{text}");
        assert!(text.contains("open_fds=11"), "{text}");
    }

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
        let _lock = GLOBAL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // The busy count is process-global and the TUI's render path now
        // marks work in flight through it, so a concurrent test may hold a
        // guard: what this test pins is that THIS guard is counted while it
        // is alive and uncounted when it drops — not that the process was
        // idle at the moment the test happened to start.
        let busy_before = is_busy();
        {
            let _guard = begin_work("test-work");
            assert!(is_busy());
            assert_eq!(current_phase(), "test-work");
        }
        assert_eq!(is_busy(), busy_before, "guard drop must clear busy state");
        assert_eq!(current_phase(), "idle");
    }

    /// The frame path is what tells the watchdog the UI is alive, and the
    /// busy flag it carries is what opens the stall gate: without it a
    /// frozen turn would be indistinguishable from an idle TUI.
    #[test]
    fn a_frame_marks_work_in_flight_while_a_turn_is_running() {
        let _lock = GLOBAL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        frame(true);
        assert!(
            is_busy(),
            "a frame during a turn must mark work in flight — the stall report is gated on it"
        );
        frame(false);
        assert!(
            !is_busy(),
            "the frame after the turn ended must clear the busy state"
        );
    }

    /// A busy process that stopped beating is the stall the module exists to
    /// report: the freeze is silent, so the log is the only witness.
    #[test]
    fn a_busy_process_that_stopped_beating_is_reported_as_stalled() {
        let now = 100_000u64;
        let threshold = STALL_THRESHOLD.as_millis() as u64;
        let last_beat = now - threshold - 1;
        assert_eq!(
            decide(now, last_beat, true, now + 1_000),
            Tick::Stall {
                silent_ms: threshold + 1
            }
        );
    }

    /// The gate that keeps the report honest: a TUI waiting for input is
    /// idle, not stalled.
    #[test]
    fn an_idle_process_that_stopped_beating_is_not_reported_as_stalled() {
        let now = 100_000u64;
        assert_eq!(decide(now, 0, false, now + 1_000), Tick::Quiet);
    }

    /// A beat inside the threshold means the process is alive, however long
    /// the last one was.
    #[test]
    fn a_beat_inside_the_threshold_keeps_the_process_out_of_the_stall_report() {
        let now = 100_000u64;
        let threshold = STALL_THRESHOLD.as_millis() as u64;
        assert_eq!(
            decide(now, now - threshold + 1, true, now + 1_000),
            Tick::Quiet
        );
    }

    /// The heartbeat is what bounds "the log stopped growing": it is written
    /// on its own interval, whether or not anything else is happening.
    #[test]
    fn the_liveness_heartbeat_is_due_on_its_interval() {
        let now = 100_000u64;
        assert_eq!(decide(now, now, false, now), Tick::Alive);
        assert_eq!(decide(now, now, false, now + 1), Tick::Quiet);
    }

    /// When both are due, the freeze is the fact worth writing down.
    #[test]
    fn a_stall_outranks_the_heartbeat_when_both_are_due() {
        let now = 100_000u64;
        let threshold = STALL_THRESHOLD.as_millis() as u64;
        assert_eq!(
            decide(now, now - threshold - 1, true, now),
            Tick::Stall {
                silent_ms: threshold + 1
            }
        );
    }

    /// The threshold has to outlast the slowest frame the TUI paints when it
    /// is doing nothing: an idle TUI is waiting for input, and a threshold
    /// shorter than its idle cadence would report the operator's silence as
    /// a freeze. This is the one number in the module that no unit test can
    /// judge on its own — it is a relationship with the render cadence.
    #[test]
    fn the_stall_threshold_outlasts_the_slowest_idle_frame_cadence() {
        let slowest_idle = crate::tui::redraw::redraw_interval(
            crate::tui::redraw::PerformanceTier::Minimal,
            false,
            Some(std::time::Duration::from_secs(60)),
            None,
            false,
        );
        assert!(
            STALL_THRESHOLD > slowest_idle,
            "the stall threshold ({STALL_THRESHOLD:?}) must outlast the slowest idle \
             frame cadence ({slowest_idle:?}), or an idle TUI reports itself as frozen"
        );
    }

    /// The wiring is invisible to the compiler: `frame` and `start_monitor`
    /// are ordinary calls, and deleting either one leaves every other test
    /// green — the watchdog would simply go back to reading nothing. The
    /// needles are call-site shaped (the call, with its receiver), so a
    /// renamed function or a comment that mentions one does not satisfy
    /// them.
    #[test]
    fn the_tui_starts_the_monitor_and_beats_every_frame() {
        const RENDER: &str = include_str!("../../render/mod.rs");
        const INIT: &str = include_str!("../../app/init.rs");
        assert!(
            RENDER.contains("watchdog::frame("),
            "the render path must beat the watchdog: a freeze stops the frames, \
             and that silence is what the monitor reports"
        );
        assert!(
            INIT.contains("watchdog::start_monitor()"),
            "the TUI must start the watchdog monitor: without a thread reading \
             the beat clock, nothing is ever reported"
        );
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
