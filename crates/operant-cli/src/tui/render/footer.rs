// render/footer.rs — Input pane, status row, footer bar, prompt suggestions.

use crate::tui::adapter_types::types::Role;
use crate::tui::app::App;
use crate::tui::prompt_input::{
    InputMode, TypeaheadSource, VimMode, input_height, render_prompt_input,
};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

use super::{shimmer_spans, truncate_end, truncate_middle, ACCENT_PRIMARY, STATUS_THINKING, STATUS_THINKING_ELLIPSIS};

fn render_input(frame: &mut Frame, app: &App, area: Rect, focused: bool) {
    // Split: 1-row model/mode status line + remaining rows for the prompt input.
    let (status_area, input_area) = if area.height > 2 {
        let splits = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(1)])
            .split(area);
        (Some(splits[0]), splits[1])
    } else {
        // Not enough room for the extra line — skip the status row.
        (None, area)
    };

    // Render model + agent mode status line above the prompt.
    if let Some(status_area) = status_area {
        // Only show a mode tag when there's an explicit agent_mode set or
        // plan_mode is active. The default "build" tag was noise — the user
        // doesn't need to see "BUILD" when they're just chatting.
        // (iter-113 — user requested removal of the redundant BUILD MODE tag.)
        let agent_mode: Option<&str> = match app.agent_mode.as_deref() {
            Some(m) if !m.is_empty() => Some(m),
            _ if app.plan_mode => Some("plan"),
            _ => None,
        };

        let pink = app.accent_color;
        let dim = Color::Rgb(110, 110, 124);
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Min(1),
                Constraint::Length(status_area.width.min(50)),
            ])
            .split(status_area);

        let left_line = if app.has_credentials {
            let (provider, model_short) =
                if let Some((provider, model)) = app.model_name.split_once('/') {
                    (provider.to_string(), model.to_string())
                } else {
                    ("local".to_string(), app.model_name.clone())
                };
            let mut spans: Vec<Span> = Vec::new();
            // Only render the mode tag when one is active (not the default "build").
            if let Some(mode) = agent_mode {
                spans.push(Span::styled(
                    format!(" {} ", mode.to_uppercase()),
                    Style::default()
                        .fg(Color::Black)
                        .bg(pink)
                        .add_modifier(Modifier::BOLD),
                ));
                spans.push(Span::raw(" "));
            }
            spans.push(Span::styled(
                model_short,
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::styled(
                format!(" · {}", provider),
                Style::default().fg(dim),
            ));

            // Iteration count — read from the Arc<AtomicUsize> the agent loop
            // bumps each turn. Shown as "· iter N" so it clusters visually
            // with the model/provider pills. Only shown when current_turn is
            // wired (i.e. after the first agent invocation).
            // (iter-209: current_turn field deleted with FileHistory stub.
            // The "iter N" pill is cut — to re-add, track iteration count in
            // a new field on App that doesn't depend on the FileHistory stub.)
            // if let Some(ref turn) = app.current_turn { ... }

            // Subagent HUD — a small "· N agents" pill when there are live
            // subagent status entries. Mirrors hermes' SpawnHud widget.
            // agent_status is a Vec<(String, String)> of (name, status); we
            // count entries whose status is not "done" / "idle" / empty.
            let live_subagents = app
                .agent_status
                .iter()
                .filter(|(_, s)| {
                    let s = s.to_ascii_lowercase();
                    !s.is_empty()
                        && !s.contains("done")
                        && !s.contains("idle")
                        && !s.contains("complete")
                })
                .count();
            if live_subagents > 0 {
                spans.push(Span::styled(
                    format!(
                        " · {} agent{}",
                        live_subagents,
                        if live_subagents == 1 { "" } else { "s" }
                    ),
                    Style::default().fg(Color::Cyan),
                ));
            }

            Line::from(spans)
        } else {
            Line::from(vec![
                Span::styled(" no provider", Style::default().fg(dim)),
                Span::styled(" · type /model to choose", Style::default().fg(dim)),
            ])
        };

        // `?` opens the shortcuts overlay which already lists Ctrl+A / Ctrl+K
        // and friends — surfacing them again here is redundant clutter.
        let right_hint = if app.has_credentials {
            Line::from(vec![Span::styled("? shortcuts", Style::default().fg(dim))])
        } else {
            Line::from(Vec::<Span>::new())
        };

        let left_padded = Rect {
            x: chunks[0].x + 1,
            y: chunks[0].y,
            width: chunks[0].width.saturating_sub(1),
            height: chunks[0].height,
        };
        let right_padded = Rect {
            x: chunks[1].x,
            y: chunks[1].y,
            width: chunks[1].width.saturating_sub(1),
            height: chunks[1].height,
        };
        frame.render_widget(Paragraph::new(vec![left_line]), left_padded);
        frame.render_widget(
            Paragraph::new(vec![right_hint]).alignment(Alignment::Right),
            right_padded,
        );
    }

    render_prompt_input(
        &app.prompt_input,
        input_area,
        frame.buffer_mut(),
        focused,
        if app.is_streaming {
            InputMode::Readonly
        } else if app.plan_mode {
            InputMode::Plan
        } else {
            InputMode::Default
        },
        app.accent_color,
        app.settings_screen.cursor_blink_enabled,
    );
}

fn should_render_status_row(app: &App) -> bool {
    let interesting_stream_status = app
        .status_message
        .as_deref()
        .map(|status| {
            let trimmed = status.trim();
            !trimmed.is_empty()
                && !trimmed.eq_ignore_ascii_case(STATUS_THINKING)
                && !trimmed.eq_ignore_ascii_case(STATUS_THINKING_ELLIPSIS)
        })
        .unwrap_or(false);

    app.voice_recording
        || app.last_turn_elapsed.is_some()
        || (!app.is_streaming && app.status_message.is_some())
        || (app.is_streaming && interesting_stream_status)
}

fn render_status_row(frame: &mut Frame, app: &App, area: Rect) {
    if area.height == 0 {
        return;
    }

    let spans = if app.voice_recording {
        vec![Span::styled(
            format!(
                "{} Recording... press Alt+V to transcribe",
                figures::black_circle()
            ),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )]
    } else if app.is_streaming {
        // Pick a label: use the status message if it has real content,
        // otherwise show a default "Thinking" shimmer so the user always
        // sees that the model is working.
        let raw_label = app
            .status_message
            .as_deref()
            .filter(|s| {
                let t = s.trim();
                !t.is_empty()
                    && !t.eq_ignore_ascii_case(STATUS_THINKING)
                    && !t.eq_ignore_ascii_case(STATUS_THINKING_ELLIPSIS)
            })
            .or(app.spinner_verb.as_deref())
            .unwrap_or("Thinking");

        let mut s = vec![Span::styled(
            spinner_char(app.frame_count).to_string(),
            Style::default()
                .fg(spinner_color(app))
                .add_modifier(Modifier::BOLD),
        )];
        let label = format!("{}…", raw_label.trim_end_matches('…'));

        s.push(Span::raw(" "));
        s.extend(shimmer_spans(&label, app.frame_count));
        s
    } else if let (Some(verb), Some(elapsed)) =
        (app.last_turn_verb, app.last_turn_elapsed.as_deref())
    {
        // "✽ Worked for 2m 5s" — mirrors TS TeammateSpinnerLine idle state
        vec![Span::styled(
            format!("{} {} for {}", figures::TEARDROP_ASTERISK, verb, elapsed),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::DIM),
        )]
    } else if let Some(status) = app.status_message.as_deref() {
        vec![Span::styled(
            status.to_string(),
            Style::default().fg(Color::DarkGray),
        )]
    } else {
        Vec::new()
    };

    if spans.is_empty() {
        return;
    }

    frame.render_widget(
        Paragraph::new(Line::from(spans)).wrap(ratatui::widgets::Wrap { trim: false }),
        area,
    );
}

/// Build spans for a text string with a right-to-left glimmer sweep, matching
/// the TS `GlimmerMessage` behaviour (glimmerSpeed=200ms, 3-char shimmer window).
///
/// At ~50ms per frame a 4-frame step ≈ 200ms, giving the same cadence as TS.
    if !run.is_empty() {
        spans.push(Span::styled(run, if run_bright { bright } else { base }));
    }
    spans
}
// Keybinding hints footer
// -----------------------------------------------------------------------

/// Single footer line matching the TS contract more closely:
/// - `? for shortcuts` is suppressed once the prompt becomes non-empty
/// - the right side shows comprehensive status info and notifications
fn render_footer(frame: &mut Frame, app: &App, area: Rect) {
    if area.height == 0 {
        return;
    }

    // Use only the first line of the footer area, leaving bottom padding
    let footer_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: 1,
    };

    // Left side: ordered pills — voice > PR badge > background task > vim > hint
    let left_spans: Vec<Span> = if app.voice_recording {
        vec![Span::styled(
            format!(" {} REC — speak now", figures::black_circle()),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )]
    } else {
        let mut spans: Vec<Span> = Vec::new();

        // (iter-142: agent_type_badge render deleted — field was always None)

        // (iter-147: PR badge render deleted — detect_pr() was never called,
        // pr_number/pr_state were always None)

        // (iter-142: background_task_count/status render deleted — fields were always 0/None)

        // Vim mode indicator — shown for all modes using neovim "-- MODE --" convention.
        // INSERT is dim (common, low-noise); other modes use bright colour.
        if app.prompt_input.vim_enabled {
            if !spans.is_empty() {
                spans.push(Span::raw("  "));
            }
            let (label, style) = match app.prompt_input.vim_mode {
                VimMode::Insert => ("-- INSERT --", Style::default().fg(Color::DarkGray)),
                VimMode::Normal => (
                    "-- NORMAL --",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
                VimMode::Visual => (
                    "-- VISUAL --",
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                ),
                VimMode::VisualLine => (
                    "-- VISUAL LINE --",
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                ),
                VimMode::VisualBlock => (
                    "-- VISUAL BLOCK --",
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                ),
                VimMode::Command => (
                    "-- COMMAND --",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                VimMode::Search => (
                    "-- SEARCH --",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
            };
            spans.push(Span::styled(label, style));
        }

        // Bash prefix indicator — shown when prompt starts with '!'
        if app.prompt_input.text.starts_with('!') {
            if !spans.is_empty() {
                spans.push(Span::raw("  "));
            }
            spans.push(Span::styled(
                "[BASH]",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ));
        }

        // Permission mode badge (left side, mirrors TS bottom-left indicator).
        // Default mode is silent; non-default modes show a badge.
        {
            use crate::tui::adapter_types::config::PermissionMode;
            match &app.settings.permission_mode {
                PermissionMode::BypassPermissions => {
                    if !spans.is_empty() {
                        spans.push(Span::raw("  "));
                    }
                    spans.push(Span::styled(
                        "\u{23f5}\u{23f5} bypass",
                        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                    ));
                }
                PermissionMode::AcceptEdits => {
                    if !spans.is_empty() {
                        spans.push(Span::raw("  "));
                    }
                    spans.push(Span::styled(
                        "accept-edits",
                        Style::default().fg(Color::Yellow),
                    ));
                }
                PermissionMode::Plan => {
                    if !spans.is_empty() {
                        spans.push(Span::raw("  "));
                    }
                    spans.push(Span::styled("plan", Style::default().fg(Color::Blue)));
                }
                PermissionMode::Default => {}
            }
        }

        // During streaming show "esc to interrupt". The "? shortcuts" hint is
        // rendered in the top-right status bar (see render_prompt area), so do
        // not duplicate it here (issue #149 follow-up).
        if spans.is_empty() && app.is_streaming {
            spans.push(Span::styled(
                "esc interrupt",
                Style::default().fg(Color::DarkGray),
            ));
        }

        spans
    };

    // Right side: status metrics and lightweight badges.
    let right_spans: Vec<Span> = {
        let mut parts: Vec<Span> = Vec::new();

        // 1. Context window usage — show "N% until auto-compact" mirroring TS TokenWarning.
        //    When an update is available and context is below 85%, show the update notification
        //    instead to keep the status bar uncluttered.
        if app.context_window_size > 0 {
            let used_pct =
                (app.context_used_tokens as f64 / app.context_window_size as f64 * 100.0) as u64;
            let left_pct = 100u64.saturating_sub(used_pct);

            if !parts.is_empty() {
                parts.push(Span::raw("  "));
            }

            if used_pct >= 85 {
                // High usage — always show context window info regardless of update status.
                if used_pct >= 95 {
                    parts.push(Span::styled(
                        format!("{}% context used — /compact now", used_pct),
                        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                    ));
                } else {
                    parts.push(Span::styled(
                        format!("{}% until auto-compact", left_pct),
                        Style::default().fg(Color::Yellow),
                    ));
                }
            } else if used_pct >= 70 {
                // 70–84%: mild warning.
                parts.push(Span::styled(
                    format!("{}% until auto-compact", left_pct),
                    Style::default().fg(Color::Yellow),
                ));
            } else {
                // Normal: dim display.
                let used_k = app.context_used_tokens / 1000;
                let total_k = app.context_window_size / 1000;
                parts.push(Span::styled(
                    format!("{}k/{}k", used_k, total_k),
                    Style::default().fg(Color::DarkGray),
                ));
            }
        }

        // 3. Cost — mirrors TS formatCost: 4 decimal places for costs < $0.50, else 2.
        // Display cost if it's >= 0.0, so free models show $0.00
        if app.cost_usd >= 0.0 {
            if !parts.is_empty() {
                parts.push(Span::raw("  "));
            }
            let cost_str = if app.cost_usd < 0.5 {
                format!("${:.4}", app.cost_usd)
            } else {
                format!("${:.2}", app.cost_usd)
            };
            parts.push(Span::styled(cost_str, Style::default().fg(Color::DarkGray)));
        }

        // 4. Rate limits
        if let Some(pct) = app.rate_limit_5h_pct {
            if pct > 0.0 {
                if !parts.is_empty() {
                    parts.push(Span::raw("  "));
                }
                let color = if pct >= 90.0 {
                    Color::Red
                } else {
                    Color::Yellow
                };
                parts.push(Span::styled(
                    format!("5h:{:.0}%", pct),
                    Style::default().fg(color),
                ));
            }
        }
        if let Some(pct) = app.rate_limit_7day_pct {
            if pct > 0.0 {
                if !parts.is_empty() {
                    parts.push(Span::raw("  "));
                }
                let color = if pct >= 90.0 {
                    Color::Red
                } else {
                    Color::Yellow
                };
                parts.push(Span::styled(
                    format!("7d:{:.0}%", pct),
                    Style::default().fg(color),
                ));
            }
        }

        // 5. Vim mode — displayed on the left side as "-- MODE --"; nothing extra on right.

        // (iter-142: agent_type_badge + worktree_branch render deleted — fields were always None)

        // 7b. Infrastructure pill — shows memory + skills counts so the
        // invisible infrastructure is visible. (P2-14 from UX audit.)
        {
            let mem_count = {
                let mem_dir = operant_core::platform::operant_home().join("memory");
                operant_core::memory::MemoryStore::new(mem_dir)
                    .read_memories()
                    .map(|m| m.len())
                    .unwrap_or(0)
            };
            let skills_count = app.skills_view.skills.len();
            if mem_count > 0 || skills_count > 0 {
                if !parts.is_empty() {
                    parts.push(Span::raw("  "));
                }
                let mut pill = String::new();
                if mem_count > 0 {
                    pill.push_str(&format!("mem:{}", mem_count));
                }
                if skills_count > 0 {
                    if !pill.is_empty() {
                        pill.push_str(" · ");
                    }
                    pill.push_str(&format!("skills:{}", skills_count));
                }
                parts.push(Span::styled(pill, Style::default().fg(Color::DarkGray)));
            }
        }

        // Git branch (if settings enabled)
        if app.settings_screen.show_git_branch {
            if let Some(ref branch) = app.git_branch {
                if !parts.is_empty() {
                    parts.push(Span::raw("  "));
                }
                parts.push(Span::styled(
                    format!("⎇ {}", branch),
                    Style::default().fg(Color::Cyan),
                ));
            }
        }

        // Current directory (if settings enabled)
        if app.settings_screen.show_cwd {
            if let Some(ref dir) = app.current_dir {
                if !parts.is_empty() {
                    parts.push(Span::raw("  "));
                }
                // Use dirs::home_dir() so this works on Windows (where $HOME
                // is unset and the home is $USERPROFILE). Guard against an
                // empty home string: `str::replace("", "~")` inserts "~"
                // between every character, producing the infamous
                // `~X~:~\~B~i~g~g~e~r~…` output.
                let home = dirs::home_dir()
                    .and_then(|p| p.to_str().map(|s| s.to_string()))
                    .filter(|s| !s.is_empty());
                let display_dir = match home {
                    Some(h) if dir.starts_with(&h) => dir.replacen(&h, "~", 1),
                    _ => dir.clone(),
                };
                parts.push(Span::styled(
                    display_dir,
                    Style::default().fg(Color::DarkGray),
                ));
            }
        }

        // Output style indicator (only when non-default)
        if app.output_style != "auto" {
            if !parts.is_empty() {
                parts.push(Span::raw("  "));
            }
            parts.push(Span::styled(
                format!("[{}]", app.output_style),
                Style::default().fg(Color::DarkGray),
            ));
        }

        // 8. Bridge/gateway connection badge
        if app.bridge_state.is_visible() {
            if !parts.is_empty() {
                parts.push(Span::raw("  "));
            }
            if let Some(badge) = app.bridge_state.status_badge(app.frame_count) {
                parts.push(badge);
            }
        }

        parts
    };

    // Gap fill
    let left_len: usize = left_spans
        .iter()
        .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
        .sum();
    let right_len: usize = right_spans
        .iter()
        .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
        .sum();
    let gap = (footer_area.width.saturating_sub(2) as usize).saturating_sub(left_len + right_len);

    let mut spans = left_spans;
    spans.push(Span::raw(" ".repeat(gap)));
    spans.extend(right_spans);

    // Add padding: 1 char on each side
    let padded_area = Rect {
        x: footer_area.x + 1,
        y: footer_area.y,
        width: footer_area.width.saturating_sub(2),
        height: footer_area.height,
    };
    frame.render_widget(Paragraph::new(vec![Line::from(spans)]), padded_area);
}

fn render_prompt_suggestions(frame: &mut Frame, app: &App, area: Rect) {
    let suggestions = &app.prompt_input.suggestions;
    if suggestions.is_empty() || area.height == 0 {
        return;
    }

    let selected = app.prompt_input.suggestion_index.unwrap_or(0);
    let max_visible = area.height as usize;
    let start = selected
        .saturating_sub(max_visible / 2)
        .min(suggestions.len().saturating_sub(max_visible));
    let end = (start + max_visible).min(suggestions.len());
    let label_width = area.width.saturating_div(3).max(12) as usize;

    for (row, suggestion) in suggestions[start..end].iter().enumerate() {
        let is_selected = start + row == selected;
        let accent_style = if is_selected {
            Style::default()
                .fg(ACCENT_PRIMARY)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let label_style = if is_selected {
            Style::default()
                .fg(ACCENT_PRIMARY)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        let detail_style = if is_selected {
            Style::default().fg(ACCENT_PRIMARY)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let mut spans = vec![Span::styled(
            if is_selected { "\u{203a} " } else { "  " },
            accent_style,
        )];
        match suggestion.source {
            TypeaheadSource::SlashCommand => {
                let display_name = truncate_text(&suggestion.text, label_width);
                spans.push(Span::styled(
                    format!("{display_name:<width$}", width = label_width),
                    label_style,
                ));
                spans.push(Span::styled(
                    " [cmd] ",
                    Style::default().fg(Color::DarkGray),
                ));
                if !suggestion.description.is_empty() {
                    spans.push(Span::styled(
                        truncate_text(
                            &suggestion.description,
                            area.width.saturating_sub(label_width as u16 + 10) as usize,
                        ),
                        detail_style,
                    ));
                }
            }
            TypeaheadSource::FileRef => {
                spans.push(Span::styled("+ ", accent_style));
                spans.push(Span::styled(
                    truncate_middle(&suggestion.text, label_width),
                    label_style,
                ));
                if !suggestion.description.is_empty() {
                    spans.push(Span::styled(
                        " \u{2014} ",
                        Style::default().fg(Color::DarkGray),
                    ));
                    spans.push(Span::styled(
                        truncate_text(&suggestion.description, area.width as usize / 2),
                        detail_style,
                    ));
                }
            }
        }

        frame.render_widget(
            Paragraph::new(Line::from(spans)),
            Rect {
                x: area.x,
                y: area.y + row as u16,
                width: area.width,
                height: 1,
            },
        );
    }
}
