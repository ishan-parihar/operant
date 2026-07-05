use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RustlePose {
    Default,
    LookRight,
    Loading { frame: u64 },
}

fn accent_style() -> Style {
    Style::default()
        .fg(Color::Rgb(255, 191, 0))
        .add_modifier(Modifier::BOLD)
}

fn dim_style() -> Style {
    Style::default().fg(Color::Rgb(140, 110, 0))
}

pub fn rustle_lines(pose: &RustlePose) -> [Line<'static>; 5] {
    let _ = pose;
    [
        Line::from(vec![Span::styled("  ┌──────────┐", dim_style())]),
        Line::from(vec![
            Span::styled("  │ ", dim_style()),
            Span::styled("OPERANT", accent_style()),
            Span::styled("    │", dim_style()),
        ]),
        Line::from(vec![
            Span::styled("  │ ", dim_style()),
            Span::styled("operant", accent_style()),
            Span::styled("   │", dim_style()),
        ]),
        Line::from(vec![
            Span::styled("  │ ", dim_style()),
            Span::styled("v", dim_style()),
            Span::styled(env!("CARGO_PKG_VERSION"), accent_style()),
            Span::styled("       │", dim_style()),
        ]),
        Line::from(vec![Span::styled("  └──────────┘", dim_style())]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rustle_lines_returns_5_lines() {
        for pose in [
            RustlePose::Default,
            RustlePose::LookRight,
            RustlePose::Loading { frame: 0 },
        ] {
            let lines = rustle_lines(&pose);
            assert_eq!(lines.len(), 5);
        }
    }
}
