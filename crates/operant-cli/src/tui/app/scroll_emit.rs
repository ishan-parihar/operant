//! `scroll_emit` — P5-2: settled-transcript emission into native scrollback.
//!
//! In terminal-scroll mode (P5-1, `Viewport::Inline`, no alt-screen) the
//! terminal's own scrollback owns the session history. This module emits
//! settled transcript rows into that history via `Terminal::insert_before`,
//! tracked by a last-emitted-row watermark so each row is emitted exactly
//! once.
//!
//! Emittable rows are a PREFIX of the prepared frame's wrapped lines: rows
//! up to the first mutable section (Streaming / Reasoning / BatchProgress —
//! those rows can still change, so emitting them early would freeze a
//! partial). The watermark advances at natural flush boundaries — a message
//! committing at `Done`, a tool row or system annotation settling mid-turn —
//! because that's when rows move from the live window into the settled
//! prefix.
//!
//! ponytail: emitted history keeps the wrap it was rendered at — native
//! scrollback cannot reflow. On resize the watermark re-syncs to the new
//! committed prefix (never re-emits at the new width); upgrade path is a
//! re-emit-diff scheme if a user ever reports it.

use super::App;
use crate::tui::operant_model::{PreparedChatFrame, PreparedSectionKind};
use ratatui::layout::Rect;
use ratatui::widgets::Widget;

impl App {
    /// Emit newly settled transcript rows into native scrollback. Called
    /// after `terminal.draw` each frame in `App::run`; a no-op unless the
    /// terminal-scroll mode is on.
    pub fn emit_settled_transcript_rows<B: ratatui::backend::Backend>(
        &self,
        terminal: &mut ratatui::Terminal<B>,
    ) {
        if !self.terminal_scroll_mode {
            return;
        }
        let Some(frame) = crate::tui::operant_ui::last_chat_frame() else {
            return;
        };
        let committed = committed_prefix_rows(&frame);
        let width = terminal.size().map(|s| s.width).unwrap_or(0);
        let recorded_width = self.scroll_emitted_width.get();
        if recorded_width == 0 || recorded_width != width {
            // First frame or resize: sync the watermark to the current
            // prefix. Never re-emit rows already in history.
            self.scroll_emitted_width.set(width);
            self.scroll_emitted_rows.set(committed);
            return;
        }
        let watermark = self.scroll_emitted_rows.get();
        if committed <= watermark {
            return;
        }
        let lines = settled_lines(&frame, watermark, committed);
        if lines.is_empty() {
            // Rows existed but none were renderable (e.g. image-region
            // placeholders). Advance the watermark anyway — those rows are
            // settled; the native history just doesn't carry them.
            self.scroll_emitted_rows.set(committed);
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
            self.scroll_emitted_rows.set(committed);
        }
    }
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
            section(PreparedSectionKind::Header, 2),
            section(PreparedSectionKind::Body, 5),
            section(PreparedSectionKind::Streaming, 3),
            section(PreparedSectionKind::InlineImages, 1),
        ]);
        assert_eq!(committed_prefix_rows(&f), 7);
    }

    #[test]
    fn committed_prefix_is_total_when_nothing_streaming() {
        let f = frame(vec![
            section(PreparedSectionKind::Header, 1),
            section(PreparedSectionKind::Body, 4),
            section(PreparedSectionKind::InlineImages, 2),
        ]);
        assert_eq!(committed_prefix_rows(&f), 7);
        assert_eq!(f.total_wrapped_lines, 7);
    }

    #[test]
    fn settled_lines_slice_stops_before_streaming() {
        let f = frame(vec![
            section(PreparedSectionKind::Body, 4),
            section(PreparedSectionKind::Streaming, 2),
        ]);
        let lines = settled_lines(&f, 1, 4);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].to_string(), "r1");
        assert_eq!(lines[2].to_string(), "r3");
        // The range is the caller's contract (production callers pass the
        // clamped committed prefix); the full-frame range includes the
        // streaming rows verbatim — which is why emit only ever asks
        // [watermark, committed_prefix).
        let lines = settled_lines(&f, 0, 4);
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0].to_string(), "r0");
    }
}
