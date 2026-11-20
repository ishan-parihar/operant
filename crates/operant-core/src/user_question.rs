//! User question channel — bridges the `clarify` tool to the TUI.
//!
//! When the agent calls the `clarify` tool, the tool pushes a
//! `UserQuestionRequest` (question + choices + reply oneshot) to the
//! `USER_QUESTION_TX` static. The TUI drains this via its
//! `user_question_rx` field, opens the `ask_user_dialog`, and sends the
//! user's answer back through the oneshot. The clarify tool awaits the
//! oneshot and returns the answer as its tool result.
//!
//! In CLI mode (no TUI), the sender is never set, and the clarify tool
//! falls back to returning the question as a JSON result immediately —
//! the user sees it in the transcript but can't respond interactively.
//!
//! This closes the last remaining audit gap (Bug #2 from iter-82 audit):
//! the drain side was wired in iter-82; this module provides the sender
//! side. (iter-97)

use std::sync::OnceLock;

use tokio::sync::{mpsc, oneshot};

/// A request from the `clarify` tool for the user to answer a question.
/// Contains the question text, optional choices, and a oneshot reply
/// channel. The receiver (TUI) MUST send a reply on `reply_tx` — if it
/// drops the sender, the clarify tool returns an error.
#[derive(Debug)]
pub struct UserQuestionRequest {
    /// The question to ask the user.
    pub question: String,
    /// Optional list of choices for the user to pick from.
    pub choices: Option<Vec<String>>,
    /// (A3-gateway H-3, the third receiver) The session this question belongs
    /// to, when the asking tool knows it.
    ///
    /// The gateway's ask-user receiver routes by ONE process-global
    /// `current_channel` cell, so a question raised inside session A could be
    /// delivered to whichever chat most recently ran a turn -- and the answer
    /// came back to A. The other two receivers were fixed in iter-2528 by
    /// resolving per event; this one could not be, because the request carried
    /// no session and `try_send_user_question` had no way to learn one.
    ///
    /// `None` is every caller with no session to name: the one-shot paths,
    /// the TUI's debug mock, and tests that construct a request directly.
    pub session_id: Option<String>,
    /// (iter-2728, Hydrogen round-2 F2) The transport the ASKING agent was
    /// serving: "tui", "telegram", "discord", ... -- the same
    /// `agent.transport()` the surface channel's `SurfaceRequest` carries.
    /// `None` for a process with no transport concept (the one-shot CLI) and
    /// for tests that construct a request directly.
    ///
    /// A drainer that serves ONE transport (the TUI) ignores it. A drainer
    /// that serves MANY (the gateway) can use it to tell a question raised
    /// inside a telegram turn from one raised inside a discord turn WITHOUT
    /// consulting the process-global `current_channel` cell -- which is the
    /// iter-2528 H-3 failure that motivated threading `session_id` through in
    /// the first place. The cell is still the fallback for a request that
    /// carries no transport (a host that predates this field), so adding it
    /// changes nothing for existing callers.
    pub transport: Option<String>,
    /// Oneshot reply channel — the user's answer flows back here.
    pub reply_tx: oneshot::Sender<String>,
}

/// Global sender for user-question requests. Set by the TUI at startup;
/// read by the clarify tool when it executes. In CLI mode, this stays None
/// and the clarify tool falls back to returning the question as JSON.
static USER_QUESTION_TX: OnceLock<mpsc::UnboundedSender<UserQuestionRequest>> = OnceLock::new();

/// Set the global user-question sender. Called by the TUI's `TuiApp::run`
/// after creating the channel. Returns the receiver so the caller can
/// drain incoming requests.
///
/// Returns `Err(sender)` if the sender was already set (e.g. the TUI was
/// initialized twice). In that case the caller should use the existing
/// channel.
pub fn set_user_question_sender(
    tx: mpsc::UnboundedSender<UserQuestionRequest>,
) -> Result<(), mpsc::UnboundedSender<UserQuestionRequest>> {
    USER_QUESTION_TX.set(tx)
}

/// Try to send a user-question request. Returns `Some(reply_rx)` if the
/// sender is set (TUI mode) — the caller should await `reply_rx` to get
/// the user's answer. Returns `None` if no sender is set (CLI mode) —
/// the caller should fall back to returning the question as a tool result.
pub fn try_send_user_question(
    question: String,
    choices: Option<Vec<String>>,
    session_id: Option<String>,
    transport: Option<String>,
) -> Option<oneshot::Receiver<String>> {
    let (reply_tx, reply_rx) = oneshot::channel();
    let req = UserQuestionRequest {
        question,
        choices,
        session_id,
        transport,
        reply_tx,
    };
    // Try to get the sender. If it's not set (CLI mode), return None.
    match USER_QUESTION_TX.get() {
        Some(tx) => {
            // send returns Err if the receiver was dropped (TUI closed).
            // In that case, treat it like CLI mode.
            if tx.send(req).is_ok() {
                Some(reply_rx)
            } else {
                None
            }
        }
        None => None,
    }
}

/// Check whether a user-question sender is registered (i.e. whether the
/// TUI is running). Used by the clarify tool to decide whether to block
/// (TUI mode) or return immediately (CLI mode).
pub fn has_user_question_sender() -> bool {
    USER_QUESTION_TX.get().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (A3-gateway H-3) The asking session survives the trip to the receiver.
    ///
    /// The gateway's ask-user receiver routes by session, so if the id is lost
    /// between the tool and the channel the receiver falls back to the
    /// process-global cell and the question lands in whichever chat ran a turn
    /// last. This pins the one hop that carries it.
    ///
    /// `USER_QUESTION_TX` is a process-global `OnceLock`, so this test can
    /// only run once per process and must not be made parallel with another
    /// test that sets a sender. It is the only one that does.
    #[test]
    fn the_asking_session_reaches_the_receiver() {
        let (tx, mut rx) = mpsc::unbounded_channel::<UserQuestionRequest>();
        // A second set is refused by design (the TUI initializes once).
        assert!(
            set_user_question_sender(tx).is_ok(),
            "the first sender must be accepted"
        );

        let reply = try_send_user_question(
            "which database?".to_string(),
            Some(vec!["primary".to_string(), "replica".to_string()]),
            Some("gw_abc123".to_string()),
            // (iter-2728, F2) The transport rides the same hop.
            Some("telegram".to_string()),
        );
        assert!(
            reply.is_some(),
            "a registered sender must accept the request"
        );

        let req = rx.try_recv().expect("the request must arrive");
        assert_eq!(
            req.session_id.as_deref(),
            Some("gw_abc123"),
            "the asking session must reach the receiver; losing it is what \\
             routes the question to a chat that never asked"
        );
        assert_eq!(
            req.transport.as_deref(),
            Some("telegram"),
            "the asking transport must reach the receiver; losing it is what \\
             lets a recency-based cell route the question to the wrong platform"
        );
        assert_eq!(req.question, "which database?");
        assert_eq!(
            req.choices.as_deref(),
            Some(&["primary".to_string(), "replica".to_string()][..]),
            "choices must be unchanged by the added field"
        );
    }
}
