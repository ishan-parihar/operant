// app/event_coalesce.rs — apply one terminal event, and drain the burst behind it.
//
// Operant used to handle exactly one `crossterm` event per loop iteration, and
// every iteration also paints a full frame. Typing faster than the frame rate
// therefore queued events, and each one cost handle + full draw serially — the
// input line visibly lags behind the keyboard. This module holds both halves of
// the fix: the per-event body, extracted so it can run more than once per frame
// ([`App::apply_terminal_event`]), and the cap on how many extra events one
// frame may absorb ([`MAX_DRAINED_EVENTS_PER_WAKE`]).
//
// Coalescing changes WHEN state is painted, never WHAT is painted: the drained
// events are handled in arrival order by the same code, and the frame that
// follows them renders the result. A burst of N keystrokes goes from N frames
// to one. Nothing here can change the final rendered state for a given input
// sequence, which is why it is safe to add while the surface rebuild is in
// flight.

use super::*;

/// How many *additional* already-buffered events one frame may absorb, on top
/// of the one it woke for.
///
/// 32 matches jcode's `MAX_DRAINED_EVENTS_PER_WAKE` (`jcode-tui/src/tui/app/
/// local.rs`), and the cap is load-bearing in both places. Draining without one
/// is unbounded work in a single frame: a paste arrives as hundreds of raw
/// character events, and a repaint storm from a slow remote peer can queue
/// thousands of mouse motions. Either way a frame that tries to absorb all of
/// them stops servicing input entirely, which is the lag the drain was added to
/// fix. 32 is comfortably above one screenful of burst (a 40-column prompt line
/// redrawn 32 times over is still under a second of typing) and comfortably
/// below the point where one frame's worth of handling becomes the lag.
pub(super) const MAX_DRAINED_EVENTS_PER_WAKE: usize = 32;

/// What [`App::apply_terminal_event`] decided the loop should do next.
pub enum EventOutcome {
    /// Handled. Keep draining, or fall through to the next frame.
    Consumed,
    /// The user submitted a non-empty prompt; the loop returns it.
    Submit(String),
    /// The user quit. The loop tears down and returns `None`.
    Exit,
}

impl App {
    /// Handle exactly one terminal event and say what the loop should do.
    ///
    /// Split out of `App::run` so the run loop can apply a burst through the
    /// same path it always used. The body is unchanged from the single-event
    /// version; only its exit route moved from `continue` / `return` to
    /// [`EventOutcome`], which is what lets the caller run it N times.
    pub(super) fn apply_terminal_event(&mut self, event: Event) -> EventOutcome {
        match event {
            Event::Key(key) => {
                // On Windows crossterm fires both Press and Release events;
                // only the Press carries meaning here.
                if key.kind != crossterm::event::KeyEventKind::Press {
                    return EventOutcome::Consumed;
                }

                // ---- Paste-burst detection -----------------------------------------
                // On Windows Terminal, Ctrl+V causes the terminal to write clipboard
                // content as raw character events (not as Event::Paste).  Every `\n`
                // fires as Enter (submitting the prompt) and stray `v` chars trigger
                // voice PTT.  We detect this by draining the event queue with a
                // zero-timeout immediately after the first character arrives — a paste
                // dumps every character at once while normal typing rarely queues more
                // than one char in the same 50 ms window.
                if (key.modifiers == KeyModifiers::NONE || key.modifiers == KeyModifiers::SHIFT)
                    && let KeyCode::Char(c) = key.code
                {
                    if self.prompt_is_accepting_text() {
                        if let Some(burst) = self.try_detect_paste_burst(c) {
                            self.handle_paste_data(burst);
                            self.refresh_prompt_input();
                            return EventOutcome::Consumed;
                        }
                    } else if self.key_input_dialog.visible
                        && let Some(burst) = self.try_detect_paste_burst(c)
                    {
                        for ch in burst.chars() {
                            self.key_input_dialog.insert_char(ch);
                        }
                        return EventOutcome::Consumed;
                    }
                }
                // -------------------------------------------------------------------

                let should_submit = self.handle_key_event(key);
                // Honour `:q`/`:wq` from vim command-line mode
                if self.prompt_input.vim_quit_requested {
                    self.prompt_input.vim_quit_requested = false;
                    self.should_exit = true;
                }
                // Exit outranks submit: a key that both quits and submits quits,
                // which is the order the single-event loop evaluated them in.
                if self.should_exit {
                    return EventOutcome::Exit;
                }
                if should_submit {
                    self.dismiss_error_notifications();
                    let input = self.take_input();
                    if !input.is_empty() {
                        self.drop_pending_images_with_notice();
                        return EventOutcome::Submit(input);
                    }
                }
                EventOutcome::Consumed
            }
            Event::Paste(data)
                if !self.is_streaming
                    && self.permission_request.is_none()
                    && !self.history_search_overlay.visible =>
            {
                if self.key_input_dialog.visible {
                    for ch in data.chars() {
                        self.key_input_dialog.insert_char(ch);
                    }
                } else {
                    self.handle_paste_data(data);
                    self.refresh_prompt_input();
                }
                EventOutcome::Consumed
            }
            Event::Mouse(mouse_event) => {
                self.handle_mouse_event(mouse_event);
                EventOutcome::Consumed
            }
            Event::FocusGained => {
                self.handle_focus_event(true);
                EventOutcome::Consumed
            }
            Event::FocusLost => {
                self.handle_focus_event(false);
                EventOutcome::Consumed
            }
            // The one arm the match used to drop on the floor. A resize
            // rewraps every transcript line, so it has to reach the
            // reading-position state before the next paint reflows it —
            // otherwise a reader parked mid-transcript is teleported to the
            // new bottom. `note_resize` is a no-op for a reader who is
            // following the tail, so following stays following.
            Event::Resize(_, _) => {
                self.note_resize();
                EventOutcome::Consumed
            }
            _ => EventOutcome::Consumed,
        }
    }
}
