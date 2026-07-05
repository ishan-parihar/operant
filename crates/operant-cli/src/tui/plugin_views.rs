// plugin_views.rs — Plugin hint/recommendation UI banner.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

/// A dismissible banner shown at the top of the message area when a plugin
/// wants to surface a hint or recommendation to the user.
#[derive(Debug, Clone)]
pub struct PluginHintBanner {
    /// The plugin's display name.
    pub plugin_name: String,
    /// The hint / recommendation message.
    pub message: String,
    /// Whether the user has dismissed this banner (it will not be rendered).
    pub dismissed: bool,
}

impl PluginHintBanner {
    pub fn new(plugin_name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            plugin_name: plugin_name.into(),
            message: message.into(),
            dismissed: false,
        }
    }

    /// Mark the banner as dismissed.
    pub fn dismiss(&mut self) {
        self.dismissed = true;
    }

    /// Return `true` if this banner should be shown.
    pub fn is_visible(&self) -> bool {
        !self.dismissed
    }
}

/// Render the first undismissed plugin hint banner into `area`.
/// Returns the height consumed (0 if nothing rendered).
pub fn render_plugin_hints(frame: &mut Frame, hints: &[PluginHintBanner], area: Rect) -> u16 {
    let hint = match hints.iter().find(|h| h.is_visible()) {
        Some(h) => h,
        None => return 0,
    };

    // 3-row banner (border + 1 content line + border)
    let banner_height = 3u16;
    if area.height < banner_height {
        return 0;
    }

    let banner_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: banner_height,
    };

    let inner_width = area.width.saturating_sub(4) as usize;
    let content = format!(" [{}] {} [Esc to dismiss]", hint.plugin_name, hint.message);
    let display = if content.len() > inner_width {
        format!("{}…", &content[..inner_width.saturating_sub(1)])
    } else {
        content
    };

    let lines = vec![Line::from(vec![Span::styled(
        display,
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::ITALIC),
    )])];

    frame.render_widget(Clear, banner_area);
    let para = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" Plugin: {} ", hint.plugin_name))
            .border_style(Style::default().fg(Color::Magenta)),
    );
    frame.render_widget(para, banner_area);

    banner_height
}
