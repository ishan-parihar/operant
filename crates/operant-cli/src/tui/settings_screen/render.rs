// settings_screen/render.rs — Settings screen rendering.
//
// Extracted from the settings_screen.rs monolith.

use super::*;
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

use crate::tui::overlays::{HINT_ESC, modal_layout, render_modal_title_frame};
use crate::tui::vendor::style::theme;

pub fn render_settings_screen(frame: &mut Frame, screen: &SettingsScreen, area: Rect) {
    if !screen.visible {
        return;
    }

    render_dark_overlay(frame, area);

    // 80% width, 90% height, centred. `modal_layout` owns the clamp against
    // the area and the readable floor. This surface is a borderless panel, so
    // only the dialog rect is taken from it; the content inset below is this
    // screen's own two-column gutter.
    let desired_width = (area.width * 4 / 5).max(60);
    let desired_height = (area.height * 9 / 10).max(20);
    let popup = modal_layout(area, desired_width, desired_height, 1, 1).dialog_area;
    render_dialog_bg(frame, popup);

    // Inset inner area
    let inner = Rect {
        x: popup.x + 2,
        y: popup.y + 1,
        width: popup.width.saturating_sub(4),
        height: popup.height.saturating_sub(2),
    };

    if inner.height < 6 {
        return;
    }

    // Split into header + search + spacer + content + description + footer
    // The tab strip takes the row the search box used to leave empty. Seven
    // rows instead of six; `layout[2]` was a spacer and is now the strip.
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Percentage(50),
            Constraint::Length(1),
        ])
        .split(inner);

    let header_area = layout[0];
    let search_area = layout[1];
    let tabs_area = layout[2];
    let content_area = layout[3];
    let description_area = layout[4];
    let footer_area = layout[5];

    // Header — the one modal title format, with the canonical dismissal hint
    // on the right of the same row.
    render_modal_title_frame(frame, header_area, "Settings — Operant", HINT_ESC);

    // Search
    let search_line = modal_search_line(
        &screen.search_query,
        "Type to search settings...",
        theme::dim_color(),
        theme_colors::accent(),
    );
    frame.render_widget(
        Paragraph::new(search_line).style(Style::default().bg(theme_colors::panel_bg())),
        search_area,
    );

    render_tab_strip(frame, screen, tabs_area);

    // Content
    render_settings_list(frame, screen, content_area);

    // Description of selected entry. The ONE filter (plan 2026-11-16 S2): tab,
    // then query — the same list the painter draws and the keyboard counts.
    let filtered = screen.visible_entries();

    // (iter-2452) The `output_style` branch that used to live here described a
    // row that no longer exists, and pulled a module-level import in with it.
    // It is deleted rather than left to read as live.
    let desc_text = filtered
        .get(screen.selected_idx)
        .map(|entry| entry.description.to_string())
        .unwrap_or_default();
    // (plan 2026-11-16 S3) A risk note renders ABOVE the description, in warning
    // styling, and only for a row that carries one. OMP reserves this for
    // settings that can get the operator rate-limited or banned — not for merely
    // advanced options (`settings-defs.ts:117-121`).
    let desc_text = if let Some(warning) = filtered.get(screen.selected_idx).and_then(|e| e.warning)
    {
        format!("⚠ {warning}\n\n{desc_text}")
    } else {
        desc_text
    };

    let desc_para = Paragraph::new(desc_text)
        .style(Style::default().fg(theme::dim_color()))
        .alignment(Alignment::Left)
        .block(Block::default().padding(ratatui::widgets::Padding::new(1, 0, 1, 0)));
    frame.render_widget(desc_para, description_area);

    // Footer
    let footer = if let Some(err) = &screen.apply_error {
        // (iter-1948) A state-gated footer: while an apply error stands, the
        // footer IS the error, so the operator cannot miss that the value they
        // typed was not taken. The key hints return once it clears.
        Line::from(Span::styled(
            err.clone(),
            Style::default().fg(theme_colors::warning()),
        ))
    } else if let Some(warn) = &screen.persist_warning {
        // (iter-1954) A value that WAS applied but will not survive a restart.
        // A different fact from an unread value, so a different message: the
        // operator must not read "kept the previous value" for a change that
        // took effect.
        Line::from(Span::styled(
            warn.clone(),
            Style::default().fg(theme_colors::warning()),
        ))
    } else if screen.edit_field.is_some() {
        Line::from(vec![
            Span::styled(
                " Enter ",
                Style::default()
                    .fg(theme_colors::accent())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("save  "),
            Span::styled(
                " Esc ",
                Style::default()
                    .fg(theme_colors::warning())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("cancel"),
        ])
    } else {
        Line::from(vec![
            Span::styled(
                " ↑↓ ",
                Style::default()
                    .fg(theme_colors::accent())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("navigate  "),
            Span::styled(
                " Enter ",
                Style::default()
                    .fg(theme_colors::accent())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("toggle/edit  "),
            Span::styled(
                " [ ] ",
                Style::default()
                    .fg(theme_colors::accent())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("tab  "),
            Span::styled(
                " Esc ",
                Style::default()
                    .fg(theme_colors::warning())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("close"),
        ])
    };
    let footer_para = Paragraph::new(vec![footer])
        .style(
            Style::default()
                .fg(theme_colors::muted())
                .bg(theme_colors::panel_bg()),
        )
        .alignment(Alignment::Center);
    frame.render_widget(footer_para, footer_area);
}

/// The tab strip: every tab, the current one highlighted, the rest dimmed.
///
/// (plan 2026-11-16 S2) OMP renders a labelled icon per tab and keeps the order
/// explicit (`SETTING_TABS`, `settings-defs.ts:22-32`).
///
/// There is deliberately NO per-tab match count. The first version of this comment
/// claimed one ("type port and see Gateway 3") and no code computed it — a comment
/// describing an affordance that does not exist is worse than no comment, because
/// the next reader trusts it. The search is tab-scoped and the empty state says so.
fn render_tab_strip(frame: &mut Frame, screen: &SettingsScreen, area: Rect) {
    let mut spans: Vec<Span> = Vec::new();
    for (index, tab) in SettingsTab::ALL.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(" │ ", Style::default().fg(theme::dim_color())));
        }
        let is_current = *tab == screen.selected_tab;
        let style = if is_current {
            Style::default()
                .fg(theme_colors::accent())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme_colors::muted())
        };
        spans.push(Span::styled(tab.label(), style));
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(theme_colors::panel_bg())),
        area,
    );
}

fn render_settings_list(frame: &mut Frame, screen: &SettingsScreen, area: Rect) {
    // The ONE filter. The list, the description pane and both keyboard helpers
    // all read this, so the marker, the count and the paint cannot disagree.
    let filtered = screen.visible_entries();

    if filtered.is_empty() {
        // The filter is TAB-SCOPED, so the message must say so. A query typed on
        // Appearance that matches a Gateway row is a zero-result search, and the
        // honest sentence names the scope rather than blaming the query.
        let empty = if screen.search_query.is_empty() {
            "No settings on this tab."
        } else {
            "No settings on this tab match your search."
        };
        let para = Paragraph::new(empty).style(Style::default().fg(theme::dim_color()));
        frame.render_widget(para, area);
        return;
    }

    // Build lines. `lines` and `visible_rows` are declared at the loop below,
    // beside the heading logic that shares them.

    // (plan 2026-11-16 S2) Heading rows. OMP orders groups explicitly
    // (`TAB_GROUPS`, `settings-defs.ts:36-97`) and renders a heading between
    // them; a heading occupies a LINE but is NOT a row, so it must not advance
    // `selected_idx` — the keyboard counts rows, not lines. Emitting it here,
    // in the same loop that builds the rows, is what keeps the two from
    // drifting: a heading appended after the window would not scroll with its
    // own group.
    let mut lines: Vec<Line> = Vec::new();
    // (iter-2452, red-team T1) Headings already EMITTED, by name. The grouping is
    // derived from key content while the row order is the registry's, so one
    // tab's groups interleave and a transition-only check printed the same
    // heading several times — "Limits" six times down the Tools tab, which is
    // noise the heading was added to remove, and six real lines spent by the
    // scroll window on duplicate bands.
    let mut emitted_groups: Vec<&str> = Vec::new();
    // (plan 2026-11-16 S2) WHICH ROW EACH LINE BELONGS TO. A heading takes a
    // line but is not a row, so the selection index counts rows and the window
    // counts lines. Deriving the scroll from `selected_idx` alone — as this did
    // before headings existed — walks the selection past the last painted line
    // the moment any group heading is present, which is exactly iter-1917 with
    // a new cause. One list, one space: the window is computed in LINE space.
    let mut line_row: Vec<Option<usize>> = Vec::new();
    let mut current_group: Option<&str> = None;

    for (i, entry) in filtered.iter().enumerate() {
        if entry.group != current_group {
            current_group = entry.group;
            if let Some(group) = entry.group
                && !emitted_groups.contains(&group)
            {
                emitted_groups.push(group);
                lines.push(Line::from(Span::styled(
                    format!("  {group}"),
                    Style::default()
                        .fg(theme_colors::muted())
                        .add_modifier(Modifier::BOLD),
                )));
                line_row.push(None);
            }
        }
        let is_selected = i == screen.selected_idx;
        let marker = if is_selected { "►" } else { " " };

        let label_len = 40usize;

        // Show edit value if currently editing this field, otherwise show the entry value
        let value_str = if screen.edit_field.as_deref() == Some(entry.key) && is_selected {
            format!("{}_ ", screen.edit_value) // Add cursor indicator
        } else {
            entry.value.clone()
        };

        let row_style = if is_selected {
            // The selected row is a chip on `accent()`, so its foreground is the
            // role for text on a selection background. A bare `Color::Black`
            // here is remapped onto the `user_bg` role per frame, which is a
            // background role — value-preserving today and wrong the moment
            // `user_bg` is configured.
            Style::default()
                .fg(theme_colors::on_selection())
                .bg(theme_colors::accent())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };

        let line = Line::from(vec![
            Span::styled(
                format!("   {} {:<label_len$}", marker, entry.label),
                row_style,
            ),
            Span::styled(value_str, row_style),
        ]);
        lines.push(line);
        line_row.push(Some(i));
    }

    // (iter-1917) THE RENDER OWNS THE WINDOW. The keyboard used to track it
    // with a hardcoded estimate of ten rows ("Rough estimate, will be actual
    // in real usage"), and the paint sliced with the REAL height — two
    // windows, two numbers. On any terminal short enough that this list shows
    // fewer than ten rows (80x24 paints five) the estimate never advanced, so
    // the selection could be walked past the last painted row: the ► marker
    // vanished, the row could not be seen, read or toggled, and the
    // description pane kept describing a row nobody could reach. Three of the
    // eight rows were unreachable that way. One window, measured here.
    // Find the LINE the selection is on, not the row. Headings occupy lines
    // without occupying row indices, so `selected_idx` is an index into a
    // different space than `lines` — comparing them directly is what let the
    // marker scroll off the bottom on the first tab that had a heading.
    let window = area.height as usize;
    let window = window.max(1);
    let selected_line = line_row
        .iter()
        .position(|row| *row == Some(screen.selected_idx))
        .unwrap_or(0);
    let scroll = if selected_line < window {
        0
    } else {
        selected_line + 1 - window
    };
    let visible_lines: Vec<Line> = lines.into_iter().skip(scroll).take(window).collect();

    let para = Paragraph::new(visible_lines);
    frame.render_widget(para, area);
}

// ---------------------------------------------------------------------------
// Key handling
// ---------------------------------------------------------------------------
