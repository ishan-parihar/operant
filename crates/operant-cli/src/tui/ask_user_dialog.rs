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
        self.question = question;
        self.options = options;
        self.selected_idx = 0;
        self.custom_text.clear();
        self.in_custom_input = self.options.is_none();
        self.reply_tx = Some(reply_tx);
        self.visible = true;
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
    // One title row + the content rows + the hint row, plus the two border rows
    // the primitive insets. `modal_layout` bounds it; nothing clamps here.
    let desired_height = 1 + 5 + question_lines + options_lines + 3;

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
            // A selection bar is a filled surface, so its foreground is the
            // dark surface role rather than a light text role.
            let sel_fg = theme::user_bg();
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
                        Style::default().fg(theme::dim_color()).bg(style_bg)
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
                    theme::user_bg()
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
                Style::default().fg(theme::user_text()).bg(style_bg),
            ));
        }
        write_line!(row, Line::from(spans));
        row += 1;
    }

    // Hint row
    row += 1;
    if row < last_row {
        let hint = if state.options.is_some() {
            "  type: custom   ↑↓/Tab: options   Enter: confirm   Esc: skip"
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
}
