// dialogs/mcp_approval.rs — MCP server approval dialog.
//
// Extracted from dialogs.rs. Owns McpApprovalChoice, McpApprovalDialogState,
// the render_mcp_approval_dialog renderer, and handle_mcp_approval_key.

use crate::tui::overlays::{HINT_ESC, MIN_MODAL_H, ModalSpec, modal_frame_buf, modal_layout};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

use crate::tui::theme_colors;
use crate::tui::vendor::style::theme;

/// Desired width. `modal_layout` clamps it and floors it at `MIN_MODAL_W`.
const DIALOG_WIDTH: u16 = 54;

// ---------------------------------------------------------------------------
// MCP Server Approval Dialog
// ---------------------------------------------------------------------------

/// Which choice the user made in the MCP server approval dialog.
#[derive(Debug, Clone, PartialEq)]
pub enum McpApprovalChoice {
    /// Allow the server for this session only.
    AllowSession,
    /// Persist approval so it survives restarts.
    AllowAlways,
    /// Deny the server connection.
    Deny,
}

impl McpApprovalChoice {
    fn all() -> &'static [McpApprovalChoice] {
        &[
            McpApprovalChoice::AllowSession,
            McpApprovalChoice::AllowAlways,
            McpApprovalChoice::Deny,
        ]
    }

    fn index(&self) -> usize {
        match self {
            McpApprovalChoice::AllowSession => 0,
            McpApprovalChoice::AllowAlways => 1,
            McpApprovalChoice::Deny => 2,
        }
    }

    fn label(&self) -> &'static str {
        match self {
            McpApprovalChoice::AllowSession => "Allow this session",
            McpApprovalChoice::AllowAlways => "Always allow",
            McpApprovalChoice::Deny => "Deny",
        }
    }
}

/// State for the MCP server approval dialog.
#[derive(Debug, Clone)]
pub struct McpApprovalDialogState {
    /// Whether the dialog is currently visible.
    pub visible: bool,
    /// Display name of the MCP server.
    pub server_name: String,
    /// Optional HTTP/WebSocket URL for the server.
    pub server_url: Option<String>,
    /// Optional command used to launch the server (for stdio servers).
    pub server_command: Option<String>,
    /// Tools the server exposes (at most first 5 shown in the UI).
    pub tool_names: Vec<String>,
    /// Currently highlighted choice.
    pub selected: McpApprovalChoice,
}

impl McpApprovalDialogState {
    /// Create a new, invisible state.
    pub fn new() -> Self {
        Self {
            visible: false,
            server_name: String::new(),
            server_url: None,
            server_command: None,
            tool_names: Vec::new(),
            selected: McpApprovalChoice::AllowSession,
        }
    }

    /// Populate and show the dialog.
    #[allow(dead_code)] // MCP approval dialog setup
    pub fn show(
        &mut self,
        server_name: &str,
        server_url: Option<&str>,
        server_command: Option<&str>,
        tool_names: Vec<String>,
    ) {
        self.server_name = server_name.to_string();
        self.server_url = server_url.map(|s| s.to_string());
        self.server_command = server_command.map(|s| s.to_string());
        self.tool_names = tool_names;
        self.selected = McpApprovalChoice::AllowSession;
        self.visible = true;
    }

    /// Move selection to the previous option (wraps around).
    pub fn select_prev(&mut self) {
        let idx = self.selected.index();
        self.selected = McpApprovalChoice::all()[(idx + 2) % 3].clone();
    }

    /// Move selection to the next option (wraps around).
    pub fn select_next(&mut self) {
        let idx = self.selected.index();
        self.selected = McpApprovalChoice::all()[(idx + 1) % 3].clone();
    }

    /// Confirm the current selection and hide the dialog.
    ///
    /// Returns the chosen action.
    pub fn confirm(&mut self) -> McpApprovalChoice {
        let choice = self.selected.clone();
        self.close();
        choice
    }

    /// Hide the dialog without returning a choice (treated as Deny by callers).
    pub fn close(&mut self) {
        self.visible = false;
    }
}

impl Default for McpApprovalDialogState {
    fn default() -> Self {
        Self::new()
    }
}

/// Render the MCP server approval dialog as a centred overlay.
///
/// The `buf` parameter is accepted per the spec; the function actually
/// delegates to the widget system which writes into the terminal buffer
/// through the `Block` / `Paragraph` widgets.  We expose both a
/// `Frame`-based variant (for use from the main render loop) and the
/// low-level `Buffer`-based variant required by the spec.
///
/// Layout:
/// ╭─ MCP Server Connection ─────────────────── Esc to close ─╮
/// │                                                  │
/// │  Server:  my-server                              │
/// │  URL:     wss://example.com/mcp                   │
/// │                                                  │
/// │  Exposes 3 tools:                                 │
/// │    • tool_one                                     │
/// │    tool_two                                       │
/// │    tool_three                                     │
/// │                                                  │
/// │  ▶ [1] Allow this session                        │
/// │    [2] Always allow                               │
/// │    [3] Deny                                       │
/// ╰──────────────────────────────────────────────────╯
pub fn render_mcp_approval_dialog(state: &McpApprovalDialogState, area: Rect, buf: &mut Buffer) {
    if !state.visible {
        return;
    }

    let dim = theme::dim_color();

    // Truncation is measured against the *clamped* width, which only
    // `modal_layout` knows; ask once, then let `modal_frame_buf` recompute the
    // identical layout to paint the chrome.
    let probe = modal_layout(area, DIALOG_WIDTH, MIN_MODAL_H, 1, 0);
    let text_width = (probe.dialog_area.width as usize).saturating_sub(4);

    // Count body rows: server row + url/command row + blank + optional tool
    // block + 3 option rows + trailing blank.
    let tool_display_count = state.tool_names.len().min(5);
    let has_tools = tool_display_count > 0;
    let has_url_or_cmd = state.server_url.is_some() || state.server_command.is_some();

    let content_height: u16 = 1 // "Server: ..."
        + if has_url_or_cmd { 1 } else { 0 }
        + 1 // blank
        + if has_tools { 1 + tool_display_count as u16 + 1 } else { 0 } // header + items + blank
        + 3 // 3 option rows
        + 1; // trailing blank

    // One title row + the body rows + the two border rows the primitive insets.
    let desired_height = 1 + content_height + 2;

    let mut lines: Vec<Line> = Vec::new();

    // Server name.
    let server_label = format!(
        "  Server:  {}",
        truncate_str(&state.server_name, text_width.saturating_sub(10))
    );
    lines.push(Line::from(vec![
        Span::styled("  Server:  ", Style::default().fg(dim)),
        Span::styled(
            truncate_str(&state.server_name, text_width.saturating_sub(10)),
            Style::default()
                .fg(theme_colors::accent())
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    let _ = server_label; // suppress unused warning

    // URL or command.
    if let Some(ref url) = state.server_url {
        lines.push(Line::from(vec![
            Span::styled("  URL:     ", Style::default().fg(dim)),
            Span::styled(
                truncate_str(url, text_width.saturating_sub(10)),
                Style::default().fg(theme_colors::text()),
            ),
        ]));
    } else if let Some(ref cmd) = state.server_command {
        lines.push(Line::from(vec![
            Span::styled("  Command: ", Style::default().fg(dim)),
            Span::styled(
                truncate_str(cmd, text_width.saturating_sub(10)),
                Style::default().fg(theme_colors::text()),
            ),
        ]));
    }

    lines.push(Line::from(""));

    // Tools list.
    if has_tools {
        let extra = state.tool_names.len().saturating_sub(5);
        lines.push(Line::from(vec![Span::styled(
            format!(
                "  Exposes {} tool{}{}:",
                state.tool_names.len(),
                if state.tool_names.len() == 1 { "" } else { "s" },
                if extra > 0 {
                    format!(" (showing first 5 of {})", state.tool_names.len())
                } else {
                    String::new()
                },
            ),
            Style::default().fg(dim),
        )]));
        for name in state.tool_names.iter().take(5) {
            lines.push(Line::from(vec![
                Span::styled("    \u{2022} ", Style::default().fg(dim)),
                Span::styled(
                    truncate_str(name, text_width.saturating_sub(6)),
                    Style::default().fg(theme_colors::text()),
                ),
            ]));
        }
        lines.push(Line::from(""));
    }

    // Options.
    for choice in McpApprovalChoice::all() {
        let is_selected = *choice == state.selected;
        let prefix = if is_selected { "  \u{25BA} " } else { "    " };
        let num = choice.index() + 1;
        let key_style = if is_selected {
            Style::default()
                .fg(theme_colors::accent())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(dim)
        };
        let label_style = if is_selected {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        // Deny option gets a red tint when selected.
        let label_style = if is_selected && *choice == McpApprovalChoice::Deny {
            Style::default()
                .fg(theme_colors::error())
                .add_modifier(Modifier::BOLD)
        } else {
            label_style
        };
        lines.push(Line::from(vec![
            Span::raw(prefix),
            Span::styled(format!("[{}]", num), key_style),
            Span::raw(" "),
            Span::styled(choice.label(), label_style),
        ]));
    }

    let layout = modal_frame_buf(
        buf,
        area,
        &ModalSpec {
            title: "MCP Server Connection",
            hint: HINT_ESC,
            width: DIALOG_WIDTH,
            height: desired_height,
            header_height: 1,
            footer_height: 0,
            ..Default::default()
        },
    );

    Paragraph::new(lines).render(layout.body_area, buf);
}

/// Handle a key event while the MCP approval dialog is open.
///
/// Returns `Some(choice)` when the user confirms (Enter or digit shortcut),
/// or `Some(Deny)` when Esc is pressed.  Returns `None` for navigation keys.
pub fn handle_mcp_approval_key(
    state: &mut McpApprovalDialogState,
    key: KeyEvent,
) -> Option<McpApprovalChoice> {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            state.select_prev();
            None
        }
        KeyCode::Down | KeyCode::Char('j') => {
            state.select_next();
            None
        }
        KeyCode::Enter => Some(state.confirm()),
        KeyCode::Char('1') => {
            state.selected = McpApprovalChoice::AllowSession;
            Some(state.confirm())
        }
        KeyCode::Char('2') => {
            state.selected = McpApprovalChoice::AllowAlways;
            Some(state.confirm())
        }
        KeyCode::Char('3') | KeyCode::Char('n') => {
            state.selected = McpApprovalChoice::Deny;
            Some(state.confirm())
        }
        KeyCode::Esc => {
            state.close();
            Some(McpApprovalChoice::Deny)
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Truncate a string to at most `max_width` display cells, appending `…` if cut.
pub(crate) fn truncate_str(s: &str, max_width: usize) -> String {
    use crate::tui::render::{display_width, take_width};
    if max_width == 0 {
        return String::new();
    }
    if display_width(s) <= max_width {
        return s.to_string();
    }
    // Reserve the final cell for the ellipsis.
    let mut out = take_width(s, max_width - 1);
    out.push('\u{2026}');
    out
}
