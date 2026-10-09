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
//   * Resize anchor — the same promise across a terminal resize, which
//     rewraps every line. A row index cannot survive that (row 12 at width
//     120 is not row 12 at width 60), so this one names CONTENT instead, and
//     the render path resolves it against the next frame's geometry.
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

/// A resize the run loop has not reconciled yet.
///
/// `captured_offset` is what tells our own adopt apart from a reader who moved
/// after the resize: comparing against it catches every path that writes
/// `scroll_offset` — keyboard and mouse wheel alike — in one place, the same
/// trick [`ScrollAnchor::applied_offset`] plays for the reflow anchor.
///
/// The target is the ported `operant_model::ContentPos` (hash-anchored),
/// captured per-frame by the run loop from the chrome reader anchor. The old
/// index-based `ContentPos{message, line}` died with the dispatch-table
/// cutover; keeping both shapes around is what wedged the resize seam (see
/// tui_state_impl::pending_resize_anchor).
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
    /// Ctrl+G target — the `scroll_offset` to jump back to, when one is armed.
    bookmark: Option<usize>,
    /// Pinned row, when the reader is parked above the tail.
    anchor: Option<ScrollAnchor>,
    /// Largest valid `scroll_offset` as of the last rendered frame
    /// (`content_height - viewport_height`). Only the render path can measure
    /// it, so the run loop hands it back here; it is what a bookmark restore
    /// clamps against.
    max_scroll: usize,
    /// A terminal resize rewrapped the transcript, so `anchor.top_row` now names
    /// a different row than it did. This holds the reader's CONTENT address
    /// instead, captured at the resize and resolved by the next paint.
    ///
    /// `None` whenever the reader is following the tail, which is what keeps a
    /// resize from unpinning them: auto-follow owns the position and the render
    /// path paints the tail whatever the geometry.
    /// `pub(crate)` because `render_messages` reads it: resolving a content address
    /// against a freshly-wrapped transcript is only possible where the wrapping
    /// is built. Nothing outside `scroll_anchor.rs` writes it.
    pub(crate) pending_resize: Option<PendingResize>,
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

    /// A terminal resize is about to rewrap every transcript line. Capture
    /// where the reader is in content coordinates so the next paint can put
    /// them back on the same message, rather than at the new bottom of the
    /// transcript or at row 0.
    ///
    /// Needs no size argument on purpose: what matters is not the terminal's
    /// dimensions but *that* a rewrap is coming, and `Event::Resize` is the only
    /// thing that says so. The render path publishes the resolution.
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

        // ---- Ghost-offset clamp -----------------------------------------
        //
        // A flush boundary (streaming bubble -> committed message) or a
        // compaction can drop rows the reader's offset still counts: the
        // growth correction above tracked the bubble's rows, but they
        // collapse when the flush reflows them into one message, leaving
        // e.g. offset 355 against a 15-row max (2026-10-09 live-audit P4-1
        // follow-up: the view parked correctly — the render clamps its own
        // paint — but PageDown needed dozens of presses to walk ghost rows
        // home). Keep the STATE honest: clamp to the frame's true max and
        // void the pin (the row it named is gone). 0 means no frame has
        // rendered yet (unit tests without a renderer) — leave the offset
        // alone then.
        let live_max = crate::tui::operant_ui::last_max_scroll();
        if live_max > 0 && self.scroll_offset > live_max {
            self.scroll_offset = live_max;
            mem.anchor = None;
        }

        // ---- Resize: adopt the row the anchored content now occupies -------
        //
        // A width change rewrapped every line, so `anchor.top_row` is stale by
        // definition and the append-growth correction below would "fix" a drift
        // that never happened. The render path resolved the captured content
        // address against THIS frame's geometry (`last_resolved_scroll`), so
        // adopting it here re-pins the reflow anchor at the row we asked for and
        // the next append corrects from there instead of treating our own
        // correction as a user scroll.
        if let Some(pending) = mem.pending_resize.take() {
            // Three ways to decline, and declining is always "leave the reader
            // where they put themselves": following the tail, having scrolled
            // again since the capture, or having no resolvable row.
            if !self.auto_scroll
                && scrolled == pending.captured_offset
                && let Some(row) = self.last_resolved_scroll.get()
            {
                // `scroll_offset` counts rows up from the bottom and the render
                // path inverts it (`scroll = max_scroll - scroll_offset`), so
                // landing on `row` means setting the offset to the complement.
                // `saturating_sub` covers a resolved row past the bottom.
                let offset = mem.max_scroll.saturating_sub(row);
                self.scroll_offset = offset;
                mem.anchor = Some(ScrollAnchor {
                    top_row: row,
                    applied_offset: offset,
                });
            }
            return;
        }

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
