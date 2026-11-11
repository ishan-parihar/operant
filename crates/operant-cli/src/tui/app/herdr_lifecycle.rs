//! herdr_lifecycle.rs — the TUI's Herdr lifecycle bridge.
//!
//! WHAT THIS IS FOR
//!
//! A pane running Operant inside Herdr used to look like a plain terminal to
//! Herdr: `herdr agent list` showed nothing for it, the sidebar had no badge,
//! and `herdr agent wait` had nothing to wait on. The operator's words for it
//! were "my active herdr instance is not able to identify the active operant
//! sessions".
//!
//! Herdr cannot fix that by reading Operant's screen. Herdr's own
//! documentation is explicit: "Adding a completely new agent still requires a
//! Herdr binary update for process detection, labels, and integration
//! behavior." A detection manifest teaches Herdr the *shape* of an agent it
//! already knows; it cannot teach it the name "operant".
//!
//! What CAN teach it is Operant itself. Herdr supports "agents that support
//! Herdr" — agents that report their own state — and that is the model this
//! bridge implements. The report itself lives in `operant_core::herdr`; this
//! file is only about knowing WHEN Operant is working, idle, or blocked.
//!
//! WHERE THE SIGNALS COME FROM
//!
//! * `working` — the agent produced something: reasoning, content, a tool
//!   call, a completed iteration.
//! * `idle` — the turn ended: a final answer, or an error.
//! * `blocked` — the model asked the operator a question and the answer is
//!   the only thing that can continue the turn. This is the state that
//!   matters most: it is the difference between "come look at this" and
//!   "this pane is busy, leave it alone".
//!
//! Every call here is a no-op outside a Herdr pane, so the TUI does not need
//! a feature check around any of it.

use operant_core::agent::AgentEvent;
use operant_core::herdr::{AgentState, HerdrReporter};

use super::App;

impl App {
    /// The Herdr reporter, when this process runs inside a Herdr pane.
    ///
    /// The config switch is read HERE, in the one place every report goes
    /// through, so `[integrations.herdr] enabled = false` turns the whole
    /// integration off without a second decision point to keep in step.
    fn herdr_reporter(&self) -> Option<&'static HerdrReporter> {
        if !self.config.integrations.herdr.enabled {
            return None;
        }
        operant_core::herdr::reporter()
    }

    /// The agent's live session id.
    ///
    /// The agent is the only authority on its own identity (RT-206 recorded
    /// that the App's remembered copy stayed `None` for a plain `operant chat`
    /// start, because the id is minted on the first turn). The session-switch
    /// source holds the agent's own reader, so the App can ask without
    /// holding the agent.
    fn herdr_session_id(&self) -> Option<String> {
        self.core_session_switch
            .as_ref()
            .and_then(|source| source.session_id())
    }

    /// Tell Herdr which session this pane is running, when that changed.
    fn sync_herdr_session(&self) {
        let Some(reporter) = self.herdr_reporter() else {
            return;
        };
        let current = self.herdr_session_id();
        if current == reporter.session_id() {
            return;
        }
        reporter.report_session(current.as_deref());
    }

    /// Map an agent event onto a Herdr lifecycle state.
    ///
    /// The session identity is synced at the turn boundaries only. A turn can
    /// emit hundreds of content deltas, and re-reading the agent's session id
    /// on every one of them would put a lock acquisition in the hot path for
    /// no benefit — the id cannot change while a turn is running.
    pub(super) fn report_herdr_event(&self, event: &AgentEvent) {
        let Some(reporter) = self.herdr_reporter() else {
            return;
        };
        let state = match event {
            AgentEvent::Thinking { .. }
            | AgentEvent::Reasoning { .. }
            | AgentEvent::Content { .. }
            | AgentEvent::ToolStart { .. }
            | AgentEvent::ToolComplete { .. }
            | AgentEvent::ToolError { .. }
            | AgentEvent::IterationComplete { .. } => AgentState::Working,
            AgentEvent::Done { .. } | AgentEvent::Error { .. } => AgentState::Idle,
            // The turn's boundaries: the session id is minted on the first
            // turn and can change on a retarget, and both happen here.
            AgentEvent::TurnStarted { .. } => {
                self.sync_herdr_session();
                AgentState::Working
            }
            _ => return,
        };
        reporter.report_state(state);
    }

    /// Report that Operant is waiting on the operator to answer a question.
    ///
    /// The message is what Herdr shows next to the `blocked` badge, so it
    /// carries the question itself — truncated, because Herdr renders it in a
    /// narrow column.
    pub(super) fn report_herdr_blocked(&self, question: &str) {
        let Some(reporter) = self.herdr_reporter() else {
            return;
        };
        let summary: String = question.trim().chars().take(120).collect();
        reporter.report_blocked(summary);
    }

    /// Report that the operator answered and the turn carries on.
    pub(super) fn report_herdr_unblocked(&self) {
        let Some(reporter) = self.herdr_reporter() else {
            return;
        };
        reporter.report_state(AgentState::Working);
    }

    /// Announce Operant to Herdr when the TUI starts.
    ///
    /// Without this the pane is invisible to Herdr until the operator's FIRST
    /// turn — which is exactly the gap the operator reported: a Herdr session
    /// full of running Operant panes showed nothing, because nothing had been
    /// said yet. A TUI that is open and waiting for input is an `idle` agent,
    /// and it says so the moment it starts.
    pub(crate) fn announce_herdr_agent(&self) {
        let Some(reporter) = self.herdr_reporter() else {
            return;
        };
        reporter.report_state(AgentState::Idle);
    }
}

/// Give the pane back to Herdr.
///
/// Herdr clears the agent name, the state and the resume command from the
/// pane when the agent releases it. Without this the pane keeps an `operant`
/// badge until Herdr notices the process is gone — a second or two of a
/// stale badge, which is exactly the kind of thing that makes an operator
/// stop trusting the badge.
pub(crate) fn release_herdr_agent() {
    if let Some(reporter) = operant_core::herdr::reporter() {
        reporter.release();
    }
}
