// Vendored from jcode (crates/operant-tui/src/tui/info_widget_swarm_gallery.rs), MIT License,
// Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805 (W7 batch-4).
//! Adapter from swarm member status into the inline gallery layout.
//!
//! All presentation logic (status colors, role glyphs, age formatting, header,
//! sorting, layout config) lives in the shared
//! `operant_tui_render::swarm_gallery` module so the live TUI and the
//! `swarm_gallery_live` demo render identically. This adapter only handles
//! turning a [`SwarmMemberStatus`] into a renderer-agnostic
//! [`GalleryMember`] (label + body lines).

use crate::tui::operant_app::protocol::SwarmMemberStatus;
// [port-decision] import list trimmed at batch-4 and re-trimmed 2026-11-14: the
// SwarmTile-bound strip renderers (render_gallery, render_swarm_dock,
// render_swarm_live_card, render_swarm_panel, render_swarm_strip,
// render_swarm_strip_vertical, SwarmStripHint) live in the excised swarm_tiles
// surface (iter-591), and the fns that called them were the gated bodies deleted
// this round. Only what the live adapters use is imported now.
use crate::tui::operant_render::swarm_gallery::GalleryMember;
use ratatui::prelude::*;

fn member_label(member: &SwarmMemberStatus) -> String {
    member
        .friendly_name
        .clone()
        .unwrap_or_else(|| member.session_id.chars().take(8).collect())
}

/// Session icon (emoji) for a member, derived from its friendly name (session
/// names come from the shared `SESSION_NAMES` word list, e.g. "fox" -> 🦊).
/// Falls back to `None` when the name is unknown so the strip shows the name.
fn member_icon(member: &SwarmMemberStatus) -> Option<String> {
    let name = member.friendly_name.as_deref()?;
    let icon = crate::tui::operant_app::id::session_icon(name);
    if icon == "💫" {
        // Unknown word: don't show the generic fallback, keep the name.
        None
    } else {
        Some(icon.to_string())
    }
}

/// Age marker appended to member bodies, e.g. "· 7s ago" or "· now".
/// `humanize_age` already yields "now" for fresh updates, which reads wrong
/// with an "ago" suffix.
/// Convert swarm members into renderer-agnostic gallery members.
pub(crate) fn members_to_gallery(members: &[SwarmMemberStatus]) -> Vec<GalleryMember> {
    members
        .iter()
        .map(|member| GalleryMember {
            label: member_label(member),
            icon: member_icon(member),
            status: member.status.clone(),
            role: member.role.clone(),
            sort_key: member.session_id.clone(),
            model: member.runtime.model.clone(),
            provider: member.runtime.provider.clone(),
            auth_method: member.runtime.auth_method.clone(),
        })
        .collect()
}

/// Render expanded member cards for insertion directly beneath a swarm tool
/// call in the transcript.
pub(crate) fn render_swarm_chat_card_lines(
    members: &[SwarmMemberStatus],
    width: usize,
) -> Vec<Line<'static>> {
    let mut gallery_members = members_to_gallery(members);
    for (gallery, member) in gallery_members.iter_mut().zip(members) {
        if let Some(label) = member
            .task_label
            .as_deref()
            .map(str::trim)
            .filter(|label| !label.is_empty())
        {
            // Transcript cards belong to the spawn call, so the user-provided
            // spawn label is the useful primary identity. The generated animal
            // name remains represented by the session icon and is still used by
            // the persistent swarm gallery/panel.
            gallery.label = label.to_string();
        }
    }
    crate::tui::operant_render::swarm_gallery::render_swarm_chat_cards(&gallery_members, width)
}

/// Render the dedicated live swarm page: a nested, ownership-aware tree of every
/// managed agent above the selected agent's detailed live card.
///
/// `[port-decision]` The live-card renderer behind it is the excised
/// `swarm_tiles` surface (iter-591). Its gated verbatim body was deleted
/// 2026-11-14 — 630 lines across 15 `#[cfg(any())]` items that could never
/// compile, which is what made the file look like a live port of a surface that
/// does not exist. The body is in git history if the strip surface is ported;
/// until then the honest degraded answer is this early return, which is what the
/// body itself produced for an empty member list.
pub(crate) fn render_swarm_page_lines(
    _members: &[SwarmMemberStatus],
    _selected: usize,
    _spinner_frame: usize,
    _width: usize,
    _max_height: usize,
) -> Vec<Line<'static>> {
    Vec::new()
}

/// Swarm strip band (upstream `swarm_tiles::render_swarm_strip[_vertical]`).
///
/// `[port-decision]` Same excision as above: the strip renderers and
/// `SwarmStripHint` are out of port (iter-591), and their gated bodies were
/// deleted 2026-11-14 with the other 630 gated lines. Empty is also the early
/// return the body produced, so behaviour is unchanged.
pub(crate) fn render_swarm_strip_lines(
    _members: &[SwarmMemberStatus],
    _selected: usize,
    _focused: bool,
    _focus_key: &str,
    _spinner_frame: usize,
    _width: usize,
    _max_height: usize,
) -> Vec<Line<'static>> {
    Vec::new()
}

/// Render the inline swarm gallery for the given members into `area`-width lines.
/// Render the list+detail swarm panel: a compact list of managed agents plus a
/// detail viewport for the `selected` one. `focused` adds an interaction hint.
/// Render the compact swarm strip (agent chips + status glyphs + todo counts)
/// shown directly above the status line.
///
/// The layout follows `agents.swarm_strip_layout`: `vertical` (default) lists
/// one agent per row (session icon + task, capped to a few rows), while
/// `horizontal` packs all agents as chips on a single row.
///
/// `focus_key` is the configured chord to enter the controls (e.g. "ctrl+t"),
/// used for the unfocused enter-hint.
/// `spinner_frame` animates active agents' glyphs. `max_height` bounds the
#[cfg(test)]
mod tests {
    use super::*;
    // [port-decision] members_to_tiles import dropped: the SwarmTile
    // converter lives in the excised swarm_tiles surface (iter-591); the
    // tests that used it are gated below. Re-activate at W7.

    fn member(
        id: &str,
        status: &str,
        detail: Option<&str>,
        role: Option<&str>,
    ) -> SwarmMemberStatus {
        SwarmMemberStatus {
            session_id: id.to_string(),
            friendly_name: Some(id.to_string()),
            status: status.to_string(),
            detail: detail.map(str::to_string),
            task_label: None,
            role: role.map(str::to_string),
            is_headless: Some(true),
            live_attachments: None,
            status_age_secs: Some(3),
            output_tail: None,
            report_back_to_session_id: None,
            todo_progress: None,
            todo_items: Vec::new(),
            runtime: crate::tui::operant_app::protocol::SwarmMemberRuntime::default(),
        }
    }

    #[test]
    fn transcript_card_prefers_spawn_label_over_generated_name() {
        let mut m = member("cow", "ready", None, None);
        m.task_label = Some("card demo".to_string());
        let rendered = render_swarm_chat_card_lines(&[m], 80)
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert!(rendered.contains("card demo"), "rendered={rendered}");
        assert!(!rendered.contains("cow"), "rendered={rendered}");
    }
}
