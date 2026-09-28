// (iter-392: the `#![allow(dead_code)]` suppression is gone — this module is
// reached from `render/welcome.rs::render_welcome_box`.)
use crate::tui::theme_colors;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

// (iter-144: RustlePose enum deleted — rustle_lines() ignores the pose
// anyway (`let _ = pose;`). The App fields (rustle_current_pose,
// rustle_pose_until, rustle_temp_pose, rustle_next_blink) and the
// tick_rustle_pose() method were also deleted — tick was never called.)

fn accent_style() -> Style {
    Style::default()
        .fg(theme_colors::accent())
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

    /// The mascot wordmark reads the active palette, so `/theme` repaints it.
    /// It used to carry its own inline `Color::Rgb(255, 191, 0)`, which meant
    /// every non-default theme rendered a default-amber mascot next to a
    /// themed banner.
    #[test]
    fn rustle_wordmark_follows_the_active_theme_accent() {
        fn wordmark_fg() -> Color {
            rustle_lines()[1].spans[1].style.fg.unwrap_or(Color::Reset)
        }

        let _guard = crate::tui::theme_colors::tests::ACTIVE_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());

        theme_colors::set_active_theme("nord");
        let nord = wordmark_fg();
        assert_eq!(
            nord,
            theme_colors::accent(),
            "mascot must use the palette accent"
        );

        theme_colors::set_active_theme("dracula");
        let dracula = wordmark_fg();
        assert_ne!(nord, dracula, "switching theme must repaint the mascot");

        theme_colors::set_active_theme("default");
    }
}
