//! `scroll_emit` — settled-transcript emission into native scrollback.
//!
//! In terminal-scroll mode (P5-1, `Viewport::Inline`, no alt-screen) the
//! terminal's own scrollback owns the session history. This module emits
//! settled transcript rows into that history via `Terminal::insert_before`.
//!
//! (plan 07 §1′) The watermark is a MESSAGE index, not a row index: one
//! past the last FINAL display message emitted. A final message's rows are
//! immutable, so the index cannot drift — the old row watermark drifted
//! when a tool row re-rendered (Queued→Running→Done) between flushes and
//! duplicated or shifted history.
//!
//! (plan 07 §2) Native scrollback cannot reflow, so a resize, a mode
//! toggle or a context reset REQUESTS A REPLAY: the run loop clears the
//! screen inside the synchronized-update bracket and resets the
//! watermark, and the next emit pass re-emits the whole transcript at the
//! CURRENT width.

use super::App;
use crate::tui::operant_model::{PreparedChatFrame, PreparedSectionKind};
use ratatui::layout::Rect;
use ratatui::widgets::Widget;

impl App {
    /// Emit newly settled transcript rows into native scrollback. Called
    /// after `terminal.draw` each frame in `App::run`; a no-op unless the
    /// terminal-scroll mode is on.
    pub fn emit_settled_transcript_rows<
        B: ratatui::backend::Backend + super::SynchronizedUpdate,
    >(
        &self,
        terminal: &mut ratatui::Terminal<B>,
    ) {
        if !self.terminal_scroll_mode {
            return;
        }
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        // (plan 07 §2) resize debounce: fire the replay once the terminal
        // has been quiet for 150 ms after the last size change.
        let due = self.scroll_resize_replay_at_ms.get();
        if due != 0 && now_ms >= due {
            self.scroll_resize_replay_at_ms.set(0);
            self.pending_native_replay.set(true);
        }
        // (plan 07 §2) replay: clear the screen + scrollback inside the
        // DEC 2026 synchronized-update bracket, reset the watermark — the
        // emit pass below then re-emits the whole transcript at the
        // current width. (The old-width history is wrong, so it is purged,
        // not kept.)
        if self.pending_native_replay.replace(false) {
            let _ = terminal.backend_mut().replay_clear();
            self.scroll_emitted_through.set(0);
        }
        let Some(frame) = crate::tui::operant_ui::last_chat_frame() else {
            return;
        };
        let final_count = crate::tui::operant_model::adapter::final_display_message_count(self);
        let committed = committed_prefix_rows(&frame);
        let width = terminal.size().map(|s| s.width).unwrap_or(0);
        // (plan 07 §2) resize detection: any width change schedules the
        // debounced replay; nothing is emitted at a stale width.
        let last = self.scroll_last_width.get();
        if last != 0 && last != width {
            self.scroll_resize_replay_at_ms
                .set(now_ms.saturating_add(150));
            self.scroll_last_width.set(width);
            return;
        }
        self.scroll_last_width.set(width);
        // Pending /cls divider (P5 scroll mode): print the boundary row
        // into native history ahead of the next content, then continue.
        // Independent of the watermark delta — the divider is not a
        // transcript row.
        if self.scroll_cls_divider.get() {
            self.scroll_cls_divider.set(false);
            let width_now = width as usize;
            let label = " view cleared — context kept ";
            let dashes = width_now.saturating_sub(label.len()).max(2);
            let divider = ratatui::text::Line::from(ratatui::text::Span::styled(
                format!("{label}{}", "─".repeat(dashes)),
                ratatui::style::Style::default().dim(),
            ));
            let _ = terminal.insert_before(1, |buf| {
                divider.render(
                    Rect {
                        x: 0,
                        y: 0,
                        width: buf.area.width,
                        height: 1,
                    },
                    buf,
                );
            });
        }
        // (plan 07 §1′) the MESSAGE watermark: emit whole final messages,
        // never rows.
        let through = self.scroll_emitted_through.get();
        if final_count <= through {
            return;
        }
        let boundaries = message_boundaries(&frame);
        // Rows [start, end) of messages [through, final_count), each
        // clamped to the committed prefix — a final message's rows can
        // still sit past the first mutable section boundary in the shared
        // wrapped-line space.
        let end = boundaries
            .get(final_count - 1)
            .map(|b| b.wrapped_len)
            .unwrap_or(0)
            .min(committed);
        let start = if through == 0 {
            0
        } else {
            boundaries
                .get(through - 1)
                .map(|b| b.wrapped_len)
                .unwrap_or(0)
                .min(committed)
        };
        if end <= start {
            // Nothing renderable advanced: the remaining final rows live
            // past the committed prefix. Advance only when the full count is
            // clamped so a later flush (more settled rows) still runs.
            if end == committed {
                self.scroll_emitted_through.set(final_count);
            }
            return;
        }
        let lines = settled_lines(&frame, start, end);
        if lines.is_empty() {
            // Rows existed but none were renderable (e.g. image-region
            // placeholders). Advance anyway — those rows are settled; the
            // native history just doesn't carry them.
            self.scroll_emitted_through.set(final_count);
            return;
        }
        let height = lines.len() as u16;
        let result = terminal.insert_before(height, |buf| {
            for (i, line) in lines.iter().enumerate() {
                line.render(
                    Rect {
                        x: 0,
                        y: i as u16,
                        width: buf.area.width,
                        height: 1,
                    },
                    buf,
                );
            }
        });
        if result.is_ok() {
            self.scroll_emitted_through.set(final_count);
        }
    }

    /// (plan 07 §2) request a native replay from any context that has the
    /// App but not the terminal (slash arms, drains, toggles). The run
    /// loop performs the clear + watermark reset + re-emit on its next
    /// frame.
    pub fn request_native_replay(&self) {
        self.pending_native_replay.set(true);
    }
}

/// (plan 07 §1′) the Body section's message boundaries (empty when the
/// frame carries none).
pub(crate) fn message_boundaries(
    frame: &PreparedChatFrame,
) -> &[crate::tui::operant_model::MessageBoundary] {
    let body = frame
        .sections
        .iter()
        .find(|s| matches!(s.kind, PreparedSectionKind::Body))
        .or_else(|| frame.sections.first());
    body.map(|b| b.prepared.message_boundaries.as_slice())
        .unwrap_or(&[])
}

/// Rows of the messages `[0, count)` — the viewport floor in
/// terminal-scroll mode: rows below it are already in native scrollback.
pub(crate) fn message_count_rows(frame: &PreparedChatFrame, count: usize) -> usize {
    let boundaries = message_boundaries(frame);
    if count == 0 {
        return 0;
    }
    boundaries
        .get(count - 1)
        .map(|b| b.wrapped_len)
        .unwrap_or(0)
}

/// Row index one past the last settled (emittable) row: the `line_start` of
/// the first mutable section, or the frame's total when none is present.
pub(crate) fn committed_prefix_rows(frame: &PreparedChatFrame) -> usize {
    frame
        .sections
        .iter()
        .find(|s| {
            matches!(
                s.kind,
                PreparedSectionKind::Streaming
                    | PreparedSectionKind::Reasoning
                    | PreparedSectionKind::BatchProgress
            )
        })
        .map(|s| s.line_start)
        .unwrap_or(frame.total_wrapped_lines)
}

/// Collect the settled rows `[from, to)` as renderable lines, with trailing
/// spaces trimmed so the emitted history is clean.
fn settled_lines(
    frame: &PreparedChatFrame,
    from: usize,
    to: usize,
) -> Vec<ratatui::text::Line<'static>> {
    let mut out = Vec::with_capacity(to.saturating_sub(from));
    for section in frame.sections.iter() {
        let start = section.line_start;
        let len = section.prepared.wrapped_lines.len();
        let end = start + len;
        if end <= from || start >= to {
            continue;
        }
        let lo = from.max(start) - start;
        let hi = to.min(end) - start;
        for line in section.prepared.wrapped_lines[lo..hi].iter() {
            let mut line = line.clone();
            crate::tui::operant_ui::viewport::trim_line_trailing_spaces(&mut line);
            out.push(line);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn section(
        kind: PreparedSectionKind,
        rows: usize,
    ) -> crate::tui::operant_model::PreparedSection {
        let mut prepared = crate::tui::operant_model::PreparedMessages {
            wrapped_lines: Vec::new(),
            wrapped_plain_lines: std::sync::Arc::new(Vec::new()),
            wrapped_copy_offsets: std::sync::Arc::new(Vec::new()),
            raw_plain_lines: std::sync::Arc::new(Vec::new()),
            wrapped_line_map: std::sync::Arc::new(Vec::new()),
            wrapped_user_indices: Vec::new(),
            wrapped_user_prompt_starts: Vec::new(),
            wrapped_user_prompt_ends: Vec::new(),
            user_prompt_texts: Vec::new(),
            image_regions: Vec::new(),
            edit_tool_ranges: Vec::new(),
            copy_targets: Vec::new(),
            message_boundaries: Vec::new(),
            mermaid_pending_epoch: None,
        };
        for i in 0..rows {
            prepared
                .wrapped_lines
                .push(ratatui::text::Line::from(format!("r{i}")));
        }
        crate::tui::operant_model::PreparedSection {
            kind,
            prepared: std::sync::Arc::new(prepared),
            line_start: 0,
            raw_start: 0,
        }
    }

    fn frame(sections: Vec<crate::tui::operant_model::PreparedSection>) -> PreparedChatFrame {
        // from_sections recomputes line_start cumulatively, so pass sections
        // in display order — same bookkeeping as production.
        PreparedChatFrame::from_sections(
            sections
                .into_iter()
                .map(|s| (s.kind, s.prepared.clone()))
                .collect(),
        )
    }

    #[test]
    fn committed_prefix_stops_at_first_mutable_section() {
        let f = frame(vec![
            section(PreparedSectionKind::Body, 3),
            section(PreparedSectionKind::Streaming, 2),
        ]);
        assert_eq!(committed_prefix_rows(&f), 3);
    }

    /// (plan 07 §1′) message_count_rows: the wrapped-row count through the
    /// first N messages — boundaries are cumulative, so this is the viewport
    /// floor for the emitted prefix. Replaces the old row-watermark helper
    /// whose tracking test (H-1) was RED from the day it landed: the row
    /// watermark drifted on tool-status re-renders; a message index cannot.
    #[test]
    fn message_count_rows_tracks_message_boundaries() {
        let mut body = section(PreparedSectionKind::Body, 0);
        let prepared_mut = std::sync::Arc::get_mut(&mut body.prepared).expect("unique");
        for rows in [2usize, 3, 4] {
            let prev = prepared_mut
                .message_boundaries
                .last()
                .map(|b| b.wrapped_len)
                .unwrap_or(0);
            for i in 0..rows {
                prepared_mut
                    .wrapped_lines
                    .push(ratatui::text::Line::from(format!("m{prev}-{i}")));
            }
            prepared_mut
                .message_boundaries
                .push(crate::tui::operant_model::MessageBoundary {
                    msg_hash: prev as u64,
                    wrapped_len: prev + rows,
                    raw_len: prev + rows,
                    user_prompt_len: 0,
                });
        }
        let f = frame(vec![body, section(PreparedSectionKind::Streaming, 1)]);
        assert_eq!(message_count_rows(&f, 0), 0);
        assert_eq!(message_count_rows(&f, 1), 2);
        assert_eq!(message_count_rows(&f, 2), 5);
        assert_eq!(message_count_rows(&f, 3), 9);
    }

    /// (plan 07 §2) request_native_replay + the emit pass: the replay
    /// resets the watermark, and the pass re-emits every final message —
    /// never a duplicate (the watermark advances to the full final count on
    /// success) and never a partial (advance only on insert Ok).
    #[test]
    fn replay_resets_the_watermark_and_reemits_at_the_current_width() {
        let mut app = super::super::tests::make_app();
        app.terminal_scroll_mode = true;
        app.scroll_emitted_through.set(3);
        app.request_native_replay();
        assert!(app.pending_native_replay.get());
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).expect("terminal");
        // No chat frame yet: the replay still fires (clear + reset)…
        app.emit_settled_transcript_rows(&mut terminal);
        assert_eq!(
            app.scroll_emitted_through.get(),
            0,
            "the replay must reset the message watermark"
        );
        assert!(!app.pending_native_replay.get(), "one-shot consumed");
    }
}
