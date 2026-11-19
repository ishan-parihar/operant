// ask_user_dialog.rs — TUI overlay for model-initiated questions.
//
// Rendered when the model calls the `AskUserQuestion` tool.  The dialog
// shows the question text, an optional list of predefined choices that the
// user can navigate with arrow keys or number shortcuts, and a free-text
// input line for a custom answer.
//
// Layout:
//   ╭─ Question ──────────────────────────── Esc to close ─╮
//   │                                                 │
//   │  How should the tests be run?                   │
//   │                                                 │
//   │  ▶ 1  cargo test --workspace                    │
//   │    2  cargo test -p operant-api                 │
//   │    3  cargo test --features dev_full            │
//   │                                                 │
//   │  ❯ _                              (custom)      │
//   │                                                 │
//   │  Tab/↑↓: navigate   Enter: confirm   Esc: skip  │
//   ╰─────────────────────────────────────────────────╯

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

use crate::tui::overlays::{HINT_ESC, ModalSpec, modal_frame_buf, modal_layout};
use crate::tui::render::balanced_wrap;
use crate::tui::theme_colors;
use crate::tui::vendor::style::theme;

/// Desired width. `modal_layout` owns the clamp and the `MIN_MODAL_W` floor.
const DIALOG_WIDTH: u16 = 58;

/// State for the ask-user question dialog overlay.
#[derive(Default)]
pub struct AskUserDialogState {
    /// Whether the dialog is currently visible.
    pub visible: bool,
    /// The question text from the model.
    pub question: String,
    /// Optional predefined choices.
    pub options: Option<Vec<String>>,
    /// Index of the currently highlighted option (0 = custom-text row when
    /// options is None, or indices into options vec, with the custom row last).
    pub selected_idx: usize,
    /// Custom text the user is typing (if they choose not to pick an option).
    pub custom_text: String,
    /// Whether cursor is in the custom-text input row.
    pub in_custom_input: bool,
    /// Pending reply channel sender — set when the dialog opens, consumed on submit.
    pub(crate) reply_tx: Option<tokio::sync::oneshot::Sender<String>>,
    /// One-line header shown above the question: the surface's own plain
    /// fallback (plan M3, "one new field: the surface's to_plain_fallback()
    /// header"). `clarify` leaves it empty, so its dialog is byte-identical
    /// to before; `ui_request_input` fills it with the card the model built,
    /// so the operator sees the FULL surface (a table the choice sits under)
    /// rather than a bare question.
    pub header: String,
    /// Whether the header shows the raw JSON op instead of the rendered
    /// card (B0, iter-2652; the plan's G2 `[v]` toggle). The raw form is
    /// the POST-SCRUB op — the gate already ran, so showing it exposes the
    /// masked strings, never the secrets.
    pub show_raw: bool,
    /// The POST-SCRUB op JSON behind `header`, when the surface supplied one
    /// (`ui_request_input` does; `clarify` does not). `[v]` toggles between
    /// the rendered card and this raw form; a dialog without one ignores
    /// the toggle, because there is nothing rawer to show.
    pub header_op_json: Option<String>,
}

impl AskUserDialogState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Open the dialog with a question and optional choices.
    pub fn open(
        &mut self,
        question: String,
        options: Option<Vec<String>>,
        reply_tx: tokio::sync::oneshot::Sender<String>,
    ) {
        self.open_with_header(question, options, String::new(), reply_tx);
    }

    /// Open the dialog with a header card above the question (plan M3).
    ///
    /// The header is the surface's plain fallback: what `ui_request_input`
    /// built before the question. `clarify` uses [`Self::open`] and passes
    /// none, so its dialog is unchanged.
    pub fn open_with_header(
        &mut self,
        question: String,
        options: Option<Vec<String>>,
        header: String,
        reply_tx: tokio::sync::oneshot::Sender<String>,
    ) {
        self.question = question;
        self.options = options;
        self.selected_idx = 0;
        self.custom_text.clear();
        self.in_custom_input = self.options.is_none();
        self.header = header;
        self.show_raw = false;
        self.header_op_json = None;
        self.reply_tx = Some(reply_tx);
        self.visible = true;
    }

    /// Whether the header has a raw op to flip to. `clarify` (no header)
    /// and any surface without JSON behind it answer false, so the `[v]`
    /// key arm never claims a toggle that cannot do anything.
    pub fn header_has_raw_op(&self) -> bool {
        self.header_op_json.is_some()
    }

    /// Flip the header between the rendered card and the raw post-scrub op
    /// JSON (B0, iter-2652; the plan's `[v]` raw toggle). The raw view is
    /// what a bug report needs: the exact op the model emitted, not the
    /// renderer's idea of it.
    ///
    /// A dialog without an op behind the header (`clarify`, or a surface
    /// whose JSON failed to serialize) ignores the toggle rather than
    /// flipping a flag nothing renders — the key arm gates on
    /// [`Self::header_has_raw_op`] anyway, so this is the second lock on the
    /// same door.
    pub fn toggle_raw(&mut self) {
        if self.header_op_json.is_some() {
            self.show_raw = !self.show_raw;
        }
    }

    /// Navigate selection up.
    pub fn select_prev(&mut self) {
        let n = self.option_count();
        if n == 0 {
            return;
        }
        if self.selected_idx == 0 {
            self.selected_idx = n; // wrap to custom row
            self.in_custom_input = true;
        } else {
            self.selected_idx -= 1;
            self.in_custom_input = self.selected_idx >= self.options_len();
        }
    }

    /// Navigate selection down.
    pub fn select_next(&mut self) {
        let n = self.option_count();
        if n == 0 {
            return;
        }
        if self.selected_idx >= n {
            self.selected_idx = 0;
            self.in_custom_input = false;
        } else {
            self.selected_idx += 1;
            self.in_custom_input = self.selected_idx >= self.options_len();
        }
    }

    /// Select an option directly by 1-based number key.
    pub fn select_by_number(&mut self, n: usize) {
        if let Some(ref opts) = self.options
            && n >= 1
            && n <= opts.len()
        {
            self.selected_idx = n - 1;
            self.in_custom_input = false;
        }
    }

    /// Append a character to the custom-text input.
    ///
    /// Any printable character auto-switches to the custom row regardless of
    /// where the selection currently is — so the user can just start typing
    /// without having to navigate down with Tab/↓ first.
    pub fn push_char(&mut self, c: char) {
        self.custom_text.push(c);
        self.in_custom_input = true;
        self.selected_idx = self.options_len();
    }

    /// Backspace in the custom-text input.
    pub fn pop_char(&mut self) {
        if self.in_custom_input || self.options.is_none() {
            self.custom_text.pop();
        }
    }

    /// Confirm the current selection and send the answer.
    ///
    /// Returns `true` if the dialog was successfully submitted (i.e. a reply
    /// channel was present).
    pub fn confirm(&mut self) -> bool {
        let answer = if self.in_custom_input || self.options.is_none() {
            self.custom_text.clone()
        } else if let Some(ref opts) = self.options {
            opts.get(self.selected_idx).cloned().unwrap_or_default()
        } else {
            self.custom_text.clone()
        };

        self.send_reply(answer)
    }

    /// Dismiss without answering (sends an empty string so the tool result
    /// signals "user dismissed").
    pub fn dismiss(&mut self) -> bool {
        self.send_reply(String::new())
    }

    fn send_reply(&mut self, answer: String) -> bool {
        self.visible = false;
        if let Some(tx) = self.reply_tx.take() {
            let _ = tx.send(answer);
            true
        } else {
            false
        }
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn options_len(&self) -> usize {
        self.options.as_ref().map(|v| v.len()).unwrap_or(0)
    }

    /// Total number of selectable rows: options + custom-text row.
    fn option_count(&self) -> usize {
        self.options_len() + 1
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Render the ask-user question dialog into the terminal buffer.
///
/// Call this only when `state.visible` is true; typically from `render_app`.
pub fn render_ask_user_dialog(state: &AskUserDialogState, area: Rect, buf: &mut Buffer) {
    if !state.visible {
        return;
    }

    // ---- size estimate ----
    // Prose: balanced wrap, so the estimate below matches the rendered line
    // count (both use `balanced_wrap`).
    let question_lines = balanced_wrap(&state.question, 52).len() as u16;
    let options_lines = state
        .options
        .as_ref()
        .map(|v| v.len() as u16 + 1)
        .unwrap_or(0);
    // The header card (plan M3) only costs rows when it carries text: a
    // `clarify` dialog has none, so its geometry is unchanged by this field.
    let header_lines = if state.header.trim().is_empty() {
        0
    } else {
        balanced_wrap(&state.header, 52).len() as u16 + 1
    };
    // One title row + the content rows + the hint row, plus the two border rows
    // the primitive insets. `modal_layout` bounds it; nothing clamps here.
    let desired_height = 1 + 5 + header_lines + question_lines + options_lines + 3;

    // The wrap width and the option-row budget both depend on the *clamped*
    // width, which only `modal_layout` knows. Ask it once for the geometry,
    // then let `modal_frame_buf` recompute the identical layout to paint the
    // chrome — same inputs, same rects.
    let probe = modal_layout(area, DIALOG_WIDTH, desired_height, 1, 0);
    let inner_w = probe.body_area.width as usize;

    // ---- frame (overlay, background, rounded border, title, hint) ----
    let layout = modal_frame_buf(
        buf,
        area,
        &ModalSpec {
            title: "Question",
            hint: HINT_ESC,
            width: DIALOG_WIDTH,
            height: desired_height,
            header_height: 1,
            footer_height: 0,
            ..Default::default()
        },
    );

    // ---- inner content area ----
    // The title now occupies the row the old top padding used to leave blank,
    // so the content starts on the body's first row instead of one below it.
    let inner = layout.body_area;
    let last_row = inner.y + inner.height;
    let mut row = inner.y;

    macro_rules! write_line {
        ($row:expr, $line:expr) => {{
            if $row < last_row {
                let r = Rect {
                    x: inner.x,
                    y: $row,
                    width: inner.width,
                    height: 1,
                };
                Paragraph::new($line).render(r, buf);
            }
        }};
    }

    // Header card (plan M3): the surface the choice sits under. Dim, so the
    // question below it stays the brightest thing in the dialog. Only drawn
    // when a surface supplied one — `clarify` passes none.
    //
    // B0 (iter-2652): `[v]` flips this between the rendered plain card and
    // the raw POST-SCRUB op JSON — the form a bug report needs. The gate ran
    // before the op got here, so the raw view only ever shows masked text.
    if state.show_raw
        && let Some(json) = state.header_op_json.as_deref()
    {
        for wrap_line in balanced_wrap(json, inner_w) {
            write_line!(
                row,
                Line::from(Span::styled(
                    wrap_line,
                    Style::default()
                        .fg(theme::dim_color())
                        .bg(theme_colors::panel_bg())
                ))
            );
            row += 1;
            if row >= last_row {
                return;
            }
        }
        row += 1; // spacer between the card and the question
        if row >= last_row {
            return;
        }
    } else if !state.header.trim().is_empty() {
        for wrap_line in balanced_wrap(&state.header, inner_w) {
            write_line!(
                row,
                Line::from(Span::styled(
                    wrap_line,
                    Style::default()
                        .fg(theme::dim_color())
                        .bg(theme_colors::panel_bg())
                ))
            );
            row += 1;
            if row >= last_row {
                return;
            }
        }
        row += 1; // spacer between the card and the question
        if row >= last_row {
            return;
        }
    }

    // Question text
    for wrap_line in balanced_wrap(&state.question, inner_w) {
        write_line!(
            row,
            Line::from(Span::styled(
                wrap_line,
                Style::default()
                    .fg(theme::ai_text())
                    .bg(theme_colors::panel_bg())
            ))
        );
        row += 1;
        if row >= last_row {
            return;
        }
    }

    // Spacer
    row += 1;

    // Option rows
    if let Some(ref opts) = state.options {
        for (i, opt) in opts.iter().enumerate() {
            if row >= last_row.saturating_sub(2) {
                break;
            }
            let is_sel = !state.in_custom_input && state.selected_idx == i;
            let prefix = if is_sel { "▶ " } else { "  " };
            let num_str = format!("{}", i + 1);
            let label = format!(" {}", opt);
            let style_bg = if is_sel {
                theme::selection_bg_color()
            } else {
                theme_colors::panel_bg()
            };
            // A selection bar is a filled surface, so its foreground is DERIVED
            // from that surface. `theme::user_bg()` was the old answer and it
            // equals `selection_bg()` in 8 of the 100 bundled palettes —
            // `dark-cyberpunk` paints `#1B1B2A` on `#1B1B2A`, leaving the
            // selected option with no readable text.
            let sel_fg = theme_colors::on_selection_bg();
            write_line!(
                row,
                Line::from(vec![
                    Span::styled(
                        prefix,
                        Style::default()
                            .fg(if is_sel { sel_fg } else { theme::dim_color() })
                            .bg(style_bg)
                    ),
                    Span::styled(
                        num_str,
                        // Gated on `is_sel`, like the prefix and the label
                        // beside it. `dim_color()` is the Dim role, and on a
                        // selection fill it is the fill: 1.00:1 on
                        // `dark-dracula` (`#44475A` on `#44475A`), 1.54:1 on
                        // the SHIPPED default theme, under 3:1 on 42 of the
                        // 98 bundled palettes. The option NUMBER is how the
                        // operator aims, so this is the worst span to lose.
                        Style::default()
                            .fg(if is_sel {
                                theme_colors::on_selection_bg()
                            } else {
                                theme::dim_color()
                            })
                            .bg(style_bg)
                    ),
                    Span::styled(
                        label,
                        Style::default()
                            .fg(if is_sel { sel_fg } else { theme::ai_text() })
                            .bg(style_bg)
                            .add_modifier(if is_sel {
                                Modifier::BOLD
                            } else {
                                Modifier::empty()
                            })
                    ),
                ])
            );
            row += 1;
        }
        row += 1; // spacer before custom row
    }

    // Custom input row
    if row < last_row.saturating_sub(1) {
        let is_sel = state.in_custom_input || state.options.is_none();
        let prefix = if is_sel { "❯ " } else { "  " };
        let cursor = if is_sel { "█" } else { "" };
        let style_bg = if is_sel {
            theme::selection_bg_color()
        } else {
            theme_colors::panel_bg()
        };
        let mut spans = vec![Span::styled(
            prefix,
            Style::default()
                .fg(if is_sel {
                    theme_colors::on_selection_bg()
                } else {
                    theme::dim_color()
                })
                .bg(style_bg),
        )];
        if state.custom_text.is_empty() && !is_sel && state.options.is_some() {
            // Not yet active: show a subtle prompt so user knows they can type
            spans.push(Span::styled(
                "type to fill custom answer…",
                Style::default().fg(theme::dim_color()).bg(style_bg),
            ));
        } else {
            // Typed characters are the user's text.
            let display_text = format!("{}{}", state.custom_text, cursor);
            spans.push(Span::styled(
                display_text,
                // Same rule as the option rows: the fill is a selection fill
                // when this row is active, so the ink comes from the fill.
                // `user_text()` is White on 93 of the 100 bundled palettes,
                // which measures 1.06:1 on `light-honeycomb`'s `#FFF8E1`.
                Style::default()
                    .fg(if is_sel {
                        theme_colors::on_selection_bg()
                    } else {
                        theme::user_text()
                    })
                    .bg(style_bg),
            ));
        }
        write_line!(row, Line::from(spans));
        row += 1;
    }

    // Hint row
    row += 1;
    if row < last_row {
        // (iter-1935) The options hint must FIT the panel. It read
        // "  type: custom   ↑↓/Tab: options   Enter: confirm   Esc: skip" — 61
        // columns against a 54-column inner area — so the border silently cut
        // the tail and the operator's hint ended at "Esc:" with no "skip"
        // after it. A clipped claim is a half-told one, so the wording is now
        // measured against `inner_w` and the custom-text tail (which the row
        // above already advertises as "type to fill custom answer…") is gone.
        let hint = if state.options.is_some() {
            // 46 columns: ↑↓/Tab pick, Enter confirm, Esc skip.
            "  ↑↓/Tab: options   Enter: confirm   Esc: skip"
        } else {
            "  Type answer, then Enter to confirm   Esc: skip"
        };
        write_line!(
            row,
            Line::from(Span::styled(
                hint,
                Style::default()
                    .fg(theme::dim_color())
                    .bg(theme_colors::panel_bg())
            ))
        );
    }

    let _ = row;
}

// ---------------------------------------------------------------------------
// Word-wrap helper
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::balanced_wrap;
    use super::*;

    #[test]
    fn word_wrap_uses_display_width_not_bytes() {
        // Two CJK words (each glyph is 3 bytes but 2 display columns).
        // "中文 中文" is 4 glyphs = 8 columns + 1 space = 9 columns; it fits
        // in width 9 on one line. A byte-length wrapper would see 13 bytes
        // and wrap it incorrectly.
        let lines = balanced_wrap("中文 中文", 9);
        assert_eq!(lines, vec!["中文 中文".to_string()]);

        // At width 4 (one CJK word = 4 columns) each word takes its own line.
        let lines = balanced_wrap("中文 中文", 4);
        assert_eq!(lines, vec!["中文".to_string(), "中文".to_string()]);
    }

    #[test]
    fn word_wrap_ascii_unchanged() {
        assert_eq!(
            balanced_wrap("the quick brown fox", 9),
            vec!["the quick".to_string(), "brown fox".to_string()]
        );
    }

    /// `open` leaves the header empty, so the `clarify` dialog's geometry and
    /// pixels are unchanged by the plan-M3 field. This is the compatibility
    /// half of the two-producer contract.
    #[test]
    fn plain_open_clears_the_surface_header() {
        let mut state = AskUserDialogState::new();
        state.header = "a stale card".to_string();
        let (tx, _rx) = tokio::sync::oneshot::channel();
        state.open("Pick one".to_string(), None, tx);
        assert!(
            state.header.is_empty(),
            "clarify's path must not inherit a previous surface's header"
        );
    }

    /// `open_with_header` carries the surface card into the dialog, and the
    /// answer channel is the one the caller awaits.
    #[test]
    fn open_with_header_keeps_the_surface_card() {
        let mut state = AskUserDialogState::new();
        let (tx, _rx) = tokio::sync::oneshot::channel();
        state.open_with_header(
            "Pick one".to_string(),
            Some(vec!["A".to_string(), "B".to_string()]),
            "| name | qty |".to_string(),
            tx,
        );
        assert_eq!(state.header, "| name | qty |");
        assert_eq!(
            state.options.as_deref(),
            Some(&["A".to_string(), "B".to_string()][..])
        );
        assert!(state.visible);
    }

    /// B0 (iter-2652): a fresh dialog has nothing raw to show, so the `[v]`
    /// arm's predicate is false and the toggle is a no-op. A dialog opened
    /// by `clarify` (no op JSON) must not claim the toggle.
    #[test]
    fn clarify_dialog_has_no_raw_op_to_toggle() {
        let mut state = AskUserDialogState::new();
        let (tx, _rx) = tokio::sync::oneshot::channel();
        state.open("Pick one".to_string(), None, tx);
        assert!(
            !state.header_has_raw_op(),
            "clarify's dialog has no surface op; [v] must not claim one"
        );
        state.toggle_raw();
        assert!(!state.show_raw, "a no-op toggle must not flip the flag");
    }

    /// B0 (iter-2652): the `[v]` toggle flips between the rendered card and
    /// the raw POST-SCRUB op. The op JSON is whatever the caller serialized
    /// (the gate ran upstream), so this test pins the MECHANICS: flip on,
    /// flip off, and the raw view only ever shows what it was given.
    #[test]
    fn raw_toggle_flips_the_surface_header() {
        let mut state = AskUserDialogState::new();
        let (tx, _rx) = tokio::sync::oneshot::channel();
        state.open_with_header(
            "Pick one".to_string(),
            Some(vec!["A".to_string(), "B".to_string()]),
            "1. A\n2. B".to_string(),
            tx,
        );
        assert!(!state.show_raw);
        assert!(!state.header_has_raw_op());
        state.header_op_json = Some("{\"op\":\"create_surface\"}".to_string());
        assert!(state.header_has_raw_op());
        state.toggle_raw();
        assert!(state.show_raw, "the toggle must flip ON");
        state.toggle_raw();
        assert!(!state.show_raw, "the toggle must flip OFF");
    }

    /// B0 (iter-2652): the `[v]` raw view must render the POST-SCRUB op —
    /// feed a card whose op carries a scrubbed secret and assert the drawn
    /// bytes contain the mask, never the key. This is the G0 promise: the
    /// toggle cannot leak what the gate removed.
    #[test]
    fn raw_toggle_shows_scrubbed_op_never_the_secret() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut state = AskUserDialogState::new();
        let (tx, _rx) = tokio::sync::oneshot::channel();
        state.open_with_header(
            "Continue?".to_string(),
            Some(vec!["yes".to_string(), "no".to_string()]),
            "ready".to_string(),
            tx,
        );
        // The post-scrub op: the gate already replaced the key.
        state.header_op_json = Some(
            "{\"op\":\"create_surface\",\"surface_id\":\"s1\",\"body\":{\"node\":\"text\",\"text\":\"[REDACTED]\"}}"
                .to_string(),
        );
        state.show_raw = true;

        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|f| super::render_ask_user_dialog(&state, f.area(), f.buffer_mut()))
            .unwrap();
        let drawn: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            drawn.contains("[REDACTED]"),
            "the raw view must show the post-scrub op: {drawn:?}"
        );
        assert!(
            !drawn.contains("sk-") && !drawn.contains("ghp_"),
            "the raw view must never leak the secret: {drawn:?}"
        );
    }
}
