//! Herdr integration — report Operant's agent lifecycle to a running Herdr server.
//!
//! Herdr recognises two kinds of agent: the ones it supports itself (it reads
//! their screen with a detection manifest), and the ones that report their own
//! state. Operant is the second kind. That is not a workaround — it is the
//! only route, because herdr's own documentation is explicit about the limit:
//!
//! > Adding a completely new agent still requires a Herdr binary update for
//! > process detection, labels, and integration behavior.
//!
//! A detection manifest can teach herdr the *shape* of a known agent's screen;
//! it cannot teach herdr a brand-new agent name. Self-reporting can, and does:
//! `herdr agent list` then shows the pane as an `operant` agent with a real
//! `working` / `idle` / `blocked` state, the sidebar rolls that state up to
//! the tab and the workspace, and `herdr agent wait` can block on it.
//!
//! The wire protocol is herdr's CLI, invoked through `$HERDR_BIN_PATH`:
//!
//! ```text
//! herdr pane report-agent <pane> --source operant --agent operant \
//!      --state working --seq 12 --agent-session-id <id>
//! herdr pane release-agent <pane> --source operant --agent operant --seq 13
//! ```
//!
//! herdr's own guidance picks this transport over the raw socket: "HERDR_BIN_PATH
//! and the CLI are the portable choice." The CLI also validates the arguments
//! and answers with a typed error instead of a silent no-op.
//!
//! Two rules from herdr's documentation shape the design here:
//!
//! * "Don't let Herdr slow your agent down. Send reports in the background or
//!   with a short timeout, and ignore failures." — every report is handed to a
//!   dedicated background thread, and only the newest pending state survives.
//! * "`--seq` ... must increase with every report from your source, including
//!   across sessions and restarts of your agent." — the sequence number is
//!   wall-clock derived and strictly increasing within the process.
//!
//! Outside a Herdr pane every function here is a no-op: the reporter is absent
//! rather than present-and-broken, so callers can call it unconditionally.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// The agent name herdr shows for this integration.
///
/// herdr asks integrations to "use your own agent's name, not the name of an
/// agent Herdr already supports" — `operant` is ours.
pub const AGENT_LABEL: &str = "operant";

/// The integration identity herdr uses to attribute reports.
///
/// herdr reserves the `herdr:` prefix for the integrations it ships
/// ("Don't start it with `herdr:`, because Herdr's own integrations use that
/// prefix"), so a self-owned integration uses a plain name.
pub const SOURCE_ID: &str = "operant";

/// How long a single `herdr` invocation may take before it is killed.
///
/// The report runs on a background thread, so this bound is about not leaking
/// a stuck child process, not about protecting the agent loop.
const REPORT_TIMEOUT: Duration = Duration::from_secs(2);

/// The lifecycle state herdr tracks for an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentState {
    /// Operant is ready for input.
    Idle,
    /// Operant is processing a turn.
    Working,
    /// Operant needs the operator to decide something.
    Blocked,
}

impl AgentState {
    /// The wire spelling herdr expects for `--state`.
    pub fn as_str(self) -> &'static str {
        match self {
            AgentState::Idle => "idle",
            AgentState::Working => "working",
            AgentState::Blocked => "blocked",
        }
    }
}

/// The Herdr context an Operant process inherited from its pane.
///
/// A process running inside a Herdr pane is handed these variables:
///
/// | Variable | Meaning |
/// | --- | --- |
/// | `HERDR_ENV` | `1` inside Herdr |
/// | `HERDR_PANE_ID` | The pane the agent runs in |
/// | `HERDR_BIN_PATH` | The Herdr binary that owns this pane |
/// | `HERDR_SOCKET_PATH` | Herdr's API socket |
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HerdrContext {
    /// The pane this process runs in.
    pub pane_id: String,
    /// The API socket of the Herdr server that owns the pane.
    pub socket_path: String,
    /// The herdr binary to invoke for reports.
    pub bin_path: String,
}

impl HerdrContext {
    /// Read the Herdr context from the environment.
    ///
    /// Returns `None` outside a Herdr pane, or when the pane's context is
    /// incomplete (no pane id, no socket). A half-present context would make
    /// every report fail, which is worse than not reporting at all.
    pub fn from_env() -> Option<Self> {
        Self::from_env_with(|key| std::env::var(key).ok())
    }

    /// Read the Herdr context through an injectable environment lookup.
    ///
    /// The seam exists so the rules can be tested without mutating the real
    /// process environment, which is shared with every other thread.
    pub fn from_env_with<F>(get: F) -> Option<Self>
    where
        F: Fn(&str) -> Option<String>,
    {
        if get("HERDR_ENV").as_deref() != Some("1") {
            return None;
        }
        let pane_id = get("HERDR_PANE_ID").filter(|v| !v.trim().is_empty())?;
        let socket_path = get("HERDR_SOCKET_PATH").filter(|v| !v.trim().is_empty())?;
        let bin_path = get("HERDR_BIN_PATH")
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| "herdr".to_string());
        Some(Self {
            pane_id,
            socket_path,
            bin_path,
        })
    }

    /// Whether a Herdr server is still reachable at the socket path.
    ///
    /// Used by `operant doctor` to tell "not in a pane" apart from "in a pane
    /// whose server is gone".
    pub fn server_socket_present(&self) -> bool {
        std::path::Path::new(&self.socket_path).exists()
    }
}

/// What the background thread has to deliver.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Command {
    /// Report a lifecycle state (and the session identity, when known).
    Report {
        state: AgentState,
        message: Option<String>,
        session_id: Option<String>,
        seq: u64,
    },
    /// Give the pane back to Herdr.
    Release { seq: u64 },
}

/// The last thing this reporter told Herdr.
///
/// Herdr only cares about the CURRENT state, so a report that would not change
/// anything is dropped before it reaches the socket. This matters: a turn can
/// produce hundreds of content deltas, and one `herdr` process per delta
/// would be a hundred processes per turn.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LastReport {
    state: AgentState,
    message: Option<String>,
    session_id: Option<String>,
}

/// The reporter: the process-wide handle onto the Herdr background thread.
pub struct HerdrReporter {
    ctx: HerdrContext,
    shared: Arc<Shared>,
    seq: AtomicU64,
    session_id: Arc<Mutex<Option<String>>>,
    last: Arc<Mutex<Option<LastReport>>>,
}

/// The hand-off between the callers and the delivery thread.
struct Shared {
    /// The newest pending command. Older ones are dropped: herdr only cares
    /// about the latest state, and a queue would let a stale `working` land
    /// after a newer `idle`.
    pending: Mutex<Option<Command>>,
    /// Signals the delivery thread that a command is pending.
    signal: Condvar,
    /// Set when the process is shutting down.
    stop: AtomicBool,
}

static REPORTER: OnceLock<Option<HerdrReporter>> = OnceLock::new();

/// The process-wide reporter, if this process runs inside a Herdr pane.
///
/// Returns `None` outside Herdr. The first call reads the environment; later
/// calls reuse the same handle.
pub fn reporter() -> Option<&'static HerdrReporter> {
    REPORTER
        .get_or_init(|| HerdrReporter::start(HerdrContext::from_env()))
        .as_ref()
}

/// Explicitly initialise the reporter.
///
/// The same as [`reporter()`], but named for call sites that want to make the
/// initialisation visible (the TUI does this once at start-up).
pub fn init() -> Option<&'static HerdrReporter> {
    reporter()
}

impl HerdrReporter {
    /// Start the background delivery thread for a context.
    ///
    /// `None` context means "not inside a Herdr pane", so there is no
    /// reporter and every caller degrades to a no-op.
    fn start(ctx: Option<HerdrContext>) -> Option<Self> {
        let ctx = ctx?;
        let shared = Arc::new(Shared {
            pending: Mutex::new(None),
            signal: Condvar::new(),
            stop: AtomicBool::new(false),
        });
        let worker = Arc::clone(&shared);
        let worker_ctx = ctx.clone();
        // A dedicated OS thread rather than a tokio task: the reporter must
        // work whether or not the caller has a runtime, and it must never
        // share a runtime's scheduling budget with the agent.
        let spawned = std::thread::Builder::new()
            .name("operant-herdr".to_string())
            .spawn(move || delivery_loop(worker_ctx, worker));
        if spawned.is_err() {
            // Without a thread there is no reporter; the agent still runs.
            return None;
        }
        Some(Self {
            ctx,
            shared,
            seq: AtomicU64::new(0),
            session_id: Arc::new(Mutex::new(None)),
            last: Arc::new(Mutex::new(None)),
        })
    }

    /// The Herdr context this reporter talks to.
    pub fn context(&self) -> &HerdrContext {
        &self.ctx
    }

    /// The session identity last reported to Herdr.
    pub fn session_id(&self) -> Option<String> {
        self.session_id.lock().ok().and_then(|id| id.clone())
    }

    /// The lifecycle state last reported to Herdr.
    ///
    /// `Idle` before anything has been reported — a fresh pane is ready for
    /// input as far as herdr is concerned.
    pub fn last_state(&self) -> AgentState {
        self.last
            .lock()
            .ok()
            .and_then(|last| last.as_ref().map(|last| last.state))
            .unwrap_or(AgentState::Idle)
    }

    /// Remember the Operant session identity and report it to Herdr.
    ///
    /// The identity rides along with the state report, so Herdr ties the pane
    /// to the session. The lifecycle state is carried over unchanged: a
    /// session switch mid-turn must not make a working agent look idle.
    pub fn report_session(&self, session_id: Option<&str>) {
        let id = session_id
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        if let Ok(mut slot) = self.session_id.lock() {
            *slot = id.clone();
        }
        self.enqueue_report(self.last_state(), None, id);
    }

    /// Report a lifecycle state.
    pub fn report_state(&self, state: AgentState) {
        let session_id = self.session_id();
        self.enqueue_report(state, None, session_id);
    }

    /// Report that Operant is blocked on an operator decision.
    ///
    /// The message is what Herdr shows next to the `blocked` badge, so it
    /// should say what the decision is, not merely that one is needed.
    pub fn report_blocked(&self, message: impl Into<String>) {
        let session_id = self.session_id();
        self.enqueue_report(AgentState::Blocked, Some(message.into()), session_id);
    }

    /// Give the pane back to Herdr.
    ///
    /// Called when Operant exits. Herdr clears the agent name, the state and
    /// the resume command from the pane.
    pub fn release(&self) {
        let seq = self.next_seq();
        self.enqueue(Command::Release { seq });
    }

    /// Queue a report, unless it would not change what Herdr already shows.
    fn enqueue_report(
        &self,
        state: AgentState,
        message: Option<String>,
        session_id: Option<String>,
    ) {
        let report = LastReport {
            state,
            message: message.clone(),
            session_id: session_id.clone(),
        };
        {
            let Ok(mut last) = self.last.lock() else {
                return;
            };
            if last.as_ref() == Some(&report) {
                return;
            }
            *last = Some(report);
        }
        let seq = self.next_seq();
        self.enqueue(Command::Report {
            state,
            message,
            session_id,
            seq,
        });
    }

    /// The next sequence number.
    ///
    /// Wall-clock derived so it keeps increasing across restarts of Operant,
    /// and strictly increasing within the process even when two reports land
    /// in the same microsecond. Herdr ignores a report whose number is not
    /// higher than the last one it accepted, so a repeat would be dropped.
    fn next_seq(&self) -> u64 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_micros() as u64)
            .unwrap_or(0);
        let mut previous = self.seq.load(Ordering::SeqCst);
        loop {
            let next = if now > previous { now } else { previous + 1 };
            match self
                .seq
                .compare_exchange_weak(previous, next, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => return next,
                Err(current) => previous = current,
            }
        }
    }

    /// Hand a command to the delivery thread, replacing any older one.
    fn enqueue(&self, command: Command) {
        if let Ok(mut pending) = self.shared.pending.lock() {
            *pending = Some(command);
            self.shared.signal.notify_one();
        }
    }
}

/// The delivery thread: take the newest pending command and send it.
fn delivery_loop(ctx: HerdrContext, shared: Arc<Shared>) {
    loop {
        let command = {
            let mut pending = match shared.pending.lock() {
                Ok(guard) => guard,
                Err(_) => return,
            };
            loop {
                if shared.stop.load(Ordering::SeqCst) {
                    return;
                }
                if let Some(command) = pending.take() {
                    break Some(command);
                }
                pending = match shared.signal.wait(pending) {
                    Ok(guard) => guard,
                    Err(_) => return,
                };
            }
        };
        if let Some(command) = command {
            deliver(&ctx, &command);
        }
    }
}

/// Send one command to Herdr.
fn deliver(ctx: &HerdrContext, command: &Command) {
    let args = match command {
        Command::Report {
            state,
            message,
            session_id,
            seq,
        } => {
            let mut args = vec![
                "pane".to_string(),
                "report-agent".to_string(),
                ctx.pane_id.clone(),
                "--source".to_string(),
                SOURCE_ID.to_string(),
                "--agent".to_string(),
                AGENT_LABEL.to_string(),
                "--state".to_string(),
                state.as_str().to_string(),
                "--seq".to_string(),
                seq.to_string(),
            ];
            if let Some(message) = message {
                args.push("--message".to_string());
                args.push(message.clone());
            }
            if let Some(session_id) = session_id {
                args.push("--agent-session-id".to_string());
                args.push(session_id.clone());
            }
            args
        }
        Command::Release { seq } => vec![
            "pane".to_string(),
            "release-agent".to_string(),
            ctx.pane_id.clone(),
            "--source".to_string(),
            SOURCE_ID.to_string(),
            "--agent".to_string(),
            AGENT_LABEL.to_string(),
            "--seq".to_string(),
            seq.to_string(),
        ],
    };
    run_with_timeout(ctx, &args);
}

/// Invoke herdr and wait, bounded by [`REPORT_TIMEOUT`].
///
/// A failure is ignored on purpose: a missing herdr binary, a stopped server
/// or a rejected report must never affect the agent.
fn run_with_timeout(ctx: &HerdrContext, args: &[String]) {
    let mut child = match std::process::Command::new(&ctx.bin_path)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return,
    };
    let deadline = Instant::now() + REPORT_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => {
                let _ = child.kill();
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |key: &str| map.get(key).cloned()
    }

    /// A reporter pointed at a binary that does not exist, so no test ever
    /// talks to a real herdr server.
    fn test_reporter() -> HerdrReporter {
        HerdrReporter::start(Some(HerdrContext {
            pane_id: "w1:p1".to_string(),
            socket_path: "/tmp/herdr-test.sock".to_string(),
            bin_path: "/nonexistent/herdr".to_string(),
        }))
        .expect("reporter")
    }

    #[test]
    fn context_requires_the_herdr_marker() {
        assert!(HerdrContext::from_env_with(env(&[("HERDR_PANE_ID", "w1:p1")])).is_none());
        assert!(HerdrContext::from_env_with(env(&[("HERDR_ENV", "0")])).is_none());
        assert!(HerdrContext::from_env_with(env(&[("HERDR_ENV", "1")])).is_none());
    }

    #[test]
    fn context_needs_a_pane_id_and_a_socket() {
        let no_pane = env(&[("HERDR_ENV", "1"), ("HERDR_SOCKET_PATH", "/tmp/sock")]);
        assert!(HerdrContext::from_env_with(no_pane).is_none());

        let no_socket = env(&[("HERDR_ENV", "1"), ("HERDR_PANE_ID", "w1:p1")]);
        assert!(HerdrContext::from_env_with(no_socket).is_none());

        let blank_pane = env(&[
            ("HERDR_ENV", "1"),
            ("HERDR_PANE_ID", "  "),
            ("HERDR_SOCKET_PATH", "/tmp/sock"),
        ]);
        assert!(HerdrContext::from_env_with(blank_pane).is_none());
    }

    #[test]
    fn context_reads_the_pane_and_socket() {
        let ctx = HerdrContext::from_env_with(env(&[
            ("HERDR_ENV", "1"),
            ("HERDR_PANE_ID", "w1:p1"),
            ("HERDR_SOCKET_PATH", "/tmp/herdr.sock"),
        ]))
        .expect("context");
        assert_eq!(ctx.pane_id, "w1:p1");
        assert_eq!(ctx.socket_path, "/tmp/herdr.sock");
        // The binary falls back to PATH when the pane does not name one.
        assert_eq!(ctx.bin_path, "herdr");
    }

    #[test]
    fn context_prefers_the_panes_own_binary() {
        let ctx = HerdrContext::from_env_with(env(&[
            ("HERDR_ENV", "1"),
            ("HERDR_PANE_ID", "w1:p1"),
            ("HERDR_SOCKET_PATH", "/tmp/herdr.sock"),
            ("HERDR_BIN_PATH", "/opt/herdr/bin/herdr"),
        ]))
        .expect("context");
        assert_eq!(ctx.bin_path, "/opt/herdr/bin/herdr");
    }

    #[test]
    fn state_spellings_match_herdr() {
        assert_eq!(AgentState::Idle.as_str(), "idle");
        assert_eq!(AgentState::Working.as_str(), "working");
        assert_eq!(AgentState::Blocked.as_str(), "blocked");
    }

    #[test]
    fn sequence_numbers_strictly_increase() {
        let reporter = test_reporter();
        let mut previous = reporter.next_seq();
        for _ in 0..64 {
            let next = reporter.next_seq();
            assert!(next > previous, "seq must increase: {next} !> {previous}");
            previous = next;
        }
    }

    #[test]
    fn reporting_outside_herdr_is_a_no_op() {
        // The process-wide reporter is absent outside a pane, so every caller
        // can call it unconditionally without a feature check.
        if HerdrContext::from_env().is_none() {
            assert!(reporter().is_none());
            assert!(init().is_none());
        }
    }

    #[test]
    fn the_session_identity_is_remembered() {
        let reporter = test_reporter();
        assert!(reporter.session_id().is_none());
        reporter.report_session(Some("sess-42"));
        assert_eq!(reporter.session_id().as_deref(), Some("sess-42"));
        // A blank identity is not an identity.
        reporter.report_session(Some("   "));
        assert!(reporter.session_id().is_none());
    }

    #[test]
    fn a_repeated_state_is_not_reported_twice() {
        let reporter = test_reporter();
        reporter.report_state(AgentState::Working);
        let first = reporter.last.lock().expect("last lock").clone();
        reporter.report_state(AgentState::Working);
        let second = reporter.last.lock().expect("last lock").clone();
        assert_eq!(first, second, "an unchanged state is not a new report");
    }

    #[test]
    fn a_session_switch_keeps_the_lifecycle_state() {
        let reporter = test_reporter();
        reporter.report_state(AgentState::Working);
        reporter.report_session(Some("sess-7"));
        // The session changed, so the state must NOT have been reset to idle:
        // a working agent that switched sessions is still working.
        assert_eq!(reporter.last_state(), AgentState::Working);
        assert_eq!(reporter.session_id().as_deref(), Some("sess-7"));
    }
}
