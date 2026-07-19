// file_injection_dialog.rs — Warning dialog for oversized @file references.
//
// Mirrors upstream file_injection_dialog.rs with Operant styling.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};

use crate::tui::overlays::centered_rect;
use crate::tui::file_injection::{AtFileRef, AtFileIssue};

/// State for the file injection warning dialog.
#[derive(Debug, Clone, Default)]
pub struct FileInjectionDialogState {
    /// Whether the dialog is currently visible.
    pub visible: bool,
    /// The oversized file references that triggered the dialog.
    pub oversized_files: Vec<AtFileRef>,
    /// User's choice: true = proceed anyway, false = cancel.
    pub proceed: bool,
}

impl FileInjectionDialogState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Show the dialog with the given oversized files.
    pub fn show(&mut self, oversized: Vec<AtFileRef>) {
        self.oversized_files = oversized;
        self.visible = true;
        self.proceed = false;
    }

    /// Dismiss the dialog.
    pub fn dismiss(&mut self) {
        self.visible = false;
        self.oversized_files.clear();
        self.proceed = false;
    }

    /// Confirm and proceed with injection.
    pub fn confirm(&mut self) {
        self.proceed = true;
        self.visible = false;
    }
}

/// Render the file injection warning dialog.
pub fn render_file_injection_dialog(
    state: &FileInjectionDialogState,
    area: Rect,
    buf: &mut Buffer,
) {
    if !state.visible || area.height < 10 || area.width < 50 {
        return;
    }

    let dialog_width = 70u16.min(area.width.saturating_sub(4));
    let oversized_count = state.oversized_files.len();
    let file_lines = oversized_count as u16;
    let dialog_height = (8 + file_lines).min(area.height.saturating_sub(4));
    let dialog_area = centered_rect(dialog_width, dialog_height, area);

    Clear.render(dialog_area, buf);

    let mut lines: Vec<Line> = Vec::new();

    // Title line
    lines.push(Line::from(""));
    lines.push(Line::from(vec![Span::styled(
        format!(" {} oversized file(s) detected ", oversized_count),
        Style::default()
            .fg(Color::Black)
            .bg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )]));
    lines.push(Line::from(""));

    // Warning message
    lines.push(Line::from(vec![Span::styled(
        " The following @file references exceed the size limit:",
        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
    )]));
    lines.push(Line::from(""));

    // File list
    for file in &state.oversized_files {
        let size_kb = file.size_kb;
        let display_path = file.path.display().to_string();
        let truncated = if display_path.len() > 55 {
            format!("...{}", &display_path[display_path.len() - 52..])
        } else {
            display_path
        };
        let issue_str = match &file.issue {
            Some(AtFileIssue::TooLarge(_)) => format!(" ({} KB)", size_kb),
            Some(AtFileIssue::Binary) => " (binary)".to_string(),
            Some(AtFileIssue::Unreadable(e)) => format!(" (unreadable: {})", e),
            Some(AtFileIssue::IsDirectory) => " (is directory)".to_string(),
            None => "".to_string(),
        };
        lines.push(Line::from(vec![
            Span::styled("   • ", Style::default().fg(Color::Cyan)),
            Span::styled(truncated, Style::default().fg(Color::White)),
            Span::styled(issue_str, Style::default().fg(Color::DarkGray)),
        ]));
    }

    lines.push(Line::from(""));

    // Instructions
    lines.push(Line::from(vec![Span::styled(
        " Injecting large files consumes significant context window. Proceed anyway?",
        Style::default().fg(Color::DarkGray),
    )]));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  [Enter] ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        Span::styled("Proceed with injection  ", Style::default().fg(Color::White)),
        Span::styled("[Esc] ", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
        Span::styled("Cancel", Style::default().fg(Color::White)),
    ]));

    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            " ⚠ File Size Warning ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::default().fg(Color::Yellow));

    let para = Paragraph::new(lines).block(block).wrap(ratatui::widgets::Wrap { trim: false });
    para.render(dialog_area, buf);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn file_injection_dialog_defaults_hidden() {
        let state = FileInjectionDialogState::new();
        assert!(!state.visible);
        assert!(state.oversized_files.is_empty());
    }

    #[test]
    fn file_injection_dialog_show_populates() {
        let mut state = FileInjectionDialogState::new();
        let file = crate::tui::file_injection::AtFileRef {
            token: "@big.txt".to_string(),
            path: std::path::PathBuf::from("/tmp/big.txt"),
            size_kb: 100,
            contents: None,
            issue: Some(crate::tui::file_injection::AtFileIssue::TooLarge(100)),
        };
        state.show(vec![file]);
        assert!(state.visible);
        assert_eq!(state.oversized_files.len(), 1);
    }

    #[test]
    fn file_injection_dialog_confirm() {
        let mut state = FileInjectionDialogState::new();
        state.show(vec![]);
        state.confirm();
        assert!(state.proceed);
        assert!(!state.visible);
    }

    #[test]
    fn file_injection_dialog_dismiss() {
        let mut state = FileInjectionDialogState::new();
        state.show(vec![]);
        state.dismiss();
        assert!(!state.visible);
        assert!(!state.proceed);
    }

    #[test]
    fn render_dialog_smoke() {
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        let mut state = FileInjectionDialogState::new();
        state.show(vec![crate::tui::file_injection::AtFileRef {
            token: "@test.txt".to_string(),
            path: "/tmp/test.txt".into(),
            size_kb: 50,
            contents: None,
            issue: Some(crate::tui::file_injection::AtFileIssue::TooLarge(50)),
        }]);
        terminal.draw(|frame| {
            render_file_injection_dialog(&state, frame.area(), frame.buffer_mut());
        }).unwrap();
    }
}