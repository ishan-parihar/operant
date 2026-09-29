// bridge_state.rs — Bridge connection state and status badge rendering.
//
// Mirrors upstream bridge_state.rs with Operant-specific styling.

use crate::tui::theme_colors;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

/// The current state of the remote gateway/bridge connection.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum BridgeConnectionState {
    /// No bridge configured / not in use.
    #[default]
    Disconnected,
    /// Currently attempting to connect.
    Connecting,
    /// Successfully connected with session info.
    Connected {
        session_url: String,
        peer_count: u32,
    },
    /// Lost connection and retrying.
    #[allow(dead_code)] // Prepared for bridge reconnection UI
    Reconnecting { attempt: u32 },
    /// Connection failed unrecoverably.
    #[allow(dead_code)] // Prepared for bridge failure UI
    Failed { reason: String },
    /// Outbound-only mode (no incoming peers).
    #[allow(dead_code)] // Prepared for outbound-only bridge mode
    OutboundOnly,
}

impl BridgeConnectionState {
    /// Return a styled status badge `Span` suitable for the status bar.
    /// Returns `None` when the state should not be shown (Disconnected).
    ///
    /// The badge backgrounds are hardcoded ANSI slots deliberately. Do NOT route
    /// them through `theme_colors` roles: those roles are tuned as FOREGROUNDS on
    /// `panel_bg()`, so using one as a background inverts the lightness
    /// relationship. Measured with `on_selection()` as the foreground, a routed
    /// badge drops below WCAG AA (4.5:1) on 7 of 8 themes via `muted()` (1.69:1
    /// on nord) and on 2 of 8 via `success()` (2.67:1 on light, whose `success`
    /// is a dark green precisely because it is meant to be read on white). As
    /// written, every badge passes: 15.54, 8.11, 5.48, 5.32.
    pub fn status_badge(&self, spinner_frame: u64) -> Option<Span<'static>> {
        const SPINNER: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
        let sp = SPINNER[(spinner_frame as usize) % SPINNER.len()];

        match self {
            BridgeConnectionState::Disconnected => None,

            BridgeConnectionState::Connected { peer_count, .. } => {
                let label = if *peer_count > 0 {
                    format!(
                        " REMOTE ({} peer{}) ",
                        peer_count,
                        if *peer_count == 1 { "" } else { "s" }
                    )
                } else {
                    " REMOTE ".to_string()
                };
                Some(Span::styled(
                    label,
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ))
            }

            BridgeConnectionState::Connecting => Some(Span::styled(
                format!(" {} CONNECTING... ", sp),
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )),

            BridgeConnectionState::Reconnecting { attempt } => Some(Span::styled(
                format!(" {} RECONNECTING (#{}) ", sp, attempt),
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )),

            BridgeConnectionState::Failed { .. } => Some(Span::styled(
                " BRIDGE ✗ ".to_string(),
                Style::default()
                    .fg(theme_colors::text())
                    .bg(Color::Red)
                    .add_modifier(Modifier::BOLD),
            )),

            BridgeConnectionState::OutboundOnly => Some(Span::styled(
                " OUTBOUND ".to_string(),
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            )),
        }
    }

    /// Whether this state represents an active/visible connection that occupies
    /// horizontal space in the status bar (i.e. not Disconnected).
    pub fn is_visible(&self) -> bool {
        !matches!(self, BridgeConnectionState::Disconnected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disconnected_produces_no_badge() {
        assert!(
            BridgeConnectionState::Disconnected
                .status_badge(0)
                .is_none()
        );
    }

    #[test]
    fn connected_produces_green_badge() {
        let state = BridgeConnectionState::Connected {
            session_url: "https://example.com".to_string(),
            peer_count: 2,
        };
        let badge = state.status_badge(0).unwrap();
        assert!(badge.content.contains("REMOTE"));
        assert!(badge.content.contains("2"));
    }

    #[test]
    fn failed_produces_red_badge() {
        let state = BridgeConnectionState::Failed {
            reason: "timeout".to_string(),
        };
        let badge = state.status_badge(0).unwrap();
        assert!(badge.content.contains("BRIDGE"));
    }
}
