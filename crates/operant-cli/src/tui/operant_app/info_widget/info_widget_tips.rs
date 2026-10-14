// Vendored from jcode (crates/operant-tui/src/tui/info_widget_tips.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805 (W7 batch-4).
use super::*;

const TIP_CYCLE_SECONDS: u64 = 15;
const STATUS_TIP_PERIOD_SECONDS: u64 = 90;
const STATUS_TIP_OFFSET_SECONDS: u64 = 28;
const STATUS_TIP_SHOW_SECONDS: u64 = 12;

struct Tip {
    text: String,
}

fn all_tips() -> Vec<Tip> {
    // 2026-10-14 batch, plan 10: the vendored jcode tips advertised features
    // operant never had (Ctrl+J/K prompt jumps, ambient mode with emailed
    // summaries, swarm auto-formation, Alt+B tool-backgrounding, Alt+G diff
    // cycling, Shift+Tab favorited models). Every tip below names a chord or
    // feature verified against the keybinding registry / dispatch arms.
    let mut tips = vec![
        "Ctrl+K opens the command palette — every registered command is one chord away",
        "Ctrl+G or Alt+G toggles a scroll bookmark — press again to teleport back",
        "PageUp / PageDown scroll 10 lines · Ctrl+Up/Down 3 · Alt+Up/Down 20",
        "Ctrl+A opens the model picker · Alt+P cycles your configured models",
        "Ctrl+T enters drag-select copy mode for exact text capture",
        "Ctrl+O expands the latest tool row's full output",
        "Ctrl+R searches prompt history across sessions",
        "/permissions chooses the permission mode — Bypass always needs an explicit confirmation",
        "/keys lists every real keybinding from the registry; /hotkeys shows which you use",
        "Most terminals can be configured to copy text on highlight - no Ctrl+C needed. Check your terminal's settings for 'copy on select'",
    ];
    if crate::tui::operant_app::config_shim::config()
        .features
        .mermaid
    {
        tips.insert(3, "```mermaid code blocks render as diagrams");
    }
    // Mac keyboards label this modifier ⌥, not Alt, so rewrite hints there.
    let alt = crate::tui::operant_app::keybind::alt_label();
    tips.into_iter()
        .map(|text| Tip {
            text: text.replace("Alt+", &format!("{alt}+")),
        })
        .collect()
}

static TIP_STATE: Mutex<Option<(usize, Instant)>> = Mutex::new(None);

fn current_tip(_max_width: usize) -> Tip {
    let tips = all_tips();
    let mut guard = TIP_STATE.lock().unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    let (idx, last) = guard.get_or_insert_with(|| (0, now));

    let should_advance = now.duration_since(*last).as_secs() >= TIP_CYCLE_SECONDS;
    if should_advance {
        *idx = (*idx + 1) % tips.len();
        *last = now;
    }

    let i = *idx % tips.len();
    drop(guard);
    Tip {
        text: tips[i].text.clone(),
    }
}

pub(crate) fn occasional_status_tip(max_width: usize, elapsed_secs: u64) -> Option<String> {
    if max_width < 16 {
        return None;
    }

    let cycle_pos = elapsed_secs % STATUS_TIP_PERIOD_SECONDS;
    let show_until = STATUS_TIP_OFFSET_SECONDS + STATUS_TIP_SHOW_SECONDS;
    if cycle_pos < STATUS_TIP_OFFSET_SECONDS || cycle_pos >= show_until {
        return None;
    }

    let prefix = "💡 ";
    let available = max_width.saturating_sub(prefix.chars().count());
    if available < 12 {
        return None;
    }

    let tip = current_tip(available);
    Some(format!(
        "{}{}",
        prefix,
        truncate_smart(&tip.text, available)
    ))
}

fn wrap_tip_text(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![text.to_string()];
    }
    let mut lines = Vec::new();
    let mut remaining = text;
    while !remaining.is_empty() {
        if remaining.len() <= width {
            lines.push(remaining.to_string());
            break;
        }
        let mut boundary = width.min(remaining.len());
        while boundary > 0 && !remaining.is_char_boundary(boundary) {
            boundary -= 1;
        }
        let split = remaining[..boundary].rfind(' ').unwrap_or(boundary);
        let (line, rest) = remaining.split_at(split);
        lines.push(line.to_string());
        remaining = rest.trim_start();
    }
    lines
}

/// Border layout: `💡 Did you know?` top-left, the wrapped tip in the body.
pub(super) fn render_tips_widget(inner: Rect) -> super::frame::Framed {
    let w = inner.width as usize;
    let tip = current_tip(w);
    let wrapped = wrap_tip_text(&tip.text, w);

    let lines: Vec<Line<'static>> = wrapped
        .into_iter()
        .map(|line_text| {
            Line::from(Span::styled(
                line_text,
                Style::default().fg(rgb(160, 160, 175)),
            ))
        })
        .collect();

    super::frame::Framed::body(lines).title(Line::from(vec![
        Span::styled("💡 ", Style::default().fg(rgb(255, 210, 80))),
        super::frame::label("Did you know?"),
    ]))
}
