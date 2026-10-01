// voice_mode_notice.rs — VoiceModeNotice surface.
//
// Shown when the user's account has voice mode available but it isn't yet
// enabled. Appears as a one-time dismissable notice below the welcome header.

use crate::tui::theme_colors;
use crate::tui::vendor::style::theme;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Widget};

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// Voice mode availability notice.
#[derive(Debug, Clone, Default)]
pub struct VoiceModeNoticeState {
    /// Whether the notice is visible.
    pub visible: bool,
    /// Whether voice mode is currently enabled by the user.
    pub voice_enabled: bool,
    /// Whether the user has dismissed this notice.
    dismissed: bool,
}

impl VoiceModeNoticeState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Show the notice if voice mode is available but not yet enabled.
    pub fn show_if_available(&mut self, voice_available: bool, voice_enabled: bool) {
        if self.dismissed {
            return;
        }
        self.voice_enabled = voice_enabled;
        self.visible = voice_available && !voice_enabled;
    }

    /// Update voice-enabled status (called when user toggles voice).
    #[allow(dead_code)] // Called externally when user toggles voice mode
    pub fn update_voice_enabled(&mut self, enabled: bool) {
        self.voice_enabled = enabled;
        if enabled {
            // Auto-dismiss when user enables voice
            self.visible = false;
            self.dismissed = true;
        }
    }

    /// Dismiss the notice for this session.
    #[allow(dead_code)] // Called externally on Esc key
    pub fn dismiss(&mut self) {
        self.visible = false;
        self.dismissed = true;
    }

    /// Height the notice occupies (0 if not visible).
    pub fn height(&self) -> u16 {
        if self.visible { 2 } else { 0 }
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Render the voice mode availability notice.
pub fn render_voice_mode_notice(state: &VoiceModeNoticeState, area: Rect, buf: &mut Buffer) {
    if !state.visible || area.height == 0 {
        return;
    }

    let notice_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: state.height().min(area.height),
    };

    Clear.render(notice_area, buf);

    let lines = vec![
        Line::from(vec![
            // State first, then the way out of it. This is the same shape as the
            // unified footer hint and jcode's toggle confirmations
            // (`Inline images: hidden (⌥+Shift+I to show)`): a surface that
            // reports a mode the user is *not* in owes them the binding that
            // changes it, and naming the state before the key is what stops the
            // key from reading as a generic hotkey.
            Span::styled(
                " Voice mode ",
                Style::default()
                    .fg(theme::user_bg())
                    .bg(theme_colors::accent())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("off", Style::default().fg(theme::info_color())),
            Span::styled(" \u{2014} ", Style::default().fg(theme_colors::muted())),
            Span::styled(
                "Alt+V",
                Style::default()
                    .fg(theme_colors::accent())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" to record", Style::default().fg(theme_colors::text())),
            Span::styled(" \u{00b7} ", Style::default().fg(theme_colors::muted())),
            Span::styled(
                "/voice",
                Style::default()
                    .fg(theme_colors::accent())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" to configure", Style::default().fg(theme_colors::text())),
            Span::styled("  ", Style::default()),
            Span::styled("[Esc dismiss]", Style::default().fg(theme::dim_color())),
        ]),
        Line::from(""),
    ];

    Paragraph::new(lines)
        // A notice band raised above the app's `user_bg` base fill. `SelectionBg`
        // is the palette's only raised-surface role; `vendor/**` is frozen, so a
        // dedicated notice role is not available here.
        .style(Style::default().bg(theme::selection_bg_color()))
        .render(notice_area, buf);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;

    #[test]
    fn voice_notice_show_when_available_not_enabled() {
        let mut state = VoiceModeNoticeState::new();
        state.show_if_available(true, false);
        assert!(state.visible);
    }

    #[test]
    fn voice_notice_hidden_when_already_enabled() {
        let mut state = VoiceModeNoticeState::new();
        state.show_if_available(true, true);
        assert!(!state.visible);
    }

    #[test]
    fn voice_notice_hidden_when_not_available() {
        let mut state = VoiceModeNoticeState::new();
        state.show_if_available(false, false);
        assert!(!state.visible);
    }

    #[test]
    fn voice_notice_dismiss() {
        let mut state = VoiceModeNoticeState::new();
        state.show_if_available(true, false);
        state.dismiss();
        assert!(!state.visible);
        // Should not re-show after dismiss
        state.show_if_available(true, false);
        assert!(!state.visible);
    }

    #[test]
    fn voice_notice_auto_dismiss_on_enable() {
        let mut state = VoiceModeNoticeState::new();
        state.show_if_available(true, false);
        assert!(state.visible);
        state.update_voice_enabled(true);
        assert!(!state.visible);
        // Should stay dismissed
        state.show_if_available(true, false);
        assert!(!state.visible);
    }

    #[test]
    fn voice_notice_render_smoke() {
        let mut state = VoiceModeNoticeState::new();
        state.show_if_available(true, false);
        let area = Rect {
            x: 0,
            y: 0,
            width: 100,
            height: 4,
        };
        let mut buf = ratatui::buffer::Buffer::empty(area);
        render_voice_mode_notice(&state, area, &mut buf);
        let rendered = buf
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<_>>()
            .join("");
        assert!(rendered.contains("Voice mode"));
        assert!(rendered.contains("Alt+V"));
    }

    #[test]
    fn voice_notice_not_rendered_when_invisible() {
        let state = VoiceModeNoticeState::new();
        let area = Rect {
            x: 0,
            y: 0,
            width: 80,
            height: 4,
        };
        let mut buf = ratatui::buffer::Buffer::empty(area);
        render_voice_mode_notice(&state, area, &mut buf);
        let rendered = buf
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<_>>()
            .join("");
        assert!(!rendered.contains("Voice"));
    }
}
