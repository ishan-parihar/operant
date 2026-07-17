use crate::tui::adapter_types::import_config::{ImportPreview, ImportSelection};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use crate::tui::overlays::{
    OPERANT_ACCENT, OPERANT_MUTED, OPERANT_PANEL_BG, OPERANT_TEXT, begin_modal_frame,
    modal_header_line_area, render_modal_title_frame,
};

#[derive(Debug, Clone, Default)]
pub struct ImportConfigDialogState {
    pub visible: bool,
    pub selection: Option<ImportSelection>,
    pub preview: Option<ImportPreview>,
}

impl ImportConfigDialogState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn open(&mut self, preview: ImportPreview) {
        self.visible = true;
        self.selection = Some(ImportSelection::Both);
        self.preview = Some(preview);
    }

    pub fn close(&mut self) {
        self.visible = false;
        self.selection = None;
        self.preview = None;
    }
}

pub fn render_import_config_dialog(frame: &mut Frame, state: &ImportConfigDialogState, area: Rect) {
    if !state.visible {
        return;
    }

    let Some(preview) = &state.preview else {
        return;
    };

    let layout = begin_modal_frame(frame, area, 92, 28, 2, 1);
    render_modal_title_frame(frame, layout.header_area, "Import config", "esc");
    if let Some(subtitle_area) = modal_header_line_area(layout.header_area, 1) {
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                " Preview the content to import from ~/.claude; Enter to confirm, Esc to cancel.",
                Style::default().fg(OPERANT_MUTED),
            )])),
            subtitle_area,
        );
    }

    let mut lines: Vec<Line<'static>> = vec![];
    if preview.claude_md {
        lines.push(section_title("CLAUDE.md"));
        lines.push(Line::from(vec![Span::styled(
            "  Will import CLAUDE.md from ~/.claude",
            Style::default().fg(OPERANT_TEXT),
        )]));
        lines.push(Line::from(""));
    }

    if preview.settings {
        lines.push(section_title("settings.json"));
        lines.push(Line::from(vec![Span::styled(
            "  Will import settings from ~/.claude/settings.json",
            Style::default().fg(OPERANT_TEXT),
        )]));
        lines.push(Line::from(""));
    }

    if preview.auth {
        lines.push(section_title("Auth credentials"));
        lines.push(Line::from(vec![Span::styled(
            "  Will import API keys from ~/.claude/.credentials",
            Style::default().fg(OPERANT_TEXT),
        )]));
        lines.push(Line::from(""));
    }

    if lines.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            "  Nothing to import.",
            Style::default().fg(OPERANT_MUTED),
        )]));
    }

    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .style(Style::default().bg(OPERANT_PANEL_BG)),
        layout.body_area,
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            " Enter to import  ·  Esc to cancel",
            Style::default()
                .fg(OPERANT_MUTED)
                .add_modifier(Modifier::ITALIC),
        )])),
        layout.footer_area,
    );
}

fn section_title(title: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(" ", Style::default()),
        Span::styled(
            title.to_string(),
            Style::default()
                .fg(OPERANT_ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
    ])
}
