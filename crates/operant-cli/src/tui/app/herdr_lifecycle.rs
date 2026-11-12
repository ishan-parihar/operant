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

    /// Claim the CURRENT session for this process (RT-522).
    ///
    /// The presence registry answers "is this session live in another
    /// process", and the claim is what makes "live" mean anything. It used
    /// to happen only at boot — guarded on a session id that did not exist
    /// yet, because the id is minted on the first turn — so no process ever
    /// claimed a session and the guard was inert. It happens at every turn
    /// start now (see `agent_events`), which covers the first turn, `/new`
    /// and every hub switch.
    ///
    /// A REFUSED claim is reported: two processes owning one session is
    /// exactly what the registry exists to surface.
    pub(super) fn claim_current_session(&self) {
        let Some(session_id) = self.herdr_session_id() else {
            return;
        };
        match operant_core::active_sessions::claim_session_here(&session_id) {
            Ok(true) => {}
            Ok(false) => tracing::warn!(
                session_id = %session_id,
                "another live process owns this session — the claim was refused"
            ),
            Err(e) => {
                tracing::debug!(error = %e, "failed to claim the session for this process")
            }
        }
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
        // The turn's boundaries: the session id is minted on the first turn and
        // can change on a retarget, and both happen here.
        if matches!(event, AgentEvent::TurnStarted { .. }) {
            self.sync_herdr_session();
        }
        if let Some(state) = herdr_state_for_event(event) {
            reporter.report_state(state);
        }
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

/// The Herdr lifecycle state an agent event maps to, if any.
///
/// A pure function on purpose: this mapping is the whole contract between
/// what Operant is doing and what Herdr shows, and it has to be provable
/// without a model, a pane, or a Herdr server. An event that says nothing
/// about the turn's progress (a token count, a cost, a rate-limit notice)
/// maps to `None` — no report, so the state Herdr shows stays the last one
/// that was true.
fn herdr_state_for_event(event: &AgentEvent) -> Option<AgentState> {
    match event {
        // The agent produced something: it is working.
        AgentEvent::Thinking { .. }
        | AgentEvent::Reasoning { .. }
        | AgentEvent::Content { .. }
        | AgentEvent::ToolStart { .. }
        | AgentEvent::ToolComplete { .. }
        | AgentEvent::ToolError { .. }
        | AgentEvent::IterationComplete { .. }
        | AgentEvent::TurnStarted { .. } => Some(AgentState::Working),
        // The turn ended: a final answer, or an error. Either way Operant is
        // ready for the next prompt.
        AgentEvent::Done { .. } | AgentEvent::Error { .. } => Some(AgentState::Idle),
        _ => None,
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

#[cfg(test)]
mod tests {
    use super::*;
    use operant_core::agent::AgentEvent;

    /// The events that mean "the agent is doing something".
    #[test]
    fn production_events_mean_working() {
        for event in [
            AgentEvent::Thinking {
                content: String::new(),
            },
            AgentEvent::Reasoning {
                text: String::new(),
            },
            AgentEvent::Content {
                text: String::new(),
            },
            AgentEvent::ToolStart {
                tool_call_id: "t1".to_string(),
                name: "read".to_string(),
                arguments: "{}".to_string(),
            },
            AgentEvent::ToolComplete {
                result: operant_core::tools::ToolResult::success("t1", "ok"),
            },
            AgentEvent::ToolError {
                tool_call_id: "t1".to_string(),
                name: "read".to_string(),
                error: "boom".to_string(),
            },
            AgentEvent::IterationComplete { iteration: 1 },
            AgentEvent::TurnStarted { turn_id: 1 },
        ] {
            assert_eq!(
                herdr_state_for_event(&event),
                Some(AgentState::Working),
                "{event:?} must mean working"
            );
        }
    }

    /// The events that end the turn.
    #[test]
    fn the_end_of_a_turn_means_idle() {
        // A finished tool is NOT the end of the turn — the model usually runs
        // another iteration after it — so the completion event is `working`
        // while only `Done` and `Error` are `idle`.
        assert_eq!(
            herdr_state_for_event(&AgentEvent::IterationComplete { iteration: 2 }),
            Some(AgentState::Working)
        );

        let done = AgentEvent::Done {
            message: operant_core::client::Message::assistant("done"),
            reason: operant_core::agent::TurnExitReason::TextResponse,
        };
        assert_eq!(herdr_state_for_event(&done), Some(AgentState::Idle));

        let error = AgentEvent::Error {
            error: "the provider is unreachable".to_string(),
        };
        assert_eq!(herdr_state_for_event(&error), Some(AgentState::Idle));
    }

    /// An event that says nothing about progress must not change the state.
    #[test]
    fn bookkeeping_events_report_nothing() {
        assert_eq!(
            herdr_state_for_event(&AgentEvent::Usage {
                input_tokens: 1,
                output_tokens: 2,
                total_tokens: 3,
            }),
            None
        );
        assert_eq!(
            herdr_state_for_event(&AgentEvent::RateLimitNotice {
                retry_after_secs: Some(30)
            }),
            None
        );
    }
}
