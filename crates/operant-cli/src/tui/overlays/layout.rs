// overlays/layout.rs — Shared geometry, constants, and modal helpers.
//
// Extracted from the overlays.rs monolith. Holds the shared semantic colour
// accessors, centered-rect / cycle helpers, dark-overlay + dialog-bg
// renderers, and the modal frame/title/search helpers used by every
// overlay in this module.
//
// The colours are no longer constants: they resolve through
// `crate::tui::theme_colors`, so `/theme` (and the persisted `theme` setting)
// repaints every overlay instead of only writing the name to disk.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};
use unicode_width::UnicodeWidthStr;

use crate::tui::space;
use crate::tui::theme_colors;

// ---------------------------------------------------------------------------
// Geometry helper (shared)
// ---------------------------------------------------------------------------

/// Compute a centred `Rect` of the given `width` × `height` inside `area`.
pub fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect {
        x,
        y,
        width: width.min(area.width),
        height: height.min(area.height),
    }
}

/// Move `selected` back by one, wrapping to `count - 1` at zero. No-op if `count == 0`.
pub fn cycle_prev(selected: &mut usize, count: usize) {
    if count == 0 {
        return;
    }
    *selected = if *selected == 0 {
        count - 1
    } else {
        *selected - 1
    };
}

/// Move `selected` forward by one, wrapping to `0` past `count - 1`. No-op if `count == 0`.
pub fn cycle_next(selected: &mut usize, count: usize) {
    if count == 0 {
        return;
    }
    *selected = (*selected + 1) % count;
}

// ---------------------------------------------------------------------------
// Reusable overlay helpers (shared by all dialog renderers)
// ---------------------------------------------------------------------------

/// Darken the entire screen with a semi-transparent overlay.
/// Call this BEFORE rendering any dialog content.
pub fn render_dark_overlay(frame: &mut Frame, area: Rect) {
    render_dark_overlay_buf(frame.buffer_mut(), area);
}

pub fn render_dark_overlay_buf(buf: &mut Buffer, area: Rect) {
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_bg(theme_colors::overlay_bg());
                cell.set_fg(theme_colors::muted());
            }
        }
    }
}

/// Fill a rectangle with the standard dialog background color (no border).
pub fn render_dialog_bg(frame: &mut Frame, area: Rect) {
    render_dialog_bg_buf(frame.buffer_mut(), area);
}

pub fn render_dialog_bg_buf(buf: &mut Buffer, area: Rect) {
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_char(' ');
                cell.set_bg(theme_colors::panel_bg());
                cell.set_fg(theme_colors::text());
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Modal floors and the one hint
// ---------------------------------------------------------------------------

/// Readable floor for a modal's width — [`space::L`].
pub const MIN_MODAL_W: u16 = space::L;

/// Readable floor for a modal's height. Not a `space` step: five content rows
/// plus the two border rows is the smallest modal that still shows a title, a
/// line of body, and a hint.
pub const MIN_MODAL_H: u16 = 6;

/// The one dismissal hint. Four spellings of this fact exist today
/// (`"Esc to cancel"` ×3, `"Esc close"`, `"Esc: close"`, `"Esc to close."`);
/// they converge here.
#[allow(dead_code)] // unread until the call sites adopt `ModalSpec::hint`
pub const HINT_ESC: &str = "Esc to close";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModalLayout {
    pub dialog_area: Rect,
    pub header_area: Rect,
    pub body_area: Rect,
    pub footer_area: Rect,
}

/// The ONE sizing rule for every modal.
///
/// Takes the modal's *desired* size, clamps it against the available area,
/// centres it, then splits the border-inset inner area into header / body /
/// footer. Every caller states a desired size and lets this bound it — no call
/// site spells `.min(area.height.saturating_sub(4))` itself any more.
///
/// The clamp is the compatibility contract with [`begin_modal_frame`] /
/// [`begin_modal_buf`] and with [`modal_frame_buf`]: for the same inputs all
/// three produce identical rects. Pinned by
/// `modal_layout_matches_begin_modal_geometry` and, structurally, by
/// `modal_clamp_formula_is_written_once`.
pub fn modal_layout(
    area: Rect,
    desired_width: u16,
    desired_height: u16,
    header_height: u16,
    footer_height: u16,
) -> ModalLayout {
    // A modal keeps a `space::M` margin from the screen edge, and never shrinks
    // below the readable floor. Spelled through `Margin` so the inset reads as
    // the margin it is rather than a magic 4 — that number was previously
    // re-typed, as `-2` or not at all, at 16 sites.
    let dialog_width = desired_width
        .min(area.width.saturating_sub(space::pad_h(space::M).horizontal))
        .max(MIN_MODAL_W);
    let dialog_height = desired_height
        .min(area.height.saturating_sub(space::pad_v(space::M).vertical))
        .max(MIN_MODAL_H);
    let dialog_area = centered_rect(dialog_width, dialog_height, area);
    // Deliberately *not* `dialog_area.inner(pad(space::XS))`: ratatui's
    // `Rect::inner` returns `Rect::ZERO` once the margin exceeds half the rect,
    // which would move a degenerate 1-cell modal. This arithmetic is the
    // byte-compatible form.
    let inner_area = Rect {
        x: dialog_area.x + space::XS,
        y: dialog_area.y + space::XS,
        width: dialog_area.width.saturating_sub(space::XS * 2),
        height: dialog_area.height.saturating_sub(space::XS * 2),
    };
    let header_h = header_height.min(inner_area.height);
    let footer_h = footer_height.min(inner_area.height.saturating_sub(header_h));
    let body_area = Rect {
        x: inner_area.x,
        y: inner_area.y.saturating_add(header_h),
        width: inner_area.width,
        height: inner_area.height.saturating_sub(header_h + footer_h),
    };
    ModalLayout {
        dialog_area,
        header_area: Rect {
            x: inner_area.x,
            y: inner_area.y,
            width: inner_area.width,
            height: header_h,
        },
        body_area,
        footer_area: Rect {
            x: inner_area.x,
            y: inner_area.y + inner_area.height.saturating_sub(footer_h),
            width: inner_area.width,
            height: footer_h,
        },
    }
}

pub fn begin_modal_frame(
    frame: &mut Frame,
    area: Rect,
    width: u16,
    height: u16,
    header_height: u16,
    footer_height: u16,
) -> ModalLayout {
    let layout = modal_layout(area, width, height, header_height, footer_height);
    render_dark_overlay(frame, area);
    frame.render_widget(Clear, layout.dialog_area);
    render_dialog_bg(frame, layout.dialog_area);
    layout
}

pub fn begin_modal_buf(
    buf: &mut Buffer,
    area: Rect,
    width: u16,
    height: u16,
    header_height: u16,
    footer_height: u16,
) -> ModalLayout {
    let layout = modal_layout(area, width, height, header_height, footer_height);
    render_dark_overlay_buf(buf, area);
    Clear.render(layout.dialog_area, buf);
    render_dialog_bg_buf(buf, layout.dialog_area);
    layout
}

/// The one modal title format: a bold themed title with a single leading space,
/// and the hint right-aligned on the same row.
///
/// Both title renderers and [`modal_frame_buf`] take the line from here, so the
/// padding arithmetic and span styles exist once. Byte-compatible with the two
/// bodies it replaces, pinned by `modal_title_line_matches_the_frame_renderer`.
pub fn modal_title_line(area_width: u16, title: &str, right_hint: &str) -> Line<'static> {
    let title_width = UnicodeWidthStr::width(title);
    let hint_width = UnicodeWidthStr::width(right_hint);
    // One space of leading pad on the title, one on either side of the hint.
    let gap = 3;
    let padding = area_width.saturating_sub((title_width + hint_width + gap) as u16) as usize;
    Line::from(vec![
        Span::styled(
            format!(" {title}"),
            Style::default()
                .fg(theme_colors::text())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            " ".repeat(padding),
            Style::default().fg(theme_colors::text()),
        ),
        Span::styled(
            right_hint.to_string(),
            Style::default().fg(theme_colors::muted()),
        ),
    ])
}

/// `Frame` adapter over [`render_modal_title_buf`]. One expression on purpose,
/// for the same reason [`modal_frame`] is: in ratatui 0.30 `Frame::render_widget`
/// is literally `widget.render(area, self.buffer)`, so the `Buffer` body already
/// serves both sinks and a second copy can only drift. Guarded by
/// `title_renderers_are_one_expression_adapters`.
pub fn render_modal_title_frame(frame: &mut Frame, area: Rect, title: &str, right_hint: &str) {
    render_modal_title_buf(frame.buffer_mut(), area, title, right_hint);
}

/// The one title body. Both the `Frame` and the `Buffer` caller reach it here.
pub fn render_modal_title_buf(buf: &mut Buffer, area: Rect, title: &str, right_hint: &str) {
    if area.height == 0 {
        return;
    }
    Paragraph::new(modal_title_line(area.width, title, right_hint)).render(
        Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: 1,
        },
        buf,
    );
}

pub fn modal_header_line_area(header_area: Rect, row: u16) -> Option<Rect> {
    if header_area.height <= row {
        return None;
    }
    Some(Rect {
        x: header_area.x,
        y: header_area.y + row,
        width: header_area.width,
        height: 1,
    })
}

pub fn modal_search_line(
    query: &str,
    placeholder: &str,
    placeholder_color: Color,
    query_color: Color,
) -> Line<'static> {
    if query.is_empty() {
        let mut chars = placeholder.chars();
        let first = chars.next().unwrap_or(' ');
        let rest: String = chars.collect();
        Line::from(vec![
            Span::styled(" ", Style::default().fg(placeholder_color)),
            Span::styled(
                first.to_string(),
                Style::default()
                    .fg(placeholder_color)
                    .add_modifier(Modifier::UNDERLINED),
            ),
            Span::styled(rest, Style::default().fg(placeholder_color)),
        ])
    } else {
        Line::from(vec![Span::styled(
            format!(" {}", query),
            Style::default().fg(query_color),
        )])
    }
}

// ---------------------------------------------------------------------------
// The modal primitive
// ---------------------------------------------------------------------------

/// Everything [`modal_frame`] needs. A struct literal plus `..Default::default()`
/// rather than a builder: the caller writes the numbers it already has and
/// nothing else.
// No production reader yet — the call sites migrate in a later, golden-verified
// step. Until then this is the only unused public API in the module.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModalSpec<'a> {
    /// Modal title, rendered bold in themed text on the header row.
    pub title: &'a str,
    /// Dismissal hint, right-aligned on the bottom border.
    pub hint: &'a str,
    /// Desired width in cells. Bounded by [`modal_layout`].
    pub width: u16,
    /// Desired height in cells. Bounded by [`modal_layout`].
    ///
    /// **Content-derived height**: pass `content_rows + this dialog's own
    /// chrome` and let the clamp bound the result. The six call sites that size
    /// from content (`ask_user_dialog`, `history_search`, `message_selector`,
    /// `dialog_select`, `memory_file_selector`, `mcp_approval`) should stop
    /// calling `.min(area.height.saturating_sub(n))` themselves — that is this
    /// function's job.
    pub height: u16,
    /// Rows of the inner area above the body. `0` renders no title row.
    pub header_height: u16,
    /// Rows of the inner area below the body.
    pub footer_height: u16,
    /// Border colour. `theme_colors::accent()` by default; a semantic colour
    /// (`error()` for a warning modal, `success()` for a state modal) where the
    /// border itself carries meaning.
    pub border_fg: Color,
}

impl Default for ModalSpec<'_> {
    fn default() -> Self {
        Self {
            title: "",
            hint: HINT_ESC,
            width: 0,
            height: 0,
            header_height: 1,
            footer_height: 0,
            border_fg: theme_colors::accent(),
        }
    }
}

/// The modal primitive. Every pixel of a modal is painted here: dark overlay,
/// `Clear`, dialog background, the rounded border, the title row, and the hint.
///
/// Takes a `&mut Buffer` rather than a `&mut Frame` on purpose. In ratatui 0.30
/// `Frame::render_widget` is exactly `widget.render(area, self.buffer)` — there
/// is no dirty-region bookkeeping in the `Frame` path to lose — so one body
/// serves both sinks with no `_buf` fork. [`modal_frame`] is the `Frame`
/// adapter and is one expression wide; any logic added to it would fork the
/// primitive.
#[allow(dead_code)] // unwired until the call sites migrate; see `ModalSpec`
pub fn modal_frame_buf(buf: &mut Buffer, area: Rect, spec: &ModalSpec<'_>) -> ModalLayout {
    let layout = modal_layout(
        area,
        spec.width,
        spec.height,
        spec.header_height,
        spec.footer_height,
    );
    // jcode's overlay primitive, ported (jcode `ui_overlays.rs:15-21`): erase the
    // rect, draw a bordered block, render content into `block.inner()`.
    //
    // The erase is `Clear`, which is operant's name for jcode's `clear_area`
    // (`jcode-tui-render/src/chrome.rs:4-10` -- a full cell `.reset()`, so the
    // terminal's own background shows through).
    //
    // Two things this deliberately does NOT do, because jcode does not do them:
    //   - no scrim over the surrounding screen (`render_dark_overlay_buf`)
    //   - no dialog background fill (`render_dialog_bg_buf`)
    // A tinted dialog body was operant inventing a surface concept that jcode
    // does not have; jcode has no Surface/elevation/opacity at all, so a panel is
    // an erased rect plus a border and nothing else. Both helpers remain
    // available for `settings_screen`, which is not a jcode-shaped surface.
    Clear.render(layout.dialog_area, buf);

    // Square `Borders::ALL`, which is jcode's default (20 of 22 uses; the only
    // rounded borders in jcode are hand-rolled per-widget in jcode-tui-render).
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(spec.border_fg))
        .title_alignment(Alignment::Right);
    if !spec.hint.is_empty() {
        block = block.title_bottom(Line::styled(
            spec.hint.to_string(),
            Style::default().fg(theme_colors::muted()),
        ));
    }
    block.render(layout.dialog_area, buf);

    // The title is the `render_modal_title_frame` format with an empty hint: the
    // hint now lives on the bottom border, so the header row is title + pad.
    if layout.header_area.height > 0 {
        modal_title_line(layout.header_area.width, spec.title, "").render(
            Rect {
                height: 1,
                ..layout.header_area
            },
            buf,
        );
    }

    layout
}

/// `Frame` adapter over [`modal_frame_buf`]. One expression on purpose.
#[allow(dead_code)] // unwired until the call sites migrate; see `ModalSpec`
pub fn modal_frame(frame: &mut Frame, area: Rect, spec: &ModalSpec<'_>) -> ModalLayout {
    modal_frame_buf(frame.buffer_mut(), area, spec)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Margin;

    /// `modal_frame` and `modal_title_line` read the process-global palette
    /// (`theme_colors::accent/text/muted/panel_bg/overlay_bg`), so any test that
    /// renders them can observe another test's theme mid-assertion. Every other
    /// palette-reading test site takes `ACTIVE_LOCK` (`render/mod.rs`,
    /// `banner.rs`, `rustle.rs`, `app/tests.rs`, `app/commands.rs`); this module
    /// did not, which made the two Frame-vs-Buffer agreement tests fail
    /// intermittently in a parallel run with the default theme's
    /// `muted()` (Rgb 204,155,31) against nord's (Rgb 76,86,106).
    ///
    /// Taking the lock for the whole comparison is what makes those two tests
    /// sound: it guarantees no other test can swap the palette between the
    /// Frame half and the Buffer half.
    fn palette_guard() -> std::sync::MutexGuard<'static, ()> {
        crate::tui::theme_colors::tests::ACTIVE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    const LARGE: Rect = Rect {
        x: 0,
        y: 0,
        width: 120,
        height: 40,
    };

    /// A spread of areas and desired sizes: roomy, exactly at the clamp
    /// boundary, over-large, under-minimum, very narrow, empty, degenerate.
    /// Deliberately spelled as literals, not composed from the constants —
    /// these are the values a call site passes today, not a restatement of the
    /// formula under test.
    const MATRIX: &[(Rect, u16, u16, u16, u16)] = &[
        (LARGE, 80, 24, 2, 1),
        (
            Rect {
                x: 0,
                y: 0,
                width: 84,
                height: 28,
            },
            80,
            24,
            2,
            1,
        ),
        (
            Rect {
                x: 0,
                y: 0,
                width: 60,
                height: 20,
            },
            90,
            30,
            2,
            1,
        ),
        (LARGE, 4, 3, 2, 1),
        (
            Rect {
                x: 0,
                y: 0,
                width: 3,
                height: 40,
            },
            80,
            24,
            1,
            0,
        ),
        (
            Rect {
                x: 0,
                y: 0,
                width: 0,
                height: 0,
            },
            80,
            24,
            1,
            1,
        ),
        (
            Rect {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            },
            80,
            24,
            1,
            1,
        ),
    ];

    /// One entry per cell — border glyphs are multi-byte, so a `String` row
    /// would index by byte, not column.
    fn row_text(buf: &Buffer, y: u16) -> Vec<String> {
        (buf.area.x..buf.area.right())
            .map(|x| {
                buf.cell((x, y))
                    .map_or_else(String::new, |c| c.symbol().to_string())
            })
            .collect()
    }

    fn assert_cells_eq(from_frame: &Buffer, from_buf: &Buffer) {
        assert_eq!(from_frame.area, from_buf.area, "areas differ");
        assert_eq!(
            from_frame.content.len(),
            from_buf.content.len(),
            "cell counts differ"
        );
        for (i, (a, b)) in from_frame
            .content
            .iter()
            .zip(from_buf.content.iter())
            .enumerate()
        {
            assert_eq!(a.symbol(), b.symbol(), "symbol differs at cell {i}");
            assert_eq!(a.fg, b.fg, "fg differs at cell {i}");
            assert_eq!(a.bg, b.bg, "bg differs at cell {i}");
            assert_eq!(a.modifier, b.modifier, "modifier differs at cell {i}");
        }
    }

    /// Paint a spec through the `Frame` sink and hand back the resulting buffer.
    fn through_frame(area: Rect, spec: &ModalSpec<'_>, size: (u16, u16)) -> (Buffer, ModalLayout) {
        let mut terminal = Terminal::new(TestBackend::new(size.0, size.1)).unwrap();
        let mut layout = None;
        terminal
            .draw(|frame| {
                layout = Some(modal_frame(frame, area, spec));
            })
            .unwrap();
        let out = terminal.backend().buffer().clone();
        (out, layout.expect("draw ran the callback"))
    }

    fn through_buf(area: Rect, spec: &ModalSpec<'_>, size: (u16, u16)) -> (Buffer, ModalLayout) {
        let mut buf = Buffer::empty(Rect {
            x: 0,
            y: 0,
            width: size.0,
            height: size.1,
        });
        let layout = modal_frame_buf(&mut buf, area, spec);
        (buf, layout)
    }

    // --- geometry: the compatibility contract with begin_modal_* ------------

    /// The primitive's rects are exactly the ones `begin_modal_*` produces today,
    /// for every entry in the matrix. This is the byte-compatibility contract.
    #[test]
    fn modal_layout_matches_begin_modal_geometry() {
        for &(area, w, h, hh, fh) in MATRIX {
            let mut buf = Buffer::empty(area);
            let from_begin = begin_modal_buf(&mut buf, area, w, h, hh, fh);
            assert_eq!(
                modal_layout(area, w, h, hh, fh),
                from_begin,
                "modal_layout disagrees with begin_modal_buf for area {area:?} \
                 desired {w}x{h} header {hh} footer {fh}"
            );
        }
    }

    /// Same contract through the `Frame` sink, and byte-for-byte on the buffer:
    /// `begin_modal_frame` and `begin_modal_buf` are one body, not two.
    #[test]
    fn begin_modal_frame_and_begin_modal_buf_agree() {
        let _palette = palette_guard();
        for &(area, w, h, hh, fh) in MATRIX {
            if area.width == 0 || area.height == 0 {
                // A zero-area terminal cannot be drawn into; the Buffer path is
                // the only way to reach that case, and
                // `modal_layout_matches_begin_modal_geometry` covers it.
                continue;
            }
            let mut terminal =
                Terminal::new(TestBackend::new(area.width, area.height)).expect("terminal");
            let mut frame_layout = None;
            terminal
                .draw(|f| {
                    frame_layout = Some(begin_modal_frame(f, area, w, h, hh, fh));
                })
                .expect("draw");
            let from_frame = terminal.backend().buffer().clone();

            let mut raw = Buffer::empty(area);
            let raw_layout = begin_modal_buf(&mut raw, area, w, h, hh, fh);

            assert_eq!(
                frame_layout,
                Some(raw_layout),
                "layouts differ for {area:?}"
            );
            assert_cells_eq(&from_frame, &raw);
        }
    }

    #[test]
    fn modal_layout_pins_exact_rects_for_a_large_area() {
        assert_eq!(
            modal_layout(LARGE, 80, 24, 2, 1),
            ModalLayout {
                dialog_area: Rect {
                    x: 20,
                    y: 8,
                    width: 80,
                    height: 24
                },
                header_area: Rect {
                    x: 21,
                    y: 9,
                    width: 78,
                    height: 2
                },
                body_area: Rect {
                    x: 21,
                    y: 11,
                    width: 78,
                    height: 19
                },
                footer_area: Rect {
                    x: 21,
                    y: 30,
                    width: 78,
                    height: 1
                },
            }
        );
    }

    #[test]
    fn modal_layout_pins_exact_rects_at_the_clamp_boundary() {
        // area.width - 4 == desired_width and area.height - 4 == desired_height,
        // so the clamp is a no-op: 84x28 asking for 80x24.
        assert_eq!(
            modal_layout(
                Rect {
                    x: 0,
                    y: 0,
                    width: 84,
                    height: 28
                },
                80,
                24,
                2,
                1
            ),
            ModalLayout {
                dialog_area: Rect {
                    x: 2,
                    y: 2,
                    width: 80,
                    height: 24
                },
                header_area: Rect {
                    x: 3,
                    y: 3,
                    width: 78,
                    height: 2
                },
                body_area: Rect {
                    x: 3,
                    y: 5,
                    width: 78,
                    height: 19
                },
                footer_area: Rect {
                    x: 3,
                    y: 24,
                    width: 78,
                    height: 1
                },
            }
        );
    }

    #[test]
    fn modal_layout_pins_exact_rects_when_clamped() {
        // 90x30 wanted in 60x20: both axes clamp to area - space::M.
        assert_eq!(
            modal_layout(
                Rect {
                    x: 0,
                    y: 0,
                    width: 60,
                    height: 20
                },
                90,
                30,
                2,
                1
            ),
            ModalLayout {
                dialog_area: Rect {
                    x: 2,
                    y: 2,
                    width: 56,
                    height: 16
                },
                header_area: Rect {
                    x: 3,
                    y: 3,
                    width: 54,
                    height: 2
                },
                body_area: Rect {
                    x: 3,
                    y: 5,
                    width: 54,
                    height: 11
                },
                footer_area: Rect {
                    x: 3,
                    y: 16,
                    width: 54,
                    height: 1
                },
            }
        );
    }

    #[test]
    fn modal_layout_pins_exact_rects_when_floored() {
        // 4x3 wanted: both axes clamp up to the readable floor.
        assert_eq!(
            modal_layout(LARGE, 4, 3, 2, 1),
            ModalLayout {
                dialog_area: Rect {
                    x: 56,
                    y: 17,
                    width: 8,
                    height: 6
                },
                header_area: Rect {
                    x: 57,
                    y: 18,
                    width: 6,
                    height: 2
                },
                body_area: Rect {
                    x: 57,
                    y: 20,
                    width: 6,
                    height: 1
                },
                footer_area: Rect {
                    x: 57,
                    y: 21,
                    width: 6,
                    height: 1
                },
            }
        );
    }

    #[test]
    fn modal_layout_pins_exact_rects_on_a_very_narrow_area() {
        // Width 3 < MIN_MODAL_W: the floor wins, then `centered_rect` clamps the
        // dialog to the area, so the inner area is a single column.
        assert_eq!(
            modal_layout(
                Rect {
                    x: 0,
                    y: 0,
                    width: 3,
                    height: 40
                },
                80,
                24,
                1,
                0
            ),
            ModalLayout {
                dialog_area: Rect {
                    x: 0,
                    y: 8,
                    width: 3,
                    height: 24
                },
                header_area: Rect {
                    x: 1,
                    y: 9,
                    width: 1,
                    height: 1
                },
                body_area: Rect {
                    x: 1,
                    y: 10,
                    width: 1,
                    height: 21
                },
                footer_area: Rect {
                    x: 1,
                    y: 31,
                    width: 1,
                    height: 0
                },
            }
        );
    }

    #[test]
    fn modal_layout_pins_exact_rects_on_an_empty_area() {
        // Degenerate, but a call site really does reach it on a 0-row viewport.
        // `area.width.saturating_sub(4)` underflows to 0 and the `.max()` floors
        // win; `centered_rect` then shrinks the dialog to nothing.
        assert_eq!(
            modal_layout(
                Rect {
                    x: 0,
                    y: 0,
                    width: 0,
                    height: 0
                },
                80,
                24,
                1,
                1
            ),
            ModalLayout {
                dialog_area: Rect {
                    x: 0,
                    y: 0,
                    width: 0,
                    height: 0
                },
                header_area: Rect {
                    x: 1,
                    y: 1,
                    width: 0,
                    height: 0
                },
                body_area: Rect {
                    x: 1,
                    y: 1,
                    width: 0,
                    height: 0
                },
                footer_area: Rect {
                    x: 1,
                    y: 1,
                    width: 0,
                    height: 0
                },
            }
        );
    }

    #[test]
    fn modal_layout_pins_exact_rects_on_a_one_by_one_area() {
        // 1x1: `Rect::inner(Margin::new(1, 1))` would return `Rect::ZERO` here and
        // move x/y to 0. The hand-rolled inset keeps (1, 1) — pinned so the
        // substitution can never be made silently.
        assert_eq!(
            modal_layout(
                Rect {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1
                },
                80,
                24,
                1,
                1
            ),
            ModalLayout {
                dialog_area: Rect {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1
                },
                header_area: Rect {
                    x: 1,
                    y: 1,
                    width: 0,
                    height: 0
                },
                body_area: Rect {
                    x: 1,
                    y: 1,
                    width: 0,
                    height: 0
                },
                footer_area: Rect {
                    x: 1,
                    y: 1,
                    width: 0,
                    height: 0
                },
            }
        );
    }

    // --- clamping / flooring -----------------------------------------------

    #[test]
    fn desired_larger_than_available_clamps_to_the_area_minus_one_margin() {
        let l = modal_layout(LARGE, u16::MAX, u16::MAX, 1, 1);
        assert_eq!(l.dialog_area.width, LARGE.width - space::M);
        assert_eq!(l.dialog_area.height, LARGE.height - space::M);
    }

    #[test]
    fn desired_smaller_than_the_minimum_floors() {
        let l = modal_layout(LARGE, 0, 0, 0, 0);
        assert_eq!(l.dialog_area.width, MIN_MODAL_W);
        assert_eq!(l.dialog_area.height, MIN_MODAL_H);
    }

    #[test]
    fn content_derived_heights_are_bounded_by_the_primitive() {
        // What the six content-sized call sites will do: hand over
        // `content_rows + chrome` and let the clamp bound it. No caller-side
        // `.min()` is involved.
        let tall = modal_layout(LARGE, 72, 200, 2, 1);
        assert_eq!(tall.dialog_area.height, LARGE.height - space::M);
        let short = modal_layout(LARGE, 72, 3, 2, 1);
        assert_eq!(short.dialog_area.height, MIN_MODAL_H);
    }

    // --- both sinks ---------------------------------------------------------

    /// The `_buf` duplication class of bug: title, hint, border, overlay, and
    /// background must land on the same cells through `Frame` and through
    /// `Buffer`. If anyone adds a `Frame`-only step to `modal_frame_buf` (or a
    /// `Buffer`-only step to `modal_frame`), this fails.
    #[test]
    fn modal_frame_and_modal_frame_buf_paint_identical_cells() {
        let _palette = palette_guard();
        for &(area, w, h, hh, fh) in MATRIX {
            if area.width == 0 || area.height == 0 {
                continue; // no zero-area terminal; covered by the geometry tests
            }
            let spec = ModalSpec {
                title: "Plugins",
                hint: HINT_ESC,
                width: w,
                height: h,
                header_height: hh,
                footer_height: fh,
                ..Default::default()
            };
            let size = (area.width, area.height);
            let (from_frame, frame_layout) = through_frame(area, &spec, size);
            let (from_buf, buf_layout) = through_buf(area, &spec, size);
            assert_eq!(
                frame_layout, buf_layout,
                "sinks disagree on the layout for {area:?}"
            );
            assert_cells_eq(&from_frame, &from_buf);
        }
    }

    /// Same contract for the pre-existing title pair the primitive shares its
    /// formatting with. If `modal_title_line` drifts from what
    /// `render_modal_title_frame` used to paint, this fails.
    ///
    /// Two halves on purpose. The buffer comparison only proves the two
    /// renderers agree with *each other* — both now share one composer, so a
    /// change to the composer would move them together and the comparison would
    /// stay green. The literal row is the real pin: it is the exact output the
    /// pre-refactor bodies produced, so the shared format cannot be changed
    /// silently.
    #[test]
    fn modal_title_line_matches_the_frame_renderer() {
        let _palette = palette_guard();
        let area = Rect {
            x: 4,
            y: 2,
            width: 40,
            height: 1,
        };
        let mut terminal = Terminal::new(TestBackend::new(60, 10)).unwrap();
        terminal
            .draw(|f| {
                render_modal_title_frame(f, area, "Choose a theme", "esc");
            })
            .unwrap();
        let from_frame = terminal.backend().buffer().clone();

        let mut raw = Buffer::empty(Rect {
            x: 0,
            y: 0,
            width: 60,
            height: 10,
        });
        render_modal_title_buf(&mut raw, area, "Choose a theme", "esc");
        assert_cells_eq(&from_frame, &raw);

        // Byte-exact title row, as painted before the collapse: one leading
        // space, the 14-cell title, 20 cells of pad (`40 - (14 + 3 + 3)`), the
        // hint, then the 2 cells the 40-wide row had left over.
        let painted = row_text(&raw, area.y);
        assert_eq!(
            painted[area.x as usize..area.right() as usize].concat(),
            format!(" Choose a theme{}esc{}", " ".repeat(20), " ".repeat(2))
        );

        // And the exact span layout the old bodies produced.
        let line = modal_title_line(area.width, "Choose a theme", "esc");
        assert_eq!(line.spans.len(), 3);
        assert_eq!(line.spans[0].content.as_ref(), " Choose a theme");
        assert_eq!(line.spans[1].content.as_ref(), " ".repeat(20));
        assert_eq!(line.spans[2].content.as_ref(), "esc");
        assert_eq!(line.spans[0].style.add_modifier, Modifier::BOLD);
        assert_eq!(line.spans[2].style.fg, Some(theme_colors::muted()));
    }

    #[test]
    fn title_padding_underflows_to_zero_instead_of_panicking() {
        let _palette = palette_guard();
        // Title wider than the row: `saturating_sub` must clamp, and the hint is
        // simply pushed off the end.
        let line = modal_title_line(2, "a very long title indeed", "esc");
        assert_eq!(line.spans[1].content.as_ref(), "");
        assert_eq!(line.spans[2].content.as_ref(), "esc");
    }

    // --- what the primitive paints ----------------------------------------

    #[test]
    fn modal_frame_draws_a_square_border_and_a_right_aligned_hint() {
        let _palette = palette_guard();
        let area = LARGE;
        let spec = ModalSpec {
            title: "Plugins",
            hint: HINT_ESC,
            width: 72,
            height: 20,
            header_height: 2,
            footer_height: 1,
            ..Default::default()
        };
        let (buf, layout) = through_buf(area, &spec, (area.width, area.height));
        let d = layout.dialog_area;

        // Square corners, not rounded: jcode's modal border is plain
        // `Borders::ALL` (20 of 22 uses across jcode). The only rounded borders
        // in jcode are hand-rolled per-widget in `jcode-tui-render`, and this
        // dialog is not one of them.
        assert_eq!(buf.cell((d.x, d.y)).map(|c| c.symbol()), Some("┌"));
        assert_eq!(
            buf.cell((d.right() - 1, d.y)).map(|c| c.symbol()),
            Some("┐")
        );
        assert_eq!(
            buf.cell((d.x, d.bottom() - 1)).map(|c| c.symbol()),
            Some("└")
        );
        assert_eq!(
            buf.cell((d.right() - 1, d.bottom() - 1))
                .map(|c| c.symbol()),
            Some("┘")
        );

        // Title on the header row, one leading space, bold. The row starts at
        // the dialog's left border, so the title begins one cell in from it.
        let title_row = row_text(&buf, layout.header_area.y);
        let at = layout.header_area.x as usize;
        assert_eq!(
            title_row[at..at + spec.title.len() + 1].concat(),
            " Plugins",
            "title row was {title_row:?}"
        );

        // Hint right-aligned on the bottom border, in the cells immediately left
        // of the bottom-right corner, which stays a corner.
        let row = row_text(&buf, d.bottom() - 1);
        let cells: Vec<&str> = row[d.x as usize..d.right() as usize]
            .iter()
            .map(String::as_str)
            .collect();
        assert_eq!(cells.first(), Some(&"└"), "bottom-left corner");
        assert_eq!(cells.last(), Some(&"┘"), "bottom-right corner");
        let n = HINT_ESC.len();
        assert_eq!(
            cells[cells.len() - 1 - n..cells.len() - 1].concat(),
            HINT_ESC,
            "bottom border row was {row:?}"
        );
    }

    #[test]
    fn modal_frame_with_a_zero_header_renders_no_title_row() {
        let _palette = palette_guard();
        let spec = ModalSpec {
            title: "Plugins",
            width: 72,
            height: 20,
            header_height: 0,
            ..Default::default()
        };
        let (buf, layout) = through_buf(LARGE, &spec, (LARGE.width, LARGE.height));
        assert_eq!(layout.header_area.height, 0);
        assert!(
            !row_text(&buf, layout.body_area.y)
                .concat()
                .contains("Plugins"),
            "title leaked into a zero-height header"
        );
    }

    #[test]
    fn modal_frame_uses_the_canonical_esc_hint_by_default() {
        let _palette = palette_guard();
        assert_eq!(ModalSpec::default().hint, "Esc to close");
        assert_eq!(ModalSpec::default().hint, HINT_ESC);
    }

    #[test]
    fn modal_frame_leaves_the_surface_under_it_untouched() {
        let _palette = palette_guard();
        // jcode has no Surface/elevation concept: a modal is an erased rect plus
        // a border, with NO scrim over the surrounding screen and NO dialog fill
        // (jcode `ui_overlays.rs:15-21` -- `clear_area` then `Borders::ALL` then
        // content into `block.inner()`). So the separation from the surface
        // under it comes from the erase + the border, not from tinting.
        //
        // This is the regression guard for the two things we used to paint here.
        let spec = ModalSpec {
            title: "Plugins",
            width: 40,
            height: 10,
            ..Default::default()
        };
        let (buf, layout) = through_buf(LARGE, &spec, (LARGE.width, LARGE.height));

        let outside = buf.cell((0, 0)).expect("cell");
        assert_ne!(
            outside.bg,
            theme_colors::overlay_bg(),
            "no scrim may be painted outside the dialog (jcode parity)"
        );

        let inside = buf
            .cell((layout.dialog_area.x + 2, layout.dialog_area.y + 2))
            .expect("cell");
        assert_ne!(
            inside.bg,
            theme_colors::panel_bg(),
            "no dialog background fill may be painted (jcode parity)"
        );
    }

    // --- the spacing scale --------------------------------------------------

    #[test]
    fn spacing_scale_is_a_doubling_ladder() {
        assert_eq!(space::XXS, 0, "XXS is the no-inset step");
        assert_eq!(space::XS * 2, space::S);
        assert_eq!(space::S * 2, space::M);
        assert_eq!(space::M * 2, space::L);
        assert_eq!(space::L * 2, space::XL);
    }

    #[test]
    fn modal_floors_and_margin_are_expressed_in_space_steps() {
        assert_eq!(MIN_MODAL_W, space::L);
        // The vertical floor is deliberately not a space step: 6 = 5 content
        // rows + 2 border rows. Asserted so a later "tidy the ladder" edit that
        // silently moves it shows up here.
        assert_eq!(MIN_MODAL_H, 6);
    }

    #[test]
    fn pad_helpers_set_one_axis_only() {
        assert_eq!(space::pad_h(space::M), Margin::new(space::M, 0));
        assert_eq!(space::pad_v(space::M), Margin::new(0, space::M));
    }

    // --- no second copy of the sizing logic --------------------------------

    /// The clamp exists once in this file. A second copy — the exact failure
    /// mode 16 divergent formulas came from — fails here. The needles are
    /// assembled at runtime so this test's own source does not contain them.
    #[test]
    fn modal_clamp_formula_is_written_once() {
        let src = include_str!("layout.rs");
        for needle in [
            ["space::pad_h", "(space::M)", ".horizontal"].concat(),
            ["space::pad_v", "(space::M)", ".vertical"].concat(),
            [".max(MIN_MODAL_", "W)"].concat(),
            [".max(MIN_MODAL_", "H)"].concat(),
        ] {
            assert_eq!(
                src.matches(needle.as_str()).count(),
                1,
                "the sizing logic is duplicated: {needle:?} appears more than once \
                 in layout.rs. Route the site through `modal_layout`."
            );
        }
    }

    /// `begin_modal_*` must route through the one sizing function, not carry a
    /// private copy of it. The needle is assembled from fragments so this
    /// test's own source does not contain it.
    #[test]
    fn begin_modal_paths_delegate_to_modal_layout() {
        let src = include_str!("layout.rs");
        let delegate = [
            "modal_layout",
            "(area, width, height, header_height",
            ", footer_height)",
        ]
        .concat();
        assert_eq!(
            src.matches(delegate.as_str()).count(),
            2,
            "begin_modal_frame and begin_modal_buf must each call modal_layout once"
        );
        let private_copy = ["compute_modal_", "layout"].concat();
        assert!(
            !src.contains(private_copy.as_str()),
            "the private sizing copy is back"
        );
    }

    /// The `Frame` adapter must stay an adapter: no logic, no second body.
    /// Counting statements rather than only nested `fn`s, because a `Frame`-only
    /// step can be a plain `if` — and a step that happens to be idempotent
    /// (`Clear` on an area already cleared) would slip past the cell-parity
    /// test.
    #[test]
    fn modal_frame_is_a_one_expression_adapter() {
        let src = include_str!("layout.rs");
        let body = src
            .split("pub fn modal_frame(")
            .nth(1)
            .expect("modal_frame is defined")
            .split("\n}")
            .next()
            .expect("modal_frame has a body");
        assert_eq!(
            body.matches("modal_frame_buf(").count(),
            1,
            "modal_frame must delegate to modal_frame_buf"
        );
        assert_eq!(
            body.matches(';').count(),
            0,
            "modal_frame must be a single tail expression, not a body: {body}"
        );
        assert_eq!(
            body.matches("fn ").count(),
            0,
            "modal_frame grew a nested function: {body}"
        );
    }

    /// The title pair has the same one-body rule. `render_modal_title_frame` must
    /// be a single delegation, and the `Rect` it used to build inline must not
    /// reappear there — that inline `Rect` was the duplicated half.
    #[test]
    fn title_renderers_are_one_expression_adapters() {
        let src = include_str!("layout.rs");
        let body = src
            .split("pub fn render_modal_title_frame(")
            .nth(1)
            .expect("render_modal_title_frame is defined")
            .split("\n}")
            .next()
            .expect("render_modal_title_frame has a body");
        assert_eq!(
            body.matches("render_modal_title_buf(").count(),
            1,
            "render_modal_title_frame must delegate to render_modal_title_buf"
        );
        assert_eq!(
            body.matches(';').count(),
            1,
            "render_modal_title_frame must be a single statement: {body}"
        );
        for needle in ["Paragraph", "height: 1", "fn "] {
            assert!(
                !body.contains(needle),
                "render_modal_title_frame grew a `{needle}` — the body is duplicated: {body}"
            );
        }
    }
}
