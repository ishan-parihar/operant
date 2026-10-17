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

    let _alt = crate::tui::operant_app::keybind::alt_chord;
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
    lines.push(help_entry("/stats", "Show session token and cost stats"));
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

    // ---- Keyboard: generated from the keybinding registry (plan 10) ----
    // The hand-written Navigation/Diagrams/Input sections here were jcode's,
    // advertising chords that do something else or nothing in operant
    // (Ctrl+J "jump to prompt" is a newline; Ctrl+P "auto-poke" is global
    // search; Ctrl+1..9 panel resize does not exist; Alt+S/A/X never
    // dispatched). The registry is the single source of truth now: every
    // described Global binding renders, and nothing else does.
    lines.push(Line::from(Span::styled("  Keyboard", section_style)));
    lines.push(Line::from(""));
    {
        use crate::tui::keybindings::{BindingContext, DEFAULT_KEYBINDINGS};
        for (context, binding) in DEFAULT_KEYBINDINGS.catalogue() {
            if context != BindingContext::Global {
                continue;
            }
            let Some(desc) = binding.description.as_deref() else {
                continue;
            };
            // OS-conflicted chords (Ctrl+h/l etc.) are explained by /keys
            // with the interception reason; the overlay skips them so the
            // list stays honest about what a plain terminal delivers.
            if crate::tui::keybindings::os_conflict_reason(&binding.chord()).is_some() {
                continue;
            }
            lines.push(key_entry(&binding.chord().to_string(), desc));
        }
    }

    lines.push(Line::from(""));
    lines.push(separator());
    lines.push(Line::from(""));

    lines.push(Line::from(Span::styled("  Input & Editing", section_style)));
    lines.push(Line::from(""));
    {
        use crate::tui::keybindings::{BindingContext, DEFAULT_KEYBINDINGS};
        for (context, binding) in DEFAULT_KEYBINDINGS.catalogue() {
            if context != BindingContext::Prompt {
                continue;
            }
            let Some(desc) = binding.description.as_deref() else {
                continue;
            };
            if crate::tui::keybindings::os_conflict_reason(&binding.chord()).is_some() {
                continue;
            }
            lines.push(key_entry(&binding.chord().to_string(), desc));
        }
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

/// The /keys and /hotkeys surfaces (2026-10-14 batch, plan 10): the same
/// full-screen overlay machinery as /help, but the body is the binding
/// catalogue report (single-sourced from the same report builders the
/// commands use), opened at the top and filterable by chord text. Replaces
/// the old bottom-dumped transcript message that opened below the fold.
pub(super) fn draw_keys_overlay(frame: &mut Frame, area: Rect, scroll: usize, app: &dyn TuiState) {
    clear_area(frame, area);

    use crate::tui::overlays::help::HelpOverlayContent;
    let hotkeys = app.help_overlay_content() == HelpOverlayContent::Hotkeys;
    let filter = app.help_filter().to_lowercase();

    let body_style = Style::default().fg(rgb(210, 210, 220));
    let dim_style = Style::default().fg(dim_color());

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut matched = 0usize;
    for raw in app.keys_report_lines(hotkeys) {
        if !filter.is_empty() && !raw.to_lowercase().contains(&filter) {
            continue;
        }
        matched += 1;
        let style = if raw.starts_with('[') || raw.starts_with("⚠") {
            Style::default()
                .fg(accent_color())
                .add_modifier(Modifier::BOLD)
        } else {
            body_style
        };
        lines.push(Line::from(Span::styled(raw, style)));
    }
    if matched == 0 {
        lines.push(Line::from(Span::styled(
            format!(" no binding matches \"{filter}\""),
            dim_style,
        )));
    }

    let title = if hotkeys { " Hotkeys " } else { " Keys " };
    let block = Block::default()
        .title(Span::styled(
            title,
            Style::default()
                .fg(rgb(200, 200, 220))
                .add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Line::from(Span::styled(
            " Esc to close · type to filter chords · ↑/↓ scroll · /help for commands ",
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
