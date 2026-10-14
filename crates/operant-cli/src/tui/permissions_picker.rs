// permissions_picker.rs — Mode picker for /permissions.
//
// 2026-10-14 batch, plan 10: Shift+Tab used to cycle Default → AcceptEdits →
// Bypass in two presses with no confirmation — a dangerous key one accidental
// press away from arming bypass. Permission modes are now chosen explicitly
// through this picker; the Bypass row defers to the existing
// bypass-permissions confirmation dialog instead of arming directly.

use crate::tui::adapter_types::config::PermissionMode;
use crate::tui::overlays::{HINT_ESC, ModalSpec, cycle_next, cycle_prev, modal_frame};
use crate::tui::vendor::style::theme;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

/// Row labels in picker order. The index matches `selected`.
const MODES: [&str; 4] = [
    "default — ask for every dangerous action",
    "accept edits — auto-approve file edits",
    "bypass permissions (dangerous)",
    "plan — no tool side effects",
];

#[derive(Debug, Default, Clone)]
pub struct PermissionsPickerState {
    pub visible: bool,
    pub selected: usize, // 0..=3, index into MODES
}

impl PermissionsPickerState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn open(&mut self, current: PermissionMode) {
        self.visible = true;
        self.selected = match current {
            PermissionMode::Default => 0,
            PermissionMode::AcceptEdits => 1,
            PermissionMode::BypassPermissions => 2,
            PermissionMode::Plan => 3,
        };
    }

    pub fn close(&mut self) {
        self.visible = false;
    }

    pub fn select_prev(&mut self) {
        cycle_prev(&mut self.selected, MODES.len());
    }

    pub fn select_next(&mut self) {
        cycle_next(&mut self.selected, MODES.len());
    }

    /// The mode the cursor sits on. Enter applies it — except Bypass, which
    /// the caller routes through the confirmation dialog first.
    pub fn current(&self) -> PermissionMode {
        match self.selected {
            0 => PermissionMode::Default,
            1 => PermissionMode::AcceptEdits,
            2 => PermissionMode::BypassPermissions,
            _ => PermissionMode::Plan,
        }
    }
}

pub fn render_permissions_picker(frame: &mut Frame, state: &PermissionsPickerState, area: Rect) {
    if !state.visible {
        return;
    }

    let layout = modal_frame(
        frame,
        area,
        &ModalSpec {
            title: "Permission mode",
            hint: HINT_ESC,
            width: 52,
            height: 9,
            header_height: 1,
            footer_height: 0,
            border_fg: theme::accent_color(),
        },
    );

    let mut lines: Vec<Line> = Vec::new();
    for (i, label) in MODES.iter().enumerate() {
        let selected = i == state.selected;
        let prefix = if selected { "›" } else { " " };
        let style = if selected {
            Style::default()
                .fg(theme::user_bg())
                .bg(theme::accent_color())
                .add_modifier(Modifier::BOLD)
        } else if i == 2 {
            Style::default().fg(theme::asap_color())
        } else {
            Style::default().fg(theme::ai_text())
        };
        lines.push(Line::from(vec![
            Span::styled(format!("  {prefix} "), style),
            Span::styled((*label).to_string(), style),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  ↑/↓ to choose · Enter to apply · Esc to cancel",
        Style::default().fg(theme::dim_color()),
    )));

    frame.render_widget(Paragraph::new(lines), layout.body_area);
}
