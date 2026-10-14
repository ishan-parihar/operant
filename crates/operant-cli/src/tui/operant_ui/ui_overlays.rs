// Vendored from jcode (crates/operant-tui/src/tui/ui_overlays.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805; imports
// re-rooted per the batch-3 map. See operant_ui/mod.rs for scope.

use super::{
    accent_color, ai_color, ai_text, asap_color, clear_area, dim_color, get_grouped_changelog,
    header_icon_color, header_name_color, header_session_color, pending_color, queued_color,
    record_chat_overlay_copy_snapshot, rgb, tool_color, user_bg, user_color, user_text,
};
use crate::tui::operant_app::info_widget::WidgetPlacement;
use crate::tui::operant_app::tui_fns::TuiState;
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Paragraph},
};

use super::selection_highlight::highlight_line_selection;

pub(super) fn draw_changelog_overlay(
    frame: &mut Frame,
    area: Rect,
    scroll: usize,
    app: &dyn TuiState,
) {
    clear_area(frame, area);

    let groups = get_grouped_changelog();
    let mut lines: Vec<Line<'static>> = Vec::new();

    if groups.is_empty() {
        lines.push(Line::from(Span::styled(
            "No changelog entries available.",
            Style::default().fg(dim_color()),
        )));
    } else {
        for group in &groups {
            let heading = match &group.released_at {
                Some(released_at) => format!("  {} · {}", group.version, released_at),
                None => format!("  {}", group.version),
            };
            lines.push(Line::from(Span::styled(
                heading,
                Style::default()
                    .fg(rgb(200, 200, 220))
                    .add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(""));
            for entry in &group.entries {
                lines.push(Line::from(vec![
                    Span::styled("    • ", Style::default().fg(dim_color())),
                    Span::styled(entry.clone(), Style::default().fg(rgb(170, 170, 185))),
                ]));
            }
            lines.push(Line::from(""));
        }
    }

    let total_lines = lines.len();
    let visible_height = area.height.saturating_sub(2) as usize;
    let max_scroll = total_lines.saturating_sub(visible_height);
    let scroll = scroll.min(max_scroll);

    let scroll_info = if total_lines > visible_height {
        #[allow(clippy::manual_checked_ops)] // vendored-verbatim / re-export for cutover consumers
        let pct = if max_scroll > 0 {
            (scroll * 100) / max_scroll
        } else {
            100
        };
        format!(" {}% ", pct)
    } else {
        String::new()
    };

    let title = format!(" Changelog {} ", scroll_info);
    let block = Block::default()
        .title(Span::styled(
            title,
            Style::default()
                .fg(rgb(200, 200, 220))
                .add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Line::from(Span::styled(
            " Esc to close · drag to select, release to copy · wheel/j/k scroll ",
            Style::default().fg(dim_color()),
        )))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(dim_color()));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let visible_end = scroll
        .saturating_add(inner.height as usize)
        .min(total_lines);

    // Register the rendered lines so the shared copy-selection machinery can map
    // mouse drags to text and highlight + copy the selection, exactly like the
    // chat viewport. Without this, mouse capture would block native terminal
    // selection and there would be no way to copy from the overlay.
    record_chat_overlay_copy_snapshot(&lines, scroll, visible_end, inner);

    let mut visible_lines: Vec<Line<'static>> =
        lines.get(scroll..visible_end).unwrap_or(&[]).to_vec();

    if let Some(range) = app.copy_selection_range().filter(|range| {
        range.start.pane == crate::tui::operant_ui::copy_selection::CopySelectionPane::Chat
            && range.end.pane == crate::tui::operant_ui::copy_selection::CopySelectionPane::Chat
    }) {
        let (start, end) = if (range.start.abs_line, range.start.column)
            <= (range.end.abs_line, range.end.column)
        {
            (range.start, range.end)
        } else {
            (range.end, range.start)
        };
        for abs_idx in start.abs_line.max(scroll)..=end.abs_line.min(visible_end.saturating_sub(1))
        {
            let rel_idx = abs_idx.saturating_sub(scroll);
            if let Some(line) = visible_lines.get_mut(rel_idx) {
                let start_col = if abs_idx == start.abs_line {
                    start.column
                } else {
                    0
                };
                let end_col = if abs_idx == end.abs_line {
                    end.column
                } else {
                    line.width()
                };
                *line = highlight_line_selection(line, start_col, end_col);
            }
        }
    }

    frame.render_widget(Paragraph::new(visible_lines), inner);
}

pub(super) fn draw_help_overlay(frame: &mut Frame, area: Rect, scroll: usize, app: &dyn TuiState) {
    clear_area(frame, area);

    let section_style = Style::default()
        .fg(accent_color())
        .add_modifier(Modifier::BOLD);
    let cmd_style = Style::default().fg(rgb(230, 230, 240));
    let desc_style = Style::default().fg(rgb(150, 150, 165));
    let key_style = Style::default().fg(rgb(200, 180, 120));
    let sep_style = Style::default().fg(rgb(50, 50, 55));

    let mut lines: Vec<Line<'static>> = Vec::new();

    let separator = || -> Line<'static> {
        Line::from(Span::styled(
            "  ─────────────────────────────────────────────────",
            sep_style,
        ))
    };

    let help_entry = |cmd: &str, desc: &str| -> Line<'static> {
        Line::from(vec![
            Span::styled("    ", Style::default()),
            Span::styled(cmd.to_string(), cmd_style),
            Span::styled("  ", Style::default()),
            Span::styled(desc.to_string(), desc_style),
        ])
    };

    let alt = crate::tui::operant_app::keybind::alt_chord;
    let key_entry = |key: &str, desc: &str| -> Line<'static> {
        Line::from(vec![
            Span::styled("    ", Style::default()),
            Span::styled(format!("{:<22}", key), key_style),
            Span::styled(desc.to_string(), desc_style),
        ])
    };

    lines.push(Line::from(""));

    lines.push(Line::from(Span::styled("  Commands", section_style)));
    lines.push(Line::from(""));
    lines.push(help_entry("/help", "Show this help overlay"));
    lines.push(help_entry(
        "/help <command>",
        "Show details for one command",
    ));
    lines.push(help_entry("/model", "List or switch models"));
    lines.push(help_entry("/model <name>", "Switch to a different model"));
    lines.push(help_entry("/agents", "Configure models for agent roles"));
    lines.push(help_entry(
        "/effort <level>",
        "Set effort (none|minimal|low|medium|high|xhigh|max|swarm|swarm-deep)",
    ));
    lines.push(help_entry(
        "/fast [on|off|status|default ...]",
        "Toggle fast mode",
    ));
    lines.push(help_entry("/config", "Show active configuration"));
    lines.push(help_entry("/config init", "Create default config file"));
    lines.push(help_entry("/config edit", "Open config in $EDITOR"));
    lines.push(help_entry(
        "/context",
        "Show the full session context snapshot",
    ));
    lines.push(help_entry(
        "/skills",
        "Show loaded skills and operant-endorsed recommendations",
    ));
    lines.push(help_entry("/info", "Show session info and token usage"));
    lines.push(help_entry(
        "/keys",
        "Show keybinding conflicts with your terminal/OS",
    ));
    lines.push(help_entry("/usage", "Show connected provider usage limits"));
    lines.push(help_entry("/version", "Show version and build details"));
    lines.push(help_entry(
        "/changelog",
        "Show recent changes in this build",
    ));

    lines.push(Line::from(""));
    lines.push(separator());
    lines.push(Line::from(""));

    lines.push(Line::from(Span::styled("  Session", section_style)));
    lines.push(Line::from(""));
    lines.push(help_entry("/clear", "Clear conversation and start fresh"));
    lines.push(help_entry(
        "/compact",
        "Summarize old messages to free context",
    ));
    lines.push(help_entry(
        "/rewind",
        "Show numbered history, /rewind N to rewind",
    ));
    lines.push(help_entry(
        "/fix",
        "Attempt recovery when model cannot continue",
    ));
    lines.push(help_entry(
        "/plan [goal]",
        "Draft a plan-only proposal as a plan card (no edits)",
    ));
    lines.push(help_entry(
        "/improve",
        "Autonomously improve the repo until returns diminish",
    ));
    lines.push(help_entry(
        "/improve resume",
        "Resume the last saved improve loop/plan",
    ));
    lines.push(help_entry(
        "/refactor",
        "Run a safe refactor loop with independent review",
    ));
    lines.push(help_entry(
        "/refactor resume",
        "Resume the last saved refactor loop/plan",
    ));
    lines.push(help_entry("/resume", "Browse and resume previous sessions"));
    lines.push(help_entry("/save [label]", "Bookmark session for /resume"));
    lines.push(help_entry(
        "/rename <name>|--clear",
        "Set or clear current session name",
    ));

    lines.push(Line::from(""));
    lines.push(separator());
    lines.push(Line::from(""));

    lines.push(Line::from(Span::styled("  Memory & Swarm", section_style)));
    lines.push(Line::from(""));
    lines.push(help_entry("/memory [on|off]", "Toggle memory features"));

    lines.push(Line::from(""));
    lines.push(separator());
    lines.push(Line::from(""));

    lines.push(Line::from(Span::styled("  Auth & Accounts", section_style)));
    lines.push(Line::from(""));
    lines.push(help_entry("/auth", "Show authentication status"));
    lines.push(help_entry(
        "/login [provider]",
        "Interactive or direct login",
    ));

    lines.push(Line::from(""));
    lines.push(separator());
    lines.push(Line::from(""));

    lines.push(Line::from(Span::styled("  System", section_style)));
    lines.push(Line::from(""));
    lines.push(help_entry("/reload", "Reload to newer binary if available"));
    if app.is_remote_mode() {
        lines.push(help_entry(
            "/continue",
            "Continue every interrupted live session that would auto-resume",
        ));
    }
    lines.push(help_entry("/quit", "Exit operant"));

    // The sections above are hand-curated for ordering, but they drift as
    // commands are added. Anything registered and not already shown gets listed
    // here so no working command is invisible in /help.
    let shown: std::collections::HashSet<String> = lines
        .iter()
        .filter_map(|line| line.spans.get(1).map(|span| span.content.to_string()))
        .map(|cmd| {
            cmd.split_whitespace()
                .next()
                .unwrap_or_default()
                .to_string()
        })
        .collect();
    let uncovered: Vec<(&str, &str)> = crate::tui::operant_app::app::registered_command_entries()
        .filter(|(name, _)| !shown.contains(*name))
        .collect();
    if !uncovered.is_empty() {
        lines.push(Line::from(""));
        lines.push(separator());
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("  More commands", section_style)));
        lines.push(Line::from(""));
        for (name, desc) in uncovered {
            lines.push(help_entry(name, desc));
        }
    }

    let skills = app.available_skills();
    if !skills.is_empty() {
        lines.push(Line::from(""));
        lines.push(separator());
        lines.push(Line::from(""));

        lines.push(Line::from(Span::styled("  Skills", section_style)));
        lines.push(Line::from(""));
        for skill in &skills {
            lines.push(help_entry(&format!("/{}", skill), "Activate skill"));
        }
    }

    lines.push(Line::from(""));
    lines.push(separator());
    lines.push(Line::from(""));

    lines.push(Line::from(Span::styled("  Navigation", section_style)));
    lines.push(Line::from(""));
    lines.push(key_entry("PageUp / PageDown", "Scroll history"));
    lines.push(key_entry("Up / Down", "Scroll history (when input empty)"));
    lines.push(key_entry(
        "Ctrl+J / Ctrl+K",
        "Jump to next / previous user prompt (also Ctrl+] / Ctrl+[)",
    ));
    lines.push(key_entry(
        "Ctrl+Shift+J / Ctrl+Shift+K",
        "Scroll history down / up one line",
    ));
    lines.push(key_entry(
        "Cmd/Super+K / J",
        "Jump to previous / next user prompt (macOS, if forwarded)",
    ));
    lines.push(key_entry("Ctrl+1..4", "Resize side panel to 25/50/75/100%"));
    lines.push(key_entry(
        "Ctrl+5..9",
        "Jump by recency (5 = 5th most recent)",
    ));

    lines.push(Line::from(""));
    lines.push(separator());
    lines.push(Line::from(""));

    lines.push(Line::from(Span::styled(
        "  Diagrams & Diffs",
        section_style,
    )));
    lines.push(Line::from(""));
    #[cfg(any())]
    // [port-decision] keybind registry not ported (side_panel_toggle_key_label); re-activate at cutover
    lines.push(key_entry(
        &crate::tui::operant_app::keybind::side_panel_toggle_key_label(),
        "Cycle side panel: split, fullscreen, hidden",
    ));
    #[cfg(any())]
    // [port-decision] keybind registry not ported (diagram_pane_visibility_key_label); re-activate at cutover
    lines.push(key_entry(
        &crate::tui::operant_app::keybind::diagram_pane_visibility_key_label(),
        "Show/hide diagram pane",
    ));
    lines.push(key_entry(&alt("T"), "Toggle diagram position (side/top)"));
    lines.push(key_entry(
        &alt("Shift+I"),
        "Show/hide inline images (persists)",
    ));
    lines.push(key_entry("Ctrl+H / Ctrl+L", "Focus chat / diagram / diffs"));
    lines.push(key_entry(
        "Ctrl+L / Cmd+L",
        "Clear screen, history stays above (no pane focused)",
    ));
    lines.push(key_entry(
        "Ctrl+Left / Right",
        "Cycle diagrams (when diagram focused)",
    ));
    lines.push(key_entry("h/j/k/l / arrows", "Pan diagram (when focused)"));
    lines.push(key_entry("[ / ]", "Zoom diagram (when focused)"));
    lines.push(key_entry("+ / -", "Resize diagram pane"));
    lines.push(key_entry(
        &format!("{} / /diff", alt("G")),
        "Cycle diff mode (Off/Inline/Pinned/File)",
    ));
    lines.push(key_entry("Shift+Tab", "Cycle favorited models"));
    lines.push(key_entry("Ctrl+O", "Set default model (in /model picker)"));
    lines.push(key_entry(
        "Ctrl+N",
        "Toggle favorite model (in /model picker)",
    ));

    lines.push(Line::from(""));
    lines.push(separator());
    lines.push(Line::from(""));

    lines.push(Line::from(Span::styled("  Input & Editing", section_style)));
    lines.push(Line::from(""));
    lines.push(key_entry(
        "Ctrl+C / Ctrl+D",
        "Quit (press twice to confirm)",
    ));
    lines.push(key_entry("Ctrl+X", "Cut entire input line to clipboard"));
    lines.push(key_entry(
        "Ctrl+A",
        "Copy visible chat viewport plus nearby context",
    ));
    lines.push(key_entry("Ctrl+U", "Clear input line"));
    lines.push(key_entry("Ctrl+K", "Delete to end of input"));
    lines.push(key_entry(
        &format!("{} / {}", alt("Backspace"), alt("Delete")),
        "Delete previous word in input",
    ));
    lines.push(key_entry(
        "Cmd/Super+Backspace / Delete",
        "Delete previous word in input",
    ));
    if cfg!(target_os = "macos") {
        // On macOS, Cmd+Left/Right default to effort cycling; Home/End and
        // Cmd+A/E still jump to the start/end of the input.
        lines.push(key_entry("Home / End", "Move to start / end of input"));
    } else {
        lines.push(key_entry(
            "Cmd/Super+Left / Right",
            "Move to start / end of input",
        ));
    }
    lines.push(key_entry("Cmd/Super+Z", "Undo input edit"));
    lines.push(key_entry("Cmd/Super+X / V", "Cut input / paste clipboard"));
    lines.push(key_entry("Ctrl+S", "Stash / pop input (save for later)"));
    lines.push(key_entry("Ctrl+Backspace", "Delete previous word in input"));
    lines.push(key_entry("Ctrl+B / Ctrl+F", "Move by word left / right"));
    lines.push(key_entry("Ctrl+Left / Right", "Move by word left / right"));
    lines.push(key_entry(
        &format!("Shift+Enter / {}", alt("Enter")),
        "Insert newline in input",
    ));
    lines.push(key_entry(
        "Trailing \\ then Enter",
        "Insert newline (fallback; run /terminal-setup to fix Shift+Enter)",
    ));
    lines.push(key_entry(
        "Ctrl+Enter / Cmd+Enter",
        "Use opposite send mode while processing",
    ));
    lines.push(key_entry("Ctrl+Up", "Retrieve pending message for editing"));
    lines.push(key_entry("Ctrl+Tab / Ctrl+T", "Toggle queue mode"));
    lines.push(key_entry(
        "Ctrl+R",
        "Search prompt history (across sessions)",
    ));
    lines.push(key_entry(
        &format!("Ctrl+V / {}", alt("V")),
        "Paste clipboard (text or image)",
    ));
    lines.push(key_entry(
        &alt("A"),
        "Quick-copy visible chat viewport plus nearby context",
    ));
    lines.push(key_entry(&alt("Y"), "Toggle chat selection/copy mode"));
    lines.push(key_entry(&alt("S"), "Toggle typing scroll lock"));
    lines.push(key_entry("Ctrl+P", "Toggle auto-poke for incomplete todos"));
    lines.push(key_entry(&alt("X"), "Show/dismiss todo list card in chat"));
    #[cfg(any())]
    // [port-decision] keybind registry not ported (effort_switch_keys_label); re-activate at cutover
    lines.push(key_entry(
        &crate::tui::operant_app::keybind::effort_switch_keys_label(),
        "Cycle effort (reasoning + swarm)",
    ));
    if cfg!(target_os = "macos") {
        lines.push(key_entry(
            &alt("Left / Right"),
            &format!("Move by word in input (also {} / {})", alt("B"), alt("F")),
        ));
    }
    if let Some(label) = app.dictation_key_label() {
        lines.push(key_entry(&label, "Run configured dictation"));
    }
    #[cfg(any())]
    // [port-decision] keybind registry not ported (load_open_resume_key); re-activate at cutover
    if let Some(label) = crate::tui::operant_app::keybind::load_open_resume_key().label {
        lines.push(key_entry(&label, "Open the /resume session picker"));
    }
    #[cfg(any())]
    // [port-decision] keybind registry not ported (load_new_terminal_key); re-activate at cutover
    if let Some(label) = crate::tui::operant_app::keybind::load_new_terminal_key().label {
        lines.push(key_entry(
            &label,
            "Spawn new operant session in a new terminal",
        ));
    }

    lines.push(Line::from(""));

    let total_lines = lines.len();
    let visible_height = area.height.saturating_sub(2) as usize;
    #[allow(clippy::manual_checked_ops)] // vendored-verbatim upstream scrollbar math
    let max_scroll = total_lines.saturating_sub(visible_height);
    let scroll = scroll.min(max_scroll);

    let scroll_info = if total_lines > visible_height {
        #[allow(clippy::manual_checked_ops)] // vendored-verbatim upstream scrollbar math
        let pct = if max_scroll > 0 {
            (scroll * 100) / max_scroll
        } else {
            100
        };
        format!(" {}% ", pct)
    } else {
        String::new()
    };

    let title = format!(" Help {} ", scroll_info);
    let block = Block::default()
        .title(Span::styled(
            title,
            Style::default()
                .fg(rgb(200, 200, 220))
                .add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Line::from(Span::styled(
            " Esc to close · mouse wheel/j/k scroll · Space/PageUp page · /help <cmd> for details ",
            Style::default().fg(dim_color()),
        )))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(dim_color()));

    let paragraph = Paragraph::new(lines)
        .block(block)
        .scroll((scroll as u16, 0));

    frame.render_widget(paragraph, area);
}

// (2026-10-14 batch, plan 02) `draw_model_status_overlay` and its
// `model_status_line_style` helper were the orphaned rendering path of the
// dead /provider-test-coverage command: `model_status_overlay()` had no
// override anywhere, so the overlay could never show. Deleted with the
// command, along with the live_tests classifier module it alone consumed.

pub(super) fn draw_debug_overlay(
    frame: &mut Frame,
    placements: &[WidgetPlacement],
    chunks: &[Rect],
) {
    if chunks.len() < 5 {
        return;
    }
    render_overlay_box(frame, chunks[0], "messages", Color::Red);
    render_overlay_box(frame, chunks[1], "queued", Color::Yellow);
    render_overlay_box(frame, chunks[2], "status", Color::Cyan);
    render_overlay_box(frame, chunks[3], "picker", Color::Magenta);
    render_overlay_box(frame, chunks[4], "input", Color::Green);
    if chunks.len() > 5 && chunks[5].height > 0 {
        render_overlay_box(frame, chunks[5], "animation", Color::Blue);
    }

    for placement in placements {
        let title = format!("widget:{}", placement.kind.as_str());
        render_overlay_box(frame, placement.rect, &title, Color::Magenta);
    }
}

fn render_overlay_box(frame: &mut Frame, area: Rect, title: &str, color: Color) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color))
        .title(Span::styled(title.to_string(), Style::default().fg(color)));
    frame.render_widget(block, area);
}

pub(super) fn debug_palette_json() -> Option<serde_json::Value> {
    Some(serde_json::json!({
        "user_color": color_to_rgb(user_color()),
        "ai_color": color_to_rgb(ai_color()),
        "tool_color": color_to_rgb(tool_color()),
        "dim_color": color_to_rgb(dim_color()),
        "accent_color": color_to_rgb(accent_color()),
        "queued_color": color_to_rgb(queued_color()),
        "asap_color": color_to_rgb(asap_color()),
        "pending_color": color_to_rgb(pending_color()),
        "user_text": color_to_rgb(user_text()),
        "user_bg": color_to_rgb(user_bg()),
        "ai_text": color_to_rgb(ai_text()),
        "header_icon_color": color_to_rgb(header_icon_color()),
        "header_name_color": color_to_rgb(header_name_color()),
        "header_session_color": color_to_rgb(header_session_color()),
    }))
}

fn color_to_rgb(color: Color) -> Option<[u8; 3]> {
    match color {
        Color::Rgb(r, g, b) => Some([r, g, b]),
        Color::Indexed(n) if n >= 16 => {
            let (r, g, b) = crate::tui::operant_app::color_support::indexed_to_rgb(n);
            Some([r, g, b])
        }
        _ => None,
    }
}
