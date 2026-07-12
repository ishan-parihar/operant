//! Markdown -> ratatui lines renderer used by transcript message families.

use crate::tui::figures;
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use regex::Regex;
use std::sync::LazyLock;
use syntect::easy::HighlightLines;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;
use unicode_width::UnicodeWidthStr;

/// Syntect syntax set — ships with the default grammar bundle (covers ~120
/// languages including rust, python, js/ts, go, c/c++, java, ruby, sql, yaml,
/// json, toml, markdown, bash, html, css, etc.). Loaded once per process.
static SYNTAX_SET: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);

/// Syntect theme set — `base16-ocean.dark` matches the diff viewer's choice so
/// inline code blocks and diff hunks read as one design system.
static THEME_SET: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);

/// Regex pattern to detect URLs (http://, https://, ftp://, www.)
static URL_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:https?|ftp)://\S+|www\.\S+").expect("Invalid URL regex pattern")
});

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

/// Render markdown text to styled ratatui lines.
pub fn render_markdown(text: &str, width: u16) -> Vec<Line<'static>> {
    // Strip bare \r (carriage return) characters as a safety net.
    // These corrupt terminal display by moving the cursor to column 0.
    // Primary stripping happens in process_stream, but this catches any
    // \r that enters through other paths (e.g. committed messages).
    let text = text.replace('\r', "");
    let all_lines: Vec<&str> = text.lines().collect();
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut in_code_block = false;
    let mut code_lang = String::new();
    // Resolved syntax reference for the current code block — computed once
    // when the fence opens, reused for every line until the fence closes.
    let mut code_syntax: Option<&'static syntect::parsing::SyntaxReference> = None;
    let mut idx = 0;

    while idx < all_lines.len() {
        let raw = all_lines[idx];
        if raw.trim_start().starts_with("```") {
            if in_code_block {
                lines.push(Line::from(vec![Span::styled(
                    "  └──────────────────────────────────────────────────".to_string(),
                    Style::default().fg(Color::Yellow),
                )]));
                in_code_block = false;
                code_lang.clear();
                code_syntax = None;
            } else {
                in_code_block = true;
                code_lang = raw.trim_start().trim_start_matches('`').trim().to_string();
                code_syntax = resolve_syntax(&code_lang);
                let lang_label = if code_lang.is_empty() {
                    String::new()
                } else {
                    format!(" {} ", code_lang)
                };
                lines.push(Line::from(vec![Span::styled(
                    format!("  ┌──────────────────────{}", lang_label),
                    Style::default().fg(Color::Yellow),
                )]));
            }
            idx += 1;
            continue;
        }

        if in_code_block {
            let base_style = Style::default().fg(Color::White);
            let code_spans = highlight_code_line_spans(raw, code_syntax, base_style);
            let mut spans: Vec<Span<'static>> = Vec::with_capacity(code_spans.len() + 1);
            spans.push(Span::styled("  │ ", Style::default().fg(Color::Yellow)));
            spans.extend(code_spans);
            lines.push(Line::from(spans));
            idx += 1;
            continue;
        }

        // Check for markdown tables
        if let Some((table, end_idx)) = super::markdown_enhanced::detect_table(&all_lines, idx) {
            lines.extend(super::markdown_enhanced::render_table(&table));
            idx = end_idx;
            continue;
        }

        if let Some(quoted) = raw.strip_prefix("> ") {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {} ", figures::BLOCKQUOTE_BAR),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(quoted.to_string(), Style::default().fg(Color::DarkGray)),
            ]));
            idx += 1;
            continue;
        }

        if let Some(rest) = raw.strip_prefix("### ") {
            lines.push(Line::from(vec![Span::styled(
                format!("  {}", rest),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )]));
            idx += 1;
            continue;
        }
        if let Some(rest) = raw.strip_prefix("## ") {
            lines.push(Line::from(vec![Span::styled(
                format!("  {}", rest),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )]));
            idx += 1;
            continue;
        }
        if let Some(rest) = raw.strip_prefix("# ") {
            lines.push(Line::from(vec![Span::styled(
                format!("  {}", rest),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD | Modifier::ITALIC | Modifier::UNDERLINED),
            )]));
            idx += 1;
            continue;
        }

        let padded = format!("  {}", raw);
        let effective_width = width.saturating_sub(4) as usize;
        for wrapped_line in word_wrap(&padded, effective_width) {
            let spans = parse_inline_spans(wrapped_line);
            lines.push(Line::from(spans));
        }

        idx += 1;
    }

    if in_code_block {
        lines.push(Line::from(vec![Span::styled(
            "  └──────────────────────────────────────────────────".to_string(),
            Style::default().fg(Color::Yellow),
        )]));
    }

    lines
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

        // Add the URL with special styling (cyan with underline)
        let url_text = url_match.as_str();
        spans.push(Span::styled(
            url_text.to_string(),
            Style::default()
                .fg(Color::Cyan)
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

            // Add the email with special styling (cyan with underline)
            let email_text = email_match.as_str();
            spans.push(Span::styled(
                email_text.to_string(),
                Style::default()
                    .fg(Color::Cyan)
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

fn parse_inline_spans(text: String) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut remaining = text.as_str();

    while !remaining.is_empty() {
        let bold_pos = remaining.find("**");
        let code_pos = remaining.find('`');

        match (bold_pos, code_pos) {
            (None, None) => {
                // No more formatting, but check for links/emails in plain text
                spans.extend(split_and_style_links(remaining));
                break;
            }
            (Some(b), Some(c)) if c < b => {
                // Code block comes first
                if c > 0 {
                    spans.extend(split_and_style_links(&remaining[..c]));
                }
                let after_tick = &remaining[c + 1..];
                if let Some(end) = after_tick.find('`') {
                    spans.push(Span::styled(
                        after_tick[..end].to_string(),
                        Style::default().fg(Color::Yellow),
                    ));
                    remaining = &after_tick[end + 1..];
                } else {
                    spans.push(Span::raw(remaining[c..].to_string()));
                    break;
                }
            }
            (Some(b), _) => {
                // Bold comes first
                if b > 0 {
                    spans.extend(split_and_style_links(&remaining[..b]));
                }
                let after_stars = &remaining[b + 2..];
                if let Some(end) = after_stars.find("**") {
                    spans.push(Span::styled(
                        after_stars[..end].to_string(),
                        Style::default().add_modifier(Modifier::BOLD),
                    ));
                    remaining = &after_stars[end + 2..];
                } else {
                    // Unmatched opening `**` — skip the markers and render the
                    // rest as plain text.  This prevents literal `**` appearing
                    // at the end of reasoning blocks when the model ends a
                    // thought mid-bold, or when word-wrap splits a bold span
                    // across lines.
                    spans.extend(split_and_style_links(after_stars));
                    break;
                }
            }
            (None, Some(c)) => {
                // Code block (no bold)
                if c > 0 {
                    spans.extend(split_and_style_links(&remaining[..c]));
                }
                let after_tick = &remaining[c + 1..];
                if let Some(end) = after_tick.find('`') {
                    spans.push(Span::styled(
                        after_tick[..end].to_string(),
                        Style::default().fg(Color::Yellow),
                    ));
                    remaining = &after_tick[end + 1..];
                } else {
                    spans.push(Span::raw(remaining[c..].to_string()));
                    break;
                }
            }
        }
    }

    if spans.is_empty() {
        spans.push(Span::raw(String::new()));
    }
    spans
}

fn word_wrap(text: &str, width: usize) -> Vec<String> {
    if width == 0 || UnicodeWidthStr::width(text) <= width {
        return vec![text.to_string()];
    }

    let mut result = Vec::new();
    let mut current_line = String::new();
    let mut current_width = 0usize;

    let push_long_word = |word: &str,
                          result: &mut Vec<String>,
                          current_line: &mut String,
                          current_width: &mut usize| {
        // Hard-break a word that on its own exceeds `width` (e.g. URLs).
        if !current_line.is_empty() {
            result.push(std::mem::take(current_line));
            *current_width = 0;
        }
        let mut chunk = String::new();
        let mut chunk_w = 0usize;
        for ch in word.chars() {
            let cw = UnicodeWidthStr::width(ch.to_string().as_str());
            if chunk_w + cw > width && !chunk.is_empty() {
                result.push(std::mem::take(&mut chunk));
                chunk_w = 0;
            }
            chunk.push(ch);
            chunk_w += cw;
        }
        if !chunk.is_empty() {
            *current_line = chunk;
            *current_width = chunk_w;
        }
    };

    for word in text.split_whitespace() {
        let word_w = UnicodeWidthStr::width(word);
        if word_w > width {
            push_long_word(word, &mut result, &mut current_line, &mut current_width);
            continue;
        }
        if current_width == 0 {
            current_line.push_str(word);
            current_width = word_w;
        } else if current_width + 1 + word_w <= width {
            current_line.push(' ');
            current_line.push_str(word);
            current_width += 1 + word_w;
        } else {
            result.push(std::mem::take(&mut current_line));
            current_line.push_str(word);
            current_width = word_w;
        }
    }

    if !current_line.is_empty() {
        result.push(current_line);
    }
    if result.is_empty() {
        result.push(text.to_string());
    }
    result
}
