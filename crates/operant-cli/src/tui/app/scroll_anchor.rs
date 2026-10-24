// app/scroll_anchor.rs — Transcript scroll state.
//
// ONE convention, measured from the renderer (ui_viewport.rs):
// `scroll_offset` is the viewport's TOP ROW. `0` is the first (oldest) line,
// `last_max_scroll()` — content height minus viewport — is the tail, and a
// tail-following frame paints exactly that max. The handlers below and the
// reconcile all speak this language; the 2026-10-24 red-team pass replaced an
// older "0 = tail, += = up" reading (iter-1213's block) that contradicted the
// paint and made the first user scroll from the tail teleport the view to row
// 0 — the top of the transcript.
//
//   * Bookmark (Ctrl+G) — an explicit spot they choose and can come back to.
//   * Resize anchor — a resize rewraps every line, so a row index cannot
//     survive it (row 12 at width 120 is not row 12 at width 60). The capture
//     names CONTENT (`operant_model::ContentPos`, hash-anchored) instead, and
//     the render path resolves it against the next frame's geometry.
//   * Tail sync — while auto-follow paints the tail, the reconcile writes that
//     same row back into `scroll_offset`, so the next user scroll starts from
//     where the reader actually is, not from wherever the stale state points.
//
// The state lives on `App` as `scroll_memory` (one-App-per-thread data; see
// the git history for why the old thread_local shape was dropped).

use super::*;

/// A resize the run loop has not reconciled yet.
///
/// `captured_offset` is what tells our own adopt apart from a reader who moved
/// after the resize: comparing against it catches every path that writes
/// `scroll_offset` — keyboard and mouse wheel alike — in one place.
///
/// The target is the ported `operant_model::ContentPos` (hash-anchored),
/// captured per-frame by the run loop from the chrome reader anchor.
#[derive(Clone, Copy)]
pub(crate) struct PendingResize {
    /// The reader's content address at the moment of the resize.
    pub(crate) target: crate::tui::operant_model::ContentPos,
    /// The offset in effect then. A different value now means the reader chose a
    /// new position, which wins over anything we captured.
    pub(crate) captured_offset: usize,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct ScrollMemory {
    /// Ctrl+G target — the `scroll_offset` (top row) to jump back to, when one
    /// is armed.
    bookmark: Option<usize>,
    /// Largest valid `scroll_offset` (top row) as of the last rendered frame.
    /// Only the render path can measure it, so the reconcile hands it back
    /// here; it is what a bookmark restore clamps against.
    max_scroll: usize,
    /// A terminal resize rewrapped the transcript, so the parked top row now
    /// names different content. This holds the reader's CONTENT address
    /// instead, captured at the resize and resolved by the next paint.
    ///
    /// `None` whenever the reader is following the tail, which is what keeps a
    /// resize from unpinning them: auto-follow owns the position and the render
    /// path paints the tail whatever the geometry.
    pub(crate) pending_resize: Option<PendingResize>,
}

impl App {
    /// Ctrl+T (plan 08) — toggle the thinking display. Hiding remembers the
    /// current non-Off mode; the next press restores it. Transient status
    /// notice only: a key toggle must not write a transcript message.
    pub(crate) fn toggle_thinking(&mut self) {
        use crate::tui::operant_app::config_shim::{self, ReasoningDisplayMode};

        let effective = config_shim::config().display.reasoning_display();
        let next = if effective == ReasoningDisplayMode::Off {
            self.remembered_reasoning_mode
        } else {
            self.remembered_reasoning_mode = effective;
            ReasoningDisplayMode::Off
        };
        config_shim::replace_reasoning_display_override(Some(next));
        self.thinking_display_on = next != ReasoningDisplayMode::Off;
        let notice = if next == ReasoningDisplayMode::Off {
            "Thinking hidden".to_string()
        } else {
            format!("Thinking shown ({:?})", next).to_lowercase()
        };
        self.set_status_notice(notice);
        self.invalidate_transcript();
    }

    /// Ctrl+Shift+O / Alt+O (plan 08) — collapse tool-call rows behind one
    /// summary row, or restore them.
    pub(crate) fn toggle_tool_visibility(&mut self) {
        self.show_tool_calls = !self.show_tool_calls;
        self.set_status_notice(if self.show_tool_calls {
            "Tool calls shown".to_string()
        } else {
            "Tool calls hidden (Alt+O to show)".to_string()
        });
        self.invalidate_transcript();
    }

    /// Ctrl+G — arm a bookmark at the current position, or jump back to the
    /// armed one and disarm it.
    pub(crate) fn toggle_scroll_bookmark(&mut self) {
        let armed = self.scroll_memory.bookmark.take();
        let max_scroll = self.scroll_memory.max_scroll;
        match armed {
            Some(offset) => {
                // The transcript can shrink under the bookmark (`/clear`,
                // `/rewind`, a compacted turn), so the stored top row may now
                // name rows past the tail. Clamp rather than restore blind —
                // the render path saturates too, but only as far as the
                // viewport and without reporting it.
                let target = offset.min(max_scroll);
                self.scroll_offset = target;
                if max_scroll > 0 && target == max_scroll {
                    // Landed on the tail: hand the view back to auto-follow.
                    self.auto_scroll = true;
                    self.new_messages_while_scrolled = 0;
                }
                self.status_message = Some(format!("Bookmark off — jumped to row {target}."));
            }
            None => {
                let at = self.scroll_offset;
                self.scroll_memory.bookmark = Some(at);
                self.status_message =
                    Some(format!("Bookmark on at row {at} — Ctrl+G returns here."));
            }
        }
    }

    /// (iter-1213) How many rows one PageUp/PageDown covers.
    ///
    /// The transcript viewport's own height minus a couple of rows of
    /// overlap, so a page jump keeps a line or two of context and the reader
    /// can still see which way they were travelling.
    ///
    /// Reads the rect the LAST frame published rather than asking the
    /// terminal: a key event arrives between frames, so the geometry that
    /// produced the transcript on screen is the previous frame's. Scroll to
    /// the geometry being looked at, not the geometry about to be drawn.
    ///
    /// Falls back to 10 when no frame has published a rect yet (a headless
    /// App with no renderer).
    pub(crate) fn scroll_page_step(&self) -> usize {
        let area = crate::tui::operant_ui::chat_content_area();
        // A zero HEIGHT is "no frame has published yet" (the doc on
        // `chat_content_area` calls the initial rect zero-sized), and a frame
        // that skipped the transcript layout lands here too. Judge the height
        // alone: a rect can be wide but empty (width 100, height 0), and
        // counting that as published yields a 1-row "page" that swallows the
        // keypress whole — measured in that iteration's own test when a probe
        // published a 100x0 rect and got a step of 1.
        let published = area.height > 0;
        if published {
            (area.height as usize).saturating_sub(2).max(1)
        } else {
            10
        }
    }

    /// The transcript's live top-row ceiling: `last_max_scroll()` when a frame
    /// has measured it, else the row the last paint published (headless tests
    /// publish no geometry).
    fn live_max_scroll(&self) -> usize {
        let live_max = crate::tui::operant_ui::last_max_scroll();
        if live_max > 0 {
            live_max
        } else {
            self.last_render_scroll_offset.get() as usize
        }
    }

    /// Scroll toward the TOP of the transcript (older content) by `rows`.
    ///
    /// While auto-following, the reader is at the tail but the state may lag
    /// the last painted frame (appends do not touch it), so the move starts
    /// from the tail itself — otherwise the first scroll-up after following
    /// teleport from a stale offset.
    pub(crate) fn scroll_up_rows(&mut self, rows: usize) {
        if self.auto_scroll {
            self.scroll_offset = self.live_max_scroll();
        }
        self.scroll_offset = self.scroll_offset.saturating_sub(rows);
        self.auto_scroll = false;
    }

    /// Scroll toward the TAIL (newer content) by `rows`, clamped to the live
    /// ceiling. Landing on the tail hands the view back to auto-follow.
    pub(crate) fn scroll_down_rows(&mut self, rows: usize) {
        let max = self.live_max_scroll();
        if self.auto_scroll {
            // Already at the tail; nothing to do but stay there.
            self.scroll_offset = max;
            return;
        }
        self.scroll_offset = (self.scroll_offset + rows).min(max);
        if max > 0 && self.scroll_offset == max {
            self.auto_scroll = true;
            self.new_messages_while_scrolled = 0;
        }
    }

    /// Jump to the top of the transcript (Shift+Home).
    pub(crate) fn scroll_to_top(&mut self) {
        self.scroll_offset = 0;
        self.auto_scroll = false;
    }

    /// Jump to the tail and resume following (Shift+End).
    pub(crate) fn scroll_to_tail(&mut self) {
        self.scroll_offset = self.live_max_scroll();
        self.auto_scroll = true;
        self.new_messages_while_scrolled = 0;
    }

    /// A terminal resize is about to rewrap every transcript line. Capture
    /// where the reader is in content coordinates so the next paint can put
    /// them back on the same message, rather than at the new bottom of the
    /// transcript or at row 0.
    ///
    /// While `auto_scroll` is set the reader is following the tail, so there is
    /// nothing to preserve and any pending anchor is dropped — preserving an
    /// offset for a follower is exactly the bug this guards against.
    pub(crate) fn note_resize(&mut self) {
        if self.auto_scroll {
            self.scroll_memory.pending_resize = None;
            return;
        }
        // `None` when the top row carried no message identity (an empty
        // transcript, or a synthetic tool/system line at the very top). The
        // reconcile then leaves the offset alone and lets the render path's own
        // clamp keep it in range, which is the honest answer: there is nothing
        // to name.
        self.scroll_memory.pending_resize =
            self.last_render_content_pos
                .get()
                .map(|target| PendingResize {
                    target,
                    captured_offset: self.scroll_offset,
                });
    }

    /// Re-anchor the view after a rendered frame. Called from the run loop
    /// immediately after `terminal.draw`.
    ///
    /// Three jobs, in order:
    /// 1. measure the frame — `max_scroll` is what the render path measured;
    /// 2. tail sync — while following, write the tail row back into
    ///    `scroll_offset` so the next user scroll is relative to reality;
    /// 3. ghost clamp — drop rows the state still counts after a flush or
    ///    compaction shrank the transcript;
    /// 4. resize adopt — a pending content address resolves to the top row the
    ///    reader should be parked on; adopt it when they have not scrolled
    ///    since the capture.
    pub(crate) fn reconcile_scroll_anchor(&mut self) {
        let max = self.live_max_scroll();
        self.scroll_memory.max_scroll = max;

        if self.auto_scroll {
            self.scroll_offset = max;
            self.scroll_memory.pending_resize = None;
            return;
        }

        if max > 0 && self.scroll_offset > max {
            // Ghost rows: a flush boundary or compaction collapsed rows the
            // offset still counts (2026-10-09 live-audit P4-1). Clamp the state
            // to the frame's true ceiling; the render already clamps its paint.
            self.scroll_offset = max;
        }

        if let Some(pending) = self.scroll_memory.pending_resize.take() {
            // Decline when the reader scrolled after the capture: their move is
            // the newer position. Otherwise the resolved top row IS where they
            // belong — the offset is a top row, so adopt it directly.
            if self.scroll_offset == pending.captured_offset
                && let Some(row) = self.last_resolved_scroll.get()
            {
                self.scroll_offset = row;
            }
        }
    }
}
