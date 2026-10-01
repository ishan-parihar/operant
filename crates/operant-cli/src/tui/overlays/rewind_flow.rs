// overlays/rewind_flow.rs — Multi-step /rewind flow (select → confirm → done).
//
// Extracted from the overlays.rs monolith.

use super::*;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

// ============================================================================
// RewindFlowOverlay  (multi-step: select → confirm → done)
// ============================================================================

/// The current step in the rewind flow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RewindStep {
    /// Step 1: user is browsing the message list.
    Selecting,
    /// Step 2: user has chosen a message and must confirm.
    Confirming { message_idx: usize },
}

/// Full multi-step overlay for the /rewind command.
#[derive(Debug)]
pub struct RewindFlowOverlay {
    pub visible: bool,
    pub step: RewindStep,
    pub selector: MessageSelectorOverlay,
}

impl Default for RewindFlowOverlay {
    fn default() -> Self {
        Self {
            visible: false,
            step: RewindStep::Selecting,
            selector: MessageSelectorOverlay::new(),
        }
    }
}

impl RewindFlowOverlay {
    pub fn new() -> Self {
        Self::default()
    }

    /// Open the overlay with the given conversation messages.
    pub fn open(&mut self, messages: Vec<SelectorMessage>) {
        self.selector = MessageSelectorOverlay::open(messages);
        self.step = RewindStep::Selecting;
        self.visible = true;
    }

    pub fn close(&mut self) {
        self.visible = false;
        self.selector.close();
        self.step = RewindStep::Selecting;
    }

    /// Confirm the current selection; advances to the `Confirming` step.
    /// Returns the selected message index if in the Selecting step.
    pub fn confirm_selection(&mut self) -> Option<usize> {
        if self.step == RewindStep::Selecting
            && let Some(msg) = self.selector.current_message()
        {
            let idx = msg.idx;
            self.step = RewindStep::Confirming { message_idx: idx };
            return Some(idx);
        }
        None
    }

    /// The user pressed 'y' in the Confirming step.
    /// Returns the final message index to rewind to.
    pub fn accept_confirm(&mut self) -> Option<usize> {
        if let RewindStep::Confirming { message_idx } = self.step {
            self.close();
            return Some(message_idx);
        }
        None
    }

    /// The user pressed 'n' or Esc in the Confirming step — go back to selector.
    pub fn reject_confirm(&mut self) {
        if matches!(self.step, RewindStep::Confirming { .. }) {
            self.step = RewindStep::Selecting;
        }
    }
}

/// Render the full rewind flow overlay.
pub fn render_rewind_flow(frame: &mut Frame, overlay: &RewindFlowOverlay, area: Rect) {
    if !overlay.visible {
        return;
    }

    match &overlay.step {
        RewindStep::Selecting => {
            render_message_selector(frame, &overlay.selector, area);
        }
        RewindStep::Confirming { message_idx } => {
            render_rewind_confirm(frame, *message_idx, area);
        }
    }
}

fn render_rewind_confirm(frame: &mut Frame, message_idx: usize, area: Rect) {
    // Five body rows (blank, question, blank, the [y]/[n] row, blank) plus the
    // modal's own chrome: two border rows and the title row. No `HINT_ESC` —
    // Esc at this step returns to the selector rather than dismissing the
    // flow, so the canonical dismissal hint would be false here.
    let layout = modal_frame(
        frame,
        area,
        &ModalSpec {
            title: "Confirm Rewind",
            hint: "",
            width: 50,
            height: 8,
            header_height: 1,
            footer_height: 0,
            border_fg: theme_colors::warning(),
        },
    );

    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("  Rewind to message "),
            Span::styled(
                format!("#{}", message_idx),
                Style::default()
                    .fg(theme_colors::warning())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("?"),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "  [y] ",
                Style::default()
                    .fg(theme_colors::success())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("Yes, rewind"),
            Span::raw("    "),
            Span::styled(
                "[n] ",
                Style::default()
                    .fg(theme_colors::error())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("Cancel"),
        ]),
        Line::from(""),
    ];

    frame.render_widget(Paragraph::new(lines), layout.body_area);
}
