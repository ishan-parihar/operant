// session_switch.rs — the hub's Enter → session-switch handle (iter-1503).
//
// WHY THIS MODULE EXISTS
//
// The operator reported "session management is broken": pick a session in the
// hub, press Enter, and the screen goes BLANK and stays that way until they
// press some unrelated key. Reproduced headless: `/session<enter><enter>`
// dumps a screen with 11 non-blank rows out of 45 — the hub closed, the
// transcript was never rebuilt.
//
// The cause is structural and the same shape as the rewind selector's
// (iter-1489): `pending_switch_session` is drained by
// `run_pending_agent_actions`, which lives on the OUTER side of `App::run`'s
// inner loop. `TuiApp::run`'s outer iteration only resumes when the inner loop
// RETURNS — which happens on a prompt submit or a quit. Enter in the hub does
// neither: it closes the overlay and hands the keyboard back to the composer
// INSIDE the inner loop. So the switch sat armed and unapplied for as long as
// the operator sat there, and the display showed neither the old session's
// transcript nor the new one.
//
// This handle is the frame-loop-side of the switch, mirroring
// `tui::rewind_marks`: the App holds an `Arc` to a read-mostly source, and the
// frame loop applies the pending retarget with no await. `set_session_id` and
// `notify_session_switch` are both `&self` sync methods on the agent, and the
// conversation is a `tokio::sync::RwLock` whose `try_read` gives a snapshot
// without blocking — so the entire retarget is expressible without an await,
// which is what a render-path frame can do.
//
// WHAT DELIBERATELY STAYS ON THE OUTER LOOP
//
// The async drain still owns the parts that must not race the streaming agent:
// retargeting while a turn is in flight, and the `is_streaming` guard around
// /clear /new /undo. The switch only runs while `!is_streaming`, and if a turn
// starts between the Enter and the frame that applies it, the flag is simply
// not cleared here — the outer drain finds it still armed and applies it there.
// The two paths are therefore compatible rather than competing: whichever
// reaches the flag first does the work, and `Option::take` means only one does.

use operant_core::client::Message;

/// A read-only, clonable handle over the pieces of a session switch the frame
/// loop needs: the agent's retarget methods and a non-blocking snapshot of its
/// conversation.
///
/// Read-only by construction (it exposes no mutating method), exactly like
/// [`crate::tui::rewind_marks::RewindMarksSource`]: the App is not supposed to
/// hold the agent, and a render path must not be able to reach a mutation.
pub struct SessionSwitchSource {
    /// Retarget the agent at `session_id`, rehydrating that session's
    /// transcript and reseeding the rewind marks for it.
    switch: std::sync::Arc<dyn Fn(&str) + Send + Sync>,
    /// Snapshot of the agent's current conversation (the hot slot).
    conversation: std::sync::Arc<dyn Fn() -> Vec<Message> + Send + Sync>,
}

impl SessionSwitchSource {
    /// Assemble the source from two closures. The CLI builds these once at
    /// agent creation, capturing the `Arc<OperantAgent>` it already owns.
    pub fn new(
        switch: impl Fn(&str) + Send + Sync + 'static,
        conversation: impl Fn() -> Vec<Message> + Send + Sync + 'static,
    ) -> Self {
        Self {
            switch: std::sync::Arc::new(switch),
            conversation: std::sync::Arc::new(conversation),
        }
    }

    /// Retarget the agent at `session_id` and return the conversation that
    /// session holds, so the caller can rebuild the display from the same
    /// source the agent holds.
    pub fn switch_to(&self, session_id: &str) -> Vec<Message> {
        (self.switch)(session_id);
        (self.conversation)()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The handle calls through to the agent exactly once per switch and hands
    /// back that session's conversation — the caller needs both halves.
    #[test]
    fn switch_calls_through_once_and_returns_the_new_conversation() {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let seen_for_closure = std::sync::Arc::clone(&seen);
        let src = SessionSwitchSource::new(
            move |id| {
                seen_for_closure
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(id.to_string());
            },
            || vec![operant_core::client::Message::user("only message")],
        );

        let conv = src.switch_to("sess-b");

        assert_eq!(
            seen.lock().unwrap_or_else(|e| e.into_inner()).as_slice(),
            &["sess-b".to_string()],
            "the retarget happened exactly once"
        );
        assert_eq!(conv.len(), 1, "the new session's conversation came back");
    }

    /// Two switches in a row are independent: the handle keeps no state of its
    /// own, so a host that resumes two different sessions in one session of
    /// use gets the right conversation both times.
    #[test]
    fn consecutive_switches_are_independent() {
        // `Fn` closures cannot mutate a captured `usize` directly, so the count
        // rides an `AtomicUsize` — which is also `Send`, as the handle requires.
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = std::sync::Arc::clone(&calls);
        let src = SessionSwitchSource::new(
            move |_| {
                counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            },
            // The type cannot be inferred from a bare `Vec::new` here (the
            // handle is a type-erased `dyn Fn() -> Vec<Message>`), so the
            // turbofish names it — a closure would only restate the return
            // type clippy already warns about.
            || -> Vec<Message> { Vec::new() },
        );
        src.switch_to("a");
        src.switch_to("b");
        src.switch_to("c");
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::Relaxed),
            3,
            "each switch is applied, none coalesced away"
        );
    }
}
