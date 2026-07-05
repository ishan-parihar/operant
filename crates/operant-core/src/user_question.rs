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
) -> Option<oneshot::Receiver<String>> {
    let (reply_tx, reply_rx) = oneshot::channel();
    let req = UserQuestionRequest {
        question,
        choices,
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
