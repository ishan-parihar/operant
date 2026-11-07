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

    let filter = app.help_filter().to_lowercase();
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

    // (iter-1786) A registered command's line reads its description from
    // the REGISTRY — the one catalogue. The hand-written copies here were a
    // SECOND table, and they drifted: `/save` promised a bookmark and opens
    // the export dialog, `/reload` promised a binary update and reloads
    // settings from disk, `/effort` listed nine levels the picker has never
    // shown, `/plan` promised a plan card and toggles a read-only mode.
    // (iter-1766 fixed four rows in the TABLE and this section kept the old
    // text — the same defect, one copy further down.)
    //
    // A row whose NAME names an argument form is not a registered command,
    // so its description stays hand-written — and only the forms the
    // commands actually honour are listed (iter-1786).
    let command_entry = |cmd: &str| -> Line<'static> {
        let desc = crate::tui::operant_app::app::registered_command_help(cmd)
            .unwrap_or("(no description — not a registered command)");
        help_entry(cmd, desc)
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
    lines.push(command_entry("/help"));
    lines.push(help_entry(
        "/help <text>",
        "Show the overlay filtered on the term",
    ));
    lines.push(command_entry("/model"));
    lines.push(command_entry("/agents"));
    lines.push(command_entry("/effort"));
    lines.push(command_entry("/config"));
    lines.push(command_entry("/context"));
    lines.push(command_entry("/skills"));
    lines.push(command_entry("/info"));
    lines.push(command_entry("/keys"));
    lines.push(command_entry("/stats"));
    lines.push(command_entry("/version"));
    lines.push(command_entry("/changelog"));

    lines.push(Line::from(""));
    lines.push(separator());
    lines.push(Line::from(""));

    lines.push(Line::from(Span::styled("  Session", section_style)));
    lines.push(Line::from(""));
    lines.push(command_entry("/clear"));
    lines.push(command_entry("/compact"));
    lines.push(command_entry("/rewind"));
    lines.push(command_entry("/fix"));
    lines.push(command_entry("/plan"));
    lines.push(command_entry("/improve"));
    lines.push(command_entry("/refactor"));
    lines.push(command_entry("/resume"));
    lines.push(command_entry("/save"));
    lines.push(command_entry("/rename"));
    // `/rename --clear` was advertised and does not exist: the arm takes any
    // non-empty argument as the new TITLE, so `/rename --clear` renames the
    // session to the literal string "--clear". (iter-1786)
    lines.push(help_entry("/rename <name>", "Set the current session name"));

    lines.push(Line::from(""));
    lines.push(separator());
    lines.push(Line::from(""));

    lines.push(Line::from(Span::styled("  Memory & Swarm", section_style)));
    lines.push(Line::from(""));
    lines.push(command_entry("/memory"));
    lines.push(help_entry(
        "/memory <cmd>",
        "Run a memory subcommand (list|get|search|store|delete|stats|import|export|clear)",
    ));

    lines.push(Line::from(""));
    lines.push(separator());
    lines.push(Line::from(""));

    lines.push(Line::from(Span::styled("  Auth & Accounts", section_style)));
    lines.push(Line::from(""));
    lines.push(command_entry("/auth"));
    lines.push(command_entry("/login"));

    lines.push(Line::from(""));
    lines.push(separator());
    lines.push(Line::from(""));

    lines.push(Line::from(Span::styled("  System", section_style)));
    lines.push(Line::from(""));
    lines.push(command_entry("/reload"));
    if app.is_remote_mode() {
        // A bridge session forwards these to the far side; the registry
        // carries their description.
        for (name, desc) in crate::tui::operant_app::app::remote_only_command_rows() {
            lines.push(help_entry(name, desc));
        }
    }
    lines.push(command_entry("/quit"));

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
    let uncovered: Vec<(&str, &str)> = {
        // (iter-1786) The sweep used `registered_command_entries` — which
        // INCLUDES the remote-only rows — so a LOCAL session's /help listed
        // `/continue` and `/resumeall`, whose answer here is "registered but
        // not wired in operant yet". The typeahead filters the same rows
        // out; the help overlay now agrees with it: a local session lists
        // local rows, and a remote session lists them all (the System
        // section above already renders the remote rows in remote mode).
        let registered: Vec<(&str, &str)> = if app.is_remote_mode() {
            crate::tui::operant_app::app::registered_command_entries().collect()
        } else {
            crate::tui::operant_app::app::locally_visible_command_rows().collect()
        };
        registered
            .into_iter()
            .filter(|(name, _)| !shown.contains(*name))
            .collect()
    };
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

    // (iter-1785) The overlay's own contract — `HelpOverlay::filter`: "only
    // commands matching this substring are shown" — was NEVER read by this
    // view: the Keys view filtered its report, the Commands view ignored
    // the filter entirely, so every keystroke an operator typed in /help
    // was swallowed by a filter no render consulted (the overlay looked
    // frozen, and the characters never reached the composer either). The
    // rows filter the same way the Keys view does.
    if !filter.is_empty() {
        lines = filter_help_lines(lines, &filter);
    }

    let total_lines = lines.len();
    let visible_height = area.height.saturating_sub(2) as usize;
    #[allow(clippy::manual_checked_ops)] // vendored-verbatim upstream scrollbar math
    let max_scroll = total_lines.saturating_sub(visible_height);
    // (iter-1795) Record the real max so the KEY handler clamps to it: the
    // hard-coded cap it used instead made the lower half of this overlay
    // unreachable.
    app.help_max_scroll().set(max_scroll);
    // (iter-1855) The page step for PageUp/PageDown is the overlay's visible
    // height — the render path is the only place that knows it.
    app.help_page_height().set(visible_height);
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

    let title = if filter.is_empty() {
        format!(" Help {} ", scroll_info)
    } else {
        format!(" Help {}· {filter} ", scroll_info)
    };
    let block = Block::default()
        .title(Span::styled(
            title,
            Style::default()
                .fg(rgb(200, 200, 220))
                .add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Line::from(Span::styled(
            // (iter-1795) The old footer was a vendored string whose three
            // scroll claims no arm implemented: j/k are NOT scroll keys (they
            // are the filter's letters — and after iter-1785 the filter was
            // live, so pressing 'j' per the footer HID the help), Space and
            // PageUp/PageDown had no arm at all, and the wheel scrolled the
            // TRANSCRIPT behind the overlay. The footer now names what the
            // panel does: arrows and the wheel scroll, typing filters, and
            // /help <cmd> filters the list. It also fits the narrow panel —
            // the old line was clipped at 80 columns ("for d").
            " Esc to close · ↑↓/pgup/pgdn/wheel scroll · type to filter · /help <cmd> ",
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

    // (iter-1795) The same record the commands view makes: the key handler
    // clamps the scroll to the content's real height, and this view's
    // content is the chord report (a different height).
    let visible_height = area.height.saturating_sub(2) as usize;
    app.help_max_scroll()
        .set(lines.len().saturating_sub(visible_height));
    app.help_page_height().set(visible_height);

    let title = if hotkeys { " Hotkeys " } else { " Keys " };
    let block = Block::default()
        .title(Span::styled(
            title,
            Style::default()
                .fg(rgb(200, 200, 220))
                .add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Line::from(Span::styled(
            " Esc to close · type to filter chords · ↑↓/pgup/pgdn scroll · /help for commands ",
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

/// Keep only the /help rows a filter matches. A row matches when ANY of its
/// spans carries the filter (the command name and its description are two
/// spans of one row, so both are searchable), and a filter that matches
/// nothing leaves a line that says so rather than an empty overlay — the
/// Keys view reports the same way.
fn filter_help_lines(lines: Vec<Line<'static>>, filter: &str) -> Vec<Line<'static>> {
    let mut kept: Vec<Line<'static>> = lines
        .into_iter()
        .filter(|line| {
            line.spans
                .iter()
                .any(|span| span.content.to_lowercase().contains(filter))
        })
        .collect();
    if kept.is_empty() {
        kept.push(Line::from(Span::styled(
            format!("  no command matches \"{filter}\""),
            Style::default().fg(rgb(150, 150, 165)),
        )));
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(text: &'static str) -> Line<'static> {
        Line::from(text.to_string())
    }

    /// An App for the render guard. `App::new` writes the process-global
    /// palette, so it takes the shared lock the other render tests take.
    fn make_app() -> crate::tui::app::App {
        let _guard = crate::tui::theme_colors::tests::ACTIVE_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let config = operant_core::config::AppConfig::default();
        let settings = crate::tui::adapter_types::config::Settings::default();
        let cost_tracker = std::sync::Arc::new(crate::tui::adapter_types::cost::CostTracker::new());
        let command_registry = crate::commands::CommandRegistry::new();
        crate::tui::app::App::new(config, settings, cost_tracker, command_registry)
    }

    /// The rendered `/help` overlay as text. The terminal is tall enough to
    /// hold the WHOLE catalogue (the curated sections, the sweep and the two
    /// generated key sections), because the guard is about every row.
    fn rendered_help_text(app: &crate::tui::app::App) -> String {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let mut terminal = Terminal::new(TestBackend::new(220, 400)).expect("terminal");
        terminal
            .draw(|frame| draw_help_overlay(frame, frame.area(), 0, app))
            .expect("draw");
        crate::tui::debug::debug_hub::buffer_rows(terminal.backend().buffer()).join("\n")
    }

    /// (iter-1795) The `/help` catalogue is far taller than the viewport —
    /// the "More commands" list, the Skills section, the whole KEYBOARD
    /// section and the Input section live BELOW the fold. The key handler
    /// used to cap the scroll at a hard-coded 50 lines ("a generous upper
    /// bound"), which was generous for the /keys report and far too small
    /// for this one: those sections were UNREACHABLE.
    ///
    /// The render path records the content's real max scroll, and the
    /// handler clamps to it — so the bottom is reachable and the scroll
    /// never runs past it.
    #[test]
    fn the_help_overlay_scrolls_to_the_bottom_of_its_catalogue() {
        let app = make_app();
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 40)).expect("terminal");
        terminal
            .draw(|frame| draw_help_overlay(frame, frame.area(), 0, &app))
            .expect("draw");
        let max = app.help_max_scroll().get();
        assert!(
            max > 49,
            "the /help catalogue is much taller than the old hard-coded cap: {max}"
        );

        // And the KEY handler reaches it — and stops there. The overlay is
        // opened and drawn first (the render path is what records the max),
        // then the arrow key is pressed far more times than the content has
        // lines: the scroll must land ON the bottom, not short of it (the
        // old 50-line cap) and not past it (a scroll that runs over the end
        // leaves the view stuck while the state walks back).
        let mut app = make_app();
        assert!(
            app.handle_tui_command("help", ""),
            "/help opens the overlay"
        );
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 40)).expect("terminal");
        terminal
            .draw(|frame| draw_help_overlay(frame, frame.area(), 0, &app))
            .expect("draw");
        let max = app.help_max_scroll().get();
        for _ in 0..(max + 50) {
            app.handle_key_event(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Down,
                crossterm::event::KeyModifiers::NONE,
            ));
        }
        assert_eq!(
            app.help_overlay.scroll_offset as usize, max,
            "the scroll must reach the bottom of the catalogue and stop there"
        );
    }

    /// (iter-1786) The overlay's command rows read their descriptions from
    /// the registry, and the registry is the ONE catalogue: every local
    /// command's row must carry the table's own description, or the overlay
    /// has drifted from the table again (iter-1766 fixed four rows in the
    /// TABLE while this section kept the old text — the same defect, one
    /// copy further down).
    #[test]
    fn every_local_command_shows_its_registry_description_in_the_overlay() {
        let app = make_app();
        let text = rendered_help_text(&app);
        for (name, desc) in crate::tui::operant_app::app::locally_visible_command_rows() {
            let row = format!("{name}  {desc}");
            assert!(
                text.contains(&row),
                "the /help overlay must show {row:?} — the registry is the one catalogue"
            );
        }
        assert!(
            !text.contains("not a registered command"),
            "a curated row names a command the registry does not have"
        );
    }

    /// (iter-1786) The remote-only rows dispatch on the OTHER side of a
    /// bridge session. A local `/help` listed them anyway, so an operator
    /// who typed `/continue` got "registered but not wired in operant yet" —
    /// while the typeahead, which filters the same rows out, did not offer
    /// them.
    #[test]
    fn a_local_help_overlay_does_not_list_the_remote_only_commands() {
        let app = make_app();
        let text = rendered_help_text(&app);
        for (name, _) in crate::tui::operant_app::app::remote_only_command_rows() {
            assert!(
                !text.contains(name),
                "{name} forwards to the far side of a bridge session — a local /help \
                 must not advertise a command that answers \"not wired\" here"
            );
        }
    }

    /// (iter-1786) The argument forms the overlay used to advertise — the
    /// ones no arm reads. `/rename --clear` renamed the session to the
    /// literal string "--clear"; `/save [label]` opened the export dialog
    /// with the label ignored; `/model <name>`, `/effort <level>`,
    /// `/config init`, `/config edit`, `/plan [goal]`, `/improve resume` and
    /// `/refactor resume` were all ignored by their arms.
    #[test]
    fn the_overlay_no_longer_advertises_argument_forms_the_commands_ignore() {
        let app = make_app();
        let text = rendered_help_text(&app);
        for gone in [
            "/model <name>",
            "/effort <level>",
            "/config init",
            "/config edit",
            "/plan [goal]",
            "/improve resume",
            "/refactor resume",
            "/save [label]",
            "/rename <name>|--clear",
            "/login [provider]",
            "/help <command>",
            "/memory [on|off]",
        ] {
            assert!(
                !text.contains(gone),
                "{gone} is advertised but no arm reads that argument — /help must \
                 not promise a form the command ignores"
            );
        }
    }

    #[test]
    fn a_filter_keeps_only_the_rows_that_carry_it() {
        let lines = vec![
            Line::from(""),
            Line::from(Span::styled("  Session", Style::default())),
            Line::from(vec![
                Span::styled("    ", Style::default()),
                Span::styled("/save".to_string(), Style::default()),
                Span::styled("  ", Style::default()),
                Span::styled("Export the conversation".to_string(), Style::default()),
            ]),
            Line::from(vec![
                Span::styled("    ", Style::default()),
                Span::styled("/model".to_string(), Style::default()),
                Span::styled("  ", Style::default()),
                Span::styled("List or switch models".to_string(), Style::default()),
            ]),
        ];

        let kept = filter_help_lines(lines, "model");
        let shown: Vec<String> = kept.iter().map(|line| line.to_string()).collect();
        // One row survives — the `/model` row — and not the `/save` row or
        // the section header beside it.
        assert_eq!(shown.len(), 1, "only the matching row survives: {shown:?}");
        assert!(
            shown[0].contains("/model") && shown[0].contains("List or switch models"),
            "the surviving row is the /model row: {shown:?}"
        );
    }

    #[test]
    fn a_filter_that_matches_nothing_says_so_rather_than_showing_an_empty_overlay() {
        let kept = filter_help_lines(vec![row("/model  List or switch models")], "zzz");
        assert_eq!(kept.len(), 1, "one line, not zero");
        assert!(
            kept[0].to_string().contains("\"zzz\""),
            "the no-match line names the filter: {:?}",
            kept[0].to_string()
        );
    }
}
