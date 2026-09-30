// (iter-392: the `#![allow(dead_code)]` suppression is gone — this module is
// reached from `render/welcome.rs::render_welcome_box`.)
use crate::tui::vendor::style::theme;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

// (iter-144: RustlePose enum deleted — rustle_lines() ignores the pose
// anyway (`let _ = pose;`). The App fields (rustle_current_pose,
// rustle_pose_until, rustle_temp_pose, rustle_next_blink) and the
// tick_rustle_pose() method were also deleted — tick was never called.)

fn accent_style() -> Style {
    Style::default()
        .fg(theme::accent_color())
        .add_modifier(Modifier::BOLD)
}

fn dim_style() -> Style {
    Style::default().fg(theme::dim_color())
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

    /// The mascot emits the `Accent` role default; the per-frame substitution
    /// resolves it. Asserting the emitted colour against the active palette
    /// would be wrong by design — see the matching note in `banner.rs`.
    #[test]
    fn rustle_emits_the_accent_role_default() {
        assert_eq!(
            rustle_lines()[1].spans[1].style.fg,
            Some(theme::accent_color())
        );
    }

    #[test]
    fn rustle_wordmark_follows_the_active_theme_accent() {
        use ratatui::layout::Rect;
        use ratatui::widgets::Widget;

        // Column 4 of row 1 is inside the `OPERANT` wordmark span.
        let paint = || {
            let mut buf = ratatui::buffer::Buffer::empty(Rect::new(0, 0, 20, 5));
            for (row, line) in rustle_lines().into_iter().enumerate() {
                line.render(Rect::new(0, row as u16, 20, 1), &mut buf);
            }
            crate::tui::vendor::style::theme_mode::adapt_buffer_for_display(&mut buf);
            buf[(4, 1)].fg
        };

        let _guard = crate::tui::theme_colors::tests::ACTIVE_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());

        crate::tui::theme_colors::set_active_theme("nord");
        let nord = paint();
        assert_eq!(nord, crate::tui::theme_colors::accent());

        crate::tui::theme_colors::set_active_theme("dracula");
        let dracula = paint();
        assert_ne!(nord, dracula, "switching theme must repaint the mascot");

        crate::tui::theme_colors::set_active_theme("default");
    }
}
