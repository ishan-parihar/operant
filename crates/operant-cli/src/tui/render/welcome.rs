// render/welcome.rs — Startup notices, banner block, welcome box.

use crate::tui::adapter_types::constants::APP_VERSION;
use crate::tui::app::App;
use crate::tui::rustle::rustle_lines;
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph, Wrap};

use super::{truncate_end, ACCENT_PRIMARY, WELCOME_BOX_HEIGHT};

fn startup_notice_lines(app: &App, width: u16) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let max_width = width.saturating_sub(10) as usize;
    // (iter-141: away_summary render branch deleted — field was always None)

    // Bridge connection state is always Disconnected today (bridge feature
    // not yet wired). When it is, restore the Connected/Reconnecting/Failed
    // match arms here from git history.

    if let Some(url) = app.remote_session_url.as_deref() {
        lines.push(Line::from(vec![
            Span::styled(" link ", Style::default().fg(ACCENT_PRIMARY)),
            Span::styled(
                truncate_end(url, max_width),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }

    // Additional directories (from --add-dir)
    // ponytail: Config.additional_dirs not in stub; omitted until added

    lines
}

fn render_startup_notices(frame: &mut Frame, app: &App, area: Rect) {
    if area.height == 0 {
        return;
    }
    let lines = startup_notice_lines(app, area.width);
    if lines.is_empty() {
        return;
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

#[derive(Clone)]
fn render_banner_block(frame: &mut Frame, _app: &App, area: Rect) {
    use crate::tui::banner;

    if area.height == 0 || area.width == 0 {
        return;
    }

    let lines = banner::banner_with_subtitle(area.width, APP_VERSION);

    // Center each line horizontally. The art is fixed-width so compute the
    // indent once from the longest line's display width.
    let max_len = lines
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.chars().count())
                .sum::<usize>()
        })
        .max()
        .unwrap_or(0);
    let indent = area.width.saturating_sub(max_len as u16) / 2;
    let pad = " ".repeat(indent as usize);

    let mut padded: Vec<Line> = Vec::with_capacity(lines.len());
    for line in lines {
        let mut spans: Vec<Span> = Vec::with_capacity(line.spans.len() + 1);
        if !pad.is_empty() {
            spans.push(Span::raw(pad.clone()));
        }
        spans.extend(line.spans);
        padded.push(Line::from(spans));
    }

    // Vertical-center within the banner area (small bias toward the top).
    let v_pad = area.height.saturating_sub(padded.len() as u16) / 2;
    let mut all_lines: Vec<Line> = Vec::with_capacity(area.height as usize);
    for _ in 0..v_pad {
        all_lines.push(Line::from(""));
    }
    all_lines.extend(padded);

    frame.render_widget(Paragraph::new(all_lines), area);
}

/// Render the two-column orange round-bordered welcome box (matches TS LogoV2).
fn render_welcome_box(frame: &mut Frame, app: &App, area: Rect) {
    // --- Box dimensions ---
    // The box should be at most the full area width, and a fixed height.
    let box_width = area.width;
    let box_height: u16 = WELCOME_BOX_HEIGHT;
    if area.height < box_height || box_width < 30 {
        // Too small: fall back to a single line
        let line = Line::from(vec![
            Span::styled(
                "Operant ",
                Style::default()
                    .fg(ACCENT_PRIMARY)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("v{}", APP_VERSION),
                Style::default().fg(Color::DarkGray),
            ),
        ]);
        frame.render_widget(Paragraph::new(vec![line]), area);
        return;
    }
    let box_area = Rect {
        x: area.x,
        y: area.y,
        width: box_width,
        height: box_height,
    };

    // Outer border with title "Operant vX.Y"
    let accent = app.accent_color;
    let outer_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(accent))
        .title(Line::from(vec![
            Span::styled(
                " Operant ",
                Style::default().fg(accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("v{} ", APP_VERSION),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    frame.render_widget(outer_block, box_area);

    // Inner area (inside the border)
    let inner = Rect {
        x: box_area.x + 1,
        y: box_area.y + 1,
        width: box_area.width.saturating_sub(2),
        height: box_area.height.saturating_sub(2),
    };

    // Split inner into left | divider(1) | right
    // Left width: ~28 chars or half the inner width, whichever is smaller
    let left_w = (inner.width / 2)
        .clamp(22, 32)
        .min(inner.width.saturating_sub(3));
    let right_w = inner.width.saturating_sub(left_w + 1);
    let h_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(left_w),
            Constraint::Length(1),
            Constraint::Length(right_w),
        ])
        .split(inner);

    // Store the right column area for error modal positioning
    app.footer_right_column_area.set(h_chunks[2]);

    // Draw vertical divider in accent color
    let divider_lines: Vec<Line> = (0..inner.height)
        .map(|_| Line::from(Span::styled("\u{2502}", Style::default().fg(accent))))
        .collect();
    frame.render_widget(Paragraph::new(divider_lines), h_chunks[1]);

    // --- Left column ---
    let username = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .ok()
        .filter(|u| !u.is_empty());
    let welcome_msg = if let Some(ref name) = username {
        format!("Welcome back {}!", name)
    } else {
        "Welcome back!".to_string()
    };
    let rustle = rustle_lines();
    let mut left_lines: Vec<Line> = Vec::new();
    left_lines.push(Line::from(Span::styled(
        welcome_msg,
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )));
    left_lines.push(Line::from(""));
    // Center mascot in left column
    let mascot_indent = left_w.saturating_sub(11) / 2;
    let pad = " ".repeat(mascot_indent as usize);
    for cl in &rustle {
        let mut spans = vec![Span::raw(pad.clone())];
        spans.extend(cl.spans.iter().cloned());
        left_lines.push(Line::from(spans));
    }
    frame.render_widget(
        Paragraph::new(left_lines).wrap(Wrap { trim: false }),
        h_chunks[0],
    );

    // --- Right column ---
    // Use a STABLE seed (session start time) so the tip stays fixed for the
    // entire session. Was using app.frame_count which increments every frame
    // (~20fps), causing the tip to rotate 20 times per second in an infinite
    // loop. (iter-118 — user-reported bug.)
    let tip_seed = app.session_start.elapsed().as_secs();
    let tip_text = crate::tui::adapter_types::tips::select_tip(tip_seed)
        .unwrap_or_else(|| "Edit AGENTS.md to add instructions for Operant".to_string());

    let mut right_lines: Vec<Line> = Vec::new();
    right_lines.push(Line::from(Span::styled(
        "Tips for getting started",
        Style::default().fg(accent).add_modifier(Modifier::BOLD),
    )));
    // Word-wrap the tip text into the right column width
    let right_w_usize = right_w.saturating_sub(1) as usize;
    for chunk in tip_text
        .chars()
        .collect::<Vec<_>>()
        .chunks(right_w_usize.max(1))
    {
        right_lines.push(Line::from(chunk.iter().collect::<String>()));
    }

    // Example prompts — reduce "blank page" paralysis by showing the user
    // what they can ask. (P1-12 from UX audit.)
    right_lines.push(Line::from(""));
    right_lines.push(Line::from(Span::styled(
        "Try asking",
        Style::default().fg(accent).add_modifier(Modifier::BOLD),
    )));
    const EXAMPLE_PROMPTS: &[&str] = &[
        "Help me understand this codebase",
        "Write a function to parse JSON",
        "What patterns do you notice in my work?",
        "Set up a morning brief — operant cron blueprint morning-brief",
    ];
    // Rotate one example per session using the same stable seed.
    let prompt_idx = (tip_seed as usize) % EXAMPLE_PROMPTS.len();
    let example = EXAMPLE_PROMPTS[prompt_idx];
    for chunk in example
        .chars()
        .collect::<Vec<_>>()
        .chunks(right_w_usize.max(1))
    {
        right_lines.push(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(
                chunk.iter().collect::<String>(),
                Style::default().fg(Color::Gray),
            ),
        ]));
    }

    right_lines.push(Line::from(""));
    right_lines.push(Line::from(Span::styled(
        "Available Tools",
        Style::default().fg(accent).add_modifier(Modifier::BOLD),
    )));
    // Show tool/MCP/skills counts like hermes-agent's header.
    // (iter-124 — user-requested: add tools/mcp/skills info to header.)
    let tool_count = app.tool_use_blocks.len();
    let mcp_count = app.config.mcp.servers.iter().filter(|s| s.enabled).count();
    let skills_count = app.skills_view.skills.len();
    let mem_count = {
        let mem_dir = operant_core::platform::operant_home().join("memory");
        operant_core::memory::MemoryStore::new(mem_dir)
            .read_memories()
            .map(|m| m.len())
            .unwrap_or(0)
    };
    right_lines.push(Line::from(vec![
        Span::styled("  ", Style::default()),
        Span::styled(
            format!("{} tools", tool_count.max(1)),
            Style::default().fg(Color::White),
        ),
        Span::styled(" · ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{} MCP", mcp_count),
            Style::default().fg(Color::White),
        ),
        Span::styled(" · ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{} skills", skills_count),
            Style::default().fg(Color::White),
        ),
        Span::styled(" · ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{} memories", mem_count),
            Style::default().fg(Color::White),
        ),
    ]));
    right_lines.push(Line::from(Span::styled(
        "  /help for commands",
        Style::default().fg(Color::DarkGray),
    )));

    frame.render_widget(
        Paragraph::new(right_lines).wrap(Wrap { trim: false }),
        h_chunks[2],
    );
}

// â”€â”€ Per-message rendering â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

/// Build a tool_use_id → tool_name lookup from all messages in the transcript.
/// This allows ToolResult blocks to dispatch to tool-specific renderers.
