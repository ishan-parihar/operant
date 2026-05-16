//! Convert markdown text to Telegram-compatible HTML.
//!
//! This module provides a single public function [`markdown_to_telegram_html`]
//! that converts AI-generated markdown text into Telegram-compatible HTML
//! format. It handles bold, italic, code blocks, inline code, headers,
//! strikethrough, links, and blockquotes.
//!
//! Markdown inside code blocks and inline code spans is preserved verbatim.
//! All remaining HTML special characters are escaped to prevent injection.

use lazy_static::lazy_static;
use regex::Regex;

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

    /// Extract fenced code blocks (` ``` … ``` `) and replace them with
    /// sentinel-bounded placeholders. The optional language tag (e.g.
    /// `rust`) is discarded.
    fn extract_code_blocks(&mut self, text: &str) -> String {
        lazy_static! {
            static ref RE: Regex = Regex::new(r"```(\w*)\n([\s\S]*?)```").unwrap();
        }
        let mut result = String::with_capacity(text.len());
        let mut last = 0;
        for caps in RE.captures_iter(text) {
            let m = caps.get(0).unwrap();
            // Push the text between the previous match and this one.
            result.push_str(&text[last..m.start()]);
            // Store the raw content and emit a placeholder.
            let content = caps.get(2).unwrap().as_str().to_string();
            let idx = self.code_blocks.len();
            self.code_blocks.push(content);
            result.push_str(&make_placeholder("CODE_BLOCK", idx));
            last = m.end();
        }
        result.push_str(&text[last..]);
        result
    }

    /// Extract inline code spans (`` `code` ``) and replace them with
    /// sentinel-bounded placeholders.
    fn extract_inline_code(&mut self, text: &str) -> String {
        lazy_static! {
            static ref RE: Regex = Regex::new(r"`([^`]+?)`").unwrap();
        }
        let mut result = String::with_capacity(text.len());
        let mut last = 0;
        for caps in RE.captures_iter(text) {
            let m = caps.get(0).unwrap();
            result.push_str(&text[last..m.start()]);
            let content = caps.get(1).unwrap().as_str().to_string();
            let idx = self.inline_codes.len();
            self.inline_codes.push(content);
            result.push_str(&make_placeholder("INLINE_CODE", idx));
            last = m.end();
        }
        result.push_str(&text[last..]);
        result
    }

    /// Reinsert all protected inline code and code block content, wrapping
    /// each in the appropriate Telegram HTML tag.
    fn reinsert_all(&self, text: &str) -> String {
        let mut result = text.to_string();
        // Inline codes first, then code blocks (groups are disjoint so
        // the order of global replacement does not matter).
        for (i, code) in self.inline_codes.iter().enumerate() {
            let ph = make_placeholder("INLINE_CODE", i);
            result = result.replace(&ph, &format!("<code>{}</code>", code));
        }
        for (i, code) in self.code_blocks.iter().enumerate() {
            let ph = make_placeholder("CODE_BLOCK", i);
            result = result.replace(&ph, &format!("<pre>{}</pre>", code));
        }
        result
    }
}

// ---------------------------------------------------------------------------
// Markdown → Telegram HTML conversions (operates on already-extracted text)
// ---------------------------------------------------------------------------

lazy_static! {
    /// Bold: `**text**` → `<b>text</b>`
    static ref BOLD_RE: Regex = Regex::new(r"\*\*(.+?)\*\*").unwrap();
    /// Italic: `*text*` → `<i>text</i>`
    static ref ITALIC_RE: Regex = Regex::new(r"\*(.+?)\*").unwrap();
    /// Strikethrough: `~~text~~` → `<s>text</s>`
    static ref STRIKETHROUGH_RE: Regex = Regex::new(r"~~(.+?)~~").unwrap();
    /// Link: `[text](url)` → `<a href="url">text</a>`
    static ref LINK_RE: Regex = Regex::new(r"\[(.+?)\]\((.+?)\)").unwrap();
    /// Header: `## text` → `<b>text</b>\n`  (multiline)
    static ref HEADER_RE: Regex = Regex::new(r"(?m)^#{1,6}\s+(.*?)$").unwrap();
    /// Blockquote: `> text` → `<blockquote>text</blockquote>`  (multiline)
    static ref BLOCKQUOTE_RE: Regex =
        Regex::new(r"(?m)^>\s?(.*?)$").unwrap();
}

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

/// Convert markdown constructs to Telegram HTML in text whose code spans
/// have already been extracted into placeholders.
fn convert_markdown(text: &str) -> String {
    // Bold first so `**text**` becomes `<b>text</b>`, preventing the
    // inner `*` from being matched by the italic pattern.
    let s = BOLD_RE.replace_all(text, "<b>$1</b>");
    // Italic now only sees standalone `*text*` spans.
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
/// 1. Escape raw HTML special characters (`&`, `<`, `>`, `"`, `'`)
/// 2. Extract fenced code blocks → placeholders (protects from further
///    markdown & HTML processing)
/// 3. Extract inline code spans → placeholders
/// 4. Apply markdown conversions: bold → italic → strikethrough → links →
///    headers → blockquotes
/// 5. Reinsert protected inline code and code blocks wrapped in Telegram
///    HTML tags
///
/// This ordering guarantees that markdown inside code fences or backtick
/// spans is never accidentally converted.
pub fn markdown_to_telegram_html(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }

    // 1. Escape HTML special characters (prevent injection).
    let escaped = escape_html(text);

    // 2. Extract and protect code blocks.
    let mut conv = MarkdownConverter::new();
    let no_blocks = conv.extract_code_blocks(&escaped);

    // 3. Extract and protect inline code.
    let no_code = conv.extract_inline_code(&no_blocks);

    // 4. Convert remaining markdown to Telegram HTML.
    let html = convert_markdown(&no_code);

    // 5. Reinsert protected content wrapped in `<pre>` / `<code>`.
    conv.reinsert_all(&html)
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
}
