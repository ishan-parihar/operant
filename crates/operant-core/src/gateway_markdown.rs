//! Convert markdown text to Telegram-compatible HTML.
//!
//! This module provides a single public function [`markdown_to_telegram_html`]
//! that converts AI-generated markdown text into Telegram-compatible HTML
//! format. It handles bold, italic, code blocks, inline code, headers,
//! strikethrough, links, and blockquotes.
//!
//! Markdown inside code blocks and inline code spans is preserved verbatim.
//! All remaining HTML special characters are escaped to prevent injection.

use regex::Regex;
use std::sync::LazyLock;

// ---------------------------------------------------------------------------
// GFM table → bullets conversion (hermes parity)
// ---------------------------------------------------------------------------

/// Matches a GFM table delimiter row: optional outer pipes, cells of dashes
/// (with optional alignment colons) separated by '|'.
/// Requires at least one internal '|' so lone '---' rules are NOT matched.
static TABLE_SEPARATOR_RE: LazyLock<Regex> =
    LazyLock::new(|| rx(r"^\s*\|?\s*:?-+:?\s*(?:\|\s*:?-+:?\s*){1,}\|?\s*$"));

/// Return true if *line* could plausibly be a table data row.
fn is_table_row(line: &str) -> bool {
    let stripped = line.trim();
    !stripped.is_empty() && stripped.contains('|')
}

/// Split a GFM table row into trimmed cell values (`| a | b | c |` →
/// `["a", "b", "c"]`).
fn split_table_row(line: &str) -> Vec<String> {
    let mut s = line.trim();
    if let Some(rest) = s.strip_prefix('|') {
        s = rest;
    }
    if let Some(rest) = s.strip_suffix('|') {
        s = rest;
    }
    s.split('|').map(|cell| cell.trim().to_string()).collect()
}

/// Render a detected GFM table as bold-heading + bullet groups.
///
/// Uses the same alignment logic as hermes' Telegram renderer: for
/// non-row-label tables, `data_cells = cells` (the full row) and the bullet
/// whose value duplicates the heading is skipped — this keeps
/// header→value alignment correct for both `| A | B |` and `| label | A | B |`
/// shaped tables.
fn render_table_block(table_block: &[String]) -> String {
    if table_block.len() < 3 {
        return table_block.join("\n");
    }
    let headers = split_table_row(&table_block[0]);
    if headers.len() < 2 {
        return table_block.join("\n");
    }
    let first_data_row = if table_block.len() > 2 {
        split_table_row(&table_block[2])
    } else {
        Vec::new()
    };
    let has_row_label_col = first_data_row.len() == headers.len() + 1;

    let mut rendered_groups: Vec<String> = Vec::new();
    for (index, row) in table_block.iter().skip(2).enumerate() {
        let mut cells = split_table_row(row);
        let heading: String;
        let data_cells: Vec<String>;
        if has_row_label_col {
            heading = if !cells.is_empty() && !cells[0].is_empty() {
                cells[0].clone()
            } else {
                format!("Row {}", index + 1)
            };
            data_cells = cells.split_off(1);
        } else {
            heading = cells
                .iter()
                .find(|cell| !cell.is_empty())
                .cloned()
                .unwrap_or_else(|| format!("Row {}", index + 1));
            data_cells = cells;
        }

        let mut data_cells = data_cells;
        if data_cells.len() < headers.len() {
            data_cells.resize(headers.len(), String::new());
        } else if data_cells.len() > headers.len() {
            data_cells.truncate(headers.len());
        }

        let mut bullets: Vec<String> = Vec::new();
        for (header, value) in headers.iter().zip(data_cells.iter()) {
            if !has_row_label_col && *value == heading {
                continue;
            }
            bullets.push(format!("• {}: {}", header, value));
        }

        let mut group_lines = vec![format!("**{}**", heading)];
        group_lines.extend(bullets);
        rendered_groups.push(group_lines.join("\n"));
    }
    rendered_groups.join("\n\n")
}

/// Rewrite GFM pipe tables into bold-heading + bullet groups.
///
/// Tables inside fenced code blocks are left alone. Runs BEFORE the
/// placeholder extraction / HTML escaping so the `**heading**` markers it
/// emits flow through the regular bold conversion and pipes never reach
/// Telegram raw.
fn convert_table_to_bullets(text: &str) -> String {
    if !text.contains('|') || !text.contains('-') {
        return text.to_string();
    }
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out: Vec<String> = Vec::new();
    let mut in_fence = false;
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let stripped = line.trim_start();
        if stripped.starts_with("```") {
            in_fence = !in_fence;
            out.push(line.to_string());
            i += 1;
            continue;
        }
        if in_fence {
            out.push(line.to_string());
            i += 1;
            continue;
        }
        if line.contains('|') && i + 1 < lines.len() && TABLE_SEPARATOR_RE.is_match(lines[i + 1]) {
            let mut table_block = vec![line.to_string(), lines[i + 1].to_string()];
            let mut j = i + 2;
            while j < lines.len() && is_table_row(lines[j]) {
                table_block.push(lines[j].to_string());
                j += 1;
            }
            out.push(render_table_block(&table_block));
            i = j;
            continue;
        }
        out.push(line.to_string());
        i += 1;
    }
    out.join("\n")
}

// ---------------------------------------------------------------------------
// Placeholder system
// ---------------------------------------------------------------------------

/// Sentinel character used to construct unique placeholders.
///
/// U+0001 (Start of Heading) is a C0 control code that is astronomically
/// unlikely to appear in human- or AI-generated text.
const SENTINEL: char = '\u{1}';

/// Build a placeholder string for a given storage kind and index.
fn make_placeholder(kind: &str, index: usize) -> String {
    format!("{}{}_{}{}", SENTINEL, kind, index, SENTINEL)
}

/// Manages extracted code blocks and inline code spans that must be
/// protected from markdown and HTML processing.
struct MarkdownConverter {
    code_blocks: Vec<String>,
    inline_codes: Vec<String>,
}

impl MarkdownConverter {
    fn new() -> Self {
        MarkdownConverter {
            code_blocks: Vec::new(),
            inline_codes: Vec::new(),
        }
    }

    #[expect(
        clippy::expect_used,
        reason = "invariant guaranteed by surrounding validation"
    )]
    /// Extract fenced code blocks (` ``` … ``` `) and replace them with
    /// sentinel-bounded placeholders. The optional language tag (e.g.
    /// `rust`) is discarded.
    fn extract_code_blocks(&mut self, text: &str) -> String {
        static RE: LazyLock<Regex> = LazyLock::new(|| rx(r"```(\w*)\n([\s\S]*?)```"));
        let mut result = String::with_capacity(text.len());
        let mut last = 0;
        for caps in RE.captures_iter(text) {
            let m = caps.get(0).expect("regex matched — group 0 always present");
            // Push the text between the previous match and this one.
            result.push_str(&text[last..m.start()]);
            // Store the raw content and emit a placeholder.
            let content = caps
                .get(2)
                .expect("code-block regex defines group 2")
                .as_str()
                .to_string();
            let idx = self.code_blocks.len();
            self.code_blocks.push(content);
            result.push_str(&make_placeholder("CODE_BLOCK", idx));
            last = m.end();
        }
        result.push_str(&text[last..]);
        result
    }

    #[expect(
        clippy::expect_used,
        reason = "invariant guaranteed by surrounding validation"
    )]
    /// Extract inline code spans (`` `code` ``) and replace them with
    /// sentinel-bounded placeholders.
    fn extract_inline_code(&mut self, text: &str) -> String {
        static RE: LazyLock<Regex> = LazyLock::new(|| rx(r"`([^`]+?)`"));
        let mut result = String::with_capacity(text.len());
        let mut last = 0;
        for caps in RE.captures_iter(text) {
            let m = caps.get(0).expect("regex matched — group 0 always present");
            result.push_str(&text[last..m.start()]);
            let content = caps
                .get(1)
                .expect("inline-code regex defines group 1")
                .as_str()
                .to_string();
            let idx = self.inline_codes.len();
            self.inline_codes.push(content);
            result.push_str(&make_placeholder("INLINE_CODE", idx));
            last = m.end();
        }
        result.push_str(&text[last..]);
        result
    }

    /// Reinsert all protected inline code and code block content, HTML-escaping
    /// the raw code content before wrapping in Telegram HTML tags.
    ///
    /// Use this variant when code blocks were extracted *before* HTML escaping
    /// (i.e. they contain the original verbatim text and must be escaped now).
    fn reinsert_all_escaped(&self, text: &str) -> String {
        let mut result = text.to_string();
        for (i, code) in self.inline_codes.iter().enumerate() {
            let ph = make_placeholder("INLINE_CODE", i);
            result = result.replace(&ph, &format!("<code>{}</code>", escape_code_content(code)));
        }
        for (i, code) in self.code_blocks.iter().enumerate() {
            let ph = make_placeholder("CODE_BLOCK", i);
            result = result.replace(&ph, &format!("<pre>{}</pre>", escape_code_content(code)));
        }
        result
    }
}

// ---------------------------------------------------------------------------
// Markdown → Telegram HTML conversions (operates on already-extracted text)
// ---------------------------------------------------------------------------

#[expect(clippy::expect_used, reason = "infallible once-init / static init")]
/// Compile a regex from a static literal pattern (see approval.rs rx()).
/// The `expect` keeps authoring-time mistakes loud while avoiding `unwrap()`
/// sites that would trip `clippy::unwrap_used` if enabled.
fn rx(pattern: &'static str) -> Regex {
    Regex::new(pattern).expect("static regex literal is invalid — authoring bug")
}

static BOLD_ITALIC_RE: LazyLock<Regex> = LazyLock::new(|| rx(r"\*\*\*(.+?)\*\*\*"));
static BOLD_RE: LazyLock<Regex> = LazyLock::new(|| rx(r"\*\*(.+?)\*\*"));
static ITALIC_RE: LazyLock<Regex> = LazyLock::new(|| rx(r"\*(.+?)\*"));
static STRIKETHROUGH_RE: LazyLock<Regex> = LazyLock::new(|| rx(r"~~(.+?)~~"));
static LINK_RE: LazyLock<Regex> = LazyLock::new(|| rx(r"\[(.+?)\]\((.+?)\)"));
static HEADER_RE: LazyLock<Regex> = LazyLock::new(|| rx(r"(?m)^#{1,6}\s+(.*?)$"));
static BLOCKQUOTE_RE: LazyLock<Regex> = LazyLock::new(|| rx(r"(?m)^&gt;\s?(.*?)$"));

/// Escape `&`, `<`, `>`, `"`, and `'` to their HTML entity equivalents.
fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// Escape only `&`, `<`, and `>` — the minimal set required inside Telegram
/// `<pre>` and `<code>` text nodes.  Quotes (`"`, `'`) do not need escaping
/// in text content and should be left verbatim so that code like
/// `println!("hi")` is displayed correctly.
fn escape_code_content(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
    out
}

/// Convert markdown constructs to Telegram HTML in text whose code spans
/// have already been extracted into placeholders.
fn convert_markdown(text: &str) -> String {
    // Bold+italic combined first: `***text***` → `<b><i>text</i></b>`.
    // This must precede the individual bold and italic passes so that the
    // triple-star sequence is consumed atomically.
    let s = BOLD_ITALIC_RE.replace_all(text, "<b><i>$1</i></b>");
    // Bold: only standalone `**text**` spans now remain.
    let s = BOLD_RE.replace_all(&s, "<b>$1</b>");
    // Italic: only standalone `*text*` spans now remain.
    let s = ITALIC_RE.replace_all(&s, "<i>$1</i>");
    // Strikethrough.
    let s = STRIKETHROUGH_RE.replace_all(&s, "<s>$1</s>");
    // Links – fix `&quot;` in the URL (from prior HTML escaping) to `%22`.
    let s = LINK_RE.replace_all(&s, |caps: &regex::Captures| {
        let link_text = &caps[1];
        let url = caps[2].replace("&quot;", "%22");
        format!("<a href=\"{}\">{}</a>", url, link_text)
    });
    // Headers – drop the `#` markers, wrap in `<b>`, keep a trailing newline
    // so subsequent content starts on a fresh line.
    let s = HEADER_RE.replace_all(&s, "<b>$1</b>\n");
    // Blockquotes – per-line conversion.
    // Note: `>` has been HTML-escaped to `&gt;` at this point, so the
    // regex anchors on `&gt;` rather than `>`.
    let s = BLOCKQUOTE_RE.replace_all(&s, "<blockquote>$1</blockquote>");
    s.to_string()
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Convert AI-generated markdown text to Telegram-compatible HTML.
///
/// The following markdown constructs are supported:
///
/// | Construct        | Markdown                    | Telegram HTML                              |
/// |------------------|-----------------------------|--------------------------------------------|
/// | Bold             | `**text**`                  | `<b>text</b>`                              |
/// | Italic           | `*text*`                    | `<i>text</i>`                              |
/// | Strikethrough    | `~~text~~`                  | `<s>text</s>`                              |
/// | Inline code      | `` `code` ``                | `<code>code</code>`                        |
/// | Code block       | ```` ```lang\ncode\n``` ````| `<pre>code</pre>`                          |
/// | Link             | `[text](url)`               | `<a href="url">text</a>`                   |
/// | Header           | `## text`                   | `<b>text</b>\n`                            |
/// | Blockquote       | `> text`                    | `<blockquote>text</blockquote>`            |
///
/// **Processing order** (critical for correctness):
/// 1. Extract fenced code blocks → placeholders (raw content is preserved so
///    HTML-escaping does not mangle characters like `"` inside `<pre>`)
/// 2. Extract inline code spans → placeholders
/// 3. Escape raw HTML special characters (`&`, `<`, `>`, `"`, `'`) in the
///    non-code body
/// 4. Apply markdown conversions: bold+italic → bold → italic → strikethrough
///    → links → headers → blockquotes
/// 5. Reinsert protected inline code and code blocks wrapped in Telegram
///    HTML tags (content is HTML-escaped at this point)
///
/// This ordering guarantees that markdown inside code fences or backtick
/// spans is never accidentally converted, and that literal characters like
/// `"` in code blocks reach the final output unescaped inside `<pre>`.
pub fn markdown_to_telegram_html(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }

    // 0. Convert GFM pipe tables to bold-heading + bullet groups FIRST
    //    (fence-aware, hermes parity) so the emitted `**heading**` markers
    //    flow through the bold pass below and raw `|` pipes never reach
    //    Telegram.
    let table_text = convert_table_to_bullets(text);

    // 1. Extract and protect code blocks and inline code BEFORE HTML escaping
    //    so that characters like `"` inside fences are preserved verbatim.
    let mut conv = MarkdownConverter::new();
    let no_blocks = conv.extract_code_blocks(&table_text);
    let no_code = conv.extract_inline_code(&no_blocks);

    // 2. Escape HTML special characters in the remaining (non-code) body.
    let escaped = escape_html(&no_code);

    // 3. Convert remaining markdown to Telegram HTML.
    let html = convert_markdown(&escaped);

    // 4. Reinsert protected content wrapped in `<pre>` / `<code>`.
    //    The raw code content is HTML-escaped here so that `<`, `>`, `&`
    //    inside code blocks/spans are safe for Telegram's HTML parser.
    conv.reinsert_all_escaped(&html)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bold() {
        assert_eq!(markdown_to_telegram_html("**bold**"), "<b>bold</b>");
        assert_eq!(
            markdown_to_telegram_html("not **bold** either"),
            "not <b>bold</b> either"
        );
    }

    #[test]
    fn test_italic() {
        assert_eq!(markdown_to_telegram_html("*italic*"), "<i>italic</i>");
    }

    #[test]
    fn test_bold_and_italic_nested() {
        // `***text***` → bold first → `<b>*text*</b>` → italic → `<b><i>text</i></b>`
        assert_eq!(
            markdown_to_telegram_html("***text***"),
            "<b><i>text</i></b>"
        );
    }

    #[test]
    fn test_strikethrough() {
        assert_eq!(markdown_to_telegram_html("~~strike~~"), "<s>strike</s>");
    }

    #[test]
    fn test_inline_code() {
        assert_eq!(
            markdown_to_telegram_html("use `code` here"),
            "use <code>code</code> here"
        );
    }

    #[test]
    fn test_code_block() {
        let input = "before\n```rust\nfn main() {\n    println!(\"hi\");\n}\n```\nafter";
        let expected = "before\n<pre>fn main() {\n    println!(\"hi\");\n}\n</pre>\nafter";
        assert_eq!(markdown_to_telegram_html(input), expected);
    }

    #[test]
    fn test_bold_not_inside_code() {
        // Markdown inside inline code must be preserved verbatim.
        let input = "`**not bold**`";
        assert_eq!(
            markdown_to_telegram_html(input),
            "<code>**not bold**</code>"
        );
    }

    #[test]
    fn test_bold_not_inside_code_block() {
        let input = "```\n**not bold**\n```";
        assert_eq!(
            markdown_to_telegram_html(input),
            "<pre>**not bold**\n</pre>"
        );
    }

    #[test]
    fn test_link() {
        assert_eq!(
            markdown_to_telegram_html("[click](https://example.com)"),
            "<a href=\"https://example.com\">click</a>"
        );
    }

    #[test]
    fn test_header() {
        assert_eq!(markdown_to_telegram_html("## Title"), "<b>Title</b>\n");
        assert_eq!(markdown_to_telegram_html("### Sub"), "<b>Sub</b>\n");
    }

    #[test]
    fn test_blockquote() {
        assert_eq!(
            markdown_to_telegram_html("> quoted"),
            "<blockquote>quoted</blockquote>"
        );
    }

    #[test]
    fn test_html_escaping() {
        let result = markdown_to_telegram_html("<script>alert('xss')</script>");
        assert!(!result.contains("<script>"));
        assert!(result.contains("&lt;script&gt;"));
        assert!(result.contains("&#39;"));
    }

    #[test]
    fn test_empty_text() {
        assert_eq!(markdown_to_telegram_html(""), "");
    }

    #[test]
    fn test_plain_text_no_markdown() {
        assert_eq!(markdown_to_telegram_html("Hello, world!"), "Hello, world!");
    }

    #[test]
    fn test_ampersand_escaping() {
        assert_eq!(markdown_to_telegram_html("AT&T"), "AT&amp;T");
    }

    #[test]
    fn test_mixed_formatting() {
        let input = "**bold** and *italic* and `code`";
        let expected = "<b>bold</b> and <i>italic</i> and <code>code</code>";
        assert_eq!(markdown_to_telegram_html(input), expected);
    }

    #[test]
    fn test_table_to_bullets() {
        // The exact shape from the live ✅ report — header row + separator +
        // two data rows. Renders as bold heading + bullet groups.
        let input = "| Component | Status | Details |\n|-----------|--------|---------|\n| Core Binary | ✅ | operant 0.2.0 |\n| Gateway | ✅ | Running |";
        let expected = "<b>Core Binary</b>\n• Status: ✅\n• Details: operant 0.2.0\n\n<b>Gateway</b>\n• Status: ✅\n• Details: Running";
        assert_eq!(markdown_to_telegram_html(input), expected);
    }

    #[test]
    fn test_table_with_row_labels() {
        // Row-label shape: first cell is the heading, the rest are values.
        let input =
            "| Metric | Value |\n|--------|-------|\n| Latency | 12ms |\n| Uptime | 99.9% |";
        let expected = "<b>Latency</b>\n• Value: 12ms\n\n<b>Uptime</b>\n• Value: 99.9%";
        assert_eq!(markdown_to_telegram_html(input), expected);
    }

    #[test]
    fn test_table_inside_code_block_untouched() {
        // Tables inside fenced code blocks must be left alone.
        let input = "```\n| A | B |\n|---|---|\n| 1 | 2 |\n```";
        let result = markdown_to_telegram_html(input);
        assert!(
            result.contains("<pre>| A | B |"),
            "table inside fence must survive: {result}"
        );
    }

    #[test]
    fn test_table_with_bold_heading_markdown() {
        // The `**heading**` markers emitted by the table renderer flow
        // through the bold pass.
        let input = "| Component | Status |\n|-----------|--------|\n| Core | ✅ |";
        let result = markdown_to_telegram_html(input);
        assert!(
            result.contains("<b>Core</b>"),
            "heading must be bold: {result}"
        );
        assert!(
            result.contains("• Status: ✅"),
            "bullet must be present: {result}"
        );
    }
}

// ---------------------------------------------------------------------------
// Slack mrkdwn conversion
// ---------------------------------------------------------------------------

/// Convert standard Markdown to Slack's mrkdwn format.
///
/// Slack uses a non-standard markup format:
/// - `**bold**` → `*bold*` (single asterisks)
/// - `*italic*` → `_italic_` (underscores)
/// - `~~strike~~` → `~strike~` (single tilde)
/// - `` `code` `` → `` `code` `` (same)
/// - ``` ```codeblock``` ``` → ``` ```codeblock``` ``` (same)
/// - `> quote` → `> quote` (same)
///
/// Code blocks and inline code are preserved verbatim (Slack renders them
/// the same way as Markdown).
///
/// (iter-102 — closes Bug #15 from iter-98 audit.)
pub fn markdown_to_slack_mrkdwn(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut in_code_block = false;

    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        // Detect code fence toggles.
        if ch == '`' && chars.peek() == Some(&'`') && chars.peek() == Some(&'`') {
            // Triple backtick — toggle code block state.
            in_code_block = !in_code_block;
            result.push_str("```");
            chars.next();
            chars.next();
            continue;
        }

        if in_code_block {
            // Inside a code block — pass through verbatim.
            result.push(ch);
            continue;
        }

        // Outside code blocks: convert markdown to mrkdwn.
        match ch {
            '*' => {
                if chars.peek() == Some(&'*') {
                    // `**bold**` → `*bold*` (consume second *, output single *)
                    chars.next();
                    result.push('*');
                } else {
                    // Single `*italic*` → `_italic_`
                    result.push('_');
                }
            }
            '~' => {
                if chars.peek() == Some(&'~') {
                    // `~~strike~~` → `~strike~` (consume second ~, output single ~)
                    chars.next();
                    result.push('~');
                } else {
                    result.push('~');
                }
            }
            _ => {
                result.push(ch);
            }
        }
    }

    result
}

#[cfg(test)]
mod slack_tests {
    use super::*;

    #[test]
    fn test_slack_bold() {
        assert_eq!(markdown_to_slack_mrkdwn("**bold**"), "*bold*");
    }

    #[test]
    fn test_slack_italic() {
        assert_eq!(markdown_to_slack_mrkdwn("*italic*"), "_italic_");
    }

    #[test]
    fn test_slack_strike() {
        assert_eq!(markdown_to_slack_mrkdwn("~~strike~~"), "~strike~");
    }

    #[test]
    fn test_slack_code_preserved() {
        assert_eq!(markdown_to_slack_mrkdwn("`code`"), "`code`");
    }

    #[test]
    fn test_slack_code_block_preserved() {
        let input = "```rust\nfn main() {}\n```";
        assert_eq!(markdown_to_slack_mrkdwn(input), input);
    }

    #[test]
    fn test_slack_mixed() {
        assert_eq!(
            markdown_to_slack_mrkdwn("**bold** and *italic*"),
            "*bold* and _italic_"
        );
    }
}
