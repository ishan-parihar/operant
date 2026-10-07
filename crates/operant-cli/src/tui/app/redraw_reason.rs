// app/redraw_reason.rs — why the next frame is not redundant.
//
// `App::run` draws unconditionally: it has no `needs_redraw` flag, so "should
// this frame have been skipped" is not a question the loop can answer about
// itself. What it CAN answer is "what is live right now that a skipped frame
// would have thrown away". That is what [`App::redraw_reason`] returns, and it
// returns a NAME rather than a bool for the reason jcode's
// `live_activity_redraw_reason` returns one: given a slow frame, "is_streaming"
// as a bool sends you into `App` to find out what `is_streaming` even gates,
// whereas `"streaming"` sends you straight to the renderer's footer path.
//
// This is deliberately a PURE predicate over `App` state. The natural place to
// record the answer is the debug hub's frame bookkeeping, so the F12 overlay can
// show it beside the render cost it explains — the run loop samples it once per
// frame, just before `terminal.draw`, and hands it to
// `TuiDebugHub::record_frame`, which stores it in its own slot and rides it on
// the `FrameRendered` event. The loop still draws unconditionally, so this is
// diagnostic rather than a gate: "why was this frame painted?" is now a
// question with an answer, and the answer is on screen.
//
// Precedence is first-match-wins and deliberate: the cases that move the most
// cells per frame come first, so the reported name is the one that explains the
// cost, not merely the first `if` that happened to be true.

use super::*;

/// Every reason [`App::redraw_reason`] can return.
///
/// Reasons are names, not messages — they are stable identifiers meant to be
/// grepped, matched against render paths, and asserted in tests. A rename here
/// is a behaviour change to the diagnostic surface.
///
/// The `debug_assert!` in [`App::redraw_reason`] checks every answer against
/// this table, and the tests below walk it both ways. It exists so the reason
/// names are a checked table rather than free-floating string literals at each
/// branch.
const REDRAW_REASONS: &[&str] = &[
    // The turn is in flight: the transcript grows and the spinner glyph /
    // shimmer sweep are `frame_count`-driven (`render/utils.rs`), so every
    // frame differs from the last whether or not a token arrived.
    "streaming",
    // The agent bridge has an open receiver. Deltas are posted asynchronously;
    // without a tick the next one would not appear until a keystroke.
    "agent_events_pending",
    // The turn's completion oneshot is armed. `is_streaming` clears and the
    // footer flips to idle on this channel, not on input.
    "run_complete_pending",
    // The agent is blocked on a tool-permission answer. The dialog has to
    // appear on the next tick or the turn looks hung.
    "permission_pending",
    // The clarify tool is blocked on an answer to a question.
    "user_question_pending",
    // A background recording/transcription task posts state and prompt text
    // asynchronously.
    "notification_ttl",
    // The footer renders `turn_started_at.elapsed()`, so the displayed seconds
    // advance with no input at all.
    "turn_timer",
    // `spinner_color` flips to the error colour once `stall_start` is older
    // than 3s — a wall-clock transition with nothing to trigger it.
    "stall_spinner",
    // A status line message is showing.
    "status_message",
    // The `/mcp r` reconnect posts its completion text through a channel the
    // loop drains every frame (iter-326).
    "mcp_reconnect_pending",
    // A model-list / session-list / session-load fetch is in flight; its
    // result replaces a picker or the transcript.
    "background_fetch_pending",
    // The reader is parked above the tail with unread messages, so the "N new
    // messages" affordance is on screen and its count moves as they land.
    "new_messages_while_scrolled",
];

impl App {
    /// The single live reason a full frame is worth drawing right now, or
    /// `None` when the next frame would paint an identical buffer.
    ///
    /// Not every frame in operant's loop is worth its cost: on a fully idle
    /// session with nothing pending, the buffer is byte-identical to the one
    /// already on screen. This names the state that would make a skipped frame
    /// wrong.
    ///
    /// **Resize is deliberately absent.** Operant stores no terminal size, so a
    /// resize is not expressible as `App` state — `Event::Resize` falls through
    /// the loop's catch-all arm and the size change is picked up by the next
    /// draw from the terminal itself. Adding a size field to answer it would be
    /// state that exists only to be compared with itself.
    // Sampled once per frame by `App::run`, immediately before
    // `terminal.draw`, and passed to `TuiDebugHub::record_frame`. The F12
    // overlay prints it as `Redraw:`; the event log carries it on every
    // `FrameRendered`. It does not gate the draw — operant's loop paints every
    // iteration and inventing a skip here would need a state field that exists
    // only to be compared with itself.
    pub(crate) fn redraw_reason(&self) -> Option<&'static str> {
        let reason = if self.is_streaming {
            "streaming"
        } else if self.agent_event_rx.is_some() {
            "agent_events_pending"
        } else if self.run_complete_rx.is_some() {
            "run_complete_pending"
        } else if self.permission_rx.is_some() {
            "permission_pending"
        } else if self.user_question_rx.is_some() {
            "user_question_pending"
        } else if !self.notifications.is_empty() {
            "notification_ttl"
        } else if self.turn_started_at.is_some() {
            "turn_timer"
        } else if self.stall_start.is_some() {
            "stall_spinner"
        } else if self.status_message.is_some() {
            "status_message"
        } else if self.mcp_reconnect_rx.is_some() {
            "mcp_reconnect_pending"
        } else if self.background_fetch_pending() {
            "background_fetch_pending"
        } else if !self.auto_scroll && self.new_messages_while_scrolled > 0 {
            "new_messages_while_scrolled"
        } else {
            return None;
        };
        debug_assert!(
            REDRAW_REASONS.contains(&reason),
            "{reason} is returned by redraw_reason but missing from REDRAW_REASONS, so the \
             documented reason table and the code have drifted apart"
        );
        Some(reason)
    }

    /// Whether any of the fire-and-forget background fetches is in flight —
    /// the armed receiver OR the flag set before its task was spawned.
    ///
    /// Both halves matter: the pending flags are set by the key handler and the
    /// spawn happens later in the same frame, so a fetch that is about to be
    /// spawned is exactly as live as one already running.
    fn background_fetch_pending(&self) -> bool {
        self.model_fetch_rx.is_some()
            || self.model_picker_fetch_pending
            || self.session_list_rx.is_some()
            || self.session_list_pending
            || self.session_load_rx.is_some()
            || self.session_load_pending.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::app::tests::make_app;

    /// A reason name is only useful if the documented table and the code agree.
    /// This walks every string the predicate can return by construction: each
    /// branch is toggled by its own state, and the reported name is asserted to
    /// be in the table. A rename that misses the table fails here.
    #[test]
    fn every_reason_the_predicate_can_return_is_in_the_reason_table() {
        let cases: Vec<(&str, Box<dyn Fn(&mut App)>)> = vec![
            ("streaming", Box::new(|a: &mut App| a.is_streaming = true)),
            (
                "agent_events_pending",
                Box::new(|a: &mut App| {
                    a.agent_event_rx = Some(tokio::sync::mpsc::channel(1).1);
                }),
            ),
            (
                "run_complete_pending",
                Box::new(|a: &mut App| {
                    let (_tx, rx) = tokio::sync::oneshot::channel();
                    a.run_complete_rx = Some(rx);
                }),
            ),
            (
                "permission_pending",
                Box::new(|a: &mut App| {
                    a.permission_rx = Some(tokio::sync::mpsc::channel(1).1);
                }),
            ),
            (
                "user_question_pending",
                Box::new(|a: &mut App| {
                    a.user_question_rx = Some(tokio::sync::mpsc::unbounded_channel().1);
                }),
            ),
            (
                "notification_ttl",
                Box::new(|a: &mut App| {
                    a.push_notification(NotificationKind::Info, "hi".to_string(), Some(5));
                }),
            ),
            (
                "turn_timer",
                Box::new(|a: &mut App| {
                    a.turn_started_at = Some(std::time::Instant::now());
                }),
            ),
            (
                "stall_spinner",
                Box::new(|a: &mut App| {
                    a.stall_start = Some(std::time::Instant::now());
                }),
            ),
            (
                "status_message",
                Box::new(|a: &mut App| {
                    a.status_message = Some("working".to_string());
                }),
            ),
            (
                "mcp_reconnect_pending",
                Box::new(|a: &mut App| {
                    a.mcp_reconnect_rx = Some(tokio::sync::mpsc::unbounded_channel().1);
                }),
            ),
            (
                "background_fetch_pending",
                Box::new(|a: &mut App| {
                    a.model_picker_fetch_pending = true;
                }),
            ),
            (
                "new_messages_while_scrolled",
                Box::new(|a: &mut App| {
                    a.auto_scroll = false;
                    a.new_messages_while_scrolled = 3;
                }),
            ),
        ];

        // The table and the toggle list must not drift apart either.
        assert_eq!(
            REDRAW_REASONS.len(),
            cases.len(),
            "REDRAW_REASONS has {} entries but the toggle list below has {} — a new reason was \
             added to one and not the other",
            REDRAW_REASONS.len(),
            cases.len()
        );

        for (expected, arm) in cases {
            let mut app = make_app();
            assert_eq!(
                app.redraw_reason(),
                None,
                "precondition for `{expected}`: a fresh App must report no reason, or the \
                 assertion below proves nothing"
            );
            arm(&mut app);
            let got = app.redraw_reason();
            assert_eq!(
                got,
                Some(expected),
                "arming only `{expected}` reported {got:?}"
            );
            assert!(
                REDRAW_REASONS.contains(&got.unwrap_or_default()),
                "{expected} is reported but missing from REDRAW_REASONS"
            );
        }
    }

    /// The reason table is a public diagnostic surface: a stale entry is a
    /// reason name nothing can ever report, which is worse than no table.
    #[test]
    fn the_reason_table_has_no_stale_entries() {
        for reason in REDRAW_REASONS {
            assert!(!reason.is_empty(), "REDRAW_REASONS carries an empty name");
            assert!(
                !reason.contains(' '),
                "`{reason}` reads like a sentence; reasons are stable identifiers"
            );
        }
    }

    /// Precedence is the point of returning one name: with two live reasons the
    /// more expensive-to-draw state must win, so the name explains the frame.
    #[test]
    fn the_first_live_reason_wins() {
        let mut app = make_app();
        // Nothing streaming, but a background channel is armed.
        app.agent_event_rx = Some(tokio::sync::mpsc::channel(1).1);
        assert_eq!(app.redraw_reason(), Some("agent_events_pending"));

        // Streaming outranks every armed channel: the transcript is moving.
        app.is_streaming = true;
        assert_eq!(app.redraw_reason(), Some("streaming"));
    }

    /// A fully idle session with nothing pending is the `None` case: the next
    /// frame would paint the buffer already on screen.
    #[test]
    fn an_idle_app_needs_no_frame() {
        let app = make_app();
        assert_eq!(app.redraw_reason(), None);
    }

    /// The predicate is only worth having if its answer survives the trip to
    /// where a human reads it. `App::run` hands `redraw_reason()` to
    /// `TuiDebugHub::record_frame`, which stores it in a slot the F12 overlay
    /// reads and rides it on the `FrameRendered` event; this walks that exact
    /// path. Re-adding the `allow(dead_code)`, or dropping the call from the
    /// draw site, leaves this green — so it is a wiring pin, not a shape pin.
    /// The shape is pinned by `TuiDebugHub`'s own test.
    #[test]
    fn the_reason_reaches_the_debug_hub_the_overlay_reads() {
        use crate::tui::debug::TuiEvent;

        let mut app = make_app();
        // The bus is off by default (`OPERANT_TUI_DEBUG`); the headless
        // simulator turns it on the same way before it starts drawing.
        app.debug_hub.event_bus().set_enabled(true);
        // Stream, so the reason is a name rather than the idle `None`.
        app.is_streaming = true;
        let reason = app.redraw_reason();
        assert_eq!(reason, Some("streaming"));
        app.debug_hub.record_frame(2.0, reason);
        assert_eq!(app.debug_hub.last_redraw_reason(), Some("streaming"));

        let events = app.debug_hub.event_bus().recent(1);
        assert!(
            events.iter().any(|e| matches!(
                e,
                TuiEvent::FrameRendered {
                    reason: Some("streaming"),
                    ..
                }
            )),
            "the FrameRendered event must carry the reason, not just the hub slot"
        );

        // And the idle case is reported rather than silently dropped.
        app.is_streaming = false;
        let reason = app.redraw_reason();
        assert_eq!(reason, None);
        app.debug_hub.record_frame(1.0, reason);
        assert_eq!(app.debug_hub.last_redraw_reason(), None);
        let events = app.debug_hub.event_bus().recent(1);
        assert!(
            events
                .iter()
                .any(|e| matches!(e, TuiEvent::FrameRendered { reason: None, .. })),
            "an idle frame must still record the fact that it was idle"
        );
    }
}
