// rewind_selector.rs — fullscreen /rewind selector (2026-10-14 batch, plan 05).
//
// Replaces the old RewindFlowOverlay modal list. The selector shows the REAL
// transcript (the body renderer's own wrapped lines), outlines the selected
// user turn, dims everything after it (the preview of what a rewind drops),
// and carries a filter over user prompts plus a keeps/tokens/drops footer.
// The marks come from the agent (`OperantAgent::rewind_marks`); Enter asks
// the agent to fork the session at the selected turn (run_pending_agent_actions).

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::tui::vendor::style::theme;

/// One rewindable user turn, mirrored from the agent's `UserTurnMark`.
#[derive(Debug, Clone)]
pub struct RewindEntry {
    pub turn_id: u64,
    /// The prompt text (relocation key; also the filter subject).
    pub text: String,
    /// True when the mark's prompt text no longer matches any live display
    /// row — the turn was folded away by compaction and cannot be rewound.
    pub compacted: bool,
}

#[derive(Debug, Default)]
pub struct RewindSelectorState {
    pub visible: bool,
    /// Mark list, synced from the agent each pass while open.
    pub entries: Vec<RewindEntry>,
    /// Cursor into `entries` (the selected turn, in mark order).
    pub selected: usize,
    /// Case-insensitive substring filter over prompt texts.
    pub filter: String,
    pub filter_open: bool,
    /// Inline notice (e.g. "That turn was folded by compaction…"), shown in
    /// the footer; cleared on the next cursor move.
    pub notice: Option<String>,
    /// Preview for the selection: (kept messages, estimated tokens).
    pub preview: Option<(usize, u64)>,
    /// Vertical scroll of the transcript body, in wrapped lines.
    pub scroll: usize,
}

impl RewindSelectorState {
    pub fn open(&mut self) {
        self.visible = true;
        self.selected = self.entries.len().saturating_sub(1);
        self.filter.clear();
        self.filter_open = false;
        self.notice = None;
        self.preview = None;
        self.scroll = 0;
    }

    pub fn close(&mut self) {
        self.visible = false;
        self.filter_open = false;
        self.notice = None;
    }

    /// Mirror the agent's marks into entries; keep the cursor on the same
    /// turn id when possible, else the last turn. Called from
    /// run_pending_agent_actions while the selector is open (marks change as
    /// turns land).
    pub fn sync_marks(&mut self, marks: Vec<(u64, String)>, display_texts: &[String]) {
        // The anchor is the SELECTED TURN'S TEXT, not its turn_id (iter-1515).
        //
        // turn_id is derived by `OperantAgent::reseed_user_turns_for_rehydration`
        // from a process-global counter, so the ids in a re-read mark list are
        // fresh values that never equal the ids the selector saw last frame.
        // Keying the cursor on the id therefore failed on EVERY frame — this
        // runs once per frame from `App::run` — and `selected` was reset to the
        // newest turn, erasing the operator's arrow-key navigation before the
        // next paint. That is the live defect: `<up>` x4 left the outline on
        // the first turn while the cursor index itself had advanced.
        //
        // A turn's TEXT is the identity that survives a re-derive (the same
        // conversation produces the same prompts), and it is what the operator
        // recognises. Nothing here is required to be unique: an operator who
        // sends the same prompt twice and rewinds gets the same turn either
        // way, and a list that grew lengthwise keeps its earlier entries in
        // place.
        // `None` when this is the FIRST sync (nothing selected yet) or the
        // previous list was empty: both mean there is no operator navigation to
        // preserve, so the selector opens on the newest turn — the same place
        // `open()` puts it. Only a list that genuinely had a selection takes the
        // position-preserving path.
        let anchor_text = self.entries.get(self.selected).map(|e| e.text.clone());
        let had_selection = self
            .entries
            .get(self.selected)
            .is_some_and(|e| !e.text.is_empty());
        let anchor_pos = self.selected;
        self.entries = marks
            .into_iter()
            .map(|(turn_id, text)| RewindEntry {
                turn_id,
                compacted: !text.is_empty()
                    && !display_texts.iter().any(|t| prompt_texts_match(t, &text)),
                text,
            })
            .collect();
        let resolved = anchor_text.as_ref().and_then(|text| {
            self.entries
                .iter()
                .position(|e| prompt_texts_match(&e.text, text))
        });
        self.selected = match resolved {
            // Same conversation, re-derived ids: the anchor text still resolves.
            Some(idx) => idx,
            // The list shape changed (a turn landed, a compaction removed one).
            // Clamp the old position into the new bounds rather than jumping to
            // the tail: navigation the operator can feel must survive a list
            // that got one entry longer.
            None if had_selection && anchor_pos < self.entries.len() => anchor_pos,
            // The selection itself was removed (compaction folded the selected
            // turn). There is nothing honest left to highlight, so fall back to
            // the newest turn — and Enter refuses a compacted turn anyway.
            None => self.entries.len().saturating_sub(1),
        };
    }

    /// Which entries the arrow keys AND the filter's live-jump may land on.
    ///
    /// omp's `#move` (rewind-selector.ts:647-668) steps over
    /// `outlineVisibility` failures only — an entry that renders no row is not
    /// a stop. Ours has the same shape in `compacted`: a turn folded out of
    /// the display still has a mark (so Enter can refuse it) but no rows, so
    /// parking on it produces an outline over nothing and a dead Enter.
    ///
    /// ONE gate for both paths, because there were two: this one and
    /// `filtered_indices`, which the key-handler's live-jump used on every
    /// keystroke. They disagreed on exactly the case that matters — a query
    /// matching only a folded turn jumped the cursor onto it, somewhere the
    /// arrow keys could not then reach (they only walk navigable entries).
    fn navigable_indices(&self) -> Vec<usize> {
        (0..self.entries.len())
            .filter(|&i| !self.entries[i].compacted)
            .collect()
    }

    pub fn select_prev(&mut self) {
        self.step(-1);
        self.notice = None;
    }

    pub fn select_next(&mut self) {
        self.step(1);
        self.notice = None;
    }

    /// Move the cursor by one navigable entry, honouring the filter and
    /// skipping turns no longer in the transcript.
    ///
    /// Called from both arrow keys; `rewind_selector.selected` is an index into
    /// the FULL mark list (the render resolves it to a display slot), so the
    /// steps are computed over the filtered/navigable subset and then mapped
    /// back to that index. An empty filter is the full list minus compacted
    /// entries, which is what open navigation uses.
    fn step(&mut self, delta: isize) {
        let candidates = self.matching_indices();
        if candidates.is_empty() {
            return;
        }
        let current = candidates
            .iter()
            .position(|&i| i == self.selected)
            .unwrap_or(candidates.len());
        let next = if delta < 0 {
            current.saturating_sub(1)
        } else {
            (current + 1).min(candidates.len() - 1)
        };
        self.selected = candidates[next];
    }

    /// The navigable entries that pass the current filter, as indices.
    ///
    /// The one definition of "what the keys may reach", shared by `step` and
    /// by the key-handler's live-jump so a typed filter cannot put the cursor
    /// somewhere the arrows will not follow.
    fn matching_indices(&self) -> Vec<usize> {
        let navigable = self.navigable_indices();
        if self.filter.is_empty() {
            return navigable;
        }
        let needle = self.filter.to_lowercase();
        navigable
            .into_iter()
            .filter(|&i| self.entries[i].text.to_lowercase().contains(&needle))
            .collect()
    }

    /// The entries that pass the current filter, with their positions.
    pub fn filtered_indices(&self) -> Vec<usize> {
        self.matching_indices()
    }

    pub fn selected_turn_id(&self) -> Option<u64> {
        self.entries.get(self.selected).map(|e| e.turn_id)
    }
}

/// Prompt-text equivalence for the compacted check: display prompt texts are
/// the wrapped first lines; the mark carries the full prompt. A prefix match
/// on the first 64 chars is the honest comparison available without storing
/// display-side turn ids.
fn prompt_texts_match(display: &str, mark: &str) -> bool {
    let d: String = display.chars().take(64).collect();
    let m: String = mark.chars().take(64).collect();
    !d.is_empty() && d == m
}

/// Paint the selector: the real transcript body (via the body renderer's own
/// prepared lines), the selected turn outlined, everything after it dimmed,
/// header with filter, footer with preview stats and hints.
pub fn draw_rewind_selector(frame: &mut Frame, area: Rect, app: &crate::tui::app::App) {
    use crate::tui::operant_ui::prepare::prepare_body_for_overlay;
    use ratatui::layout::{Constraint, Layout};

    let state = &app.rewind_selector;
    let width = area.width;

    // Header: title (+ filter line when open) — PINNED.
    let title_style = Style::default()
        .fg(theme::accent_color())
        .add_modifier(Modifier::BOLD);
    let dim = Style::default().fg(theme::dim_color());
    let mut header: Vec<Line> = vec![Line::from(vec![
        Span::styled("  Rewind to a previous prompt ", title_style),
        Span::styled("· Enter to rewind · f filter · Esc cancel", dim),
    ])];
    if state.filter_open {
        header.push(Line::from(vec![
            Span::styled("  filter: ", dim),
            Span::raw(state.filter.clone()),
            Span::styled("█", title_style),
        ]));
    }

    // The real transcript, from the body renderer — the scrollable middle.
    let prepared = prepare_body_for_overlay(app, width);
    let body = &prepared.wrapped_lines;

    // Resolve the selection to a row in the prepared body. Preferred path is
    // position (`wrapped_user_prompt_starts[selected]`); the text fallback
    // covers a mark list whose ids were re-derived between frames.
    let sel_prompt_idx = user_prompt_index(&prepared, app);
    let sel_start =
        sel_prompt_idx.and_then(|idx| prepared.wrapped_user_prompt_starts.get(idx).copied());

    // THE THREE ZONES (oh-my-pi `#markRun`, rewind-selector.ts:1066-1075).
    //
    // A TURN is the prompt plus everything the session produced in response:
    // the assistant's prose, its tool calls, and their results. Its block
    // therefore runs from its prompt row to the NEXT prompt's row (or the end
    // of the body for the newest turn).
    //
    // The defect this replaces: the block used to end at
    // `wrapped_user_prompt_ends[selected]` — the end of the PROMPT ROW alone.
    // So the outlined zone was one row, and the selected turn's own answer,
    // tool call and tool result all fell on the DIM side, painted as content
    // the rewind would drop. The operator read that as a broken preview: the
    // turn they picked looked empty, and the pane below was all "dropped".
    //
    //   [0, sel_start)        normal   — kept by the rewind
    //   [sel_start, sel_end)  PICK     — the outlined turn, whole block
    //   [sel_end, ∞)          DROP     — what the rewind removes
    let sel_end = sel_start.map(|start| {
        let next = prepared
            .wrapped_user_prompt_starts
            .iter()
            .copied()
            .find(|&s| s > start);
        next.unwrap_or(body.len())
    });

    let mut mid: Vec<Line> = Vec::new();

    // omp puts a caption directly over the picked turn naming what Enter does
    // and how many turns it drops. Without it the selector shows the outline
    // but never says the draft returns to the composer, which is the one
    // behaviour the operator cannot guess from a highlighted row.
    //
    // Always the same two rows when a turn is resolved, which keeps the offset
    // a constant instead of a per-frame computation the zone check re-derives.
    const CAPTION_ROWS: usize = 2;
    if let Some(end) = sel_end {
        let below = prepared
            .wrapped_user_prompt_starts
            .iter()
            .filter(|&&s| s >= end)
            .count();
        let tail = if below > 0 {
            format!(
                "{below} turn{} below dropped",
                if below == 1 { "" } else { "s" }
            )
        } else {
            "nothing below to drop".to_string()
        };
        mid.push(Line::from(vec![Span::styled(
            format!("  ⏪ Rewind to here · the prompt returns to the composer · {tail}"),
            Style::default()
                .fg(theme::accent_color())
                .add_modifier(Modifier::BOLD),
        )]));
        // A blank rule row, so the caption never reads as transcript content.
        mid.push(Line::from(""));
    }
    let sel_start = sel_start.map(|s| s + CAPTION_ROWS);
    let sel_end = sel_end.map(|e| e + CAPTION_ROWS);

    for (i, line) in body.iter().enumerate() {
        let row = i + CAPTION_ROWS;
        // Drop zone: everything from the end of the picked turn onward.
        let is_dimmed = sel_end.is_some_and(|from| row >= from);
        // Pick zone: the whole turn block, not just its prompt row.
        let outlined = sel_start.is_some_and(|s| row >= s && sel_end.is_some_and(|e| row < e));
        let mut styled = line.clone();
        if outlined {
            // REVERSED over the whole block: the operator sees exactly which
            // rows Enter keeps. Dimming only the prompt row was the other half
            // of the same defect.
            styled = Line::from(
                styled
                    .spans
                    .iter()
                    .map(|s| {
                        Span::styled(
                            s.content.to_string(),
                            s.style.add_modifier(Modifier::REVERSED),
                        )
                    })
                    .collect::<Vec<_>>(),
            );
        } else if is_dimmed {
            styled = restyle_dim(styled);
        }
        mid.push(styled);
    }
    if state.entries.is_empty() {
        mid.push(Line::from(""));
        mid.push(Line::styled(
            "  No rewindable turns yet — send a prompt first.",
            dim,
        ));
    }

    // Footer: preview stats + inline notice — PINNED.
    let mut footer: Vec<Line> = vec![Line::from("")];
    // POSITION (omp `#frame`: `${selected + 1}/${targets.length}`,
    // rewind-selector.ts:926). The cheapest honest answer to "my arrows do
    // nothing": this counter moves on every step even when the pane does not,
    // so the operator can correlate a keypress with a change. Reused by the
    // drop count below so the two cannot disagree.
    let position = if state.entries.is_empty() {
        String::new()
    } else {
        format!("  turn {}/{}", state.selected + 1, state.entries.len())
    };
    let stats = match state.preview {
        Some((msgs, tokens)) => format!(
            "  Keeps {msgs} messages · ~{tokens} tokens · drops {} turns{}",
            state.entries.len().saturating_sub(state.selected + 1),
            position,
        ),
        None => format!("  …{position}"),
    };
    footer.push(Line::from(Span::styled(stats, dim)));
    if let Some(notice) = &state.notice {
        footer.push(Line::from(Span::styled(
            format!("  {notice}"),
            Style::default().fg(theme::asap_color()),
        )));
    }

    // Header/body/footer split: only the middle scrolls.
    let header_h = header.len() as u16;
    let footer_h = footer.len() as u16;
    let chunks = Layout::vertical([
        Constraint::Length(header_h),
        Constraint::Min(0),
        Constraint::Length(footer_h),
    ])
    .split(area);

    frame.render_widget(Paragraph::new(header), chunks[0]);

    let body_h = chunks[1].height as usize;
    let max_scroll = mid.len().saturating_sub(body_h);
    let scroll = reveal_offset(state.scroll, sel_start, sel_end, body_h).min(max_scroll);
    // (RT-205) ERASE EVERY CELL OF THE BODY — this is a FULLSCREEN surface (see
    // the paint-site comment in `render/operant_overlays.rs`), so the paragraph
    // owns the whole area and must write all of it.
    //
    // ratatui's `render_line` writes ONLY the line's own graphemes
    // (`widgets/paragraph.rs`, `for StyledGrapheme { symbol, style } in
    // wrapped.graphemes`) and never pads to `area.width`, and
    // `Terminal::try_draw` diffs against the PREVIOUS buffer — so any cell a
    // line does not reach keeps whatever the last frame left there, symbols
    // included. Ratatui's own contract: "each frame is compared to the previous
    // frame... If the render function does not fully render the frame, the
    // terminal will not be in a consistent state."
    //
    // MEASURED, live, on the deployed binary, opening this selector over a
    // 4-turn transcript at 120x41: the pre-overlay frame held
    // ` operant · replay · perf:reduced` at row 13, and the overlay's row 13
    // painted `2› beta two brown lazy dog` — 27 columns — leaving cols 28..31
    // holding `educed` in the banner's own amber `rgb:CC9B1F`. At 120x45 the
    // survivor was `ck` (the tail of `/model to switch · Small Stack`) in the
    // model badge's pink `rgb:FF96C8`, so row 17 read
    // `4› delta four lazy sleeps nowck`. The leaked fragment differs by height
    // (41 -> `educed`/`ck`, 44/45 -> `ced`/`ck`, 40/42/43 -> none, because the
    // overlay's row content length relative to the stale banner cell changes)
    // which is exactly the RT-205 signature: one frame's content surviving into
    // the next under a DIFFERENT style.
    //
    // The repo's own unit instrument could not see this: it draws the overlay
    // over a FRESH empty buffer, so there is no stale cell to survive.
    //
    // Pad each line to the area's full width with the body's own background,
    // and pad the line COUNT so no row is left unpainted either. A line shorter
    // than the area is what leaks, so every line must reach the right edge.
    let body_w = chunks[1].width as usize;
    let blank = " ".repeat(body_w);
    for line in &mut mid {
        let used: usize = line.spans.iter().map(|s| s.content.chars().count()).sum();
        if used < body_w {
            line.spans
                .push(Span::styled(blank[used..].to_string(), Style::default()));
        }
    }
    // Rows beyond `mid` are likewise never reached by `render_line`, so a short
    // body would leave the bottom of the area stale the same way. The pad rows
    // must be REAL SPACES, not empty `Line::from("")`: an empty line has no
    // graphemes, so `render_line` writes no cells for it at all and the row
    // keeps whatever the previous frame left there. MEASURED: the first version
    // of this fix padded with empty lines and the leak was unchanged — rows
    // 16-26 still held frame one's banner, model line and status bar.
    if mid.len() < body_h {
        mid.resize(
            body_h,
            Line::from(Span::styled(blank.clone(), Style::default())),
        );
    }
    frame.render_widget(Paragraph::new(mid).scroll((scroll as u16, 0)), chunks[1]);

    frame.render_widget(Paragraph::new(footer), chunks[2]);
}

/// THE ANCHOR REVEAL (omp `#outlineAnchor` → `ScrollView.revealRange`).
///
/// omp's frame carries the outline's row span
/// (`selStart..selEnd`, rewind-selector.ts:989-997) and `TranscriptBrowser::render`
/// calls `revealRange` on every paint (transcript-browser.ts:232), which moves the
/// viewport only when that span leaves it and then clears follow-tail
/// (scroll-view.ts:331-345). We kept the scroll state and none of the follow:
/// `scroll` was written ONLY by PageUp/PageDown (key_handling.rs:2556-2561), so
/// stepping up through a transcript taller than the pane walked the outline above
/// the top edge. The operator saw the tail unmoved while only the footer's
/// "drops N turns" counter changed, which reads as "the arrow keys do nothing" —
/// the keys did move `selected`. This function is the missing half.
///
/// `alignment: "nearest"`: if the zone starts above the viewport, scroll to its
/// start; else if it ends below, scroll to show its end. A zone taller than the
/// viewport reveals its start, so the picked turn's prompt is always on screen.
/// Pure, so a paint with no input cannot disagree with its predecessor — no
/// oscillation, hence no flicker — and an operator who paged away keeps their
/// page (the reveal pulls the zone in once, then it sits still until the
/// selection changes).
fn reveal_offset(
    committed: usize,
    zone_start: Option<usize>,
    zone_end: Option<usize>,
    body_h: usize,
) -> usize {
    match (zone_start, zone_end) {
        // omp's FIRST arm (scroll-view.ts:334): the range starts above the
        // viewport, so reveal its start. This must be checked before anything
        // that looks at the zone's END, because a zone sitting wholly above the
        // pane has a small end that satisfies every "is it below?" test —
        // `bottom <= committed` included — and answering that first left the
        // picked turn entirely off-screen with the pane unmoved. That is the
        // exact symptom iter-1601 existed to fix, still reachable for a short
        // zone.
        (Some(top), Some(_)) if top < committed => top,
        // omp's SECOND arm (scroll-view.ts:341): the range ends below the
        // viewport, so reveal its end. The `saturating_sub` keeps a zone
        // taller than the pane from underflowing; it end-aligns, which is
        // what omp's `scrollOffsetForRow(..., "end")` does too
        // (scroll-viewport.ts:83-85: `selected - height + 1`).
        (Some(_), Some(bottom)) if bottom > committed + body_h => bottom.saturating_sub(body_h),
        // Otherwise the range is already on screen: leave the operator's scroll
        // exactly where it is, so a repaint with no input cannot move the pane
        // (no oscillation, hence no flicker) and an operator who paged away by
        // hand keeps their page.
        _ => committed,
    }
}

/// Which prepared user-prompt slot the selector's cursor points at.
///
/// `selected` is an index into the MARK list, and that list is NOT the display
/// list: it keeps one entry per turn (so a folded turn can still be refused by
/// Enter) while the display holds only the surviving prompts. A compaction that
/// folds a prompt shifts the two lists apart, and the shift is silent — a
/// positional shortcut still returns a value, just for the wrong turn.
///
/// A turn's TEXT is the identity that survives. It is the same anchor
/// [`RewindSelectorState::sync_marks`] already keys on (iter-1515): a
/// conversation produces the same prompts every frame, so text resolves across
/// a re-derived list while position does not. omp resolves this by row range
/// instead — `OutlineTarget` carries `start`/`end` (transcript-outline.ts:20-37)
/// so `targets[selected]` names its own blocks by identity — and text is the
/// closest identity this side has, because the turn_id is re-derived from a
/// process-global counter between frames.
///
/// Position remains the last resort, for a selected turn whose text somehow
/// fails to resolve (an empty mark, a prompt the display rewrote). Returning
/// `None` there would blank the outline rather than mis-aim it.
fn user_prompt_index(
    prepared: &crate::tui::operant_model::PreparedMessages,
    app: &crate::tui::app::App,
) -> Option<usize> {
    let sel = app.rewind_selector.selected;
    let sel_text = app.rewind_selector.entries.get(sel)?.text.clone();
    let by_text = prepared.user_prompt_texts.iter().position(|t| {
        let a: String = t.chars().take(64).collect();
        let b: String = sel_text.chars().take(64).collect();
        a == b
    });
    if by_text.is_some() {
        return by_text;
    }
    if sel < prepared.user_prompt_texts.len() {
        return Some(sel);
    }
    None
}

fn restyle_dim(line: Line<'static>) -> Line<'static> {
    Line::from(
        line.spans
            .iter()
            .map(|s| {
                Span::styled(
                    s.content.to_string(),
                    s.style.fg(theme::dim_color()).add_modifier(Modifier::DIM),
                )
            })
            .collect::<Vec<_>>(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (iter-1515) The arrow keys must move the SELECTION, not merely the
    /// cursor's index. The operator reported "none of its functionalities are
    /// working" against the double-Esc rewind selector, and the live repro
    /// showed the reversed highlight frozen on one row across `<down><down>`.
    ///
    /// `select_next`/`select_prev` were correct in isolation; the render was
    /// what never followed. This pins the pair together so the cursor and the
    /// highlighted turn cannot drift apart again.
    #[test]
    fn arrow_keys_move_the_selection_the_highlight_follows() {
        let mut st = RewindSelectorState::default();
        st.entries = vec![
            RewindEntry {
                turn_id: 1,
                compacted: false,
                text: "first prompt".to_string(),
            },
            RewindEntry {
                turn_id: 2,
                compacted: false,
                text: "second prompt".to_string(),
            },
            RewindEntry {
                turn_id: 3,
                compacted: false,
                text: "third prompt".to_string(),
            },
        ];
        st.selected = 0;

        st.select_next();
        assert_eq!(st.selected, 1, "down moves the cursor");
        assert_eq!(st.selected_turn_id(), Some(2), "and names the same turn");

        st.select_next();
        assert_eq!(st.selected, 2);

        st.select_prev();
        assert_eq!(st.selected, 1, "up moves back");

        st.select_prev();
        st.select_prev();
        assert_eq!(
            st.selected, 0,
            "up clamps at the oldest turn, does not wrap"
        );

        st.select_next();
        st.select_next();
        st.select_next();
        assert_eq!(
            st.selected, 2,
            "down clamps at the newest turn, does not wrap"
        );
    }

    /// A selector whose entries are re-synced (marks arrive from the agent
    /// mid-open) must keep the cursor on the SAME turn id rather than jump to
    /// the new last entry. Otherwise a live turn landing while the operator
    /// navigates silently moves their selection.
    #[test]
    fn a_resync_keeps_the_cursor_on_the_same_turn() {
        let mut st = RewindSelectorState::default();
        st.entries = vec![
            RewindEntry {
                turn_id: 11,
                compacted: false,
                text: "a".into(),
            },
            RewindEntry {
                turn_id: 12,
                compacted: false,
                text: "b".into(),
            },
            RewindEntry {
                turn_id: 13,
                compacted: false,
                text: "c".into(),
            },
        ];
        st.selected = 1; // on turn 12

        // The agent's mark list re-reads the same turns (this is what
        // sync_marks receives from the frame loop).
        let marks = vec![
            (11u64, "a".to_string()),
            (12, "b".to_string()),
            (13, "c".to_string()),
        ];
        let display_texts: Vec<String> = marks.iter().map(|(_, t)| t.clone()).collect();
        st.sync_marks(marks, &display_texts);

        assert_eq!(
            st.selected, 1,
            "the cursor stayed on turn 12, not the new last entry"
        );
        assert_eq!(st.selected_turn_id(), Some(12));
    }

    /// (iter-1515) THE LIVE DEFECT: the frame loop re-syncs the marks EVERY
    /// frame, and `OperantAgent::reseed_user_turns_for_rehydration` derives
    /// fresh `turn_id`s from a process-global counter — so the ids in the
    /// marks are never equal to the ids the selector saw last frame.
    ///
    /// `sync_marks` keyed its cursor preservation on the turn id, so the lookup
    /// failed on every single frame and the cursor was reset to the LAST
    /// entry. Measured on the deployed binary with `<up>` x4: the reversed
    /// row's text stayed "1› Capacity retry probe…" — the FIRST turn — while
    /// `rewind_selector.selected` had genuinely advanced. The operator read
    /// this as "none of its functionalities are working", because from outside
    /// the cursor really never moved.
    ///
    /// The fix: preserve the cursor by POSITION (and re-anchor by text when the
    /// list shape changed), so a re-derive cannot move the selection.
    #[test]
    fn a_resync_whose_turn_ids_were_re_derived_keeps_the_cursor_position() {
        let mut st = RewindSelectorState::default();
        let marks = vec![
            (1u64, "prompt one".to_string()),
            (2, "prompt two".to_string()),
            (3, "prompt three".to_string()),
        ];
        let texts: Vec<String> = marks.iter().map(|(_, t)| t.clone()).collect();
        st.sync_marks(marks, &texts);
        // sync_marks anchors on the newest turn, so the cursor opens at 2.
        assert_eq!(st.selected, 2, "cursor opens on the newest turn");
        st.select_prev(); // the operator moves up one, onto "prompt two"
        assert_eq!(st.selected, 1, "up moved the cursor to the middle turn");

        // The frame loop re-reads the marks: the SAME turns, but the agent had
        // re-derived its ids (a rehydration, or simply a fresh counter) AND a
        // NEW turn landed ahead of the anchor.
        //
        // The positions SHIFT: prompt two moves from index 1 to index 2. A
        // position-clamping fallback would leave the cursor on index 1 (now
        // "prompt one"), so this assertion discriminates the text anchor from
        // the clamp — without the new turn, both paths answer 1 and the test
        // proves nothing.
        let re_derived = vec![
            (77u64, "session opened".to_string()),
            (78, "prompt one".to_string()),
            (79, "prompt two".to_string()),
            (80, "prompt three".to_string()),
        ];
        st.sync_marks(re_derived, &texts);

        assert_eq!(
            st.selected, 2,
            "the cursor FOLLOWED its turn to its new position, rather than \
             clamping onto whatever now occupies the old index"
        );
        assert_eq!(
            st.entries[2].text, "prompt two",
            "and it is the same TEXT the operator navigated to"
        );
    }

    /// A turn whose text no longer appears in the displayed conversation was
    /// folded by compaction, and must be flagged so Enter can refuse it
    /// rather than forking at an index that no longer exists.
    #[test]
    fn a_turn_compacted_away_is_flagged_not_forked() {
        let mut st = RewindSelectorState::default();
        let marks = vec![
            (21u64, "kept prompt".to_string()),
            (22, "vanished prompt".to_string()),
        ];
        // The conversation now only shows the first: the second was folded.
        let display_texts = vec!["kept prompt".to_string()];
        st.sync_marks(marks, &display_texts);

        assert!(
            !st.entries[0].compacted,
            "a turn still in the transcript is rewindable"
        );
        assert!(
            st.entries[1].compacted,
            "a turn the compaction folded must be flagged, so Enter refuses it \\
             instead of forking at a cut point that no longer resolves"
        );
    }

    /// Reveal arithmetic traced line-by-line from omp, not assumed.
    ///
    /// `revealRange` (scroll-view.ts:310-341) takes
    /// `margin=0`, `alignment="nearest"`, `oversized="nearest"` (the anchor
    /// builder never sets `oversized`, so the default stands). Two arms run in
    /// order: `top < visible.start` → `setActiveRow(top, "start")`;
    /// else `bottom > visible.end` → `setActiveRow(bottom-1, "end")`.
    /// `setActiveRow` then derives the offset through `scrollOffsetForRow`
    /// (scroll-viewport.ts:62-93), where alignment `"end"` is
    /// `selected - height + 1`.
    ///
    /// For a 40-row zone at 4..44 in a 10-row viewport committed at 0: top=4
    /// is NOT below the viewport, so arm 2 wins and the offset is
    /// `43 - 10 + 1 = 34` — the turn's END rows are shown and its prompt is
    /// scrolled off. That is omp's real behaviour, oversized and all, so parity
    /// requires the same number. This test pins the arithmetic so a later
    /// "let's improve this" rewrite is caught disagreeing with the reference.
    #[test]
    fn the_reveal_offset_matches_omps_traced_arithmetic() {
        // A 40-row turn in a 10-row viewport, committed at the top.
        // omp: arm 2 (bottom > visible.end) -> active row 43 -> offset 34.
        let revealed = reveal_offset(0, Some(4), Some(44), 10);
        assert_eq!(
            revealed, 34,
            "an oversized zone end-aligns as omp's revealRange does: offset \
             43-10+1, not the zone's start — pinning parity, not intent"
        );

        // The same zone with the operator paged DOWN into it: its start is now
        // ABOVE the viewport, so omp's arm 1 fires and reveals the start.
        // Committed at 30, visible is [30,40): top=4 < 30 -> active row 4 →
        // offset 4. This is the case an "end always wins" mistake gets wrong.
        let paged_back = reveal_offset(30, Some(4), Some(44), 10);
        assert_eq!(
            paged_back, 4,
            "once the zone's start is scrolled above the viewport, omp's first \
             arm reveals it — the prompt returns to the top of the pane"
        );

        // And the ordinary short-zone case: end-aligned when below the viewport.
        let pulled = reveal_offset(0, Some(30), Some(33), 10);
        assert_eq!(
            pulled, 23,
            "a short zone below the viewport is pulled up to show its end"
        );

        // And the arm-1 pull-back: a tall zone whose start scrolled above the
        // viewport returns it to the top. Committed 6, zone 4..44, height 10:
        // omp's arm 1 fires (top=4 < visible.start=6) and so does ours.
        let start_returns = reveal_offset(6, Some(4), Some(44), 10);
        assert_eq!(
            start_returns, 4,
            "a tall zone whose start scrolled above the viewport pulls it back \
             into view — omp's first arm, and ours"
        );

        // The first arm on a SHORT zone, which is what a `drop-arm1`
        // mutation lets through: the zone sits wholly above the viewport.
        // Committed 20, visible [20,30), zone 3..6: omp's
        // `top(3) < visible.start(20)` -> start-aligned at 3. Without arm 1
        // none of the other guards fire and the offset stays at 20, leaving
        // the picked turn entirely above the pane.
        let wholly_above = reveal_offset(20, Some(3), Some(6), 10);
        assert_eq!(
            wholly_above, 3,
            "a zone wholly above the viewport is revealed at its start — the \
             arm-1 pull-back that keeps a picked turn from being scrolled off \
             the top"
        );

        // The keep fall-through: NO arm fires, so the committed offset
        // survives untouched. omp's arms are `top < visible.start` and
        // `bottom > visible.end`; with visible [0,10) and the zone at 3..7,
        // top=3 >= 0 and bottom=7 <= 10, so neither fires. Scrolling the pane
        // here would be a per-frame yank the operator never asked for.
        let untouched = reveal_offset(0, Some(3), Some(7), 10);
        assert_eq!(
            untouched, 0,
            "a zone fully inside the viewport leaves the scroll exactly where \
             the operator left it"
        );
    }
}
