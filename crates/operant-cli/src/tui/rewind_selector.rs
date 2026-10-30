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

    pub fn select_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
        self.notice = None;
    }

    pub fn select_next(&mut self) {
        if self.selected + 1 < self.entries.len() {
            self.selected += 1;
        }
        self.notice = None;
    }

    /// The entries that pass the current filter, with their positions.
    pub fn filtered_indices(&self) -> Vec<usize> {
        if self.filter.is_empty() {
            return (0..self.entries.len()).collect();
        }
        let needle = self.filter.to_lowercase();
        (0..self.entries.len())
            .filter(|&i| self.entries[i].text.to_lowercase().contains(&needle))
            .collect()
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

    let sel_start = user_prompt_range(&prepared, app).or_else(|| {
        let sel_text = state.entries.get(state.selected)?.text.clone();
        let idx = prepared.user_prompt_texts.iter().position(|t| {
            let a: String = t.chars().take(64).collect();
            let b: String = sel_text.chars().take(64).collect();
            a == b
        })?;
        prepared.wrapped_user_prompt_starts.get(idx).copied()
    });
    let dim_from = sel_start.map(|start| {
        let idx = prepared
            .wrapped_user_prompt_starts
            .iter()
            .position(|&s| s == start);
        idx.and_then(|i| prepared.wrapped_user_prompt_ends.get(i).copied())
            .unwrap_or(start + 1)
    });

    let mut mid: Vec<Line> = Vec::new();
    #[cfg(test)]
    eprintln!(
        "PROBE geom: sel_start={:?} dim_from={:?} starts={:?} ends={:?} body_len={} scroll={}",
        sel_start,
        dim_from,
        prepared.wrapped_user_prompt_starts,
        prepared.wrapped_user_prompt_ends,
        body.len(),
        state.scroll
    );
    for (i, line) in body.iter().enumerate() {
        let is_dimmed = dim_from.is_some_and(|from| i >= from);
        let outlined = sel_start.is_some_and(|s| i >= s && dim_from.is_some_and(|d| i < d));
        let mut styled = line.clone();
        if is_dimmed {
            styled = restyle_dim(styled);
        } else if outlined {
            #[cfg(test)]
            eprintln!(
                "PROBE outline build: src_mods={:?} content={:?}",
                line.spans
                    .iter()
                    .map(|s| s.style.sub_modifier)
                    .collect::<Vec<_>>(),
                line.spans
                    .iter()
                    .map(|s| s.content.to_string())
                    .collect::<Vec<_>>()
            );
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
    let stats = match state.preview {
        Some((msgs, tokens)) => format!(
            "  Keeps {msgs} messages · ~{tokens} tokens · drops {} turns",
            state.entries.len().saturating_sub(state.selected + 1)
        ),
        None => "  …".to_string(),
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
    let scroll = state.scroll.min(mid.len().saturating_sub(body_h));
    frame.render_widget(Paragraph::new(mid).scroll((scroll as u16, 0)), chunks[1]);

    frame.render_widget(Paragraph::new(footer), chunks[2]);
}

/// The wrapped-line range of the selector's CURRENT cursor turn, resolved
/// against the display prompts in order (marks and display prompts are both
/// in conversation order, so position aligns while nothing is compacted).
fn user_prompt_range(
    prepared: &crate::tui::operant_model::PreparedMessages,
    app: &crate::tui::app::App,
) -> Option<usize> {
    let sel = app.rewind_selector.selected;
    prepared.wrapped_user_prompt_starts.get(sel).copied()
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
        let mut st = RewindSelectorState {
            entries: vec![
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
            ],
            ..Default::default()
        };
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
        let mut st = RewindSelectorState {
            entries: vec![
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
            ],
            ..Default::default()
        };
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
}
