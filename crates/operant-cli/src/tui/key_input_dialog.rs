// key_input_dialog.rs — Masked text input overlay for entering API keys.
//
// Provides a modal dialog that collects an API key from the user with
// masked display (showing only the last 4 characters).

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::prelude::Stylize;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::tui::overlays::{HINT_ESC, ModalSpec, modal_frame};
use crate::tui::theme_colors;
use crate::tui::vendor::style::theme;

/// Desired size. The height was previously an unclamped `9`; `modal_layout` now
/// bounds it against the area and floors it at `MIN_MODAL_H`.
const DIALOG_WIDTH: u16 = 60;
const DIALOG_HEIGHT: u16 = 9;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// State for the API key input dialog.
pub struct KeyInputDialogState {
    pub visible: bool,
    pub provider_id: String,
    pub provider_name: String,
    pub input: String,
    pub cursor_pos: usize,
}

impl KeyInputDialogState {
    pub fn new() -> Self {
        Self {
            visible: false,
            provider_id: String::new(),
            provider_name: String::new(),
            input: String::new(),
            cursor_pos: 0,
        }
    }

    /// Open the dialog for a specific provider.
    pub fn open(&mut self, provider_id: String, provider_name: String) {
        self.visible = true;
        self.provider_id = provider_id;
        self.provider_name = provider_name;
        self.input.clear();
        self.cursor_pos = 0;
    }

    /// Close and clear the dialog.
    pub fn close(&mut self) {
        self.visible = false;
        self.input.clear();
        self.cursor_pos = 0;
    }

    /// Insert a character at the cursor position.
    pub fn insert_char(&mut self, c: char) {
        self.input.insert(self.cursor_pos, c);
        self.cursor_pos += c.len_utf8();
    }

    /// Delete the character before the cursor.
    pub fn backspace(&mut self) {
        if self.cursor_pos > 0 {
            // Find the previous char boundary
            let prev = self.input[..self.cursor_pos]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.input.remove(prev);
            self.cursor_pos = prev;
        }
    }

    /// Take the entered key and close the dialog.
    pub fn take_key(&mut self) -> String {
        let key = self.input.clone();
        self.close();
        key
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Render the key input dialog overlay: dark overlay, rounded modal frame,
/// masked key row.
pub fn render_key_input_dialog(frame: &mut Frame, state: &KeyInputDialogState, area: Rect) {
    if !state.visible {
        return;
    }

    let accent = theme_colors::accent();
    let dim = theme::dim_color();

    let title_text = format!("Connect {}", state.provider_name);

    let layout = modal_frame(
        frame,
        area,
        &ModalSpec {
            title: &title_text,
            hint: HINT_ESC,
            width: DIALOG_WIDTH,
            height: DIALOG_HEIGHT,
            header_height: 1,
            footer_height: 0,
            ..Default::default()
        },
    );

    // ── Build lines ──
    let mut lines: Vec<Line<'static>> = Vec::new();

    // "API Key:" label
    lines.push(Line::from(vec![Span::styled(
        " API Key:",
        Style::default().fg(theme::ai_text()),
    )]));

    // Masked key display (show last 4 chars, mask the rest)
    let masked = if state.input.is_empty() {
        "paste your API key here...".to_string()
    } else {
        let len = state.input.len();
        if len <= 4 {
            state.input.clone()
        } else {
            format!("{}{}", "\u{2022}".repeat(len - 4), &state.input[len - 4..])
        }
    };

    let input_style = if state.input.is_empty() {
        Style::default().fg(dim)
    } else {
        Style::default().fg(theme_colors::text())
    };

    lines.push(Line::from(vec![
        Span::styled(format!(" {}", masked), input_style),
        Span::styled("_", Style::default().fg(accent)), // cursor
    ]));

    // Blank line
    lines.push(Line::from(""));

    // Hint row
    lines.push(Line::from(vec![
        Span::styled(" enter", Style::default().fg(dim)),
        Span::styled(" confirm", Style::default().fg(dim)),
    ]));

    frame.render_widget(
        Paragraph::new(lines).bg(theme_colors::panel_bg()),
        layout.body_area,
    );
}
