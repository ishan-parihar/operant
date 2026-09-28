// app/scroll_anchor.rs — Transcript scroll bookmark and reflow anchor.
//
// Two related pieces of reading-position state, both answering the same
// complaint: a transcript that is still growing keeps taking the reader's spot
// away.
//
//   * Bookmark (Ctrl+G) — an explicit spot they choose and can come back to.
//   * Anchor — implicit. While the reader is parked above the tail, hold the
//     line they are looking at still when new output reflows the content above
//     it (jcode #1412). Without it the view jumps on every append, because
//     `scroll_offset` counts rows *from the bottom*: `top_row =
//     max_scroll - scroll_offset`, so a taller transcript slides the whole
//     viewport down by the growth even though the offset never changed.
//
// The state lives on `App` as `scroll_memory`. It used to be a `thread_local!`
// because `App::new` (tui/app/init.rs) was locked by a parallel lane at the
// time; that is the wrong shape for what is one-App-per-thread data, and it
// meant two `App`s on one thread shared one reading position. The sibling
// per-frame transcript caches in tui/render/cache.rs are genuinely frame-local
// and stay thread-local.

use super::*;

/// The row the view is pinned to, plus the offset the last reconcile wrote.
///
/// `applied_offset` is what tells our own reflow corrections apart from a real
/// user scroll: when `scroll_offset` no longer matches it, something else moved
/// the view and the pin is void.
#[derive(Clone, Copy)]
struct ScrollAnchor {
    top_row: usize,
    applied_offset: usize,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct ScrollMemory {
    /// Ctrl+G target — the `scroll_offset` to jump back to, when one is armed.
    bookmark: Option<usize>,
    /// Pinned row, when the reader is parked above the tail.
    anchor: Option<ScrollAnchor>,
    /// Largest valid `scroll_offset` as of the last rendered frame
    /// (`content_height - viewport_height`). Only the render path can measure
    /// it, so the run loop hands it back here; it is what a bookmark restore
    /// clamps against.
    max_scroll: usize,
}

impl App {
    /// Ctrl+G — arm a bookmark at the current position, or jump back to the
    /// armed one and disarm it.
    pub(crate) fn toggle_scroll_bookmark(&mut self) {
        let armed = self.scroll_memory.bookmark.take();
        let max_scroll = self.scroll_memory.max_scroll;
        match armed {
            Some(offset) => {
                // The transcript can shrink under the bookmark (`/clear`,
                // `/rewind`, a compacted turn), so the stored offset may now
                // point past the end. Clamp rather than restore blind — the
                // render path saturates too, but only as far as the viewport
                // and without reporting it.
                let target = offset.min(max_scroll);
                self.scroll_offset = target;
                if target == 0 {
                    // Landed on the tail: hand the view back to auto-follow,
                    // the same way PageDown does when it reaches the bottom.
                    self.auto_scroll = true;
                    self.new_messages_while_scrolled = 0;
                }
                self.status_message =
                    Some(format!("Bookmark off — jumped back up {target} line(s)."));
            }
            None => {
                let at = self.scroll_offset;
                self.scroll_memory.bookmark = Some(at);
                self.status_message = Some(format!(
                    "Bookmark on at {at} line(s) up — Ctrl+G returns here."
                ));
            }
        }
    }

    /// Pin the current position as the reader's deliberate choice, so later
    /// reflow keeps it still.
    ///
    /// `prev_offset` is the value from before the key handler moved it. The row
    /// about to be painted is `painted_row + prev_offset - new_offset`, derived
    /// from the row the last frame painted — no need for the transcript's total
    /// height, which is only measurable at render time.
    pub(crate) fn note_user_scroll(&mut self, prev_offset: usize) {
        if self.auto_scroll {
            // Pinned to the tail: auto-follow already owns the position, so
            // there is nothing to hold still.
            return;
        }
        let painted = self.last_render_scroll_offset.get() as usize;
        let top_row = painted
            .saturating_add(prev_offset)
            .saturating_sub(self.scroll_offset);
        let applied_offset = self.scroll_offset;
        self.scroll_memory.anchor = Some(ScrollAnchor {
            top_row,
            applied_offset,
        });
    }

    /// Re-anchor the view after a rendered frame. Called from the run loop
    /// immediately after `terminal.draw`.
    ///
    /// The render path is the only place that knows how many rows the
    /// transcript now occupies, so it publishes the row it actually painted
    /// into `last_render_scroll_offset`. A row *past* the pinned one means new
    /// output was inserted above the reader, and `scroll_offset` has to grow by
    /// the same amount for the pinned line to stay put on the next paint. The
    /// correction therefore lands one frame after the growth: invisible at any
    /// real frame rate, and it only ever corrects toward a row the reader
    /// already chose, never invents motion.
    pub(crate) fn reconcile_scroll_anchor(&mut self) {
        let painted = self.last_render_scroll_offset.get() as usize;
        let scrolled = self.scroll_offset;
        // The render path computes `top_row = max_scroll - scroll_offset`, so
        // the row it painted plus the offset it painted with is the scrollable
        // height. Under auto-follow it ignores the offset and paints the tail.
        let max_scroll = if self.auto_scroll {
            painted
        } else {
            painted.saturating_add(scrolled)
        };
        let mem = &mut self.scroll_memory;
        mem.max_scroll = max_scroll;

        let Some(anchor) = mem.anchor else { return };

        if self.auto_scroll {
            // Auto-follow owns the view again — nothing to hold still.
            mem.anchor = None;
            return;
        }
        if scrolled != anchor.applied_offset {
            // The view moved behind our back: a PageUp, a wheel tick,
            // `/rewind`, `/clear`. A scroll the reader made is a new reading
            // position, so the old pin is void. Comparing against the offset
            // we last wrote catches every such path in one place instead of
            // instrumenting each call site — the mouse wheel included.
            mem.anchor = None;
            return;
        }

        // Content above grew by however far the painted row drifted past
        // the pin. `saturating_sub` covers the shrink case for free: a
        // transcript that lost rows above the reader needs no correction,
        // because the pinned line is already painted where the reader was
        // looking.
        let growth = painted.saturating_sub(anchor.top_row);
        if growth == 0 {
            return;
        }
        let corrected = scrolled.saturating_add(growth);
        self.scroll_offset = corrected;
        mem.anchor = Some(ScrollAnchor {
            top_row: anchor.top_row,
            applied_offset: corrected,
        });
    }
}
