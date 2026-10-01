//! Markdown -> ratatui lines renderer used by transcript message families.

use crate::tui::figures;
use crate::tui::theme_colors;
use crate::tui::vendor::style::theme;
use pulldown_cmark::{
    Alignment as MdAlignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd,
};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use regex::Regex;
use std::collections::BTreeMap;
use std::sync::LazyLock;
use syntect::easy::HighlightLines;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Syntect syntax set — ships with the default grammar bundle (covers ~120
/// languages including rust, python, js/ts, go, c/c++, java, ruby, sql, yaml,
/// json, toml, markdown, bash, html, css, etc.). Loaded once per process.
static SYNTAX_SET: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);

/// Syntect theme set — `base16-ocean.dark` matches the diff viewer's choice so
/// inline code blocks and diff hunks read as one design system.
static THEME_SET: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);

#[expect(clippy::expect_used, reason = "infallible once-init / static init")]
/// Regex pattern to detect URLs (http://, https://, ftp://, www.)
static URL_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:https?|ftp)://\S+|www\.\S+").expect("Invalid URL regex pattern")
});

#[expect(clippy::expect_used, reason = "infallible once-init / static init")]
/// Regex pattern to detect email addresses
static EMAIL_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}")
        .expect("Invalid email regex pattern")
});

/// Map a markdown code-fence language tag (e.g. `rust`, `py`, `ts`) to a
/// syntect syntax reference. Returns None for unknown languages or empty tags
/// — the caller falls back to plain white-on-yellow rendering in that case.
fn resolve_syntax(lang: &str) -> Option<&'static syntect::parsing::SyntaxReference> {
    let ss = &*SYNTAX_SET;
    if lang.is_empty() {
        return None;
    }
    // Try the language tag directly (syntect uses lowercase names like "rust", "python").
    if let Some(s) = ss.find_syntax_by_token(lang) {
        return Some(s);
    }
    // Try common file extensions for the language.
    let ext_candidates: &[&str] = match lang.to_lowercase().as_str() {
        "rs" | "rust" => &["rs"],
        "py" | "python" | "python3" => &["py"],
        "js" | "javascript" | "node" => &["js"],
        "ts" | "typescript" => &["ts"],
        "tsx" | "jsx" => &["tsx", "jsx"],
        "go" | "golang" => &["go"],
        "c" => &["c"],
        "cpp" | "c++" | "cxx" => &["cpp"],
        "h" | "hpp" => &["h", "hpp"],
        "java" => &["java"],
        "kt" | "kotlin" => &["kt"],
        "rb" | "ruby" => &["rb"],
        "swift" => &["swift"],
        "sh" | "bash" | "shell" | "zsh" => &["sh"],
        "sql" => &["sql"],
        "yaml" | "yml" => &["yaml", "yml"],
        "json" => &["json"],
        "toml" => &["toml"],
        "html" | "htm" => &["html"],
        "css" | "scss" | "sass" => &["css", "scss"],
        "xml" => &["xml"],
        "md" | "markdown" => &["md"],
        "dockerfile" => &["Dockerfile"],
        "php" => &["php"],
        "scala" => &["scala"],
        "lua" => &["lua"],
        "perl" | "pl" => &["pl"],
        "r" => &["r"],
        "haskell" | "hs" => &["hs"],
        "elixir" | "ex" | "exs" => &["ex", "exs"],
        "erlang" | "erl" => &["erl"],
        "clojure" | "clj" => &["clj"],
        "dart" => &["dart"],
        "groovy" => &["groovy"],
        "proto" | "protobuf" => &["proto"],
        "graphql" | "gql" => &["graphql"],
        "nim" => &["nim"],
        "ocaml" | "ml" => &["ml"],
        _ => &[],
    };
    for ext in ext_candidates {
        if let Some(s) = ss.find_syntax_by_extension(ext) {
            return Some(s);
        }
    }
    // Last resort: syntect keeps an alias table — try by name.
    ss.find_syntax_by_name(lang)
}

#[expect(
    clippy::expect_used,
    reason = "invariant guaranteed by surrounding validation"
)]
/// Highlight a single line of source code in the given language, returning
/// ratatui Spans. Falls back to a single plain-white span on the yellow code
/// base style if the language is unknown or syntect fails.
///
/// Mirrors `diff_viewer::highlight_code_line` but without the diff-color
/// blending — code blocks use the syntect foreground colors verbatim.
fn highlight_code_line_spans(
    line: &str,
    syntax: Option<&'static syntect::parsing::SyntaxReference>,
    base_style: Style,
) -> Vec<Span<'static>> {
    let Some(syntax) = syntax else {
        return vec![Span::styled(line.to_string(), base_style)];
    };

    let ts = &*THEME_SET;
    let theme = ts
        .themes
        .get("base16-ocean.dark")
        .or_else(|| ts.themes.values().next())
        .unwrap_or_else(|| {
            ts.themes
                .values()
                .next()
                .expect("ThemeSet has at least one theme")
        });

    let ss = &*SYNTAX_SET;
    let mut h = HighlightLines::new(syntax, theme);
    match h.highlight_line(line, ss) {
        Ok(ranges) => {
            let mut result: Vec<Span<'static>> = Vec::new();
            for (style, text) in ranges {
                if text.is_empty() {
                    continue;
                }
                let fg = style.foreground;
                // Skip "default" near-white foregrounds (syntect emits these for
                // plain text — we want the base code-block style to show through
                // so the gutter alignment stays consistent).
                let is_default = fg.r > 200 && fg.g > 200 && fg.b > 200;
                if is_default {
                    result.push(Span::styled(text.to_string(), base_style));
                } else {
                    result.push(Span::styled(
                        text.to_string(),
                        Style::default().fg(Color::Rgb(fg.r, fg.g, fg.b)),
                    ));
                }
            }
            if result.is_empty() {
                vec![Span::styled(line.to_string(), base_style)]
            } else {
                result
            }
        }
        Err(_) => vec![Span::styled(line.to_string(), base_style)],
    }
}

/// Left margin applied to every block this renderer emits.
const MD_INDENT: usize = 2;
/// Extra left margin added per level of list nesting.
const MD_LIST_STEP: usize = 2;
/// Never squeeze a block below this many content columns, even on a
/// pathologically narrow terminal.
const MD_MIN_CONTENT: usize = 8;
/// Task-list checkbox glyphs.
const TASK_CHECKED: &str = "\u{2611} ";
const TASK_UNCHECKED: &str = "\u{2610} ";
const LIST_BULLET: char = '\u{2022}';
const IMAGE_GLYPH: &str = "\u{1F5BC}";
/// Dashes drawn in each cap of a code-fence box.
const CODE_FENCE_WIDTH: usize = 22;

/// Render markdown text to styled ratatui lines.
///
/// The document is parsed by `pulldown-cmark` into a flat event stream and
/// walked exactly once. Every event is folded into the same `Line`/`Span`
/// representation the rest of `tui/render/` uses, so downstream wrapping,
/// caching and OSC 8 hyperlink injection keep working — no ANSI escapes are
/// produced here.
///
/// Every emitted line is at most `width` display columns wide, including the
/// indentation added for nested lists, block quotes and footnote definitions.
pub fn render_markdown(text: &str, width: u16) -> Vec<Line<'static>> {
    // Strip \r and collapse mid-word soft breaks. Providers like mimo send \n
    // within JSON content at mid-word positions, which fragments words before
    // the parser ever sees them; a garbled word is worse than a joined line.
    let normalized = normalize_markdown_newlines(text).replace('\r', "");

    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_TASKLISTS);

    let parser = Parser::new_ext(&normalized, options);
    let mut renderer = MdRenderer::new(width);
    renderer.run(parser);
    renderer.finish()
}

/// Number of decimal digits in `n` (at least 1).
fn digits_of(n: u64) -> usize {
    let mut digits = 1;
    let mut value = n;
    while value >= 10 {
        value /= 10;
        digits += 1;
    }
    digits
}

/// Map a parser alignment onto the table renderer's own enum. `None` means the
/// column had no explicit marker, which the renderer treats as left-aligned.
fn table_alignment(align: &MdAlignment) -> super::markdown_enhanced::TableAlignment {
    use super::markdown_enhanced::TableAlignment;
    match align {
        MdAlignment::Center => TableAlignment::Center,
        MdAlignment::Right => TableAlignment::Right,
        MdAlignment::None | MdAlignment::Left => TableAlignment::Left,
    }
}

/// One enclosing list, and the counter for the next item marker.
#[derive(Debug, Clone, Copy)]
struct ListState {
    ordered: bool,
    next: u64,
    digits: usize,
}

/// A table while the walker is inside `Start(Table)` / `End(Table)`.
#[derive(Debug, Default)]
struct TableState {
    alignments: Vec<super::markdown_enhanced::TableAlignment>,
    headers: Vec<String>,
    rows: Vec<Vec<String>>,
    row: Vec<String>,
}

/// One styled word of a flattened inline run.
struct Word {
    text: String,
    style: Style,
}

/// Streaming markdown -> `Line`/`Span` converter.
struct MdRenderer {
    width: usize,
    out: Vec<Line<'static>>,
    /// Pending inline spans of the block currently being built.
    spans: Vec<Span<'static>>,
    /// Inline style stack for `*`, `**`, `~~` and links.
    styles: Vec<Style>,
    /// Text accumulated while a table cell is open.
    cell: String,
    /// Destination of the image currently being built.
    image_dest: Option<String>,
    /// Index into `spans` where the current link's text begins, and its target.
    link_start: usize,
    link_dest: Option<String>,
    /// Left margin of the block being built (base indent + list depth).
    indent: usize,
    /// Block-quote nesting depth.
    quote: usize,
    /// Enclosing lists, outermost first.
    lists: Vec<ListState>,
    /// Marker for the current list item; consumed by the first emitted line.
    marker: Option<String>,
    /// A blank separator line is owed before the next top-level block.
    need_sep: bool,
    /// Fenced code: language label, raw body, resolved syntax.
    code: Option<(
        String,
        String,
        Option<&'static syntect::parsing::SyntaxReference>,
    )>,
    /// Table being accumulated, if any.
    table: Option<TableState>,
    /// Footnote definition currently being captured: (label, rendered lines).
    capture: Option<(String, Vec<Line<'static>>)>,
    /// Captured footnote definitions keyed by their markdown label.
    defs: BTreeMap<String, Vec<Line<'static>>>,
    /// Labels in first-reference order; index + 1 is the rendered number.
    order: Vec<String>,
}

impl MdRenderer {
    fn new(width: u16) -> Self {
        Self {
            width: width as usize,
            out: Vec::new(),
            spans: Vec::new(),
            styles: Vec::new(),
            cell: String::new(),
            image_dest: None,
            link_start: 0,
            link_dest: None,
            indent: MD_INDENT,
            quote: 0,
            lists: Vec::new(),
            marker: None,
            need_sep: false,
            code: None,
            table: None,
            capture: None,
            defs: BTreeMap::new(),
            order: Vec::new(),
        }
    }

    fn run<'a>(&mut self, parser: Parser<'a>) {
        for event in parser {
            self.event(event);
        }
    }

    fn event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => self.text(&text),
            Event::Code(code) => {
                if self.table.is_some() {
                    self.cell.push_str(&code);
                } else {
                    // Matches the pre-parser inline-code colour.
                    self.push_inline_spanned(&code, Style::default().fg(theme::warning_color()));
                }
            }
            Event::SoftBreak => self.push_inline(" "),
            Event::HardBreak => self.flush(),
            Event::Rule => {
                self.gap();
                let rule = self.rule();
                self.push_line(rule);
                self.done();
            }
            Event::TaskListMarker(checked) => {
                self.marker = Some(if checked {
                    TASK_CHECKED.to_string()
                } else {
                    TASK_UNCHECKED.to_string()
                });
            }
            Event::FootnoteReference(name) => {
                let number = self.footnote_number(&name);
                let style = self.style().fg(theme_colors::accent());
                self.push_inline_spanned(&format!("[{number}]"), style);
            }
            Event::Html(html) | Event::InlineHtml(html) => {
                if self.table.is_some() {
                    self.cell.push_str(&html);
                } else {
                    self.push_inline_spanned(&html, Style::default().fg(theme::dim_color()));
                }
            }
            // `InlineMath` / `DisplayMath` require Options::ENABLE_MATH, which
            // this renderer does not turn on.
            Event::InlineMath(_) | Event::DisplayMath(_) => {}
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => self.gap(),
            Tag::Heading { level, .. } => {
                self.gap();
                let style = if level == HeadingLevel::H1 {
                    Style::default()
                        .fg(theme_colors::text())
                        .add_modifier(Modifier::BOLD | Modifier::ITALIC | Modifier::UNDERLINED)
                } else {
                    Style::default()
                        .fg(theme_colors::text())
                        .add_modifier(Modifier::BOLD)
                };
                self.styles.push(style);
            }
            Tag::BlockQuote(_) => {
                self.flush();
                self.gap();
                self.quote += 1;
            }
            Tag::List(start) => {
                // A nested list follows its parent's leading text, so that text
                // must become its own line before the child items are numbered.
                self.flush();
                self.gap();
                let first = start.unwrap_or(1);
                self.lists.push(ListState {
                    ordered: start.is_some(),
                    next: first,
                    digits: digits_of(first),
                });
            }
            Tag::Item => {
                let (marker, indent) = self.item_marker();
                self.marker = Some(marker);
                self.indent = indent;
            }
            Tag::CodeBlock(kind) => {
                self.flush();
                self.gap();
                let lang = match kind {
                    CodeBlockKind::Fenced(lang) => lang.to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                let syntax = resolve_syntax(lang.trim());
                self.code = Some((lang, String::new(), syntax));
            }
            Tag::Table(alignments) => {
                self.flush();
                self.gap();
                self.table = Some(TableState {
                    alignments: alignments.iter().map(table_alignment).collect(),
                    ..TableState::default()
                });
            }
            Tag::TableHead | Tag::TableRow => {
                if let Some(state) = &mut self.table {
                    state.row.clear();
                }
            }
            Tag::TableCell => self.cell.clear(),
            Tag::Emphasis => self
                .styles
                .push(Style::default().add_modifier(Modifier::ITALIC)),
            Tag::Strong => self
                .styles
                .push(Style::default().add_modifier(Modifier::BOLD)),
            Tag::Strikethrough => self
                .styles
                .push(Style::default().add_modifier(Modifier::CROSSED_OUT)),
            Tag::Link { dest_url, .. } => {
                self.link_start = self.spans.len();
                self.link_dest = Some(dest_url.to_string());
                self.styles.push(
                    Style::default()
                        .fg(theme_colors::accent())
                        .add_modifier(Modifier::UNDERLINED),
                );
            }
            Tag::Image { dest_url, .. } => {
                self.image_dest = Some(dest_url.to_string());
                self.push_inline(&format!("{IMAGE_GLYPH} "));
            }
            Tag::FootnoteDefinition(name) => {
                self.gap();
                self.need_sep = false;
                // Definitions are captured flush-left so `finish` can label the
                // first line and align continuations under it.
                self.indent = 0;
                self.footnote_number(&name);
                self.capture = Some((name.to_string(), Vec::new()));
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                self.flush();
                self.done();
            }
            TagEnd::Heading(_) => {
                self.styles.pop();
                self.flush();
                self.done();
            }
            TagEnd::BlockQuote(_) => {
                self.flush();
                self.quote = self.quote.saturating_sub(1);
                self.done();
            }
            TagEnd::List(_) => {
                self.lists.pop();
                self.done();
            }
            TagEnd::Item => {
                self.flush();
                self.marker = None;
                self.indent = MD_INDENT;
                if let Some(state) = self.lists.last_mut() {
                    state.next += 1;
                }
            }
            TagEnd::CodeBlock => {
                self.render_code();
                self.done();
            }
            TagEnd::Table => self.render_table(),
            TagEnd::TableHead => self.commit_row(true),
            TagEnd::TableRow => self.commit_row(false),
            TagEnd::TableCell => self.commit_cell(),
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                self.styles.pop();
            }
            TagEnd::Link => self.end_link(),
            TagEnd::Image => self.end_image(),
            TagEnd::FootnoteDefinition => self.end_footnote(),
            _ => {}
        }
    }

    fn text(&mut self, text: &str) {
        if let Some((_, body, _)) = &mut self.code {
            body.push_str(text);
            return;
        }
        if self.table.is_some() {
            self.cell.push_str(text);
            return;
        }
        // Bare URLs and e-mail addresses in prose still get link styling.
        let style = self.style();
        for span in split_and_style_links(text) {
            self.spans.push(span.style(style));
        }
    }

    /// The composed style of the enclosing inline tags.
    fn style(&self) -> Style {
        self.styles
            .iter()
            .fold(Style::default(), |acc, style| acc.patch(*style))
    }

    fn push_inline(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let style = self.style();
        self.push_inline_spanned(text, style);
    }

    fn push_inline_spanned(&mut self, text: &str, style: Style) {
        if !text.is_empty() {
            self.spans.push(Span::styled(text.to_string(), style));
        }
    }

    /// Emit the pending inline spans as width-respecting lines. A block with
    /// nothing pending (a tight list item, or a block already flushed by a
    /// nested tag) must not produce a blank line.
    fn flush(&mut self) {
        if self.spans.is_empty() {
            return;
        }
        let spans = std::mem::take(&mut self.spans);
        let marker = self.marker.take();
        let head = self.prefix(marker.as_deref());
        let tail = self.prefix(None);
        // The first line carries the marker, so it is the widest prefix; using
        // it for every line keeps the content column identical and can only
        // ever under-fill, never overflow.
        let avail = self
            .width
            // The extra 2 columns are the right margin the pre-parser renderer
            // reserved (`width - 4` behind a 2-column indent), kept so wrapped
            // text does not touch the terminal edge.
            .saturating_sub(head.width() + 2)
            .max(MD_MIN_CONTENT);
        let mut chunks = wrap_spans(spans, avail);
        if chunks.is_empty() {
            chunks.push(Vec::new());
        }
        for (i, chunk) in chunks.into_iter().enumerate() {
            let mut line = vec![Span::raw(if i == 0 { head.clone() } else { tail.clone() })];
            line.extend(chunk);
            self.push_line(Line::from(line));
        }
    }

    /// Left margin for a line, plus the item marker on an item's first line.
    fn prefix(&self, marker: Option<&str>) -> String {
        let mut prefix = " ".repeat(self.indent);
        for _ in 0..self.quote {
            prefix.push_str(figures::BLOCKQUOTE_BAR);
            prefix.push(' ');
        }
        if let Some(marker) = marker {
            prefix.push_str(marker);
        }
        prefix
    }

    /// Marker text and left margin for the list item about to start.
    fn item_marker(&self) -> (String, usize) {
        let depth = self.lists.len().saturating_sub(1);
        let indent = MD_INDENT + MD_LIST_STEP * depth;
        match self.lists.last() {
            // ponytail: markers are right-aligned to the digit width of the
            // list's first number, since a streaming walk never learns the
            // final item number. Lists of 10+ items therefore step the marker
            // column once; pre-scan the list to align it exactly.
            Some(state) if state.ordered => (
                format!("{:>digits$}. ", state.next, digits = state.digits),
                indent,
            ),
            _ => (format!("{LIST_BULLET} "), indent),
        }
    }

    fn push_line(&mut self, line: Line<'static>) {
        match &mut self.capture {
            Some((_, lines)) => lines.push(line),
            None => self.out.push(line),
        }
    }

    /// Emit the blank separator line owed before the next top-level block.
    fn gap(&mut self) {
        if self.need_sep {
            self.need_sep = false;
            self.push_line(Line::from("  ".to_string()));
        }
    }

    /// Mark a block boundary. Only blocks at quote depth 0 and list depth <= 1
    /// owe the next block a blank line, so a tight list never gets one between
    /// its items while a loose list does.
    fn done(&mut self) {
        if self.quote == 0 && self.lists.len() <= 1 {
            self.need_sep = true;
        }
    }

    fn rule(&self) -> Line<'static> {
        let indent = " ".repeat(self.indent);
        let width = self
            .width
            .saturating_sub(indent.width() + 1)
            .max(MD_MIN_CONTENT);
        Line::from(Span::styled(
            format!("{indent}{}", "\u{2500}".repeat(width)),
            Style::default().fg(theme::dim_color()),
        ))
    }

    fn render_code(&mut self) {
        let Some((lang, body, syntax)) = self.code.take() else {
            return;
        };
        let indent = " ".repeat(self.indent);
        // A ```mermaid fence holds a diagram, not source to read. The ladder in
        // `tui::mermaid` decides whether it is drawn inline or shown as source
        // with the reason; either way this block is never a syntax-highlighted
        // code listing.
        if crate::tui::mermaid::is_mermaid_lang(&lang) {
            for line in crate::tui::mermaid::diagram_lines(&body, &indent) {
                self.push_line(line);
            }
            return;
        }
        // A ```latex / ```math / ```tex fence holds a formula, not source to
        // read. Same shape as the mermaid branch above and the same deal: the
        // ladder in `tui::latex` decides whether it is drawn inline or shown as
        // source with the reason, and either way the source is never swallowed.
        if crate::tui::latex::is_latex_lang(&lang) {
            for line in crate::tui::latex::formula_lines(&body, &indent) {
                self.push_line(line);
            }
            return;
        }
        let label = if lang.is_empty() {
            String::new()
        } else {
            format!(" {lang} ")
        };
        let border = Style::default().fg(theme::warning_color());
        // One constant for both caps so the fence is a matched pair and its
        // width stays bounded, instead of the mismatched fixed-length strings
        // this renderer used before.
        let rule = "\u{2500}".repeat(CODE_FENCE_WIDTH);
        self.push_line(Line::from(vec![Span::styled(
            format!("{indent}\u{250C}{rule}{label}"),
            border,
        )]));
        let base = Style::default().fg(theme_colors::text());
        for line in body.trim_end_matches('\n').lines() {
            let mut spans = vec![Span::styled(format!("{indent}\u{2502} "), border)];
            spans.extend(highlight_code_line_spans(line, syntax, base));
            self.push_line(Line::from(spans));
        }
        self.push_line(Line::from(vec![Span::styled(
            format!("{indent}\u{2514}{rule}"),
            border,
        )]));
    }

    fn commit_cell(&mut self) {
        let cell = std::mem::take(&mut self.cell);
        if let Some(state) = &mut self.table {
            state.row.push(cell.trim().to_string());
        }
    }

    fn commit_row(&mut self, header: bool) {
        let Some(state) = &mut self.table else {
            return;
        };
        let row = std::mem::take(&mut state.row);
        if row.is_empty() {
            return;
        }
        if header {
            state.headers = row;
        } else {
            state.rows.push(row);
        }
    }

    fn render_table(&mut self) {
        let Some(state) = self.table.take() else {
            return;
        };
        if state.headers.is_empty() {
            return;
        }
        let table = super::markdown_enhanced::Table {
            headers: state.headers,
            rows: state.rows,
            alignments: state.alignments,
        };
        for line in super::markdown_enhanced::render_table(&table, self.width as u16) {
            self.push_line(line);
        }
        self.done();
    }

    fn end_link(&mut self) {
        self.styles.pop();
        let Some(dest) = self.link_dest.take() else {
            return;
        };
        let text: String = self
            .spans
            .get(self.link_start..)
            .unwrap_or_default()
            .iter()
            .map(|span| span.content.to_string())
            .collect();
        if !dest.is_empty() && text != dest {
            self.spans.push(Span::styled(
                format!(" ({dest})"),
                Style::default().fg(theme::dim_color()),
            ));
        }
    }

    fn end_image(&mut self) {
        if let Some(dest) = self.image_dest.take()
            && !dest.is_empty()
        {
            self.spans.push(Span::styled(
                format!(" ({dest})"),
                Style::default().fg(theme::dim_color()),
            ));
        }
    }

    fn footnote_number(&mut self, name: &str) -> usize {
        if let Some(index) = self.order.iter().position(|label| label == name) {
            return index + 1;
        }
        self.order.push(name.to_string());
        self.order.len()
    }

    fn end_footnote(&mut self) {
        self.flush();
        if let Some((label, lines)) = self.capture.take() {
            self.defs.insert(label, lines);
        }
        self.indent = MD_INDENT;
        self.need_sep = false;
    }

    fn finish(mut self) -> Vec<Line<'static>> {
        self.flush();
        if self.order.is_empty() {
            return self.out;
        }
        let rule = self.rule();
        self.out.push(rule);
        for (index, label) in self.order.iter().enumerate() {
            let Some(lines) = self.defs.get(label) else {
                continue;
            };
            let mut iter = lines.iter();
            if let Some(first) = iter.next() {
                let mut spans = vec![Span::styled(
                    format!("  [{}] ", index + 1),
                    Style::default()
                        .fg(theme_colors::accent())
                        .add_modifier(Modifier::BOLD),
                )];
                spans.extend(first.spans.iter().cloned());
                self.out.push(Line::from(spans));
            }
            for line in iter {
                let mut spans = vec![Span::raw("      ".to_string())];
                spans.extend(line.spans.iter().cloned());
                self.out.push(Line::from(spans));
            }
        }
        self.out
    }
}

/// Wrap styled spans to `width` display columns, carrying each span's style
/// across wrap boundaries. Words longer than `width` are hard-broken.
fn wrap_spans(spans: Vec<Span<'static>>, width: usize) -> Vec<Vec<Span<'static>>> {
    let mut lines: Vec<Vec<Word>> = Vec::new();
    let mut current: Vec<Word> = Vec::new();
    let mut current_w = 0usize;

    for word in flatten_words(spans) {
        let word_w = word.text.width();
        if word_w > width {
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
                current_w = 0;
            }
            for chunk in hard_break(&word.text, width) {
                lines.push(vec![Word {
                    text: chunk,
                    style: word.style,
                }]);
            }
            continue;
        }
        if current.is_empty() {
            current.push(word);
            current_w = word_w;
        } else if current_w + 1 + word_w <= width {
            current_w += 1 + word_w;
            current.push(word);
        } else {
            lines.push(std::mem::take(&mut current));
            current.push(word);
            current_w = word_w;
        }
    }

    if !current.is_empty() {
        lines.push(current);
    }
    lines.into_iter().map(words_to_spans).collect()
}

/// Flatten spans into styled words. Intra-span runs of whitespace collapse to
/// a single separator, matching the previous `split_whitespace` behaviour.
fn flatten_words(spans: Vec<Span<'static>>) -> Vec<Word> {
    let mut words = Vec::new();
    for span in spans {
        for token in span.content.split_whitespace() {
            words.push(Word {
                text: token.to_string(),
                style: span.style,
            });
        }
    }
    words
}

/// Re-join styled words into spans separated by single spaces.
fn words_to_spans(words: Vec<Word>) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::with_capacity(words.len() * 2);
    for (i, word) in words.into_iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(" ".to_string()));
        }
        spans.push(Span::styled(word.text, word.style));
    }
    spans
}

/// Split a word that cannot fit on one line, never cutting a grapheme cluster
/// (so a combining mark stays attached to its base character).
fn hard_break(word: &str, width: usize) -> Vec<String> {
    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_w = 0usize;
    for grapheme in word.graphemes(true) {
        let grapheme_w = grapheme.width();
        if !current.is_empty() && current_w + grapheme_w > width {
            chunks.push(std::mem::take(&mut current));
            current_w = 0;
        }
        current.push_str(grapheme);
        current_w += grapheme_w;
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    if chunks.is_empty() {
        chunks.push(String::new());
    }
    chunks
}

/// Split plain text into spans with URL/email detection and styling.
/// URLs and emails are styled with cyan color and underline.
fn split_and_style_links(text: &str) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut last_end = 0;

    // Check for URLs first
    for url_match in URL_PATTERN.find_iter(text) {
        let match_start = url_match.start();
        let match_end = url_match.end();

        // Add text before the URL
        if match_start > last_end {
            spans.push(Span::raw(text[last_end..match_start].to_string()));
        }

        // Add the URL with special styling (accent with underline)
        let url_text = url_match.as_str();
        spans.push(Span::styled(
            url_text.to_string(),
            Style::default()
                .fg(theme_colors::accent())
                .add_modifier(Modifier::UNDERLINED),
        ));
        last_end = match_end;
    }

    // Check for emails in remaining text (only if no URLs were found)
    if last_end == 0 {
        for email_match in EMAIL_PATTERN.find_iter(text) {
            let match_start = email_match.start();
            let match_end = email_match.end();

            // Add text before the email
            if match_start > last_end {
                spans.push(Span::raw(text[last_end..match_start].to_string()));
            }

            // Add the email with special styling (accent with underline)
            let email_text = email_match.as_str();
            spans.push(Span::styled(
                email_text.to_string(),
                Style::default()
                    .fg(theme_colors::accent())
                    .add_modifier(Modifier::UNDERLINED),
            ));
            last_end = match_end;
        }
    }

    // Add any remaining text
    if last_end < text.len() {
        spans.push(Span::raw(text[last_end..].to_string()));
    }

    // If no links/emails were found, return a simple raw span
    if spans.is_empty() {
        spans.push(Span::raw(text.to_string()));
    }

    spans
}

/// Normalizes soft breaks (single newlines) to spaces or empty strings (if mid-word),
/// while preserving hard breaks (double newlines, lists, headings, code blocks, blockquotes).
pub fn normalize_markdown_newlines(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let lines: Vec<&str> = text.lines().collect();

    let mut in_code_block = false;
    let mut i = 0;

    while i < lines.len() {
        let current_line = lines[i];
        let trimmed = current_line.trim_start();

        if trimmed.starts_with("```") {
            in_code_block = !in_code_block;
        }

        result.push_str(current_line);

        if i + 1 < lines.len() {
            let next_line = lines[i + 1];
            let next_trimmed = next_line.trim_start();

            // Check if the NEXT line is a structural element
            let next_is_list_item = next_trimmed.starts_with("- ")
                || next_trimmed.starts_with("* ")
                || next_trimmed.starts_with("+ ")
                || (next_trimmed.contains(". ")
                    && next_trimmed
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_ascii_digit()));
            let next_is_heading = next_trimmed.starts_with("#");
            let next_is_blockquote = next_trimmed.starts_with("> ");
            let next_is_code_fence = next_trimmed.starts_with("```");

            // Check if the CURRENT line is a structural element — newline must
            // be preserved after headings, list items, and blockquotes so they
            // don't merge with the following line.
            let current_is_heading = trimmed.starts_with("#");
            let current_is_list_item = trimmed.starts_with("- ")
                || trimmed.starts_with("* ")
                || trimmed.starts_with("+ ")
                || (trimmed.contains(". ")
                    && trimmed.chars().next().is_some_and(|c| c.is_ascii_digit()));
            let current_is_blockquote = trimmed.starts_with("> ");
            // Table rows must each stay on their own line, otherwise the
            // header and the `---` separator collapse into a single line and
            // the table is never recognised.
            let next_is_table_row = next_trimmed.starts_with('|');
            let current_is_table_row = trimmed.starts_with('|');
            // A closing fence must keep its own newline, otherwise it gets
            // glued to the first line of the following block and the parser
            // re-opens a code block that was never closed.
            let current_is_code_fence = trimmed.starts_with("```");

            if in_code_block
                || current_line.is_empty()
                || next_line.is_empty()
                || next_is_list_item
                || next_is_heading
                || next_is_blockquote
                || next_is_code_fence
                || next_is_table_row
                || current_is_heading
                || current_is_list_item
                || current_is_blockquote
                || current_is_code_fence
                || current_is_table_row
            {
                result.push('\n');
            } else {
                let char_a = current_line.chars().last();
                let char_b = next_line.chars().next();
                let is_mid_word = match (char_a, char_b) {
                    (Some(a), Some(b)) => a.is_ascii_alphabetic() && b.is_ascii_alphabetic(),
                    _ => false,
                };

                if is_mid_word {
                    // Mid-word newline: join the word directly (no space/newline)
                } else if !current_line.ends_with(' ') && !next_line.starts_with(' ') {
                    result.push(' ');
                }
            }
        }
        i += 1;
    }
    result
}
