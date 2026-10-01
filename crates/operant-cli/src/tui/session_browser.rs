//! Session browser overlay (/session, /resume, /rename, /export).
//! Mirrors TS session management in REPL.tsx

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use crate::tui::overlays::{HINT_ESC, ModalSpec, cycle_next, cycle_prev, modal_frame_buf};
use crate::tui::theme_colors;
use crate::tui::vendor::style::theme;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// The interaction mode of the session browser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionBrowserMode {
    /// Default: list sessions, navigate with arrow keys.
    Browse,
    /// User is typing a new name for the selected session.
    Rename,
}

/// A single session entry shown in the browser list.
#[derive(Debug, Clone)]
pub struct SessionEntry {
    pub id: String,
    pub title: String,
    /// Human-readable relative time, e.g. "2 hours ago".
    pub last_updated: String,
    pub message_count: usize,
    /// Estimated USD cost for the session.
    pub cost_usd: f64,
}

/// State for the session browser overlay.
pub struct SessionBrowserState {
    pub visible: bool,
    pub selected_idx: usize,
    pub sessions: Vec<SessionEntry>,
    pub mode: SessionBrowserMode,
    /// Input buffer used while in `Rename` mode.
    pub rename_input: String,
}

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

impl SessionBrowserState {
    /// Create a new, hidden browser with an empty session list.
    pub fn new() -> Self {
        Self {
            visible: false,
            selected_idx: 0,
            sessions: Vec::new(),
            mode: SessionBrowserMode::Browse,
            rename_input: String::new(),
        }
    }

    /// Open the browser with the provided session list.
    pub fn open(&mut self, sessions: Vec<SessionEntry>) {
        self.sessions = sessions;
        self.selected_idx = 0;
        self.mode = SessionBrowserMode::Browse;
        self.rename_input.clear();
        self.visible = true;
    }

    /// Close the browser entirely.
    pub fn close(&mut self) {
        self.visible = false;
        self.mode = SessionBrowserMode::Browse;
        self.rename_input.clear();
    }

    /// Move selection up one row, wrapping to the end.
    pub fn select_prev(&mut self) {
        cycle_prev(&mut self.selected_idx, self.sessions.len());
    }

    /// Move selection down one row, wrapping to the start.
    pub fn select_next(&mut self) {
        cycle_next(&mut self.selected_idx, self.sessions.len());
    }

    /// Return a reference to the currently selected session, if any.
    #[allow(dead_code)] // Session selection getter
    pub fn selected_session(&self) -> Option<&SessionEntry> {
        #[allow(dead_code)]
        self.sessions.get(self.selected_idx)
    }

    /// Switch to rename mode, pre-populating the input with the current title.
    pub fn start_rename(&mut self) {
        if let Some(session) = self.sessions.get(self.selected_idx) {
            self.rename_input = session.title.clone();
            self.mode = SessionBrowserMode::Rename;
        }
    }

    /// Append a character to the rename input buffer.
    pub fn push_rename_char(&mut self, c: char) {
        if self.mode == SessionBrowserMode::Rename {
            self.rename_input.push(c);
        }
    }

    /// Remove the last character from the rename input buffer.
    pub fn pop_rename_char(&mut self) {
        if self.mode == SessionBrowserMode::Rename {
            self.rename_input.pop();
        }
    }

    /// Confirm the rename. Returns `(session_id, new_name)` when in rename mode
    /// with a non-empty name and a valid selection. Resets to browse mode.
    pub fn confirm_rename(&mut self) -> Option<(String, String)> {
        if self.mode != SessionBrowserMode::Rename {
            return None;
        }
        let new_name = self.rename_input.trim().to_string();
        if new_name.is_empty() {
            return None;
        }
        let session_id = self.sessions.get(self.selected_idx)?.id.clone();
        // Apply the rename in the local list immediately for UI consistency.
        if let Some(session) = self.sessions.get_mut(self.selected_idx) {
            session.title = new_name.clone();
        }
        self.mode = SessionBrowserMode::Browse;
        self.rename_input.clear();
        Some((session_id, new_name))
    }

    /// Cancel the current mode:
    /// - In `Rename` or `Confirm` mode: return to `Browse`.
    /// - In `Browse` mode: close the overlay.
    pub fn cancel(&mut self) {
        match self.mode {
            SessionBrowserMode::Browse => self.close(),
            SessionBrowserMode::Rename => {
                self.mode = SessionBrowserMode::Browse;
                self.rename_input.clear();
            }
        }
    }
}

impl Default for SessionBrowserState {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Rendering helpers
// ---------------------------------------------------------------------------

/// Format a cost as a dollar string with 4 decimal places.
fn fmt_cost(usd: f64) -> String {
    if usd < 0.0001 {
        "$0.0000".to_string()
    } else {
        format!("${:.4}", usd)
    }
}

/// Truncate `s` to fit within `max_width` display columns, appending `…` if cut.
fn truncate_display(s: &str, max_width: usize) -> String {
    use crate::tui::render::{display_width, take_width};
    if display_width(s) <= max_width {
        return s.to_string();
    }
    if max_width <= 1 {
        return "…".to_string();
    }
    // Reserve the final cell for the ellipsis.
    let mut out = take_width(s, max_width - 1);
    out.push('…');
    out
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Render the session browser overlay directly into `buf`.
///
/// Draws a centred modal (≈70 wide × ≈20 tall) with:
/// - A scrollable list of sessions (id, title, date, messages, cost)
/// - Selection highlight on the focused row
/// - Mode-sensitive hint bar at the bottom
/// - A rename input field shown when in `Rename` mode
pub fn render_session_browser(state: &SessionBrowserState, area: Rect, buf: &mut Buffer) {
    if !state.visible {
        return;
    }

    // Natural desired size is 70×20. The old formula used a `-2` margin, which is
    // the same fact `space::M` states; the primitive now owns it and adds the
    // readable floors the old `.min(w - 2)` could not guarantee.
    let layout = modal_frame_buf(
        buf,
        area,
        &ModalSpec {
            title: "Sessions",
            hint: HINT_ESC,
            width: 70,
            height: 20,
            header_height: 1,
            footer_height: 0,
            border_fg: theme_colors::accent(),
        },
    );
    let inner = layout.body_area;

    let inner_w = inner.width as usize;
    let mut lines: Vec<Line> = Vec::new();

    // --- Session list -----------------------------------------------------
    if state.sessions.is_empty() {
        // "nothing here yet", not "no matches" — this surface has no filter
        // field, so there is nothing to type and nothing that failed to match.
        // A generic "No sessions found." would read as a failed search.
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(
            "  No sessions yet.",
            Style::default().fg(theme::dim_color()),
        )]));
        lines.push(Line::from(vec![Span::styled(
            "  Sessions are saved automatically as you chat.",
            Style::default().fg(theme::dim_color()),
        )]));
    } else {
        // Column widths (approximate):
        //   title: ~40 chars  |  date: ~14 chars  |  msgs: 5  |  cost: 9
        let date_w: usize = 14;
        let msgs_w: usize = 5;
        let cost_w: usize = 9;
        let fixed = date_w + msgs_w + cost_w + 6; // separators & padding
        let title_w = inner_w.saturating_sub(fixed).max(10);

        // Header row
        lines.push(Line::from(vec![Span::styled(
            format!(
                "  {:<title_w$}  {:<date_w$}  {:>msgs_w$}  {:>cost_w$}",
                "Title",
                "Last Updated",
                "Msgs",
                "Cost",
                title_w = title_w,
                date_w = date_w,
                msgs_w = msgs_w,
                cost_w = cost_w
            ),
            Style::default()
                .fg(theme::dim_color())
                .add_modifier(Modifier::UNDERLINED),
        )]));
        lines.push(Line::from(""));

        for (i, session) in state.sessions.iter().enumerate() {
            let is_selected = i == state.selected_idx;

            let title_cell = truncate_display(&session.title, title_w);
            let date_cell = truncate_display(&session.last_updated, date_w);
            let msgs_cell = format!("{:>msgs_w$}", session.message_count, msgs_w = msgs_w);
            let cost_cell = format!("{:>cost_w$}", fmt_cost(session.cost_usd), cost_w = cost_w);

            let row_bg = if is_selected {
                theme::selection_bg_color()
            } else {
                // transparent — ratatui uses reset/default for "no background"
                Color::Reset
            };

            let title_style = if is_selected {
                Style::default()
                    .fg(theme_colors::accent())
                    .bg(row_bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme_colors::text())
            };

            let meta_style = if is_selected {
                Style::default().fg(theme::header_name_color()).bg(row_bg)
            } else {
                Style::default().fg(theme::dim_color())
            };

            let prefix_style = Style::default().bg(row_bg);

            lines.push(Line::from(vec![
                Span::styled("  ", prefix_style),
                Span::styled(
                    format!("{:<title_w$}", title_cell, title_w = title_w),
                    title_style,
                ),
                Span::styled("  ", meta_style),
                Span::styled(
                    format!("{:<date_w$}", date_cell, date_w = date_w),
                    meta_style,
                ),
                Span::styled("  ", meta_style),
                Span::styled(msgs_cell, meta_style),
                Span::styled("  ", meta_style),
                Span::styled(cost_cell, meta_style),
            ]));
        }
    }

    lines.push(Line::from(""));

    // --- Mode-sensitive bottom section -----------------------------------
    match &state.mode {
        SessionBrowserMode::Browse => {
            lines.push(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(
                    "\u{2191}\u{2193}",
                    Style::default()
                        .fg(theme_colors::accent())
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" navigate  ", Style::default().fg(theme::dim_color())),
                Span::styled(
                    "Enter",
                    Style::default()
                        .fg(theme_colors::accent())
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("=resume  ", Style::default().fg(theme::dim_color())),
                Span::styled(
                    "r",
                    Style::default()
                        .fg(theme_colors::accent())
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("=rename  ", Style::default().fg(theme::dim_color())),
                Span::styled(
                    "Esc",
                    Style::default()
                        .fg(theme_colors::accent())
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("=close", Style::default().fg(theme::dim_color())),
            ]));
        }
        SessionBrowserMode::Rename => {
            // Show rename input field.
            let label = "  Rename: ";
            let cursor = "\u{2588}"; // block cursor
            let input_display = format!("{}{}", state.rename_input, cursor);
            lines.push(Line::from(vec![
                Span::styled(
                    label,
                    Style::default()
                        .fg(theme_colors::warning())
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    input_display,
                    Style::default()
                        .fg(theme_colors::text())
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
            lines.push(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(
                    "Enter",
                    Style::default()
                        .fg(theme_colors::accent())
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("=confirm  ", Style::default().fg(theme::dim_color())),
                Span::styled(
                    "Esc",
                    Style::default()
                        .fg(theme_colors::accent())
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("=cancel", Style::default().fg(theme::dim_color())),
            ]));
        }
    }

    let para = Paragraph::new(lines)
        .alignment(Alignment::Left)
        .wrap(Wrap { trim: false })
        .style(Style::default().bg(theme_colors::panel_bg()));

    use ratatui::widgets::Widget;
    para.render(inner, buf);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_sessions() -> Vec<SessionEntry> {
        vec![
            SessionEntry {
                id: "sess-001".to_string(),
                title: "Refactor auth module".to_string(),
                last_updated: "2 hours ago".to_string(),
                message_count: 34,
                cost_usd: 0.0124,
            },
            SessionEntry {
                id: "sess-002".to_string(),
                title: "Write unit tests".to_string(),
                last_updated: "yesterday".to_string(),
                message_count: 12,
                cost_usd: 0.0045,
            },
            SessionEntry {
                id: "sess-003".to_string(),
                title: "Debug memory leak".to_string(),
                last_updated: "3 days ago".to_string(),
                message_count: 57,
                cost_usd: 0.0289,
            },
        ]
    }

    // 1. new() starts hidden with no sessions.
    #[test]
    fn new_starts_hidden() {
        let s = SessionBrowserState::new();
        assert!(!s.visible);
        assert!(s.sessions.is_empty());
        assert_eq!(s.mode, SessionBrowserMode::Browse);
    }

    // 2. open() populates sessions and becomes visible.
    #[test]
    fn open_populates_and_shows() {
        let mut s = SessionBrowserState::new();
        s.open(sample_sessions());
        assert!(s.visible);
        assert_eq!(s.sessions.len(), 3);
        assert_eq!(s.selected_idx, 0);
        assert_eq!(s.mode, SessionBrowserMode::Browse);
    }

    // 3. select_next() advances selection and wraps to the start.
    #[test]
    fn select_next_wraps_to_start() {
        let mut s = SessionBrowserState::new();
        s.open(sample_sessions());
        s.select_next();
        assert_eq!(s.selected_idx, 1);
        s.select_next();
        assert_eq!(s.selected_idx, 2);
        s.select_next();
        assert_eq!(s.selected_idx, 0);
    }

    // 4. select_prev() decrements and wraps to the end.
    #[test]
    fn select_prev_wraps_to_end() {
        let mut s = SessionBrowserState::new();
        s.open(sample_sessions());
        s.select_prev();
        assert_eq!(s.selected_idx, 2);
    }

    // 5. selected_session() returns correct entry.
    #[test]
    fn selected_session_correct() {
        let mut s = SessionBrowserState::new();
        s.open(sample_sessions());
        s.selected_idx = 1;
        let sess = s.selected_session().unwrap();
        assert_eq!(sess.id, "sess-002");
    }

    // 6. start_rename() switches mode and pre-fills input.
    #[test]
    fn start_rename_prefills_title() {
        let mut s = SessionBrowserState::new();
        s.open(sample_sessions());
        s.selected_idx = 0;
        s.start_rename();
        assert_eq!(s.mode, SessionBrowserMode::Rename);
        assert_eq!(s.rename_input, "Refactor auth module");
    }

    // 7. push_rename_char / pop_rename_char edit the input buffer.
    #[test]
    fn rename_char_editing() {
        let mut s = SessionBrowserState::new();
        s.open(sample_sessions());
        s.start_rename();
        s.rename_input.clear(); // clear prefill for clean test
        s.push_rename_char('H');
        s.push_rename_char('i');
        assert_eq!(s.rename_input, "Hi");
        s.pop_rename_char();
        assert_eq!(s.rename_input, "H");
    }

    // 8. confirm_rename() returns (id, new_name) and resets mode.
    #[test]
    fn confirm_rename_returns_pair() {
        let mut s = SessionBrowserState::new();
        s.open(sample_sessions());
        s.selected_idx = 0;
        s.start_rename();
        s.rename_input = "  New Title  ".to_string(); // intentional whitespace
        let result = s.confirm_rename();
        assert_eq!(
            result,
            Some(("sess-001".to_string(), "New Title".to_string()))
        );
        assert_eq!(s.mode, SessionBrowserMode::Browse);
        assert!(s.rename_input.is_empty());
        // Also check local title was updated
        assert_eq!(s.sessions[0].title, "New Title");
    }

    // 9. confirm_rename() with empty input returns None.
    #[test]
    fn confirm_rename_empty_returns_none() {
        let mut s = SessionBrowserState::new();
        s.open(sample_sessions());
        s.start_rename();
        s.rename_input = "   ".to_string(); // whitespace only
        let result = s.confirm_rename();
        assert!(result.is_none());
    }

    // 10. cancel() in Rename mode returns to Browse without closing.
    #[test]
    fn cancel_rename_goes_to_browse() {
        let mut s = SessionBrowserState::new();
        s.open(sample_sessions());
        s.start_rename();
        s.cancel();
        assert_eq!(s.mode, SessionBrowserMode::Browse);
        assert!(
            s.visible,
            "overlay should remain visible after cancel-from-rename"
        );
    }

    // 11. cancel() in Browse mode closes the overlay.
    #[test]
    fn cancel_browse_closes() {
        let mut s = SessionBrowserState::new();
        s.open(sample_sessions());
        assert_eq!(s.mode, SessionBrowserMode::Browse);
        s.cancel();
        assert!(!s.visible);
    }

    // 12. render_session_browser does not panic.
    #[test]
    fn render_does_not_panic() {
        let mut s = SessionBrowserState::new();
        s.open(sample_sessions());
        let area = Rect::new(0, 0, 120, 40);
        let mut buf = Buffer::empty(area);
        render_session_browser(&s, area, &mut buf);
    }

    // 13. render is a no-op when hidden.
    #[test]
    fn render_noop_when_hidden() {
        let s = SessionBrowserState::new(); // visible = false
        let area = Rect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        render_session_browser(&s, area, &mut buf);
        for cell in buf.content() {
            assert_eq!(
                cell.symbol(),
                " ",
                "buffer should be empty when browser is hidden"
            );
        }
    }

    // 14. fmt_cost formats correctly.
    #[test]
    fn fmt_cost_formats() {
        assert_eq!(fmt_cost(0.0), "$0.0000");
        assert_eq!(fmt_cost(0.0124), "$0.0124");
        assert_eq!(fmt_cost(1.5), "$1.5000");
    }

    // 15. truncate_display trims long strings.
    #[test]
    fn truncate_display_trims() {
        let long = "abcdefghij"; // 10 chars
        let result = truncate_display(long, 5);
        assert!(
            crate::tui::render::display_width(&result) <= 6,
            "truncated string should fit within budget"
        );
        assert!(result.ends_with('…'));
    }
}
