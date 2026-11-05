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
    //
    // (iter-1762) That sentence was not true of three of them, and the tips
    // are the one surface the operator reads on a loop: a chord that no
    // longer does what the tip says sends the operator to the wrong feature.
    //   * "Ctrl+G or Alt+G toggles a scroll bookmark" — Ctrl+G is the EXTERNAL
    //     EDITOR (the bookmark moved to Alt+G at iter-1267); Ctrl+G opened the
    //     operator's editor instead of the bookmark.
    //   * "PageUp / PageDown scroll 10 lines" — the live step is one PAGE
    //     (the transcript viewport's height minus two rows of overlap); 10 is
    //     only the fallback for a headless app that has published no frame.
    //   * "Ctrl+T enters drag-select copy mode" — Ctrl+T toggles the THINKING
    //     display (copy mode moved from Ctrl+T to Alt+Y at plan 08).
    // The guard `every_chord_a_tip_names_agrees_with_the_catalogue` (below)
    // now pins each tip's chord to the CATALOGUE's own description for it, so
    // a repurposed chord cannot keep a stale tip alive.
    let mut tips = vec![
        "Ctrl+K opens the command palette — every registered command is one chord away",
        "Alt+G toggles a scroll bookmark — press again to teleport back",
        "PageUp / PageDown scroll one page · Ctrl+Up/Down 3 · Alt+Up/Down 20",
        "Ctrl+A opens the model picker · Alt+P cycles your configured models",
        "Alt+Y enters drag-select copy mode for exact text capture",
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::keybindings::{BindingContext, DEFAULT_KEYBINDINGS};

    /// (the tip's identifying substring, the chord it names, the words the
    /// CATALOGUE must use for that chord). The row pins BOTH sides: the
    /// substring is the tip's prose (so the prose cannot drift without this
    /// table changing), and the claim is the catalogue's own description for
    /// the chord (so a repurposed chord cannot keep a stale tip alive). The
    /// prose may be reworded; the table must change with it.
    ///
    /// iter-1762 is why this exists. Three tips named a chord that no longer did
    /// what the tip said — Ctrl+G as the scroll bookmark (it is the external
    /// editor), Ctrl+T as drag-select copy mode (it toggles thinking display),
    /// and PageUp's step as "10 lines" when the live step is one page (the
    /// viewport's height minus two rows of overlap) — while the module's own
    /// comment asserted every tip had been "verified against the keybinding
    /// registry / dispatch arms". It had not.
    const TIP_CLAIMS: &[(&str, &str, &str)] = &[
        (
            "Ctrl+K opens the command palette",
            "Ctrl+K",
            "command palette",
        ),
        (
            "Alt+G toggles a scroll bookmark",
            "Alt+G",
            "scroll bookmark",
        ),
        ("PageUp / PageDown scroll one page", "PageUp", "one page"),
        ("PageUp / PageDown scroll one page", "PageDown", "one page"),
        ("Ctrl+Up/Down 3", "Ctrl+Up", "up 3"),
        ("Ctrl+Up/Down 3", "Ctrl+Down", "down 3"),
        ("Alt+Up/Down 20", "Alt+Up", "up 20"),
        ("Alt+Up/Down 20", "Alt+Down", "down 20"),
        ("Ctrl+A opens the model picker", "Ctrl+A", "model picker"),
        (
            "Alt+P cycles your configured models",
            "Alt+P",
            "configured models",
        ),
        ("Alt+Y enters drag-select copy mode", "Alt+Y", "copy mode"),
        (
            "Ctrl+O expands the latest tool row",
            "Ctrl+O",
            "expand latest tool output",
        ),
        ("Ctrl+R searches prompt history", "Ctrl+R", "history search"),
    ];

    /// The chords a tip names, as written in the tip.
    fn chords_in(text: &str) -> Vec<String> {
        let mut found = Vec::new();
        for prefix in ["Ctrl+", "Alt+", "Shift+"] {
            let mut rest = text;
            while let Some(start) = rest.find(prefix) {
                let after = &rest[start + prefix.len()..];
                let end = after
                    .find(|c: char| !c.is_ascii_alphanumeric())
                    .unwrap_or(after.len());
                found.push(format!("{prefix}{}", &after[..end]));
                rest = &after[end..];
            }
        }
        for name in ["PageUp", "PageDown", "F1", "F8"] {
            if text.contains(name) {
                found.push(name.to_string());
            }
        }
        found
    }

    /// A chord named in a tip must agree with what the CATALOGUE says it does.
    ///
    /// The prose may vary; the meaning may not. A chord the catalogue does not
    /// bind fails here too, because a tip pointing at a nonexistent binding is
    /// the same failure from the other side.
    #[test]
    fn every_chord_a_tip_names_agrees_with_the_catalogue() {
        // The whole catalogue, not just the Global context: a tip may name a
        // Prompt-context chord (Ctrl+R searches the prompt history), and the
        // catalogue is the authority for every context.
        let catalogue: Vec<(String, String)> = BindingContext::ALL
            .iter()
            .flat_map(|context| DEFAULT_KEYBINDINGS.get_bindings(*context))
            .map(|binding| {
                (
                    binding.chord(),
                    binding.description.clone().unwrap_or_default(),
                )
            })
            .collect();
        let tips = all_tips();

        for (tip_substring, chord, claim) in TIP_CLAIMS {
            assert!(
                tips.iter().any(|tip| tip.text.contains(tip_substring)),
                "no tip contains \"{tip_substring}\" — the claim this test pins is gone"
            );
            let description = catalogue
                .iter()
                .find(|(bound, _)| bound.eq_ignore_ascii_case(chord))
                .map(|(_, description)| description)
                .unwrap_or_else(|| {
                    panic!("the tip names {chord}, which the catalogue does not bind")
                });
            assert!(
                description.to_lowercase().contains(&claim.to_lowercase()),
                "the catalogue says \"{description}\" for {chord}, which does not include \
                 \"{claim}\" — the tip and the catalogue disagree about what the chord does"
            );
        }

        // Every chord a tip names must be pinned by a row above: a chord added
        // to a tip without a claim here would otherwise sail past the checks.
        for tip in &tips {
            // The copy-on-select tip names Ctrl+C to say the operator does NOT
            // need it — a negative mention, not a claim about the chord.
            if tip.text.contains("copy text on highlight") {
                continue;
            }
            for chord in chords_in(&tip.text) {
                let pinned = TIP_CLAIMS.iter().any(|(substring, bound, _)| {
                    bound.eq_ignore_ascii_case(&chord) && tip.text.contains(substring)
                });
                assert!(
                    pinned,
                    "the tip \"{}\" names {chord}, which this test does not pin — add a \
                     TIP_CLAIMS row for it (the catalogue's own description is the words \
                     the tip and the catalogue must agree on)",
                    tip.text
                );
            }
        }
    }
}

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
