// skills_view.rs — Skills browser overlay.
//
// Mirrors hermes-agent/ui-tui/src/components/skillsHub.tsx (309 LOC, 3-stage
// category→skill→actions). The operant TUI listed `/skills` in the help
// command list but never intercepted it — it fell through to a basic command
// registry handler that just printed a help line. This file gives /skills a
// real overlay: list of installed skills, scrollable, with name / category /
// version / description, and an Inspect action that opens the SKILL.md body.
//
// Data source: operant_core::skills::SkillManager::load_all() — the same
// primitive cmd_skills.rs uses for `operant skills list`.

use operant_core::skills::Skill;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use std::cell::Cell;
use std::path::PathBuf;

use crate::tui::overlays::{HINT_ESC, ModalSpec, cycle_next, cycle_prev, modal_frame};
use crate::tui::theme_colors;
use crate::tui::vendor::style::theme;

/// What view stage the overlay is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SkillsStage {
    /// List of all skills (the default landing view).
    #[default]
    List,
    /// Detail view for a single skill (SKILL.md body + metadata).
    Detail,
}

#[derive(Debug, Clone, Default)]
pub struct SkillsViewState {
    pub visible: bool,
    pub stage: SkillsStage,
    /// All loaded skills, sorted by category then name.
    pub skills: Vec<Skill>,
    /// Cursor index in the list view.
    pub selected: usize,
    /// Vertical scroll offset for the list view (lines from top).
    pub scroll: usize,
    /// Vertical scroll offset for the detail view.
    pub detail_scroll: usize,
    /// Last error from load_all (shown inline if non-empty).
    pub last_error: String,
    /// Last known viewport height (set by render_list_stage, read by
    /// scroll_down so the key handler knows how many lines fit). Without
    /// this, scroll_down hardcoded viewport=24 which overshot on short
    /// terminals and undershot on tall ones. (Bug #16 from iter-82 audit.)
    pub last_viewport_height: Cell<usize>,
}

impl SkillsViewState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn open(&mut self, skills_dir: PathBuf) {
        self.visible = true;
        self.stage = SkillsStage::List;
        self.selected = 0;
        self.scroll = 0;
        self.detail_scroll = 0;
        self.last_error.clear();
        self.skills.clear();

        // Load synchronously — the skills dir is a local filesystem read and
        // SkillManager::load_all is fast (one readdir + one read per skill).
        // Errors (e.g. dir doesn't exist yet on a fresh install) are surfaced
        // inline rather than crashing the overlay.
        let mut mgr = operant_core::skills::SkillManager::new(skills_dir);
        match mgr.load_all() {
            Ok(mut loaded) => {
                // Sort by (category, name) so related skills cluster visually.
                loaded.sort_by(|a, b| {
                    a.category
                        .cmp(&b.category)
                        .then_with(|| a.name.cmp(&b.name))
                });
                self.skills = loaded;
            }
            Err(e) => {
                self.last_error = format!("Failed to load skills: {}", e);
            }
        }
    }

    pub fn close(&mut self) {
        self.visible = false;
    }

    pub fn select_prev(&mut self) {
        cycle_prev(&mut self.selected, self.skills.len());
        // Snap scroll up if the cursor went above the viewport.
        if self.selected < self.scroll {
            self.scroll = self.selected;
        }
    }

    pub fn select_next(&mut self) {
        cycle_next(&mut self.selected, self.skills.len());
        // Snap scroll down if the cursor went below the viewport.
        // The viewport height isn't known here, so we just bump by 1 —
        // render() clamps to the actual visible area.
        if self.selected > self.scroll + 12 {
            self.scroll = self.selected.saturating_sub(12);
        }
    }

    pub fn scroll_down(&mut self, viewport: usize) {
        match self.stage {
            SkillsStage::List => {
                let max = self.skills.len().saturating_sub(viewport);
                if self.scroll < max {
                    self.scroll += 1;
                }
            }
            SkillsStage::Detail => {
                self.detail_scroll = self.detail_scroll.saturating_add(1);
            }
        }
    }

    pub fn scroll_up(&mut self) {
        match self.stage {
            SkillsStage::List => {
                self.scroll = self.scroll.saturating_sub(1);
            }
            SkillsStage::Detail => {
                self.detail_scroll = self.detail_scroll.saturating_sub(1);
            }
        }
    }

    pub fn open_detail(&mut self) {
        if self.skills.is_empty() {
            return;
        }
        self.stage = SkillsStage::Detail;
        self.detail_scroll = 0;
    }

    pub fn back_to_list(&mut self) {
        self.stage = SkillsStage::List;
    }

    pub fn current_skill(&self) -> Option<&Skill> {
        self.skills.get(self.selected)
    }
}

/// Render the skills overlay. Called from render.rs after all other overlays
/// so it sits on top of the transcript.
pub fn render_skills_view(frame: &mut Frame, state: &SkillsViewState, area: Rect) {
    if !state.visible {
        return;
    }

    // Natural desired size is 80×24; `modal_frame` clamps it against the
    // terminal and guarantees the readable floors this panel previously had
    // none of (the old `w.min(area.width - 4)` could collapse to 0 wide).
    let layout = modal_frame(
        frame,
        area,
        &ModalSpec {
            title: "Skills",
            hint: HINT_ESC,
            width: 80,
            height: 24,
            header_height: 1,
            footer_height: 0,
            ..Default::default()
        },
    );
    let inner = layout.body_area;

    if !state.last_error.is_empty() {
        let lines = vec![
            Line::from(Span::styled(
                "Could not load skills:",
                Style::default()
                    .fg(theme_colors::error())
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                state.last_error.clone(),
                Style::default().fg(theme_colors::warning()),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Press Esc to close.",
                Style::default().fg(theme::dim_color()),
            )),
        ];
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
        return;
    }

    if state.skills.is_empty() {
        let lines = vec![
            Line::from(Span::styled(
                "No skills installed.",
                Style::default()
                    .fg(theme_colors::warning())
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from("Skills are markdown files (SKILL.md) living in subdirectories"),
            Line::from("of your operant skills directory. Install one with:"),
            Line::from(""),
            Line::from(Span::styled(
                "  operant skills install <path-or-url>",
                Style::default().fg(theme_colors::accent()),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Press Esc to close.",
                Style::default().fg(theme::dim_color()),
            )),
        ];
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
        return;
    }

    match state.stage {
        SkillsStage::List => render_list_stage(frame, state, inner),
        SkillsStage::Detail => render_detail_stage(frame, state, inner),
    }
}

/// Cells the row spends before the first data column: the selection glyph, the
/// `#` index, and their separators. Measured from the row spans below, not
/// assumed — `format!(" {} ", prefix)` + `format!("{{:<3}}  ", idx)` is 3 + 5.
const ROW_PREFIX_W: u16 = 8;

/// Column widths for the list table at `avail` cells: `(name, category,
/// version)`, where `0` means the column is dropped.
///
/// The natural widths are an UPPER bound and each column has a floor, per
/// jcode's split discipline: a column is given its natural width when it fits,
/// and when a floor cannot be met the column is **dropped whole** rather than
/// crushing the remaining ones to slivers. Dropping is what makes the row fit — a
/// crushed cell wraps, and a wrapped row pushes its neighbours down onto the
/// border (the 40x16 failure: a 56-cell row in a 34-cell body).
///
/// The cost of a present column set is `sum(widths) + (count - 1)`: the row
/// separates columns *from each other*, so only the gaps between them cost a
/// cell. That is what the original `format!` chain did, and it is why the
/// thresholds below are `+2` and `+1` rather than one per column — charging a
/// trailing separator to the last column is what made a 47-cell body produce a
/// 48-cell row.
fn column_widths(avail: u16) -> (u16, u16, u16) {
    const NAME: u16 = 24;
    const CATEGORY: u16 = 14;
    const VERSION: u16 = 8;
    let free = avail.saturating_sub(ROW_PREFIX_W);
    if free >= NAME + CATEGORY + VERSION + 2 {
        (NAME, CATEGORY, VERSION)
    } else if free >= NAME + CATEGORY + 1 {
        (NAME, CATEGORY, 0)
    } else if free >= NAME {
        (NAME, 0, 0)
    } else {
        // Below the natural name width the name column takes everything the
        // prefix leaves. It is the only column, so it costs no separator.
        (free, 0, 0)
    }
}

fn render_list_stage(frame: &mut Frame, state: &SkillsViewState, area: Rect) {
    let mut lines: Vec<Line> = Vec::new();
    let avail = area.width;
    let (name_w, cat_w, ver_w) = column_widths(avail);

    // Header row, laid out from the same widths as the body so the two can never
    // disagree about where a column ends. The selection glyph and its space are
    // what keep the body's `› ` prefix aligned with this leading space.
    let mut header = format!(" {:<3}  ", "#");
    header.push_str(&format!("{:<name_w$}", "Name", name_w = name_w as usize));
    if cat_w > 0 {
        header.push_str(&format!(" {:<cat_w$}", "Category", cat_w = cat_w as usize));
    }
    if ver_w > 0 {
        header.push_str(&format!(" {:<ver_w$}", "Version", ver_w = ver_w as usize));
    }
    header.push(' ');
    lines.push(Line::from(vec![Span::styled(
        header,
        Style::default()
            .fg(theme::dim_color())
            .add_modifier(Modifier::BOLD),
    )]));
    lines.push(Line::from(Span::styled(
        " ".repeat(avail as usize),
        Style::default().fg(theme::dim_color()),
    )));

    let viewport = area.height.saturating_sub(6) as usize; // header + footer
    // Record the viewport height so the key handler's scroll_down knows how
    // many lines fit. (Bug #16 fix.)
    state.last_viewport_height.set(viewport);
    let start = state
        .scroll
        .min(state.skills.len().saturating_sub(viewport));
    let end = (start + viewport).min(state.skills.len());

    for display_idx in start..end {
        let skill = &state.skills[display_idx];
        let is_selected = display_idx == state.selected;
        let prefix = if is_selected { "›" } else { " " };
        let row_style = if is_selected {
            // `Color::Black`/`Color::Cyan` were the palette's *named stand-ins*
            // for the `user_bg` and `accent` roles, so this is value-preserving
            // and follows the theme instead of being rewritten by name.
            Style::default()
                .fg(theme::user_bg())
                .bg(theme::accent_color())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme_colors::text())
        };
        let mut row = Line::from(vec![
            Span::styled(format!(" {} ", prefix), row_style),
            Span::styled(format!("{:<3}  ", display_idx + 1), row_style),
            Span::styled(
                format!(
                    "{:<name_w$}",
                    truncate(skill.name.as_str(), name_w as usize),
                    name_w = name_w as usize
                ),
                row_style,
            ),
        ]);
        // Each further column is introduced by a one-cell separator and carries
        // no trailing one. That is the original layout — `"{:<24} "`, `"{:<14} "`,
        // `"{:<8}"` — re-expressed, and it is what keeps the assembled row the
        // width `column_widths` budgeted for.
        if cat_w > 0 {
            row.spans.push(Span::styled(
                format!(
                    " {:<cat_w$}",
                    truncate(skill.category.as_str(), cat_w as usize),
                    cat_w = cat_w as usize
                ),
                row_style,
            ));
        }
        if ver_w > 0 {
            row.spans.push(Span::styled(
                format!(
                    " {:<ver_w$}",
                    truncate(skill.version.as_str(), ver_w as usize),
                    ver_w = ver_w as usize
                ),
                row_style,
            ));
        }
        lines.push(row);
    }

    // Footer with the description of the highlighted skill + keybindings.
    let pad_lines = viewport.saturating_sub(state.skills.len() - start);
    for _ in 0..pad_lines {
        lines.push(Line::from(""));
    }

    if let Some(skill) = state.current_skill() {
        lines.push(Line::from(Span::styled(
            // `saturating_sub` because a degenerate body can be 0 wide, and the
            // hint below is chrome: at any width it truncates, never wraps.
            truncate_to_width(&skill.description, avail.saturating_sub(2) as usize),
            Style::default().fg(theme_colors::warning()),
        )));
    } else {
        lines.push(Line::from(""));
    }

    lines.push(Line::from(Span::styled(
        truncate_to_width(
            " ↑/↓ navigate · Enter inspect · Esc close ",
            avail.saturating_sub(1) as usize,
        ),
        Style::default().fg(theme::dim_color()),
    )));

    // No `wrap`: a table row is chrome, and chrome that wraps consumes a second
    // terminal row and lands the next row on the border. `Paragraph` without a
    // wrap clips at the area, which is the truncation this table wants.
    frame.render_widget(Paragraph::new(lines), area);
}

fn render_detail_stage(frame: &mut Frame, state: &SkillsViewState, area: Rect) {
    let Some(skill) = state.current_skill() else {
        return;
    };

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(vec![
        Span::styled("Name:        ", Style::default().fg(theme::dim_color())),
        Span::styled(
            skill.name.clone(),
            Style::default()
                .fg(theme_colors::text())
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("Category:    ", Style::default().fg(theme::dim_color())),
        Span::styled(
            skill.category.clone(),
            Style::default().fg(theme_colors::accent()),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("Version:     ", Style::default().fg(theme::dim_color())),
        Span::styled(
            skill.version.clone(),
            Style::default().fg(theme_colors::warning()),
        ),
    ]));
    if !skill.tags.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("Tags:        ", Style::default().fg(theme::dim_color())),
            Span::styled(
                skill.tags.join(", "),
                Style::default().fg(theme_colors::text()),
            ),
        ]));
    }
    if !skill.platforms.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("Platforms:   ", Style::default().fg(theme::dim_color())),
            Span::styled(
                skill.platforms.join(", "),
                Style::default().fg(theme_colors::text()),
            ),
        ]));
    }
    if !skill.prerequisites_env.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("Env vars:    ", Style::default().fg(theme::dim_color())),
            Span::styled(
                skill.prerequisites_env.join(", "),
                Style::default().fg(theme_colors::warning()),
            ),
        ]));
    }
    if !skill.prerequisites_commands.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("Commands:    ", Style::default().fg(theme::dim_color())),
            Span::styled(
                skill.prerequisites_commands.join(", "),
                Style::default().fg(theme_colors::warning()),
            ),
        ]));
    }
    lines.push(Line::from(""));

    // Body — render the SKILL.md content. We don't run the full markdown
    // renderer here because the detail view already has its own padding /
    // scroll discipline; a plain monospace dump with a thin separator reads
    // better in a fixed-height modal.
    lines.push(Line::from(Span::styled(
        "─".repeat(area.width.saturating_sub(2) as usize),
        Style::default().fg(theme::dim_color()),
    )));
    lines.push(Line::from(""));

    let body_lines: Vec<&str> = skill.content.lines().collect();
    let viewport = area.height.saturating_sub(lines.len() as u16 + 2) as usize;
    let start = state
        .detail_scroll
        .min(body_lines.len().saturating_sub(viewport));
    let end = (start + viewport).min(body_lines.len());
    for line in &body_lines[start..end] {
        lines.push(Line::from(Span::styled(
            line.to_string(),
            Style::default().fg(theme_colors::text()),
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        " ↑/↓ scroll · Backspace back to list · Esc close ",
        Style::default().fg(theme::dim_color()),
    )));

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

/// Truncate `s` to `max` display cells, appending `…` if truncated.
fn truncate(s: &str, max: usize) -> String {
    use crate::tui::render::{display_width, take_width};
    if display_width(s) <= max {
        s.to_string()
    } else if max == 0 {
        String::new()
    } else {
        let mut out = take_width(s, max - 1);
        out.push('…');
        out
    }
}

/// Word-wrap-aware truncation for the description footer line. Hard-truncates
/// at `max` display columns (no wrapping), appending `…` if cut.
fn truncate_to_width(s: &str, max: usize) -> String {
    use crate::tui::render::{display_width, take_width};
    if max == 0 {
        return String::new();
    }
    let mut out = take_width(s, max);
    if display_width(&out) < display_width(s) {
        out.push('…');
    }
    out
}
// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;

    fn skill(name: &str, category: &str, version: &str) -> Skill {
        Skill {
            name: name.to_string(),
            description: format!("{name} does a thing, and has a description long enough to wrap"),
            version: version.to_string(),
            content: String::new(),
            platforms: Vec::new(),
            tags: Vec::new(),
            category: category.to_string(),
            prerequisites_env: Vec::new(),
            prerequisites_commands: Vec::new(),
            references: std::collections::HashMap::new(),
        }
    }

    fn state() -> SkillsViewState {
        let mut s = SkillsViewState::new();
        s.visible = true;
        s.skills = vec![
            skill("codebase-inspection", "software-development", "1.0.0"),
            skill("demo-code-review", "demo", "0.1.0"),
            skill("demo-testing", "demo", "0.1.0"),
        ];
        s
    }

    /// Paint the list stage into a `w`x`h` buffer and return it.
    fn painted(w: u16, h: u16) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("test backend");
        let st = state();
        terminal
            .draw(|f| {
                render_skills_view(f, &st, Rect::new(0, 0, w, h));
            })
            .expect("draw");
        terminal.backend().buffer().clone()
    }

    /// The rendered row `y` as plain text.
    fn row_text(buf: &Buffer, y: u16) -> String {
        (0..buf.area.width)
            .map(|x| buf[(x, y)].symbol())
            .collect::<String>()
    }

    /// The cells one table row spends, derived from `column_widths` the same way
    /// `render_list_stage` assembles it: the selection/index prefix, then each
    /// present column introduced by a one-cell separator.
    fn row_width(avail: u16) -> u16 {
        let (name, cat, ver) = column_widths(avail);
        let mut w = ROW_PREFIX_W;
        if name > 0 {
            w += name;
        }
        if cat > 0 {
            w += 1 + cat;
        }
        if ver > 0 {
            w += 1 + ver;
        }
        w
    }

    /// Locate the dialog's own rect from the rounded corners it painted, rather
    /// than recomputing `modal_layout`'s arithmetic here. A test that duplicates
    /// the layout formula tests the formula; this tests what was painted.
    fn dialog_rect(buf: &Buffer) -> Option<(u16, u16, u16, u16)> {
        let sym = |x: u16, y: u16| buf[(x, y)].symbol();
        let mut tl = None;
        'outer: for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                if matches!(sym(x, y), "╭" | "┌") {
                    tl = Some((x, y));
                    break 'outer;
                }
            }
        }
        let (x0, y0) = tl?;
        let mut br = None;
        'outer2: for y in (0..buf.area.height).rev() {
            for x in (0..buf.area.width).rev() {
                if matches!(sym(x, y), "╯" | "┘") {
                    br = Some((x, y));
                    break 'outer2;
                }
            }
        }
        let (xr, yr) = br?;
        Some((x0, y0, xr + 1, yr + 1))
    }

    /// The invariant that makes the table fit: whatever `column_widths` returns,
    /// the assembled row is never wider than the body it has to live in. This is
    /// the property whose absence produced all three symptoms — a too-wide row
    /// wraps, a wrapped row eats a second terminal row, and the row after it
    /// lands on the border.
    ///
    /// Below `ROW_PREFIX_W` even the selection/index prefix cannot fit, and no
    /// column choice can rescue that; `Paragraph` clips instead, which is why
    /// the bound below is the wider of the two.
    #[test]
    fn every_row_fits_the_body_it_was_given() {
        for avail in 0..=120u16 {
            assert!(
                row_width(avail) <= avail.max(ROW_PREFIX_W),
                "at avail={avail} the row is {} cells wide",
                row_width(avail),
            );
        }
    }

    /// jcode's rule, stated as a test: the configured widths are an UPPER bound,
    /// and a column that cannot meet its floor is dropped whole rather than
    /// crushing the others.
    #[test]
    fn columns_are_an_upper_bound_and_are_dropped_rather_than_crushed() {
        // Wide enough for all three at their natural widths. This is the 120x40
        // case, and pinning the tuple pins byte-compatibility with it.
        assert_eq!(column_widths(78), (24, 14, 8));
        // One cell short of the third column: it goes, the other two keep natural.
        assert_eq!(column_widths(55), (24, 14, 0));
        // Too narrow for two: the second goes, the first keeps natural.
        assert_eq!(column_widths(46), (24, 0, 0));
        // Narrower still: the name itself yields, and is still a real column —
        // `>= 1`, never a zero-width cell pretending to be one.
        assert_eq!(column_widths(20), (12, 0, 0));
        assert!(column_widths(9).0 >= 1);
    }

    /// A table row is chrome. At every width the keybind hint — the last line of
    /// the stage — must still be on screen, which it cannot be if the rows above
    /// it wrapped.
    ///
    /// Asserted on `↑/↓`, the first glyph of the hint, because past ~20 cells the
    /// hint is truncated to a prefix and the word "navigate" is legitimately gone.
    /// Truncation is the intended behaviour here; losing the row entirely is not.
    #[test]
    fn the_keybind_hint_survives_at_every_width() {
        for w in [12u16, 16, 20, 24, 30, 40, 60, 80, 120] {
            let buf = painted(w, 24);
            let text: String = (0..buf.area.height).map(|y| row_text(&buf, y)).collect();
            assert!(
                text.contains('↑'),
                "at {w}x24 the keybind hint was pushed off screen by wrapping:\n{text}",
            );
        }
    }

    /// The header is chrome too, and it is built from the same `column_widths`
    /// call as the body. It must therefore fit the same box, and it must name
    /// exactly the columns the body has.
    #[test]
    fn the_header_fits_and_names_the_columns_the_body_has() {
        for w in [12u16, 16, 20, 24, 30, 40, 60, 80, 120] {
            let buf = painted(w, 24);
            let (x0, y0, x1, _y1) = dialog_rect(&buf).unwrap_or_else(|| panic!("at {w} no dialog"));
            // border row, title row, then the table's own header row.
            let header_y = y0 + 2;
            let header = row_text(&buf, header_y);
            // The inner area only: both border columns are the dialog's, so a
            // `│` in here is a row of content that ate its border.
            let inside: String = header
                .chars()
                .skip((x0 + 1) as usize)
                .take((x1 - x0).saturating_sub(2) as usize)
                .collect();

            assert!(
                !inside.contains('│'),
                "at {w} the header overran the dialog's inner area:\n{header}",
            );
            let (name, cat, ver) = column_widths((x1 - x0).saturating_sub(2));
            if name >= 7 {
                assert!(
                    inside.contains("Name"),
                    "at {w} the header lost its name column:\n{header}",
                );
            }
            if cat >= 8 {
                assert!(
                    inside.contains("Category"),
                    "at {w} the header lost a category column it has room for:\n{header}",
                );
            }
            if ver >= 7 {
                assert!(
                    inside.contains("Version"),
                    "at {w} the header lost a version column it has room for:\n{header}",
                );
            }
        }
    }

    /// Nothing may be written outside the dialog's own rect, and no row of the
    /// table may eat a border. Together these are the third symptom: a wrapped
    /// row landing on the border row.
    #[test]
    fn nothing_is_written_outside_the_dialog_rect() {
        for (w, h) in [
            (40u16, 16u16),
            (30, 12),
            (24, 10),
            (20, 8),
            (60, 20),
            (80, 24),
            (120, 40),
        ] {
            let buf = painted(w, h);
            let (x0, y0, x1, y1) =
                dialog_rect(&buf).unwrap_or_else(|| panic!("at {w}x{h} no dialog was painted"));

            for y in y0..y1 {
                let row = row_text(&buf, y);
                let left = row.chars().nth(x0 as usize).unwrap_or(' ');
                let right = row.chars().nth((x1 - 1) as usize).unwrap_or(' ');
                assert!(
                    matches!(left, '│' | '╭' | '╰' | '┌' | '└'),
                    "at {w}x{h} the dialog's left border at row {y} is {left:?}, not a border",
                );
                assert!(
                    matches!(right, '│' | '╮' | '╯' | '┐' | '┘'),
                    "at {w}x{h} the dialog's right border at row {y} is {right:?}, not a border",
                );
            }
        }
    }
}
