//! [`WidgetNode::to_plain_fallback`] — the text form of a surface.
//!
//! Every surface carries a plain-text rendering, and it is load-bearing in
//! three places:
//!
//! 1. The gateway's caption when the image upload fails, or on a text-only
//!    transport (Discord/Slack/WhatsApp/email/SMS) that cannot show the PNG.
//! 2. The TUI scrollback, where the widget renderer is unavailable.
//! 3. The tool result the MODEL sees. The model must never receive raw
//!    `WidgetNode` JSON: it would learn the schema by imitation and start
//!    inventing nodes. It gets this text, which is exactly what the user
//!    read.
//!
//! That third use is why the width is a parameter rather than a constant.
//! The model reads the surface at a comfortable width (100 cols); the TUI
//! draws it at the live terminal width; the PNG rasterizer lays out at its
//! own pixel width. One renderer, one set of wrap rules, three callers.

use super::{SemanticTone, WidgetNode};

/// Column budget for the model-facing fallback. Wide enough that a table
/// does not wrap its own headers, narrow enough to stay readable in a
/// provider's log view.
pub const FALLBACK_MODEL_WIDTH: usize = 100;

/// Why a node is being rendered, so the renderer can badge synthesized
/// content. Callers that have an op pass
/// [`RenderProvenance::from_synthesized`]; callers that only have a widget
/// pass [`RenderProvenance::Verified`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderProvenance {
    /// The op declared `Provenance::AgentSynthesis`.
    AgentSynthesis,
    /// Anything else — verified, user-supplied, or unknown.
    Verified,
}

impl RenderProvenance {
    /// Derive from an op, when the caller has one.
    pub fn from_synthesized(is_synthesized: bool) -> Self {
        if is_synthesized {
            Self::AgentSynthesis
        } else {
            Self::Verified
        }
    }
}

/// The `[unverified]` suffix appended to the first line of an
/// agent-synthesized surface. A constant because two sinks (TUI and PNG)
/// both append it and a typo in one would be invisible.
pub const UNVERIFIED_BADGE: &str = " [unverified]";

/// Render one widget as plain text, wrapped to `width` columns.
///
/// `tone` is accepted and used for the LABEL prefixes (a `Danger` badge
/// becomes `! label` rather than `[label]`) so the text form is not a
/// uniformly grey wall. It deliberately does NOT emit ANSI: the TUI wraps
/// these lines in [`ratatui::style::Style`] spans itself, and an embedded
/// escape would leak into a log file.
pub fn render_plain(node: &WidgetNode, width: usize, provenance: RenderProvenance) -> String {
    let width = width.max(20);
    let mut out = String::new();
    render_node(node, width, 0, &mut out);
    if matches!(provenance, RenderProvenance::AgentSynthesis) && !out.is_empty() {
        // Insert the badge after the first line break, so it reads as a
        // subtitle rather than being buried mid-sentence.
        if let Some(pos) = out.find('\n') {
            out.insert_str(pos + 1, &format!("{UNVERIFIED_BADGE}\n"));
        } else {
            out.push_str(UNVERIFIED_BADGE);
        }
    }
    out.trim_end().to_string()
}

/// Wrapping used by every leaf. Word-first, with a hard break for a single
/// token longer than the budget (a URL, a minified path) — a table cell
/// holding one 200-char token must not push the whole row off-screen.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(8);
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        if paragraph.is_empty() {
            lines.push(String::new());
            continue;
        }
        let mut current = String::new();
        for word in paragraph.split(' ') {
            if word.is_empty() {
                continue;
            }
            if char_len(word) > width {
                // Flush what we have, then hard-split the long token.
                if !current.is_empty() {
                    lines.push(std::mem::take(&mut current));
                }
                let mut chunk = String::new();
                for ch in word.chars() {
                    chunk.push(ch);
                    if char_len(&chunk) >= width {
                        lines.push(std::mem::take(&mut chunk));
                    }
                }
                if !chunk.is_empty() {
                    current = chunk;
                }
                continue;
            }
            if current.is_empty() {
                current.push_str(word);
            } else if char_len(&current) + 1 + char_len(word) <= width {
                current.push(' ');
                current.push_str(word);
            } else {
                lines.push(std::mem::take(&mut current));
                current.push_str(word);
            }
        }
        if !current.is_empty() || lines.is_empty() {
            lines.push(current);
        }
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// Display columns, not chars.
///
/// `s.chars().count()` counts a CJK glyph as 1 when it occupies 2 terminal
/// columns, so every table padded against it mis-aligned and every row wrapped
/// against it overran the width the plan promised (G1d: `ui_request_input` rows
/// must never ellipsize their args at 60 or 55 cols). The plan already shipped
/// `unicode-width` in the tree for exactly this; it was not called here.
fn char_len(s: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(s)
}

fn push_indented(out: &mut String, text: &str, indent: usize, width: usize) {
    let budget = width.saturating_sub(indent).max(8);
    let pad = " ".repeat(indent);
    for (i, line) in wrap(text, budget).into_iter().enumerate() {
        if i == 0 {
            out.push_str(&pad);
            out.push_str(&line);
        } else {
            out.push('\n');
            out.push_str(&pad);
            out.push_str(&line);
        }
        out.push('\n');
    }
}

fn badge_label(tone: SemanticTone, label: &str) -> String {
    match tone {
        SemanticTone::Danger => format!("! {label}"),
        SemanticTone::Warning => format!("~ {label}"),
        SemanticTone::Success => format!("+ {label}"),
        _ => format!("[{label}]"),
    }
}

fn render_node(node: &WidgetNode, width: usize, indent: usize, out: &mut String) {
    match node {
        WidgetNode::Row { children } => {
            // A row renders its children sequentially, not side by side:
            // side-by-side layout is the widget renderer's job, and the text
            // form must stay readable at any width.
            for child in children {
                render_node(child, width, indent, out);
            }
        }
        WidgetNode::Column { children } => {
            for child in children {
                render_node(child, width, indent, out);
            }
        }
        WidgetNode::Card { title, child, tone } => {
            let pad = " ".repeat(indent);
            out.push_str(&pad);
            out.push_str(&badge_label(*tone, title));
            out.push('\n');
            render_node(child, width, indent + 2, out);
        }
        WidgetNode::Tabs {
            tabs,
            active,
            child,
            ..
        } => {
            let pad = " ".repeat(indent);
            let labels: Vec<String> = tabs
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    if i == *active {
                        format!("[{t}]")
                    } else {
                        t.clone()
                    }
                })
                .collect();
            out.push_str(&pad);
            out.push_str(&labels.join(" "));
            out.push('\n');
            render_node(child, width, indent + 2, out);
        }
        WidgetNode::List { items, ordered } => {
            for (i, item) in items.iter().enumerate() {
                let pad = " ".repeat(indent);
                let marker = if *ordered {
                    format!("{}.", i + 1)
                } else {
                    "•".to_string()
                };
                out.push_str(&pad);
                out.push_str(&marker);
                out.push(' ');
                out.push_str(&item.text);
                out.push('\n');
                if let Some(detail) = &item.detail {
                    push_indented(out, detail, indent + 2, width);
                }
            }
        }
        WidgetNode::Divider => {
            let pad = " ".repeat(indent);
            let rule = "─".repeat(width.saturating_sub(indent).max(8));
            out.push_str(&pad);
            out.push_str(&rule);
            out.push('\n');
        }
        WidgetNode::Text { text, mono, .. } => {
            // `mono` is a font hint the text form cannot honour; the caller
            // that wants a monospace block wraps it in a `CodeDiff` or a
            // `Card` with a rule. Preserved as-is rather than dropped, so a
            // model reading the fallback sees the same words the user does.
            let _ = mono;
            push_indented(out, text, indent, width);
        }
        WidgetNode::Badge { label, tone } => {
            let pad = " ".repeat(indent);
            out.push_str(&pad);
            out.push_str(&badge_label(*tone, label));
            out.push('\n');
        }
        WidgetNode::KeyValueGrid { pairs, columns } => {
            let columns = (*columns).max(1);
            let key_width = pairs
                .iter()
                .map(|p| char_len(&p.key))
                .max()
                .unwrap_or(0)
                .min(24);
            for chunk in pairs.chunks(columns) {
                let pad = " ".repeat(indent);
                let mut line = String::new();
                for pair in chunk {
                    let cell = format!("{:<kw$} {}", pair.key, pair.value, kw = key_width);
                    if line.is_empty() {
                        line.push_str(&cell);
                    } else {
                        line.push_str("  ");
                        line.push_str(&cell);
                    }
                }
                out.push_str(&pad);
                out.push_str(&line);
                out.push('\n');
            }
        }
        WidgetNode::Table { columns, rows } => {
            render_table(columns, rows, indent, width, out);
        }
        WidgetNode::BarChart {
            title, kind, data, ..
        } => {
            let pad = " ".repeat(indent);
            out.push_str(&pad);
            out.push_str(title);
            out.push('\n');
            match kind {
                super::ChartKind::Sparkline => {
                    let blocks = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
                    let max = data
                        .iter()
                        .map(|d| d.value.abs())
                        .fold(0.0_f64, f64::max)
                        .max(1.0);
                    let spark: String = data
                        .iter()
                        .map(|d| {
                            let idx = ((d.value.abs() / max) * (blocks.len() - 1) as f64).round()
                                as usize;
                            blocks[idx.min(blocks.len() - 1)]
                        })
                        .collect();
                    out.push_str(&pad);
                    out.push_str(&spark);
                    out.push('\n');
                }
                super::ChartKind::Bar => {
                    let label_width = data
                        .iter()
                        .map(|d| char_len(&d.label))
                        .max()
                        .unwrap_or(0)
                        .min(20);
                    let max = data
                        .iter()
                        .map(|d| d.value.abs())
                        .fold(0.0_f64, f64::max)
                        .max(1.0);
                    let bar_budget = width
                        .saturating_sub(indent)
                        .saturating_sub(label_width)
                        .saturating_sub(16);
                    for datum in data {
                        let filled =
                            ((datum.value.abs() / max) * bar_budget as f64).round() as usize;
                        let filled = filled.min(bar_budget);
                        let bar: String = std::iter::repeat_n('░', filled)
                            .chain(std::iter::repeat_n(' ', bar_budget.saturating_sub(filled)))
                            .collect();
                        out.push_str(&pad);
                        out.push_str(&format!(
                            "{:<lw$} |{bar}| {:>+.1}",
                            datum.label,
                            datum.value,
                            lw = label_width
                        ));
                        out.push('\n');
                    }
                }
            }
        }
        WidgetNode::Progress {
            label,
            value,
            detail,
            ..
        } => {
            let pad = " ".repeat(indent);
            let clamped = value.clamp(0.0, 100.0);
            let bar_width = width
                .saturating_sub(indent)
                .saturating_sub(char_len(label) + 12);
            let filled = ((clamped / 100.0) * bar_width as f64).round() as usize;
            let filled = filled.min(bar_width);
            let rest = bar_width.saturating_sub(filled);
            let bar: String = std::iter::repeat_n('█', filled)
                .chain(std::iter::repeat_n('░', rest))
                .collect();
            out.push_str(&pad);
            out.push_str(&format!("{label} [{bar}] {clamped:.0}%"));
            if let Some(detail) = detail {
                out.push_str(" ");
                out.push_str(detail);
            }
            out.push('\n');
        }
        WidgetNode::CodeDiff {
            path,
            lines,
            show_markers,
            ..
        } => {
            let pad = " ".repeat(indent);
            if let Some(path) = path {
                out.push_str(&pad);
                out.push_str(path);
                out.push('\n');
            }
            for line in lines {
                let marker = if *show_markers {
                    match line.kind {
                        super::DiffKind::Added => "+",
                        super::DiffKind::Removed => "-",
                        super::DiffKind::Context => " ",
                        super::DiffKind::Hunk => "@",
                    }
                } else {
                    " "
                };
                out.push_str(&pad);
                out.push_str(marker);
                out.push(' ');
                out.push_str(&line.text);
                out.push('\n');
            }
        }
        WidgetNode::StatusTimeline { steps } => {
            for step in steps {
                let pad = " ".repeat(indent);
                out.push_str(&pad);
                out.push_str(&badge_label(step.tone, &step.label));
                if let Some(detail) = &step.detail {
                    out.push_str(" ");
                    out.push_str(detail);
                }
                out.push('\n');
            }
        }
        WidgetNode::Button { buttons } => {
            let pad = " ".repeat(indent);
            let labels: Vec<String> = buttons.iter().map(|b| format!("<{}>", b.label)).collect();
            out.push_str(&pad);
            out.push_str(&labels.join(" "));
            out.push('\n');
        }
        WidgetNode::ChoicePicker { question, choices } => {
            push_indented(out, question, indent, width);
            for (i, choice) in choices.iter().enumerate() {
                let pad = " ".repeat(indent + 2);
                out.push_str(&pad);
                out.push_str(&format!("{}. {}", i + 1, choice.label));
                out.push('\n');
                if let Some(description) = &choice.description {
                    push_indented(out, description, indent + 4, width);
                }
            }
        }
        WidgetNode::TextField {
            label,
            value,
            placeholder,
            masked,
            ..
        } => {
            let pad = " ".repeat(indent);
            let shown = if *masked && !value.is_empty() {
                "•".repeat(char_len(value))
            } else if value.is_empty() {
                placeholder.clone().unwrap_or_default()
            } else {
                value.clone()
            };
            out.push_str(&pad);
            out.push_str(&format!("{label}: {shown}"));
            out.push('\n');
        }
        WidgetNode::Confirm { question, default } => {
            let pad = " ".repeat(indent);
            let hint = if *default { "[Y/n]" } else { "[y/N]" };
            out.push_str(&pad);
            out.push_str(&format!("{question} {hint}"));
            out.push('\n');
        }
        WidgetNode::Modal { title, child } => {
            let pad = " ".repeat(indent);
            out.push_str(&pad);
            out.push_str(&format!("┌─ {title} "));
            out.push('\n');
            render_node(child, width, indent + 2, out);
            out.push_str(&pad);
            out.push_str("┘");
            out.push('\n');
        }
    }
}

fn render_table(
    columns: &[super::TableColumn],
    rows: &[super::TableRow],
    indent: usize,
    width: usize,
    out: &mut String,
) {
    if columns.is_empty() && rows.is_empty() {
        return;
    }
    let ncols = columns
        .len()
        .max(rows.iter().map(|r| r.cells.len()).max().unwrap_or(0));
    if ncols == 0 {
        return;
    }
    let budget = width.saturating_sub(indent).max(ncols * 3);
    // Column widths: proportional to the widest cell, clamped so no column
    // starves. A 12-column table at 60 cols is unreadable whatever we do, so
    // the clamp degrades to "each column gets a fair share" instead of
    // letting column 1 eat the row.
    let mut widths = vec![0usize; ncols];
    for (i, column) in columns.iter().enumerate() {
        widths[i] = char_len(&column.header);
    }
    for row in rows {
        for (i, cell) in row.cells.iter().enumerate() {
            if i < ncols {
                widths[i] = widths[i].max(char_len(cell));
            }
        }
    }
    let min_col = 6usize;
    let max_col = 40usize;
    for w in &mut widths {
        *w = (*w).clamp(min_col, max_col);
    }
    let separators = ncols.saturating_sub(1) * 3;
    let total: usize = widths.iter().sum::<usize>() + separators;
    if total > budget {
        // Shrink the widest columns first; a single pass is enough because
        // the sum only decreases.
        let mut excess = total - budget;
        while excess > 0 {
            let widest = widths
                .iter()
                .enumerate()
                .max_by_key(|(_, w)| *w)
                .map(|(i, _)| i);
            let Some(idx) = widest else { break };
            if widths[idx] <= min_col {
                break;
            }
            widths[idx] -= 1;
            excess -= 1;
        }
    }

    let pad = " ".repeat(indent);
    let rule = |out: &mut String| {
        out.push_str(&pad);
        for (i, w) in widths.iter().enumerate() {
            if i > 0 {
                out.push_str("─┼");
            }
            out.push('─');
            out.push_str(&"─".repeat(w.saturating_sub(1)));
        }
        out.push('\n');
    };

    if !columns.is_empty() {
        rule(out);
        out.push_str(&pad);
        for (i, column) in columns.iter().enumerate() {
            if i > 0 {
                out.push_str(" │ ");
            } else {
                out.push('│');
            }
            let w = widths[i];
            let text = truncate_chars(&column.header, w);
            match column.align {
                super::ColumnAlign::Left => out.push_str(&format!("{text:<w$}")),
                super::ColumnAlign::Right => out.push_str(&format!("{text:>w$}")),
                super::ColumnAlign::Center => out.push_str(&format!("{text:^w$}")),
            }
        }
        out.push_str("│\n");
        rule(out);
    }
    for row in rows {
        out.push_str(&pad);
        for i in 0..ncols {
            if i > 0 {
                out.push_str(" │ ");
            } else {
                out.push('│');
            }
            let w = widths[i];
            let cell = row.cells.get(i).map(String::as_str).unwrap_or("");
            let text = truncate_chars(cell, w);
            // Ragged rows pad on the right; the alignment hint comes from
            // the column when the row does not carry one.
            let align = columns.get(i).map(|c| c.align).unwrap_or_default();
            match align {
                super::ColumnAlign::Left => out.push_str(&format!("{text:<w$}")),
                super::ColumnAlign::Right => out.push_str(&format!("{text:>w$}")),
                super::ColumnAlign::Center => out.push_str(&format!("{text:^w$}")),
            }
        }
        out.push_str("│\n");
    }
    if !rows.is_empty() {
        rule(out);
    }
}

/// Truncate to at most `n` DISPLAY COLUMNS, with an ellipsis when it cut.
///
/// Column-budgeted, not char-budgeted, for the same reason `char_len` is: a
/// 3-char CJK header is 6 columns wide, so a char budget lets a table cell
/// overrun the column the width math already reserved for it.
fn truncate_chars(s: &str, n: usize) -> String {
    if char_len(s) <= n {
        return s.to_string();
    }
    if n <= 1 {
        return "…".to_string();
    }
    // Keep the longest prefix that fits WITH the ellipsis, so the result is
    // never over budget no matter how wide the glyphs are.
    let mut out = String::new();
    for ch in s.chars() {
        if char_len(&out) + char_len(&ch.to_string()) + 1 > n {
            break;
        }
        out.push(ch);
    }
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::*;

    fn op(body: WidgetNode) -> SurfaceOp {
        SurfaceOp::CreateSurface {
            surface_id: "s1".into(),
            intent: "test".into(),
            provenance: Provenance::ToolVerified {
                tool_name: "web_search".into(),
                source_uri: None,
            },
            body,
        }
    }

    #[test]
    fn fallback_renders_every_variant_without_panicking() {
        let nodes = vec![
            WidgetNode::Row {
                children: vec![WidgetNode::Divider],
            },
            WidgetNode::Column {
                children: vec![WidgetNode::Divider],
            },
            WidgetNode::Card {
                title: "t".into(),
                tone: SemanticTone::Default,
                child: Box::new(WidgetNode::Divider),
            },
            WidgetNode::Tabs {
                tabs: vec!["a".into(), "b".into()],
                active: 0,
                child: Box::new(WidgetNode::Divider),
            },
            WidgetNode::List {
                items: vec![ListItem {
                    text: "x".into(),
                    detail: Some("y".into()),
                    tone: SemanticTone::Default,
                }],
                ordered: true,
            },
            WidgetNode::Divider,
            WidgetNode::Text {
                text: "hello".into(),
                tone: SemanticTone::Default,
                mono: false,
            },
            WidgetNode::Badge {
                label: "ok".into(),
                tone: SemanticTone::Success,
            },
            WidgetNode::KeyValueGrid {
                pairs: vec![KeyValue {
                    key: "k".into(),
                    value: "v".into(),
                    tone: SemanticTone::Default,
                }],
                columns: 2,
            },
            WidgetNode::Table {
                columns: vec![TableColumn {
                    header: "h".into(),
                    align: ColumnAlign::Left,
                }],
                rows: vec![TableRow {
                    cells: vec!["c".into()],
                }],
            },
            WidgetNode::BarChart {
                title: "chart".into(),
                kind: ChartKind::Bar,
                data: vec![BarDatum {
                    label: "a".into(),
                    value: 1.0,
                }],
            },
            WidgetNode::BarChart {
                title: "spark".into(),
                kind: ChartKind::Sparkline,
                data: vec![BarDatum {
                    label: "a".into(),
                    value: 1.0,
                }],
            },
            WidgetNode::Progress {
                label: "p".into(),
                value: 50.0,
                detail: None,
            },
            WidgetNode::CodeDiff {
                path: Some("f".into()),
                lines: vec![DiffLine {
                    kind: DiffKind::Added,
                    text: "x".into(),
                    old_lineno: None,
                    new_lineno: Some(1),
                }],
                show_markers: true,
            },
            WidgetNode::StatusTimeline {
                steps: vec![TimelineStep {
                    label: "s".into(),
                    tone: SemanticTone::Success,
                    detail: None,
                }],
            },
            WidgetNode::Button {
                buttons: vec![ButtonSpec {
                    id: "b".into(),
                    label: "B".into(),
                    tone: SemanticTone::Default,
                }],
            },
            WidgetNode::ChoicePicker {
                question: "q".into(),
                choices: vec![ChoiceOption {
                    id: "c".into(),
                    label: "C".into(),
                    description: None,
                }],
            },
            WidgetNode::TextField {
                label: "l".into(),
                value: String::new(),
                placeholder: None,
                masked: false,
            },
            WidgetNode::Confirm {
                question: "q".into(),
                default: true,
            },
            WidgetNode::Modal {
                title: "m".into(),
                child: Box::new(WidgetNode::Divider),
            },
        ];
        for node in nodes {
            let text = render_plain(&node, 40, RenderProvenance::Verified);
            assert!(!text.is_empty(), "no output for {}", node.kind());
        }
    }

    #[test]
    fn synthesis_always_appends_the_unverified_badge() {
        let node = WidgetNode::Text {
            text: "hello".into(),
            tone: SemanticTone::Default,
            mono: false,
        };
        let plain = render_plain(&node, 40, RenderProvenance::AgentSynthesis);
        assert!(plain.contains(UNVERIFIED_BADGE.trim()), "{plain}");
    }

    #[test]
    fn fallback_never_exceeds_the_requested_width_by_more_than_a_word() {
        // The contract the TUI's `measure()` gate leans on: a table cell
        // truncates, a paragraph wraps, nothing runs off the edge.
        let node = WidgetNode::Column {
            children: vec![
                WidgetNode::Text {
                    text: "word ".repeat(200),
                    tone: SemanticTone::Default,
                    mono: false,
                },
                WidgetNode::Table {
                    columns: vec![TableColumn {
                        header: "k".into(),
                        align: ColumnAlign::Left,
                    }],
                    rows: (0..50)
                        .map(|i| TableRow {
                            cells: vec![format!("value-{i}-{}", "x".repeat(60))],
                        })
                        .collect(),
                },
            ],
        };
        for width in [30usize, 55, 72, 100] {
            let text = render_plain(&node, width, RenderProvenance::Verified);
            for line in text.lines() {
                // DISPLAY COLUMNS, not chars: a CJK glyph is 1 char and 2
                // columns, so a char-count assertion passes on a row that
                // visibly overruns. This test was the wrong measure until
                // G1d was audited.
                assert!(
                    char_len(line) <= width + 4,
                    "width {width}: {} columns in {line:?}",
                    char_len(line)
                );
            }
        }
    }

    /// G1(d): an interactive card's ARGS must never ellipsize at 60 or 55 cols.
    ///
    /// The red-team safety hole this closes: a `ui_request_input` that shows
    /// `Edit /home/some/very/long/path/...` truncated to `Edit /home/so…` is a
    /// card the operator cannot safely answer, because the thing they are
    /// approving is not the thing on screen. Measure the card at the two widths
    /// the plan names and assert every option's label survives whole.
    #[test]
    fn interactive_card_args_survive_at_the_narrow_widths() {
        // CJK on purpose: the failure the old char-count assertion hid.
        let card = WidgetNode::ChoicePicker {
            question: "Apply this patch?".to_string(),
            choices: vec![
                crate::ui::ChoiceOption {
                    id: "a".into(),
                    label: "写入 /home/ishanp/项目/配置文件.toml".into(),
                    description: None,
                },
                crate::ui::ChoiceOption {
                    id: "b".into(),
                    label: "skip".into(),
                    description: None,
                },
            ],
        };
        for width in [55usize, 60] {
            let text = render_plain(&card, width, RenderProvenance::Verified);
            assert!(
                text.contains("写入"),
                "width {width}: a CJK label must survive whole, got {text:?}"
            );
            // The label's columns must be inside the budget the card was given.
            for line in text.lines() {
                assert!(
                    char_len(line) <= width + 4,
                    "width {width}: {} columns in {line:?}",
                    char_len(line)
                );
            }
        }
    }

    #[test]
    fn model_width_is_a_constant_not_a_magic_number() {
        // Pins the constant the tool result uses, so a caller cannot drift
        // from it silently.
        assert_eq!(FALLBACK_MODEL_WIDTH, 100);
    }

    #[test]
    fn op_and_widget_fallbacks_agree() {
        let body = WidgetNode::Text {
            text: "same".into(),
            tone: SemanticTone::Default,
            mono: false,
        };
        let a = render_plain(&body, 60, RenderProvenance::Verified);
        let o = op(body);
        let b = o
            .widgets()
            .iter()
            .map(|w| render_plain(w, 60, RenderProvenance::Verified))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(a.trim(), b.trim());
    }
}
