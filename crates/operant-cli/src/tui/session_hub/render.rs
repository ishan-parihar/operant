//! session_hub rendering — jcode's picker geometry, operant's theme.
//!
//! Ported from jcode `session_picker.rs::render` (2382-2470): the vertical
//! constraint stack (search row when active, then `Min(8)` for the body) and
//! the horizontal 40/60 split — list left, preview right. jcode's crash
//! banner and onboarding band constraints had no operant sources and were
//! not ported.

use ratatui::widgets::Wrap;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
};

use super::row_style;
use super::{HubGroupMode, PaneFocus, SessionHubState};
use crate::tui::theme_colors;
use operant_core::session::listing::{RowKind, RowStatus, SessionEntryV2};

/// jcode's split: 40% list, 60% preview.
const SPLIT: (Constraint, Constraint) = (Constraint::Percentage(40), Constraint::Percentage(60));

/// How many rows of organising chrome sit above the session rows inside the
/// list block: the header row, the filter tab bar, and the column header.
/// `state.selected` indexes the body set, so every consumer that maps a
/// selection onto a rendered row adds this — see the `ListState` wiring in
/// `render_list_body`.
pub(crate) const CHROME_ROWS: usize = 3;

/// The three chrome rows as list items, so they render INSIDE the block's
/// borders (drawing them above the block produced a left pane whose top rows
/// carried no border glyph, which is both wrong to look at and wrong to every
/// border-parity analysis here).
///
/// The column header comes last so it sits directly above the column it names.
/// Codex's `agent_center` renders exactly this — a dim
/// `Tasks | Status | Updated` line so the eye knows what the right-hand columns
/// mean — and without it a row's status is "the dim bit at the end", which is
/// not a column, it is a guess.
fn chrome_items(state: &SessionHubState, inner_width: u16) -> Vec<ListItem<'static>> {
    let mut items = vec![
        ListItem::new(render_header(state)),
        ListItem::new(render_tab_bar(state, inner_width)),
    ];
    // Right-align the two columns whose values are right-aligned in the rows,
    // so the header sits above its own data rather than floating at the left.
    // The column header labels sit at the SAME position the row's values do:
    // immediately after the identifiers, one two-space gap each. Padding them
    // to fixed column widths is what pushed a row's `$cost` off the 48-column
    // pane (measured 2026-11-02), and a header that costs the operator data
    // below it is a net loss.
    items.push(ListItem::new(Line::from(vec![
        Span::styled(
            " Tasks",
            Style::default()
                .fg(theme_colors::muted())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "  Status",
            Style::default()
                .fg(theme_colors::muted())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "  Updated",
            Style::default()
                .fg(theme_colors::muted())
                .add_modifier(Modifier::BOLD),
        ),
    ])));
    items
}

/// Insert one heading row before each group's first row.
///
/// `rows` is the body items in FLAT `visible_idx` order, so `rows[k]` is the
/// row at `visible_idx[k]` and the mapping is positional. Grouping reorders the
/// READING, not the items, so the heading positions come from the groups'
/// first-appearance offsets in that same flat list.
///
/// `Auto` returns the list untouched: the default must not add chrome, and a
/// heading that repeats the whole list's contents is noise.
fn interpose_group_headings(
    state: &SessionHubState,
    rows: Vec<ListItem<'static>>,
    inner_width: u16,
) -> Vec<ListItem<'static>> {
    if state.group_mode == HubGroupMode::Auto || rows.is_empty() {
        return rows;
    }
    let groups = state.visible_groups();
    // `visible_groups` yields ROW indices (into `state.rows`), while the items
    // here are positioned in `visible_idx`. Translating between the two is
    // mandatory, not defensive: under a filter the two sets differ, and a
    // heading interposed at the wrong position is worse than no grouping.
    let mut opens: Vec<(usize, String, usize)> = groups
        .iter()
        .filter_map(|(key, idxs)| {
            let first = *idxs.first()?;
            // The flat position of `first`, i.e. where it sits in the built
            // items — which is exactly where its heading must be inserted.
            let pos = state.visible_idx.iter().position(|&i| i == first)?;
            Some((pos, key.clone(), idxs.len()))
        })
        .collect();
    opens.sort_by_key(|(pos, _, _)| *pos);
    let mut out: Vec<ListItem<'static>> = Vec::with_capacity(rows.len() + opens.len());
    let mut next_heading = 0usize;
    for (flat_pos, item) in rows.into_iter().enumerate() {
        while next_heading < opens.len() && opens[next_heading].0 == flat_pos {
            let (_, key, count) = &opens[next_heading];
            out.push(group_heading_row(key, *count, inner_width));
            next_heading += 1;
        }
        out.push(item);
    }
    out
}

/// How many group-heading rows the renderer draws ABOVE the body row at flat
/// index `selected`.
///
/// The interposer draws one heading at each group's first row's FLAT position,
/// immediately ABOVE that row (interpose_group_headings' `opens`), so the
/// headings above the row at flat index `selected` are exactly the groups
/// whose first-row flat position is `<= selected`. The containing group's own
/// heading always qualifies — its first row sits at or above the selected row
/// — which is what the old take_while-over-non-containing-groups missed: an
/// off-by-one in every non-Auto mode, and wrong outright for interleaved
/// groups, whose iteration order diverged from the drawn positions. With
/// apply_group_order keeping sections contiguous, drawn layout and arithmetic
/// coincide by construction; the position lookup is what keeps the count
/// correct even for a hand-built non-contiguous state.
///
/// This exists because `state.selected` indexes the flat `visible_idx` while
/// the renderer draws a flattened list of headings + rows. Subtracting the
/// headings from the rendered position is what keeps the highlight on the row
/// the operator is on — the Stage A offset class, one level deeper.
pub(super) fn group_heading_rows_above(state: &SessionHubState, selected: usize) -> usize {
    if state.group_mode == HubGroupMode::Auto {
        return 0;
    }
    // `Auto` renders one flat group with no heading, so it contributes zero;
    // every other mode renders one heading per group.
    state
        .visible_groups()
        .iter()
        .filter_map(|(_, idxs)| {
            let first = *idxs.first()?;
            state.visible_idx.iter().position(|&i| i == first)
        })
        .filter(|pos| *pos <= selected)
        .count()
}

/// One group heading row: the group's key and its row count, dimmed. Codex's
/// `render_group_header` renders the same line — the count is the part that
/// carries information (how many sessions live in this directory), so a bare
/// heading would be a divider with no content.
fn group_heading_row(key: &str, count: usize, inner_width: u16) -> ListItem<'static> {
    let truncated: String = key
        .chars()
        .take((inner_width as usize).saturating_sub(16))
        .collect();
    let ellipsis = if key.chars().count() > truncated.chars().count() {
        "…"
    } else {
        ""
    };
    let mut spans = vec![Span::styled(
        format!(" {truncated}{ellipsis}"),
        Style::default()
            .fg(theme_colors::muted())
            .add_modifier(Modifier::BOLD),
    )];
    // The count rides at the row's right-hand edge, where the Updated column's
    // values sit, so it reads as "how many rows are under this heading" rather
    // than as stray text at the end of the line.
    let label = format!("{count} ");
    let pad = 15usize.saturating_sub(label.chars().count());
    spans.push(Span::styled(" ".repeat(pad), Style::default()));
    spans.push(Span::styled(
        label,
        Style::default().fg(theme_colors::muted()),
    ));
    ListItem::new(Line::from(spans))
}

/// The compact age form the TUI already uses (app/mod.rs's session-list
/// drain) — moved here so the hub owns one formatter.
pub fn relative_time(updated_at: chrono::DateTime<chrono::Utc>) -> String {
    let age = chrono::Utc::now().signed_duration_since(updated_at);
    if age.num_minutes() < 1 {
        "just now".to_string()
    } else if age.num_hours() < 1 {
        format!("{}m ago", age.num_minutes())
    } else if age.num_hours() < 24 {
        format!("{}h ago", age.num_hours())
    } else {
        format!("{}d ago", age.num_days())
    }
}

fn status_glyph(status: RowStatus) -> &'static str {
    match status {
        RowStatus::Live => "⚡",
        // The interrupted marker: a session the previous process left mid-run.
        // Distinct from `Idle` because the operator's whole question at a
        // session hub is "which of these was cut off".
        RowStatus::Interrupted => "⚠",
        RowStatus::Idle => "·",
        RowStatus::Archived => "📦",
    }
}

fn row_title(row: &SessionEntryV2) -> String {
    match &row.kind {
        RowKind::Subagent { goal, .. } => {
            let short: String = goal.chars().take(48).collect();
            format!("🤖 {short}")
        }
        _ => {
            let base = row
                .title
                .clone()
                .unwrap_or_else(|| "(untitled)".to_string());
            if matches!(row.kind, RowKind::Branch) {
                format!("↳ {base}")
            } else {
                base
            }
        }
    }
}

/// Compact token count for the row's meta line: a 14,103-message session's
/// token total is seven digits, which at a 40%-width list would push the
/// session id off the row entirely. `k`/`M` keeps the identifier visible.
///
/// (2026-11-02, measured) The unit went too: with the Status and Updated
/// columns also on the line, " · 1.2k tok" cost 11 cells and pushed the row's
/// `$cost` off the 46-column inner pane entirely. Dropping the repeated " tok"
/// (the column header says what the number is) fits the row at 48-wide panes
/// again, and a token count nobody reads to three digits.
fn tokens_suffix(tokens: u64) -> String {
    const K: f64 = 1_000.0;
    const M: f64 = 1_000_000.0;
    let t = tokens as f64;
    if tokens == 0 {
        String::new()
    } else if t >= M {
        format!(" \u{2022} {:.1}M", t / M)
    } else if t >= K {
        format!(" \u{2022} {:.1}k", t / K)
    } else {
        format!(" \u{2022} {tokens}")
    }
}

/// Per-char search highlight, jcode's `highlight_spans` semantics: tokens
/// matched independently (OR), overlapping/adjacent merged.
fn highlight_spans(text: &str, tokens: &[String], base: Style, hl: Style) -> Vec<Span<'static>> {
    if tokens.is_empty() || text.is_empty() {
        return vec![Span::styled(text.to_string(), base)];
    }
    let lower = text.to_lowercase();
    let mut mask = vec![false; lower.len()];
    for t in tokens {
        let mut from = 0;
        while let Some(at) = lower[from..].find(t.as_str()) {
            for cell in mask.iter_mut().skip(from + at).take(t.len()) {
                *cell = true;
            }
            from += at + t.chars().count();
        }
    }
    let mut spans = Vec::new();
    let mut cur_hl = mask.first().copied().unwrap_or(false);
    let mut buf = String::new();
    for (ch, &m) in text.chars().zip(mask.iter()) {
        if m != cur_hl {
            spans.push(Span::styled(
                std::mem::take(&mut buf),
                if cur_hl { hl } else { base },
            ));
            cur_hl = m;
        }
        buf.push(ch);
    }
    if !buf.is_empty() {
        spans.push(Span::styled(buf, if cur_hl { hl } else { base }));
    }
    spans
}

/// The preview renders one line per message, so a multi-paragraph message
/// needs its first non-empty line (the operator is scanning shape, not reading
/// the body — that is what Enter-switch is for). Blank bodies collapse to a
/// single placeholder so a tool-only message still produces a row.
fn first_line(content: &str) -> String {
    let line = content
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("(no text)");
    line.chars().take(200).collect()
}

/// Render the hub full-frame. Callers gate on `state.visible` (the overlay
/// guard table), so this renders INSTEAD of the chat frame — never both
/// (the one-renderer rule).
pub fn render_session_hub(
    frame: &mut Frame,
    state: &SessionHubState,
    area: ratatui::layout::Rect,
    ctx: &mut crate::tui::render::FrameCtx,
) {
    let has_search = state.search_active || !state.search_query.is_empty();
    let mut v = Vec::new();
    if has_search {
        v.push(Constraint::Length(1));
    }
    v.push(Constraint::Min(8));
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(v)
        .split(area);
    let mut idx = 0;
    if has_search {
        let search_area = chunks[idx];
        idx += 1;
        let cursor = if state.search_active { "▎" } else { "" };
        let line = Line::from(vec![
            Span::styled(" 🔍 ", Style::default().fg(theme_colors::accent())),
            Span::styled(
                state.search_query.clone(),
                Style::default()
                    .fg(theme_colors::text())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                cursor.to_string(),
                Style::default().fg(theme_colors::accent()),
            ),
            Span::styled(
                if state.search_active {
                    "  Esc to clear"
                } else {
                    "  / to edit"
                },
                Style::default().fg(theme_colors::muted()),
            ),
        ]);
        frame.render_widget(Paragraph::new(line), search_area);
    }
    let body = chunks[idx];
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([SPLIT.0, SPLIT.1])
        .split(body);

    render_list(frame, state, cols[0]);
    render_preview(frame, state, cols[1], ctx);
}

fn render_list(frame: &mut Frame, state: &SessionHubState, area: ratatui::layout::Rect) {
    // Stage A: the header + live-count tab bar are the organising chrome the
    // operator's left pane lacked. They render INSIDE the bordered block, above
    // the rows — not above the block. Measured on the drawn buffer at 120x40,
    // drawing them outside produced a left pane whose top rows carry no border
    // glyph at all, which both looks broken (a label floating two rows above
    // its box) and breaks every border-parity analysis in this tree that finds
    // the pane split by counting the second `│` per row.
    //
    // The chrome replaces the box title `Sessions [all]`, which could name the
    // view but could not carry the counts codex's `render_filled_tab_bar` does:
    // with 3,266 rows in the local DB, "which of the six filters even has
    // anything in it" was unanswerable without cycling all of them.
    render_list_body(frame, state, area);
}

/// The header row. Codex's equivalent is `Agent command center  Group: Project
/// g`: one line that answers "where am I" AND "what changes the organisation".
/// Stage C adds the `Group:` clause; the span vector is shaped for it now so
/// adding it will not restructure this function.
fn render_header(state: &SessionHubState) -> Line<'static> {
    let mut spans = vec![
        Span::styled(
            " Sessions",
            Style::default()
                .fg(theme_colors::accent())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  {} rows", state.rows.len()),
            Style::default().fg(theme_colors::muted()),
        ),
    ];
    // The search row sits above this list, so say what the list is showing
    // rather than letting the operator wonder whether the counts include the
    // search. The counts deliberately do NOT: see `filter_counts`.
    if !state.search_query.is_empty() {
        spans.push(Span::styled(
            format!("  matching \"{}\"", state.search_query),
            Style::default().fg(theme_colors::text()),
        ));
    }
    // Without the key hints the tab bar and the group headings read as
    // decoration.
    spans.push(Span::styled(
        "  s cycle",
        Style::default().fg(theme_colors::muted()),
    ));
    // The `Group:` clause, codex's own header line shape. Stage C fills in the
    // mode and the key that changes it: the header alone now answers both
    // "which axis is this list ordered by" and "which key reorders it".
    spans.push(Span::styled(
        format!(
            "  Group: {}  {}",
            state.group_mode.label(),
            state.group_mode.key()
        ),
        Style::default().fg(theme_colors::text()),
    ));
    Line::from(spans)
}

/// Every filter mode with its live count, in the order `s` cycles them. The
/// active tab carries a real background fill and the rest are dimmed; that
/// contrast alone is the whole interaction, because it lets the operator aim
/// at a filter before committing to it.
fn render_tab_bar(state: &SessionHubState, width: u16) -> Line<'static> {
    let counts = state.filter_counts();
    // The sort clause (Stage E). It rides the END of this row because this row
    // is already the controls row, and the header has no room: the header
    // measures 45 of the pane's 46 inner columns before this clause exists, so
    // appending it there would cost the operator the row count or the `s`/`g`
    // affordance. The Stage B finding ("a header that costs the operator data
    // below it is a net loss") is measured again here rather than re-learned.
    //
    // Being last also makes it the FIRST thing the ladder below drops, which is
    // the right priority: the filter tabs are the operator's aim, the sort is a
    // refinement of what they already aimed at.
    let sort_clause = |long: bool| -> String {
        if long {
            format!(
                "  Sort: {}  {}",
                state.sort_mode.label(),
                state.sort_mode.key()
            )
        } else {
            format!(" {}", state.sort_mode.label())
        }
    };
    let sort_width = |long: bool| -> u16 { sort_clause(long).chars().count() as u16 };

    // The tab bar's degradation ladder. Codex's `toolbar_for_width` does the
    // same: full labels while they fit, compact ones when they do not, and the
    // single active filter when even that does not. Measured on the 120x40
    // frame, the left pane is 48 columns, which fits SIX compact labels
    // ("all 12 cur 3 emp 0 live 0 br 5 arch 2" = 36) but not SIX long ones
    // ("current dir", "this branch's tree" and "by employee" pushed `archived`
    // clean off the edge and the count the operator needed was the silent
    // casualty). codex degrades, so we do too.
    let total_width = |long: bool, sep: u16, with_sort: bool| -> u16 {
        let tabs = counts
            .iter()
            .map(|(m, n)| {
                let label = if long { m.label() } else { m.short_label() };
                (label.chars().count() + 1 + n.to_string().len()) as u16 + sep
            })
            .sum::<u16>();
        if with_sort {
            tabs + sort_width(long)
        } else {
            tabs
        }
    };
    const SEP: u16 = 2;

    // The ladder's rungs, richest first. The sort clause is dropped between the
    // long and compact rungs: long labels + sort overflow, compact labels +
    // sort fit, and the filter tabs alone are the last resort.
    let (use_long, sep, show_sort) = if total_width(true, SEP, true) <= width {
        (true, SEP, true)
    } else if total_width(false, SEP, true) <= width {
        (false, SEP, true)
    } else if total_width(false, 1, true) <= width {
        // The measured rung that matters most: at the 48-column pane (120x40,
        // the common case) the six compact labels plus a two-column gap leave
        // three columns, and the sort clause needs eight — but the SAME labels
        // with a one-column gap fit exactly. Dropping the sort here instead
        // would hide the key on the exact width most operators work at, which
        // is the Stage B finding one rung earlier.
        (false, 1, true)
    } else if total_width(false, 1, false) <= width {
        (false, 1, false)
    } else {
        // Degrade to the active filter alone; a count nobody can read is
        // decoration, and the header above still names the filter.
        let active = counts
            .iter()
            .find(|(m, _)| *m == state.filter_mode)
            .map_or_else(|| "0".to_string(), |(_, n)| n.to_string());
        return Line::from(vec![Span::styled(
            format!(" {} {}", state.filter_mode.label(), active),
            Style::default()
                .bg(theme_colors::selection_bg())
                .fg(theme_colors::accent())
                .add_modifier(Modifier::BOLD),
        )]);
    };

    let mut spans: Vec<Span<'static>> = Vec::with_capacity(counts.len() * 3 + 1);
    spans.push(Span::styled(
        " ",
        Style::default().fg(theme_colors::muted()),
    ));
    for (i, (mode, n)) in counts.iter().enumerate() {
        // A mode with nothing in it still renders: "active 0" is the answer to
        // "is anything running", which is the operator's question at a session
        // hub, and hiding the tab would hide the zero that answers it.
        let label = if use_long {
            mode.label().to_string()
        } else {
            mode.short_label().to_string()
        };
        let text = format!("{label} {n}");
        let style = if *mode == state.filter_mode {
            Style::default()
                .bg(theme_colors::selection_bg())
                .fg(theme_colors::accent())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme_colors::muted())
        };
        spans.push(Span::styled(text, style));
        if i + 1 < counts.len() {
            spans.push(Span::styled(" ".repeat(sep as usize), Style::default()));
        }
    }
    if show_sort {
        // `text`, not `accent`: the sort is a refinement of what the operator
        // already aimed at, and `selection_bg` here would make it compete with
        // the ACTIVE filter tab for the eye. The long clause names its own key
        // (`o`) because a control whose affordance is invisible is decoration.
        spans.push(Span::styled(
            sort_clause(use_long),
            Style::default().fg(theme_colors::text()),
        ));
    }
    Line::from(spans)
}

/// The bordered list itself — the old `render_list` body, unchanged. Split out
/// so the chrome above it is an additive layer rather than an edit to the row
/// rendering every session row depends on.
fn render_list_body(frame: &mut Frame, state: &SessionHubState, area: ratatui::layout::Rect) {
    // The block is built here, not further down, because Stage D needs the
    // inner width to paint each row's fill to the right border. Deriving it in
    // two places is how the fill ends up one cell short.
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme_colors::border()));
    let inner_width = block.inner(area).width;
    let body_items: Vec<ListItem> = if state.loading {
        vec![
            ListItem::new(Line::from(vec![
                Span::styled("  ⏳ ", Style::default().fg(theme_colors::accent())),
                Span::styled(
                    "Loading sessions…",
                    Style::default()
                        .fg(theme_colors::text())
                        .add_modifier(Modifier::BOLD),
                ),
            ])),
            ListItem::new(Line::from(Span::styled(
                "     scanning local sessions and live subagents…",
                Style::default().fg(theme_colors::muted()),
            ))),
        ]
    } else if state.visible_rows().is_empty() {
        vec![
            ListItem::new(Line::from(Span::styled(
                "  No sessions match",
                Style::default()
                    .fg(theme_colors::text())
                    .add_modifier(Modifier::BOLD),
            ))),
            ListItem::new(Line::from(Span::styled(
                "     s cycles filters · Esc closes",
                Style::default().fg(theme_colors::muted()),
            ))),
        ]
    } else {
        let tokens = if state.search_query.is_empty() {
            Vec::new()
        } else {
            SessionHubState::search_tokens()(&state.search_query)
        };
        state
            .visible_rows()
            .iter()
            .enumerate()
            .map(|(i, row)| {
                let selected = i == state.selected;
                // Stage E: an armed row swaps its status glyph for the archive
                // marker. The status returns when the confirm is disarmed, so
                // the operator never has to hold two states in their head —
                // and the row that WILL be archived is the row that says so,
                // not just the notice line at the bottom of the frame.
                let armed = state.pending_archive_pos() == Some(i);
                let mut spans = vec![Span::styled(
                    if armed {
                        " \u{1f5d1} ".to_string()
                    } else {
                        format!(" {} ", status_glyph(row.status))
                    },
                    Style::default().fg(if armed || matches!(row.status, RowStatus::Live) {
                        theme_colors::warning()
                    } else {
                        theme_colors::muted()
                    }),
                )];
                // (2026-10-30) Subagent rows already carry their badge in
                // `row_title` ("🤖 <goal>", the branch prefix is "↳ " there
                // for the same reason). A second badge span here rendered
                // "⚡ 🤖 🤖 do research" — proven on the drawn buffer, not
                // inferred from the source. What the row was actually missing
                // is COLOR, so give the title the accent color a subagent row
                // never had and leave the single glyph where it belongs.
                let subagent_row = matches!(row.kind, RowKind::Subagent { .. });
                let title_style = match (selected, subagent_row) {
                    (true, _) => Style::default()
                        .fg(theme_colors::accent())
                        .add_modifier(Modifier::BOLD),
                    (false, true) => Style::default()
                        .fg(theme_colors::accent())
                        .add_modifier(Modifier::BOLD),
                    (false, false) => Style::default().fg(theme_colors::text()),
                };
                spans.extend(highlight_spans(
                    &row_title(row),
                    &tokens,
                    title_style,
                    Style::default()
                        .fg(theme_colors::warning())
                        .add_modifier(Modifier::BOLD),
                ));
                // (2026-10-29) The operator could not see the session's ID,
                // employee, message count or tokens: the meta was ONE appended
                // span, and the list is 40% of the frame, so a 100+ column
                // overflow truncated it to "age · count · $". jcode's picker
                // renders a title row plus a dedicated subtitle row
                // (`session_picker.rs`'s ListItem of two lines); the second
                // line is what carries the identifiers.
                //
                // ID and employee are DIFFERENT things and both are shown:
                // the id is the handle for /resume and `sessions show`, the
                // employee is who is executing (and it is the branch/anchor
                // key when the operator filters by employee).
                // An employee session's id IS the employee id
                // (`emp-cron_fa1e44c`), so print the pair once: showing both
                // doubled a row and pushed the metrics off a 40%-width pane.
                let emp = row
                    .employee_id
                    .as_deref()
                    .filter(|e| *e != row.id)
                    .map(|e| format!("  🪪 {e}"))
                    .unwrap_or_default();
                // 12 chars fits `#emp-cron_fa1e4` and `#sess_87b7abb9-`
                // inside a 40%-width pane alongside the metrics; the operator
                // resolves the full id from /resume's completion or
                // `sessions list --json` when they need the whole string.
                //
                // (2026-11-02, measured) 12 chars is what the SIMPLE row fits.
                // A row carrying all of employee + 120 messages + 90k tokens +
                // a $12 cost measured 63 cells against the pane's 46 inner
                // columns, and it was truncated mid-word: the operator lost the
                // age AND the cost, which are the point of the row. The id is
                // the one field here that is fully recoverable — the preview
                // pane prints it in full and `/resume` completes it — so it is
                // the field that yields. Everything else on the line is a fact
                // the operator cannot get back without opening the session.
                //
                // The budget is this render's own inner width (already computed
                // above as `inner_width`), not a second constant to keep in sync
                // with the pane geometry.
                //
                // MEASURED, and the finding that matters: the metrics of a fully
                // loaded row are themselves 55 cells against the pane's 46 inner
                // columns — employee + 120 msg + 90k tokens + Idle + just now +
                // $12.3400 — so no amount of id-shrinking makes it fit. Squeezing
                // the id to 3 chars would have produced a row with an unreadable
                // handle and STILL no age. So the row degrades the way the Stage
                // B rule demands: drop the LEAST load-bearing field, in a fixed
                // order, until the rest fits.
                //
                // The order is chosen by recoverability, not by preference:
                //   1. STATUS goes first — it has its own named column in the
                //      header above (Stage B), so it is recoverable by looking up.
                //   2. The ID is next — the preview pane prints it in full.
                //   Age, messages, tokens and cost are NEVER dropped: they are
                //   the reason the operator opened the list.
                let metrics_width = emp.chars().count()
                    + format!("{} msg", row.message_count).chars().count()
                    + tokens_suffix(row.tokens).chars().count()
                    + row.status.label().chars().count()
                    + relative_time(row.updated_at).chars().count()
                    + if matches!(row.kind, RowKind::Subagent { .. }) {
                        0
                    } else {
                        format!("${:.4}", row.cost_usd).chars().count()
                    }
                    + 8;
                // Drop the status column BEFORE shrinking the id: the header
                // above already names it (Stage B), so it is the one field here
                // the row can afford to lose. Shrinking the id first would burn
                // the handle to save a word that is already one row up.
                let show_status = metrics_width as usize + 13 <= inner_width as usize;
                let metrics_width = if show_status {
                    metrics_width
                } else {
                    metrics_width.saturating_sub(row.status.label().chars().count())
                };
                let mut id_budget = 12usize;
                // Then shrink the id while it still reads as a handle.
                while metrics_width as usize + id_budget + 1 > inner_width as usize && id_budget > 3
                {
                    id_budget -= 1;
                }

                let short_id: String = row.id.chars().take(id_budget).collect();
                // (2026-10-30) A subagent row printed "$0.0000 · 0 msg · 0.000k
                // tok" and asserted a fact it did not have: `records_snapshot`
                // in async_delegation.rs hardcodes message_count 0, cost 0.0
                // and tokens 0, because the delegation registry holds no usage
                // counters. omp's roster rules state "missing data appears as
                // `usage —` rather than an estimate" (agent-hub.md:36) and that
                // is the honest shape here — a dash says "not measured", a
                // zero says "dispatched and consumed nothing", which for a
                // `Pending` delegation is simply false.
                //
                // The row's real gap was the delegation STATUS: a Completed and
                // a Failed subagent looked identical unless focused. Both live
                // on `RowKind::Subagent.status`, and the Status column below is
                // where they are shown now.
                //
                // Stage B moved the delegation status out of this inline spot
                // and into the Status column, where a header row names it:
                // printing it in both places would render `pending  pending`.
                let usage_span = match &row.kind {
                    RowKind::Subagent { .. } => {
                        Span::styled("  usage —", Style::default().fg(theme_colors::muted()))
                    }
                    _ => {
                        let token_span = tokens_suffix(row.tokens);
                        Span::styled(
                            format!("  {} msg{token_span}", row.message_count),
                            Style::default().fg(theme_colors::muted()),
                        )
                    }
                };
                let mut meta_spans = vec![
                    // ID first: it is the row's handle, and the one thing an
                    // operator is never otherwise shown on screen.
                    Span::styled(
                        format!("#{short_id}"),
                        Style::default().fg(theme_colors::muted()),
                    ),
                    Span::styled(emp, Style::default().fg(theme_colors::muted())),
                    usage_span,
                ];
                // Stage B also fills a real gap: a session row's status was
                // carried ONLY by the title line's colour-coded glyph, so an
                // Idle and an Interrupted row differed by a hue and nothing
                // more. The column now carries the word for both session rows
                // and delegation rows, from one value so the two cannot drift.
                let status_value: (&str, Color) = match &row.kind {
                    RowKind::Subagent { status, .. } => (
                        status,
                        match *status {
                            "pending" => theme_colors::success(),
                            "failed" => theme_colors::error(),
                            _ => theme_colors::muted(),
                        },
                    ),
                    _ => (
                        row.status.label(),
                        match row.status {
                            RowStatus::Live => theme_colors::success(),
                            // Interrupted is the state the operator is hunting
                            // for at this pane, so it warns rather than dims.
                            RowStatus::Interrupted => theme_colors::warning(),
                            RowStatus::Archived | RowStatus::Idle => theme_colors::muted(),
                        },
                    ),
                };
                if show_status {
                    meta_spans.push(Span::styled(
                        format!("  {}", status_value.0),
                        Style::default().fg(status_value.1),
                    ));
                }
                meta_spans.push(Span::styled(
                    format!("  {}", relative_time(row.updated_at)),
                    Style::default().fg(theme_colors::muted()),
                ));
                if !matches!(row.kind, RowKind::Subagent { .. }) {
                    meta_spans.push(Span::styled(
                        format!("  ${:.4}", row.cost_usd),
                        Style::default().fg(theme_colors::muted()),
                    ));
                }
                let meta_line = Line::from(meta_spans);
                // ONE ListItem holding BOTH lines, not two items: the
                // selection index must stay 1:1 with `visible_idx`, and a
                // second item per session would drift Up/Down onto the meta
                // row. ratatui's List supports a multi-line item body.
                //
                // Stage D: the row's own background, painted to the full inner
                // width so the highlight reaches the right border instead of
                // stopping where the text does. Applied here, before the
                // heading pass below, so a group heading keeps the block's own
                // background and the tint marks a session rather than a section.
                let mut lines = vec![Line::from(spans), meta_line];
                let fill = if selected {
                    row_style::selected_fill_style()
                } else {
                    row_style::zebra_fill_style(i)
                };
                if let Some(fill) = fill {
                    for line in &mut lines {
                        *line = row_style::apply_row_background(
                            std::mem::take(line),
                            fill,
                            inner_width,
                        );
                    }
                }
                ListItem::new(lines)
            })
            .collect()
    };

    // Stage C interposes one heading before each group's first row. The rows
    // above are built in the FLAT `visible_idx` order (so `i == state.selected`
    // still works unchanged), so grouping is a presentation pass over the
    // finished list rather than a change to how a row is built. That keeps the
    // row-building closure readable and stops grouping from spreading into
    // every per-row branch.
    //
    // The block is built first because the heading's count column needs the
    // inner width too.
    let body_items: Vec<ListItem> = interpose_group_headings(state, body_items, inner_width);
    let items: Vec<ListItem> = chrome_items(state, inner_width)
        .into_iter()
        .chain(body_items)
        .collect();
    let mut list_state = ListState::default();
    if !state.loading && !state.visible_rows().is_empty() {
        // `state.selected` indexes `visible_idx`, which is the BODY set. The
        // highlight must land on the row's RENDERED position, which is the
        // chrome rows plus every group heading drawn above it. Counting them
        // here (rather than assuming `CHROME_ROWS + selected`) is what keeps
        // the highlight on the operator's row once Stage C inserts one heading
        // per group: with three groups and five rows, row 4 renders at
        // position 3 + 3 + 4, not 3 + 4.
        list_state.select(Some(
            CHROME_ROWS + group_heading_rows_above(state, state.selected) + state.selected,
        ));
    }
    frame.render_stateful_widget(
        List::new(items)
            .block(block)
            // Stage D: the row's own fill IS the highlight now (painted above,
            // across the full inner width). A `highlight_style` here would
            // patch the row's background with a single flat colour, undoing the
            // tint and the zebra on exactly the row the operator is on. The
            // List still needs a selected state to position the viewport, so
            // the highlight is an empty Style rather than no highlight at all.
            .highlight_style(Style::default()),
        area,
        &mut list_state,
    );
}

fn render_preview(
    frame: &mut Frame,
    state: &SessionHubState,
    area: ratatui::layout::Rect,
    ctx: &mut crate::tui::render::FrameCtx,
) {
    let focused = state.preview_focused();
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(if focused {
            theme_colors::accent()
        } else {
            theme_colors::border()
        }))
        .title(Span::styled(
            // jcode marks the focused pane in its title
            // (session_picker.rs:1430) so a keyboard-only operator can see
            // where Tab put them.
            if focused {
                format!(
                    " Preview [focused — Tab to {}] ",
                    PaneFocus::Sessions.label()
                )
            } else {
                " Preview ".to_string()
            },
            Style::default()
                .fg(if focused {
                    theme_colors::warning()
                } else {
                    theme_colors::accent()
                })
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    let inner_height = inner.height;
    frame.render_widget(block, area);

    let Some(row) = state.selected_row() else {
        // The rows have not loaded yet. Painting an empty box here is the
        // "right-pane glitch" the operator reported: the left pane honestly
        // shows its loading banner, the right pane shows NOTHING — not the
        // banner, not a spinner, not the session id — so the overlay reads as
        // a pane that is broken for the handful of frames between opening the
        // hub and the row load landing. Naming the state costs one line and
        // makes the two panes legible together.
        let placeholder = Paragraph::new(Line::from(Span::styled(
            if state.loading {
                "Loading sessions…"
            } else {
                "No sessions to show."
            },
            Style::default().fg(theme_colors::muted()),
        )))
        .wrap(Wrap { trim: false });
        frame.render_widget(placeholder, inner);
        return;
    };

    let lines: Vec<Line> = match (&row.kind, state.focused_subagent.as_deref()) {
        // iter-C focus view: the subagent row's own detail replaces the
        // message preview (omp's chat view — goal/status/result inline).
        (
            RowKind::Subagent {
                goal,
                model,
                dispatch_time,
                status,
            },
            Some(focused),
        ) if focused == row.id => {
            let dispatched = chrono::DateTime::from_timestamp(*dispatch_time as i64, 0)
                .map(|dt| dt.with_timezone(&chrono::Utc).to_rfc3339())
                .unwrap_or_else(|| "unknown".to_string());
            vec![
                Line::from(Span::styled(
                    format!("🤖 subagent {} [{status}]", row.id),
                    Style::default()
                        .fg(theme_colors::accent())
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "goal: ",
                    Style::default().fg(theme_colors::muted()),
                )),
                Line::from(Span::styled(
                    goal.clone(),
                    Style::default().fg(theme_colors::text()),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    format!("model: {model}"),
                    Style::default().fg(theme_colors::muted()),
                )),
                Line::from(Span::styled(
                    format!("dispatched: {dispatched}"),
                    Style::default().fg(theme_colors::muted()),
                )),
                Line::from(Span::styled(
                    "←← / Esc returns to the list",
                    Style::default().fg(theme_colors::muted()),
                )),
            ]
        }
        _ => {
            let mut lines: Vec<Line> = vec![
                Line::from(Span::styled(
                    row_title(row),
                    Style::default()
                        .fg(theme_colors::text())
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    format!(
                        "id: {} · {} · ${:.4} · {} msg",
                        row.id,
                        relative_time(row.updated_at),
                        row.cost_usd,
                        row.message_count
                    ),
                    Style::default().fg(theme_colors::muted()),
                )),
                Line::from(Span::styled(
                    match &row.employee_id {
                        Some(e) => format!("employee: {e}"),
                        None => "employee: — (personal)".to_string(),
                    },
                    Style::default().fg(theme_colors::muted()),
                )),
            ];
            // The transcript itself (jcode's preview body). The load is
            // asynchronous, so the pane shows an explicit state rather than
            // silently looking empty — the two states that read as "the
            // preview is broken" are "still loading" and "no content", and
            // naming both is what makes a slow load legible.
            match &state.preview {
                Some((id, msgs, hidden)) if id == &row.id && !msgs.is_empty() => {
                    if *hidden > 0 {
                        lines.push(Line::from(Span::styled(
                            format!("— {hidden} earlier prompt(s) not shown (tail window) —"),
                            Style::default().fg(theme_colors::muted()),
                        )));
                    }
                    lines.push(Line::from(""));
                    for (i, msg) in msgs.iter().enumerate() {
                        let (label, color) = match msg.role.as_str() {
                            "user" => ("you", theme_colors::accent()),
                            "assistant" => ("agent", theme_colors::success()),
                            "system" => ("system", theme_colors::muted()),
                            other => (other, theme_colors::muted()),
                        };
                        let mut spans = vec![Span::styled(
                            format!(
                                "{}. {label}: ",
                                // Global prompt numbering: the hidden prompts
                                // above offset the window's own index, so the
                                // operator can match a preview row to the
                                // prompt number they typed (jcode's
                                // `hidden_prompts` reasoning).
                                if msg.role == "user" {
                                    *hidden
                                        + msgs[..i].iter().filter(|m| m.role == "user").count()
                                        + 1
                                } else {
                                    i + 1
                                }
                            ),
                            Style::default().fg(color).add_modifier(Modifier::BOLD),
                        )];
                        for call in &msg.tool_calls {
                            spans.push(Span::styled(
                                format!("[{call}] "),
                                Style::default().fg(theme_colors::warning()),
                            ));
                        }
                        spans.push(Span::raw(first_line(&msg.content)));
                        lines.push(Line::from(spans));
                    }
                }
                Some((id, _, _)) if id == &row.id => {
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled(
                        "No stored messages for this session.",
                        Style::default().fg(theme_colors::muted()),
                    )));
                }
                _ => {
                    // jcode's third state: no preview cached yet and none in
                    // flight. That is a MISS, not a load — writing "loading"
                    // here is exactly the operator's stuck "Loading session…"
                    // card. Name what the operator can do about it.
                    lines.push(Line::from(""));
                    lines.push(Line::from(Span::styled(
                        if state.preview_loading {
                            "⏳ Loading transcript…"
                        } else if state.has_preview_content() {
                            "Preview available."
                        } else {
                            "No transcript cached — press r to load."
                        },
                        Style::default().fg(theme_colors::muted()),
                    )));
                }
            }
            lines.push(Line::from(Span::styled(
                "Enter switches to this session",
                Style::default().fg(theme_colors::accent()),
            )));
            // The scroll bound must count WRAPPED rows, not source lines: the
            // paragraph wraps below, so a 200-char message occupies three
            // visual rows while `lines.len()` still sees one. Measuring the
            // unwrapped count leaves the operator unable to reach the last
            // message and makes the pane look like it scrolls "past" content
            // (the right-pane glitch the operator reported).
            //
            // Only the FIRST span of each line decides the wrap (a row is one
            // grammatical unit here: `N. label: text`), so measuring the
            // joined spans is exact for every row this pane builds.
            let width = inner.width.max(1) as usize;
            let visual_rows: usize = lines
                .iter()
                .map(|line| {
                    let text: String = line
                        .spans
                        .iter()
                        .map(|s| s.content.as_ref())
                        .collect::<Vec<_>>()
                        .join("");
                    if text.is_empty() {
                        1
                    } else {
                        crate::tui::render::utils::balanced_wrap(&text, width)
                            .len()
                            .max(1)
                    }
                })
                .sum();
            ctx.preview_max_scroll = Some((visual_rows as u16).saturating_sub(inner_height));
            lines
        }
    };

    let paragraph = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((state.preview_scroll, 0));
    frame.render_widget(paragraph, inner);
}
