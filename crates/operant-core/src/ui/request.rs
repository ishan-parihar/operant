//! Surface request channel — bridges the `ui_request_input` tool to a host UI.
//!
//! Same shape as [`crate::user_question`] (the `clarify` tool's channel), and
//! deliberately so: the plan's M3 rules that the HITL seam is clarify's and
//! that a second parallel mechanism must not be invented. The two channels
//! differ only in what they carry. A `UserQuestionRequest` carries a question
//! and its answer comes back as a plain string; a `SurfaceRequest` carries a
//! widget tree (a `choice_picker`/`confirm`/`text_field` card) and its answer
//! comes back as STRUCTURED JSON — `{ "kind": "choice", "choice_id": ... }` —
//! because the surfaces this plan grows (approval with an optional comment, a
//! form, an enum-keyed choice) cannot be expressed by "the string the user
//! typed" alone.
//!
//! `None` means no host UI is attached (the one-shot CLI path): the request is
//! not sent, and the caller answers itself — for `ui_request_input` that means
//! the model is told, in the result, that there is no interactive surface.

use std::sync::OnceLock;

use tokio::sync::{mpsc, oneshot};

use crate::ui::SurfaceOp;

/// One interactive surface the model asked for an answer to.
///
/// The card the host renders is `body` — the widget the op carries — while
/// `question` is a one-line header for surfaces that can show one (a chat
/// message, a dialog title). Both are already gate-scrubbed: the caller runs
/// [`crate::ui::gate::validate_surface`] before sending, so a secret never
/// reaches a host UI through this channel.
#[derive(Debug)]
pub struct SurfaceRequest {
    /// The surface this request belongs to (its `surface_id`).
    pub surface_id: String,
    /// Which session asked, so a multi-chat host (the gateway) resolves the
    /// card to the chat that raised it rather than to whichever chat ran a
    /// turn last. `None` in a process with no session concept (the one-shot
    /// CLI) and in the TUI, which is a single session by construction.
    ///
    /// Read exactly the way `user_question::UserQuestionRequest::session_id`
    /// is read: the iter-2528 H-3 fix threaded it through `clarify` after a
    /// question raised in session A was delivered to the wrong chat, leaving
    /// A's turn blocked on an answer nobody was shown.
    pub session_id: Option<String>,
    /// One-line header: what the model is asking for.
    pub question: String,
    /// The widget the host should render (a `choice_picker`/`confirm`/
    /// `text_field`/`button`).
    pub body: crate::ui::WidgetNode,
    /// The op this request came from, for a host that re-renders the whole
    /// surface around the question.
    pub op: SurfaceOp,
    /// Where the answer goes. Dropping the sender without sending is the
    /// host's "nobody answered" — the tool reports a dismissed prompt.
    pub reply_tx: oneshot::Sender<serde_json::Value>,
}

/// Global sender for surface requests. Set by a host UI at startup; read by
/// the tool when it executes. In the one-shot CLI mode it stays `None`.
static SURFACE_REQUEST_TX: OnceLock<mpsc::UnboundedSender<SurfaceRequest>> = OnceLock::new();

/// Register the host UI's receiver. Returns the receiver the caller drains,
/// or `Err(sender)` when a host was already registered (the same contract as
/// [`crate::user_question::set_user_question_sender`]).
pub fn set_surface_request_sender(
    tx: mpsc::UnboundedSender<SurfaceRequest>,
) -> Result<(), mpsc::UnboundedSender<SurfaceRequest>> {
    SURFACE_REQUEST_TX.set(tx)
}

/// Send a surface request. `Some(reply_rx)` when a host is attached — await
/// it for the user's answer. `None` when no host is registered, or when the
/// host's receiver is gone (the UI closed): the caller answers itself.
pub fn try_send_surface_request(
    request: SurfaceRequest,
) -> Option<oneshot::Receiver<serde_json::Value>> {
    try_send_surface_request_inner(request).ok()
}

/// The same send, with the reason a delivery failed.
///
/// A caller that must DISTINGUISH "no host, answer it yourself" (the one-shot
/// CLI mode, which is a supported deployment) from "a host exists but its
/// receiver is gone" (a bug in that host) needs the cause; the plain
/// [`try_send_surface_request`] deliberately collapses both to `None` because
/// the tool's behaviour is identical either way.
fn try_send_surface_request_inner(
    request: SurfaceRequest,
) -> Result<oneshot::Receiver<serde_json::Value>, SurfaceRequestFailure> {
    let (reply_tx, reply_rx) = oneshot::channel();
    let request = SurfaceRequest {
        reply_tx,
        ..request
    };
    match SURFACE_REQUEST_TX.get() {
        None => Err(SurfaceRequestFailure::NoHost),
        Some(tx) => tx
            .send(request)
            .map(|()| reply_rx)
            .map_err(|_| SurfaceRequestFailure::HostGone),
    }
}

/// Why a surface request could not be delivered. Test- and diagnostic-only:
/// the tool treats every variant as "answer it yourself".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SurfaceRequestFailure {
    /// No host ever registered — the one-shot CLI mode.
    NoHost,
    /// A host registered, but its receiver has been dropped.
    HostGone,
}

/// Whether a host UI can answer surface requests.
pub fn has_surface_request_sender() -> bool {
    SURFACE_REQUEST_TX.get().is_some()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::ui::{Provenance, WidgetNode};

    /// Serializes the tests that touch the process-global
    /// `SURFACE_REQUEST_TX` `OnceLock`.
    ///
    /// The lock is set-once per process, so without this two tests race: one
    /// registers a host, the other registers a SECOND host, and each then
    /// blocks on a receiver the other owns — which is what
    /// `a_registered_host_receives_the_request_and_the_answer_returns` did
    /// when it ran beside the `ui_tool` request test (measured: it read
    /// `s1` where the tool had written `s2`). The `user_question` tests are
    /// safe only because they happen to be few; this makes the rule explicit.
    ///
    /// `pub(crate)` because `tools/ui_tool.rs` has a test that registers a host
    /// too, and the two modules are separate test binaries only in separate
    /// crates — here they share one process, so they must share one lock.
    pub(crate) static SURFACE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Held for the whole test body.
    struct SurfaceTestLock {
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl SurfaceTestLock {
        fn acquire() -> Self {
            // A poisoned lock means another surface test panicked; the flag
            // state is still well-defined, so recover rather than cascade.
            Self {
                _guard: SURFACE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner()),
            }
        }
    }

    fn choice_request() -> SurfaceRequest {
        let body = WidgetNode::ChoicePicker {
            question: "Pick one".to_string(),
            choices: vec![
                crate::ui::ChoiceOption {
                    id: "a".to_string(),
                    label: "A".to_string(),
                    description: None,
                },
                crate::ui::ChoiceOption {
                    id: "b".to_string(),
                    label: "B".to_string(),
                    description: None,
                },
            ],
        };
        let op = SurfaceOp::CreateSurface {
            surface_id: "s1".to_string(),
            intent: "choose".to_string(),
            provenance: Provenance::AgentSynthesis,
            body: body.clone(),
        };
        SurfaceRequest {
            surface_id: "s1".to_string(),
            session_id: Some("gw_test".to_string()),
            question: "Pick one".to_string(),
            body,
            op,
            reply_tx: oneshot::channel().0,
        }
    }

    /// A registered host receives the request, and the answer flows back.
    ///
    /// `SURFACE_REQUEST_TX` is a process-global `OnceLock` that can be set only
    /// ONCE per process, so exactly one test in the binary may register a host.
    /// That is this one. The sibling tests assert the degraded path, which is
    /// what they see once this test has run — and because test order is not
    /// guaranteed, they accept EITHER outcome and assert the contract that is
    /// true of both: a call never blocks when nobody can answer.
    #[tokio::test]
    async fn a_registered_host_receives_the_request_and_the_answer_returns() {
        let _lock = SurfaceTestLock::acquire();
        let (tx, mut rx) = mpsc::unbounded_channel();
        // A second registration is refused by design (the TUI initializes
        // once), so if a host is already attached this test has nothing left
        // to prove and must not fail the binary.
        if set_surface_request_sender(tx).is_err() {
            return;
        }

        let reply = try_send_surface_request(choice_request()).expect("a host is attached");
        let received = rx.recv().await.expect("the host drains the request");
        assert_eq!(received.surface_id, "s1");
        assert_eq!(received.question, "Pick one");
        assert!(matches!(received.body, WidgetNode::ChoicePicker { .. }));

        received
            .reply_tx
            .send(serde_json::json!({ "kind": "choice", "choice_id": "b" }))
            .expect("the reply reaches the tool");
        let answer = reply.await.expect("the tool receives the answer");
        assert_eq!(answer["choice_id"], "b");
    }

    /// No host (the one-shot CLI mode): the caller gets `None` and answers
    /// itself rather than blocking on a channel nobody reads.
    ///
    /// This is the branch the tool actually takes in CLI mode. It is also the
    /// branch a process sees once the sibling test has registered its host AND
    /// dropped the receiver, so both orderings are checked below.
    #[tokio::test]
    async fn a_hostless_call_degrades_instead_of_blocking() {
        let _lock = SurfaceTestLock::acquire();
        match try_send_surface_request_inner(choice_request()) {
            // No host ever registered: the one-shot CLI mode.
            Err(SurfaceRequestFailure::NoHost) => {}
            // A host is attached: a dropped receiver resolves the oneshot as
            // an error, never a hang.
            Ok(rx) => {
                let _ = rx.await;
            }
            Err(SurfaceRequestFailure::HostGone) => {}
        }
    }
}
