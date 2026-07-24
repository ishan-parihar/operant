#![allow(dead_code)] // Foundation modules for future multi-crate extraction — wired in Phase 2I
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

// (iter-144: RustlePose enum deleted — rustle_lines() ignores the pose
// anyway (`let _ = pose;`). The App fields (rustle_current_pose,
// rustle_pose_until, rustle_temp_pose, rustle_next_blink) and the
// tick_rustle_pose() method were also deleted — tick was never called.)

fn accent_style() -> Style {
    Style::default()
        .fg(Color::Rgb(255, 191, 0))
        .add_modifier(Modifier::BOLD)
}

fn dim_style() -> Style {
    Style::default().fg(Color::Rgb(140, 110, 0))
}

pub fn rustle_lines() -> [Line<'static>; 5] {
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
        let lines = rustle_lines();
        assert_eq!(lines.len(), 5);
    }
}
