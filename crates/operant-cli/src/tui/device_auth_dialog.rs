// device_auth_dialog.rs — Device code / browser-based auth overlay.
//
// Provides a modal dialog that shows the device code flow status for GitHub
// Copilot (RFC 8628) and browser-based OAuth for Anthropic.  The actual
// network requests run in a background tokio task; this module only owns the
// display state.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::prelude::Stylize;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::tui::overlays::{HINT_ESC, ModalSpec, modal_frame, modal_layout};
use crate::tui::theme_colors;
use crate::tui::vendor::style::theme;

/// Desired width. `modal_layout` clamps it and floors it at `MIN_MODAL_W`.
const DIALOG_WIDTH: u16 = 64;
/// Desired height in the non-URL statuses.
const DIALOG_HEIGHT: u16 = 14;

// ---------------------------------------------------------------------------
// Status enum
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum DeviceAuthStatus {
    /// Dialog is idle / not started.
    Idle,
    /// Requesting the device code from the authorization server.
    WaitingForCode,
    /// User code is displayed; waiting for the user to authorize in-browser.
    ShowingCode,
    /// Actively polling the token endpoint.
    Polling,
    /// Browser-based auth (Anthropic OAuth) — browser was opened.
    BrowserAuth,
    /// Successfully obtained a token.
    Success(String),
    /// An error occurred.
    Error(String),
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

pub struct DeviceAuthDialogState {
    pub visible: bool,
    pub provider_id: String,
    pub provider_name: String,
    pub status: DeviceAuthStatus,
    pub user_code: String,
    pub verification_uri: String,
    pub device_code: String,
    pub interval: u64,
    /// OAuth URL for browser-based flows (Codex). Shown in the dialog so the
    /// user can copy-paste it when automatic browser launch fails.
    pub auth_url: String,
}

impl DeviceAuthDialogState {
    pub fn new() -> Self {
        Self {
            visible: false,
            provider_id: String::new(),
            provider_name: String::new(),
            status: DeviceAuthStatus::Idle,
            user_code: String::new(),
            verification_uri: String::new(),
            device_code: String::new(),
            interval: 5,
            auth_url: String::new(),
        }
    }

    /// Close and reset the dialog.
    pub fn close(&mut self) {
        self.visible = false;
        self.status = DeviceAuthStatus::Idle;
        self.auth_url.clear();
    }
}

// ---------------------------------------------------------------------------
// Events sent from the background task to the main loop
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Render the device auth dialog overlay: dark overlay, rounded modal frame,
/// status-dependent body.
pub fn render_device_auth_dialog(frame: &mut Frame, state: &DeviceAuthDialogState, area: Rect) {
    if !state.visible {
        return;
    }

    let accent = theme_colors::accent();
    let dim = theme::dim_color();

    let title_text = format!("Connect {}", state.provider_name);

    // ── Desired size — taller when showing a browser URL ──
    // The URL wraps against the *clamped* width, so ask `modal_layout` for the
    // geometry once before counting its rows.
    let probe = modal_layout(area, DIALOG_WIDTH, DIALOG_HEIGHT, 1, 0);
    let desired_height = if matches!(state.status, DeviceAuthStatus::BrowserAuth)
        && !state.auth_url.is_empty()
    {
        let wrap_width = probe.body_area.width.saturating_sub(2).max(1);
        let url_lines = (state.auth_url.len() as u16).saturating_add(wrap_width - 1) / wrap_width;
        DIALOG_HEIGHT + url_lines + 2
    } else {
        DIALOG_HEIGHT
    };

    let layout = modal_frame(
        frame,
        area,
        &ModalSpec {
            title: &title_text,
            hint: HINT_ESC,
            width: DIALOG_WIDTH,
            height: desired_height,
            header_height: 1,
            footer_height: 0,
            ..Default::default()
        },
    );
    let inner = layout.body_area;

    // ── Build lines ──
    let mut lines: Vec<Line<'static>> = Vec::new();

    // Status-dependent content
    match &state.status {
        DeviceAuthStatus::Idle | DeviceAuthStatus::WaitingForCode => {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                " Requesting device code...",
                Style::default().fg(theme_colors::warning()),
            )));
        }
        DeviceAuthStatus::ShowingCode | DeviceAuthStatus::Polling => {
            let status_text = if state.status == DeviceAuthStatus::Polling {
                " Checking for authorization..."
            } else {
                " Waiting for authorization..."
            };
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                " Enter this code:",
                Style::default().fg(theme::ai_text()),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!("    {}", state.user_code),
                Style::default()
                    .fg(theme_colors::text())
                    .add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled(" at ", Style::default().fg(dim)),
                Span::styled(
                    state.verification_uri.clone(),
                    Style::default()
                        .fg(accent)
                        .add_modifier(Modifier::UNDERLINED),
                ),
            ]));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                status_text,
                Style::default().fg(theme_colors::warning()),
            )));
        }
        DeviceAuthStatus::BrowserAuth => {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                " Opening browser for authentication...",
                Style::default().fg(theme_colors::warning()),
            )));
            if !state.auth_url.is_empty() {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    " If browser didn't open, visit:",
                    Style::default().fg(theme::ai_text()),
                )));
                lines.push(Line::from(""));
                // Wrap URL to dialog width
                let max_w = inner.width.saturating_sub(2) as usize;
                for chunk in state.auth_url.as_bytes().chunks(max_w.max(1)) {
                    let s = String::from_utf8_lossy(chunk).into_owned();
                    lines.push(Line::from(Span::styled(
                        format!(" {}", s),
                        Style::default()
                            .fg(accent)
                            .add_modifier(Modifier::UNDERLINED),
                    )));
                }
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    " (URL copied to clipboard)",
                    Style::default().fg(dim),
                )));
            } else {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    " Complete the login in your browser.",
                    Style::default().fg(dim),
                )));
                lines.push(Line::from(Span::styled(
                    " This dialog will update when done.",
                    Style::default().fg(dim),
                )));
            }
        }
        DeviceAuthStatus::Success(_) => {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                " \u{2714} Connected successfully!",
                Style::default()
                    .fg(theme::success_color())
                    .add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                " Press any key to continue.",
                Style::default().fg(dim),
            )));
        }
        DeviceAuthStatus::Error(msg) => {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!(" Error: {}", msg),
                Style::default().fg(theme_colors::error()),
            )));
        }
    };

    frame.render_widget(
        Paragraph::new(lines).bg(theme_colors::panel_bg()),
        layout.body_area,
    );
}
