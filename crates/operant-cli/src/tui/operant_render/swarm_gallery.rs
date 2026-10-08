// Vendored from jcode (crates/operant-tui-render/src/swarm_gallery.rs), MIT
// License, Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805;
// imports re-rooted (operant_tui_style::color::rgb ->
// crate::tui::vendor::style::color::rgb).
//
// TRUTH CLAUSE: this is the bounded transcript-card closure of upstream
// swarm_gallery.rs (3,099) — the status/age/glyph helpers, the spinner
// cadence consts, the GalleryMember/GalleryTodo/GalleryToolIntent data
// types, the member sort, and `render_swarm_chat_cards`, which the live
// transcript path (ui_messages/ui_prepare -> info_widget::swarm_gallery::
// render_swarm_chat_card_lines) renders beneath `swarm spawn` tool calls.
//
// [port-excision] the strip/dock/panel/gallery-grid/live-card renderers
// (gallery_header, role_glyph, STRIP_SPINNER_FPS, members_to_tiles,
// render_gallery, render_swarm_live_card, render_swarm_panel,
// SwarmStripHint, render_swarm_strip, render_swarm_strip_vertical,
// render_swarm_dock, render_swarm_compact, format_elapsed,
// display_index_to_tile_index, list_row, count_digits and the module's own
// test block) depend on swarm_tiles.rs (SwarmTile/SwarmGalleryConfig),
// excised with this module at iter-591; their call sites in
// info_widget_swarm_gallery.rs are gated with [port-decision] markers.
// Re-activate at W7 when the strip surface ports (see the note in
// operant_render/mod.rs and docs/JCODE-VISUAL-LAYER-IMPLEMENTATION-PLAN.md).

use ratatui::prelude::*;

use crate::tui::vendor::style::color::rgb;

/// Accent color for a member lifecycle status.
/// [port-source] swarm_gallery.rs:17-29
pub fn status_accent(status: &str) -> Color {
    match status {
        "spawned" => rgb(140, 140, 150),
        "ready" => rgb(120, 180, 120),
        "running" | "streaming" => rgb(255, 200, 100),
        "thinking" => rgb(140, 180, 255),
        "blocked" | "waiting_network" => rgb(255, 170, 80),
        "failed" | "crashed" => rgb(255, 100, 100),
        "completed" | "done" => rgb(100, 200, 100),
        "stopped" => rgb(140, 140, 150),
        _ => rgb(140, 140, 150),
    }
}

/// Compact age formatting for member viewports (now/Ns/Nm/Nh).
/// [port-source] swarm_gallery.rs:39-50
pub fn humanize_age(age: u64) -> String {
    if age < 2 {
        "now".to_string()
    } else if age < 60 {
        format!("{age}s")
    } else if age < 3600 {
        format!("{}m", age / 60)
    } else {
        format!("{}h", age / 3600)
    }
}

/// Whether a status counts as "active" for the header's active-agent tally.
/// [port-source] swarm_gallery.rs:52-55
pub fn is_active_status(status: &str) -> bool {
    matches!(status, "running" | "streaming" | "thinking")
}

/// Cadence for active-agent spinner frames.
///
/// Keep this aligned with the TUI redraw interval. 80 ms matches the primary
/// status spinner and avoids the visibly stepped motion of the old 125 ms
/// cadence without redrawing faster than the glyph can change.
/// [port-source] swarm_gallery.rs:57-62
pub const STRIP_SPINNER_FRAME_MS: u64 = 80;

/// Frames for the inline status spinner used by active agents on the strip.
/// [port-source] swarm_gallery.rs:65-66
pub const STRIP_SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// A glyph summarizing a member's lifecycle status. Active members (running,
/// thinking, streaming) animate via the spinner frame; terminal states get a
/// fixed glyph. `spinner_frame` selects the spinner cell for active members.
/// [port-source] swarm_gallery.rs:68-84
pub fn status_glyph(status: &str, spinner_frame: usize) -> &'static str {
    match status {
        "running" | "streaming" | "thinking" => {
            STRIP_SPINNER_FRAMES[spinner_frame % STRIP_SPINNER_FRAMES.len()]
        }
        "completed" | "done" => "✓",
        "ready" => "•",
        "blocked" | "waiting_network" => "⏸",
        "failed" | "crashed" => "✗",
        "stopped" => "◼",
        "spawned" => "·",
        _ => "•",
    }
}

/// Calm lifecycle marker for transcript cards.
///
/// Transcript content should remain stable while users read or scroll it. The
/// dedicated swarm strip retains animated status glyphs for live motion.
/// [port-source] swarm_gallery.rs:86-101
fn card_status_glyph(status: &str) -> &'static str {
    match status {
        "running" | "streaming" | "thinking" => "●",
        "completed" | "done" => "✓",
        "ready" => "•",
        "blocked" | "waiting_network" => "⏸",
        "failed" | "crashed" => "✗",
        "stopped" => "◼",
        "spawned" => "·",
        _ => "•",
    }
}

/// [port-source] swarm_gallery.rs:103-113
fn card_status_label(status: &str) -> &'static str {
    match status {
        "running" | "streaming" | "thinking" => "Working",
        "completed" | "done" => "Completed",
        "ready" | "spawned" => "Ready",
        "blocked" | "waiting_network" => "Blocked",
        "failed" | "crashed" => "Failed",
        "stopped" => "Stopped",
        _ => "Working",
    }
}

/// [port-source] swarm_gallery.rs:126-137
fn format_model(model: &str) -> String {
    let routed = model.rsplit([':', '/']).next().unwrap_or(model);
    let model = routed
        .strip_suffix("-sol")
        .or_else(|| routed.strip_suffix("-luna"))
        .unwrap_or(routed);
    if let Some(rest) = model.strip_prefix("gpt-") {
        format!("GPT-{rest}")
    } else {
        model.to_string()
    }
}

/// Combine provider display name and credential route into one metadata piece,
/// e.g. "OpenAI oauth". Returns `None` when both are blank.
/// [port-source] swarm_gallery.rs:139-150
fn format_route(provider: Option<&str>, auth_method: Option<&str>) -> Option<String> {
    let provider = provider.map(str::trim).filter(|p| !p.is_empty());
    let auth_method = auth_method.map(str::trim).filter(|m| !m.is_empty());
    match (provider, auth_method) {
        (Some(provider), Some(method)) => Some(format!("{provider} {method}")),
        (Some(provider), None) => Some(provider.to_string()),
        (None, Some(method)) => Some(method.to_string()),
        (None, None) => None,
    }
}

/// Sort rank for stable placement: coordinator first, then everything else.
/// [port-source] swarm_gallery.rs:152-158
fn role_rank(role: Option<&str>) -> u8 {
    match role {
        Some("coordinator") => 0,
        _ => 2,
    }
}

/// Sort rank for lifecycle status within a role bucket: still-working agents
/// first, then agents needing attention, then idle ones, then finished ones.
/// This keeps active agents visible on the strip instead of letting them
/// collapse into the "+N" overflow behind completed agents.
/// [port-source] swarm_gallery.rs:160-172
fn status_rank(status: &str) -> u8 {
    match status {
        s if is_active_status(s) => 0,
        "blocked" | "waiting_network" | "failed" | "crashed" => 1,
        "completed" | "done" | "stopped" => 3,
        // ready/spawned/unknown: idle but not finished.
        _ => 2,
    }
}

/// A member of the swarm gallery, renderer-agnostic: the live TUI adapter
/// builds these from session state, and the renderer owns how they look.
///
/// Callers are responsible for building the `body` lines (e.g. choosing live
/// output tail vs. status detail); everything else about how the tile looks is
/// handled here.
/// [port-source] swarm_gallery.rs:195-233
#[derive(Clone, Debug)]
pub struct GalleryMember {
    /// Display title (friendly name or short id).
    pub label: String,
    /// Optional session icon (emoji) shown in place of the name on the
    /// vertical strip, e.g. "🦊" for a session named "fox".
    pub icon: Option<String>,
    /// Lifecycle status string (drives the badge text and accent color).
    pub status: String,
    /// Short label of the task this member was spawned/assigned for. Shown
    /// dimmed next to the name on the strip so the line answers "who is doing
    /// what", not just "who exists".
    pub task: Option<String>,
    /// Swarm role, if any (drives the title glyph and sort order).
    pub role: Option<String>,
    /// Pre-rendered body lines shown inside the tile.
    pub body: Vec<String>,
    /// Stable tiebreaker for sorting members with equal role rank (e.g. id).
    pub sort_key: String,
    /// Optional todo progress as (completed, total) for the agent's plan/todos.
    /// Rendered as "C/T" next to the agent on the strip when present.
    pub todo: Option<(u32, u32)>,
    /// Compact todo entries (content, status) for the focused detail view.
    /// Status is one of "pending", "in_progress", "completed".
    pub todo_items: Vec<GalleryTodo>,
    /// Provider model currently running this member, when known.
    pub model: Option<String>,
    /// Provider display name and credential route used by this member.
    pub provider: Option<String>,
    pub auth_method: Option<String>,
    /// Reasoning effort selected for this member.
    pub effort: Option<String>,
    /// Seconds since this member was spawned.
    pub elapsed_secs: Option<u64>,
}

/// One compact todo entry shown in the focused swarm detail view.
/// [port-source] swarm_gallery.rs:235-243
#[derive(Clone, Debug)]
pub struct GalleryTodo {
    pub content: String,
    /// "pending", "in_progress", or "completed".
    pub status: String,
    /// Up to three recent tool calls made while this item was active.
    pub tool_intents: Vec<GalleryToolIntent>,
}

/// [port-source] swarm_gallery.rs:245-253
#[derive(Clone, Debug)]
pub struct GalleryToolIntent {
    pub tool_name: String,
    pub intent: String,
    /// "running", "completed", or "error".
    pub status: String,
    /// Optional live progress as (current, total, unit).
    pub progress: Option<(u64, u64, Option<String>)>,
}

/// Render stable, one-line swarm summaries beneath `swarm spawn` tool calls.
///
/// Transcript content deliberately excludes elapsed time, output tails, todos,
/// tool progress, and animated glyphs. Those fields update frequently and make
/// old chat rows move while the user is reading them. The dedicated live swarm
/// page owns the detailed, animated representation instead. Spawn-time-stable
/// metadata (model, provider/auth route) is shown since it answers "what is
/// this agent running on" without churning.
/// [port-source] swarm_gallery.rs:304-366
pub fn render_swarm_chat_cards(members: &[GalleryMember], width: usize) -> Vec<Line<'static>> {
    if members.is_empty() || width < 8 {
        return Vec::new();
    }

    let mut out = Vec::new();
    for member in sort_members_for_display(members) {
        let accent = status_accent(&member.status);
        let lead = format!(
            "    {} {} ",
            member.icon.as_deref().unwrap_or("🐝"),
            card_status_glyph(&member.status)
        );
        let label = member.label.clone();

        // Stable runtime metadata (model and provider/auth route) is fixed at
        // spawn time, so it can live on the transcript card without making old
        // chat rows churn. Drop trailing pieces first when width is tight.
        let mut metadata = vec![card_status_label(&member.status).to_string()];
        if let Some(model) = member
            .model
            .as_deref()
            .filter(|model| !model.trim().is_empty())
        {
            metadata.push(format_model(model));
        }
        if let Some(route) = format_route(member.provider.as_deref(), member.auth_method.as_deref())
        {
            metadata.push(route);
        }
        let mut tail = format!(" · {}", metadata.join(" · "));
        while metadata.len() > 1 && disp_w(&lead) + disp_w(&label) + disp_w(&tail) > width {
            metadata.pop();
            tail = format!(" · {}", metadata.join(" · "));
        }

        let mut header = vec![
            Span::styled(lead.clone(), Style::default().fg(rgb(255, 200, 100))),
            Span::styled(
                label.clone(),
                Style::default().fg(accent).add_modifier(Modifier::BOLD),
            ),
        ];
        let consumed = disp_w(&lead) + disp_w(&label);
        if consumed + disp_w(&tail) <= width {
            header.push(Span::styled(tail, Style::default().fg(rgb(150, 150, 160))));
        }
        out.push(Line::from(header));
    }

    for line in &mut out {
        clamp_line_to_width(line, width);
    }
    out
}

/// Splits mid-span if needed, dropping a trailing wide glyph that would
/// straddle the boundary.
/// [port-source] swarm_gallery.rs:1495-1524
fn clamp_line_to_width(line: &mut Line<'static>, max_width: usize) {
    use unicode_width::UnicodeWidthChar;
    let mut used = 0usize;
    let mut clamped: Vec<Span<'static>> = Vec::new();
    for span in line.spans.drain(..) {
        let w = disp_w(&span.content);
        if used + w <= max_width {
            used += w;
            clamped.push(span);
            continue;
        }
        // Partial span: take chars while they fit.
        let mut taken = String::new();
        for ch in span.content.chars() {
            let cw = ch.width().unwrap_or(0);
            if used + cw > max_width {
                break;
            }
            used += cw;
            taken.push(ch);
        }
        if !taken.is_empty() {
            clamped.push(Span::styled(taken, span.style));
        }
        break;
    }
    line.spans = clamped;
}

/// [port-source] swarm_gallery.rs:1805-1815
pub fn display_order(members: &[GalleryMember]) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..members.len()).collect();
    idx.sort_by(|&a, &b| {
        let (a, b) = (&members[a], &members[b]);
        role_rank(a.role.as_deref())
            .cmp(&role_rank(b.role.as_deref()))
            .then_with(|| status_rank(&a.status).cmp(&status_rank(&b.status)))
            .then_with(|| a.sort_key.cmp(&b.sort_key))
    });
    idx
}

/// References to `members` in [`display_order`], for rendering.
/// [port-source] swarm_gallery.rs:1817-1823
fn sort_members_for_display(members: &[GalleryMember]) -> Vec<&GalleryMember> {
    display_order(members)
        .into_iter()
        .map(|i| &members[i])
        .collect()
}

/// Terminal display width of a string (wide glyphs like 🐝 count as 2).
/// [port-source] swarm_gallery.rs:1925-1930
fn disp_w(s: &str) -> usize {
    use unicode_width::UnicodeWidthStr;
    s.width()
}
