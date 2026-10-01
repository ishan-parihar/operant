// effort_picker.rs — Small modal picker for /effort command.
//
// Replaces the prior text-only `/effort` status message with an interactive
// 4-row select dialog (issue #149 follow-up).

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::tui::model_picker::EffortLevel;
use crate::tui::overlays::{HINT_ESC, ModalSpec, cycle_next, cycle_prev, modal_frame};
use crate::tui::vendor::style::theme;

#[derive(Debug, Default, Clone)]
pub struct EffortPickerState {
    pub visible: bool,
    pub selected: usize, // 0..=3
}

impl EffortPickerState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn open(&mut self, current: EffortLevel) {
        self.visible = true;
        self.selected = match current {
            EffortLevel::Low => 0,
            EffortLevel::Normal => 1,
            EffortLevel::High => 2,
            EffortLevel::Max => 3,
        };
    }

    pub fn close(&mut self) {
        self.visible = false;
    }

    pub fn select_prev(&mut self) {
        cycle_prev(&mut self.selected, 4);
    }

    pub fn select_next(&mut self) {
        cycle_next(&mut self.selected, 4);
    }

    pub fn current(&self) -> EffortLevel {
        match self.selected {
            0 => EffortLevel::Low,
            1 => EffortLevel::Normal,
            2 => EffortLevel::High,
            _ => EffortLevel::Max,
        }
    }
}

pub fn render_effort_picker(frame: &mut Frame, state: &EffortPickerState, area: Rect) {
    if !state.visible {
        return;
    }

    // Natural desired size is 44×11; `modal_frame` clamps it to the terminal.
    let layout = modal_frame(
        frame,
        area,
        &ModalSpec {
            title: "Effort level",
            hint: HINT_ESC,
            width: 44,
            height: 11,
            header_height: 1,
            footer_height: 0,
            // Same accent role `journey_view` uses for a panel border.
            border_fg: theme::accent_color(),
        },
    );

    let mut lines: Vec<Line> = Vec::new();
    let options: [(EffortLevel, &str); 4] = [
        (EffortLevel::Low, "low"),
        (EffortLevel::Normal, "normal"),
        (EffortLevel::High, "high"),
        (EffortLevel::Max, "max"),
    ];
    for (i, (lvl, label)) in options.iter().enumerate() {
        let selected = i == state.selected;
        let prefix = if selected { "›" } else { " " };
        let style = if selected {
            Style::default()
                .fg(theme::user_bg())
                .bg(theme::accent_color())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme::ai_text())
        };
        lines.push(Line::from(vec![
            Span::styled(format!("  {} ", prefix), style),
            Span::styled(format!("{}  {}", lvl.symbol(), label), style),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  ↑/↓ to choose · Enter to apply · Esc to cancel",
        Style::default().fg(theme::dim_color()),
    )));

    frame.render_widget(Paragraph::new(lines), layout.body_area);
}
