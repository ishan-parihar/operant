//! Code filtering: the direct answer to a raw-source read.
//!
//! Measured: `aft_read` returns up to 2000 lines of raw source — roughly
//! 10-25K tokens per call — and nothing in operant compresses it. This module
//! is that compression.
//!
//! Two levels, one scanner:
//!
//! * [`CodeLevel::Minimal`] removes comments. The code is still code.
//! * [`CodeLevel::Aggressive`] additionally removes function and method
//!   bodies, leaving the signature and a marker that names the line range the
//!   body covered.
//!
//! The scanner is **string-aware**: a `//` inside a string literal is not a
//! comment, and that is the classic bug in every hand-rolled comment
//! stripper. Each language gets its own string rules (raw strings, char
//! literals, lifetimes, template literals, Python's triple quotes).
//!
//! **Contract:** stripping comments and bodies loses information. Both
//! filters therefore declare `lossy = true` and `recovery = Recovery::Recall`;
//! the full text must be in the recall store before either runs. That is why
//! they live here and not in `lossless.rs`.

use super::super::types::{
    Block, ContentFormat, CostClass, Method, MethodCtx, MethodInfo, Outcome, Recovery,
};

/// How hard the code filter works.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeLevel {
    /// Strip comments; every statement survives.
    Minimal,
    /// Strip comments and function bodies; the signatures survive.
    Aggressive,
}

/// Strip comments from source code.
#[derive(Debug, Clone, Copy, Default)]
pub struct CodeFilter;

/// Strip comments and function bodies from source code.
#[derive(Debug, Clone, Copy, Default)]
pub struct CodeFilterAggressive;

/// The contract both code filters declare.
///
/// Stripping is lossy, so the full text must be recoverable from the recall
/// store; the filter never claims otherwise.
fn code_info() -> MethodInfo {
    MethodInfo {
        lossy: true,
        recovery: Recovery::Recall,
        consumes: &[ContentFormat::Code],
        cost: CostClass::Fast,
        ..MethodInfo::LOSSY_RECALLABLE
    }
}

impl Method for CodeFilter {
    fn name(&self) -> &'static str {
        "code-filter"
    }
    fn info(&self) -> MethodInfo {
        code_info()
    }
    fn apply(&self, block: &Block, _ctx: &MethodCtx) -> Outcome {
        let language = block.origin.path.as_deref();
        filter(block.text, CodeLevel::Minimal, language).map_or(Outcome::Skip, Outcome::Continue)
    }
}

impl Method for CodeFilterAggressive {
    fn name(&self) -> &'static str {
        "code-filter-aggressive"
    }
    fn info(&self) -> MethodInfo {
        code_info()
    }
    fn apply(&self, block: &Block, _ctx: &MethodCtx) -> Outcome {
        let language = block.origin.path.as_deref();
        filter(block.text, CodeLevel::Aggressive, language).map_or(Outcome::Skip, Outcome::Continue)
    }
}

/// Filter `text` at `level`.
///
/// `language` is a file path or a language name; the extension picks the
/// grammar. `None` (or an unknown name) falls back to a conservative text
/// mode that only removes block comments and whole-line `//` comments, so
/// prose and URLs are never mangled.
///
/// Returns `None` when nothing was removed, or when the result would not be
/// smaller than the input.
pub fn filter(text: &str, level: CodeLevel, language: Option<&str>) -> Option<String> {
    if text.is_empty() {
        return None;
    }
    let lang = detect(language, text);
    let spans = scan(text, lang);
    let comments: Vec<Span> = spans
        .iter()
        .filter(|span| span.kind == SpanKind::Comment)
        .copied()
        .collect();
    let result = match level {
        CodeLevel::Minimal => apply_edits(text, &comments, &[]),
        CodeLevel::Aggressive => {
            let replacements = find_bodies(text, lang, &spans);
            apply_edits(text, &comments, &replacements)
        }
    };
    if result == text {
        return None;
    }
    if result.len() >= text.len() {
        return None;
    }
    Some(result)
}

/// The languages the scanner knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lang {
    Rust,
    Python,
    JavaScript,
    TypeScript,
    Go,
    C,
    Cpp,
    Java,
    /// No known grammar: only unambiguous comment forms are removed.
    Text,
}

/// Pick the grammar for a path or a language name.
///
/// A path is the strongest signal, so it wins. When there is no path the
/// content is sniffed, because the evidence layer has no file path to offer:
/// the payload arrives as text with a tool name. A file that matches no
/// signal stays in text mode, where only unambiguous comment forms go.
fn detect(hint: Option<&str>, text: &str) -> Lang {
    if let Some(hint) = hint {
        let lower = hint.to_ascii_lowercase();
        if let Some(dot) = lower.rfind('.') {
            match &lower[dot + 1..] {
                "rs" => return Lang::Rust,
                "py" | "pyi" => return Lang::Python,
                "js" | "jsx" | "mjs" | "cjs" => return Lang::JavaScript,
                "ts" | "tsx" | "mts" | "cts" => return Lang::TypeScript,
                "go" => return Lang::Go,
                "c" | "h" => return Lang::C,
                "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => return Lang::Cpp,
                "java" => return Lang::Java,
                _ => {}
            }
        }
        match lower.as_str() {
            "rust" | "rs" => return Lang::Rust,
            "python" | "py" => return Lang::Python,
            "javascript" | "js" | "node" => return Lang::JavaScript,
            "typescript" | "ts" => return Lang::TypeScript,
            "go" | "golang" => return Lang::Go,
            "c" => return Lang::C,
            "cpp" | "c++" | "cxx" => return Lang::Cpp,
            "java" => return Lang::Java,
            _ => {}
        }
    }
    sniff(text).unwrap_or(Lang::Text)
}

/// Guess the language from the content.
///
/// The signals are deliberately narrow: a wrong guess would strip text that
/// was never a comment, so anything that matches no signal stays in text
/// mode.
fn sniff(text: &str) -> Option<Lang> {
    let mut end = text.len().min(4096);
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    let head = &text[..end];
    let starts = |prefix: &str| {
        head.lines()
            .any(|line| line.trim_start().starts_with(prefix))
    };
    if head.contains("use std::") || starts("fn ") || starts("impl ") {
        return Some(Lang::Rust);
    }
    if starts("def ") || starts("async def ") {
        return Some(Lang::Python);
    }
    if head.contains("#include <") {
        let cpp =
            head.contains("iostream") || head.contains("namespace ") || head.contains("std::");
        return Some(if cpp { Lang::Cpp } else { Lang::C });
    }
    if head.contains("import java.")
        || head.contains("public class ")
        || head.contains("public static ")
    {
        return Some(Lang::Java);
    }
    if starts("func ") || head.contains("package main") {
        return Some(Lang::Go);
    }
    if head.contains("export interface ") || head.contains(": string") || head.contains(": number")
    {
        return Some(Lang::TypeScript);
    }
    if head.contains("export default ") || head.contains("import React") || starts("function ") {
        return Some(Lang::JavaScript);
    }
    None
}

/// What a scanned span is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpanKind {
    /// A string, character, or raw-string literal: never a comment.
    Literal,
    /// A comment: removed by the filter.
    Comment,
}

/// A byte range in the source.
#[derive(Debug, Clone, Copy)]
struct Span {
    start: usize,
    end: usize,
    kind: SpanKind,
}

/// Scan `text` for literal and comment spans.
///
/// Literals are recognised **before** comments: a `#` inside a Python string
/// and a `//` inside a JavaScript string are content, not comments. That
/// ordering is the whole trick.
fn scan(text: &str, lang: Lang) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut i = 0usize;
    while i < text.len() {
        if let Some(end) = literal_end(text, i, lang) {
            spans.push(Span {
                start: i,
                end,
                kind: SpanKind::Literal,
            });
            i = end;
            continue;
        }
        if let Some(end) = line_comment_end(text, i, lang) {
            spans.push(Span {
                start: i,
                end,
                kind: SpanKind::Comment,
            });
            i = end;
            continue;
        }
        if let Some(end) = block_comment_end(text, i, lang) {
            spans.push(Span {
                start: i,
                end,
                kind: SpanKind::Comment,
            });
            i = end;
            continue;
        }
        i += text[i..].chars().next().map_or(1, char::len_utf8);
    }
    spans
}

/// The end of a line comment starting at `i`, if there is one.
fn line_comment_end(text: &str, i: usize, lang: Lang) -> Option<usize> {
    let token = match lang {
        Lang::Python => "#",
        _ => "//",
    };
    if !text[i..].starts_with(token) {
        return None;
    }
    // In text mode a `//` only opens a comment at the start of a line: prose
    // and URLs (`https://example.com`) must survive untouched.
    if lang == Lang::Text && !at_line_start(text, i) {
        return None;
    }
    Some(text[i..].find('\n').map_or(text.len(), |p| i + p))
}

/// The end of a block comment starting at `i`, if there is one.
fn block_comment_end(text: &str, i: usize, lang: Lang) -> Option<usize> {
    if lang == Lang::Python {
        return None;
    }
    if !text[i..].starts_with("/*") {
        return None;
    }
    let bytes = text.as_bytes();
    let mut depth = 1usize;
    let mut j = i + 2;
    while j < bytes.len() {
        // Rust nests block comments; the others do not.
        if lang == Lang::Rust && bytes[j] == b'/' && bytes.get(j + 1) == Some(&b'*') {
            depth += 1;
            j += 2;
            continue;
        }
        if bytes[j] == b'*' && bytes.get(j + 1) == Some(&b'/') {
            depth -= 1;
            j += 2;
            if depth == 0 {
                return Some(j);
            }
            continue;
        }
        j += 1;
    }
    // An unterminated comment runs to the end of the file.
    Some(text.len())
}

/// Whether only whitespace precedes `i` on its line.
fn at_line_start(text: &str, i: usize) -> bool {
    let bytes = text.as_bytes();
    let mut j = i;
    while j > 0 {
        match bytes[j - 1] {
            b'\n' => return true,
            b' ' | b'\t' | b'\r' => j -= 1,
            _ => return false,
        }
    }
    true
}

/// The end of a `"`- or `'`-delimited literal that honours backslash escapes.
///
/// The literal never spans a line: an unterminated quote is a syntax error in
/// every language here, and swallowing the rest of the file over it would be
/// worse than leaving it alone.
fn quoted_end(text: &str, i: usize, quote: u8) -> Option<usize> {
    let bytes = text.as_bytes();
    if bytes.get(i) != Some(&quote) {
        return None;
    }
    let mut j = i + 1;
    while j < bytes.len() {
        match bytes[j] {
            b'\\' => j += 2,
            b'\n' => return None,
            b if b == quote => return Some(j + 1),
            _ => j += 1,
        }
    }
    None
}

/// The end of a raw literal that has no escape processing.
fn raw_end(text: &str, from: usize, close: &str) -> Option<usize> {
    let at = text.get(from..)?.find(close)? + from;
    Some(at + close.len())
}

/// The end of a literal starting at `i`, for this language.
fn literal_end(text: &str, i: usize, lang: Lang) -> Option<usize> {
    match lang {
        Lang::Rust => rust_literal_end(text, i),
        Lang::Python => python_literal_end(text, i),
        Lang::JavaScript | Lang::TypeScript => js_literal_end(text, i),
        Lang::Go => go_literal_end(text, i),
        Lang::C => c_literal_end(text, i),
        Lang::Cpp => cpp_literal_end(text, i),
        Lang::Java => java_literal_end(text, i),
        // Text mode only knows double quotes: an apostrophe in prose is not a
        // string delimiter.
        Lang::Text => quoted_end(text, i, b'"'),
    }
}

/// The end of a Rust literal: raw strings, byte strings, chars, lifetimes.
fn rust_literal_end(text: &str, i: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut p = i;
    if bytes.get(p) == Some(&b'b') {
        p += 1;
    }
    if bytes.get(p) == Some(&b'r') {
        let mut q = p + 1;
        let mut hashes = 0usize;
        while bytes.get(q) == Some(&b'#') {
            hashes += 1;
            q += 1;
        }
        if bytes.get(q) == Some(&b'"') {
            let close = format!("\"{}", "#".repeat(hashes));
            return raw_end(text, q + 1, &close);
        }
    }
    match bytes.get(i) {
        Some(&b'"') => quoted_end(text, i, b'"'),
        Some(&b'\'') => rust_char_end(text, i),
        _ => None,
    }
}

/// The end of a Rust character literal.
///
/// `'a'` is a character; `'a` is a lifetime. Treating every apostrophe as a
/// character literal would swallow the rest of the file.
fn rust_char_end(text: &str, i: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut p = i;
    if bytes.get(p) == Some(&b'b') {
        p += 1;
    }
    if bytes.get(p) != Some(&b'\'') {
        return None;
    }
    let start = p + 1;
    match bytes.get(start).copied() {
        Some(b'\\') => {
            let mut q = start + 1;
            while q < bytes.len() && bytes[q] != b'\'' && bytes[q] != b'\n' {
                q += 1;
            }
            if bytes.get(q) == Some(&b'\'') {
                Some(q + 1)
            } else {
                None
            }
        }
        Some(_) => {
            let ch = text.get(start..)?.chars().next()?;
            let after = start + ch.len_utf8();
            if bytes.get(after) == Some(&b'\'') {
                Some(after + 1)
            } else {
                None
            }
        }
        None => None,
    }
}

/// The end of a Python literal, including the `r`/`b`/`f`/`u` prefixes and
/// the triple-quoted forms.
fn python_literal_end(text: &str, i: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut p = i;
    let mut prefix = 0usize;
    while prefix < 2
        && bytes
            .get(p)
            .is_some_and(|b| matches!(*b, b'r' | b'R' | b'b' | b'B' | b'f' | b'F' | b'u' | b'U'))
    {
        p += 1;
        prefix += 1;
    }
    let quote = bytes.get(p).copied()?;
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    if bytes.get(p + 1) == Some(&quote) && bytes.get(p + 2) == Some(&quote) {
        let close = [quote, quote, quote];
        let tail = bytes.get(p + 3..)?;
        let at = tail.windows(3).position(|w| w == &close[..])? + p + 3;
        return Some(at + 3);
    }
    quoted_end(text, p, quote)
}

/// The end of a JavaScript or TypeScript literal.
fn js_literal_end(text: &str, i: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    match bytes.get(i) {
        Some(&b'"') => quoted_end(text, i, b'"'),
        Some(&b'\'') => quoted_end(text, i, b'\''),
        Some(&b'`') => {
            // A template literal may span lines. `${...}` interpolation is
            // not parsed, so the literal ends at the next unescaped backtick.
            let mut j = i + 1;
            while j < bytes.len() {
                match bytes[j] {
                    b'\\' => j += 2,
                    b'`' => return Some(j + 1),
                    _ => j += 1,
                }
            }
            None
        }
        _ => None,
    }
}

/// The end of a Go literal.
fn go_literal_end(text: &str, i: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    match bytes.get(i) {
        Some(&b'"') => quoted_end(text, i, b'"'),
        Some(&b'`') => {
            // A Go raw string has no escapes and may span lines.
            let mut j = i + 1;
            while j < bytes.len() {
                if bytes[j] == b'`' {
                    return Some(j + 1);
                }
                j += 1;
            }
            None
        }
        Some(&b'\'') => quoted_end(text, i, b'\''),
        _ => None,
    }
}

/// The end of a C literal.
fn c_literal_end(text: &str, i: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    match bytes.get(i) {
        Some(&b'"') => quoted_end(text, i, b'"'),
        Some(&b'\'') => quoted_end(text, i, b'\''),
        _ => None,
    }
}

/// The end of a C++ literal, including the `R"delim(...)delim"` raw string.
fn cpp_literal_end(text: &str, i: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    if bytes.get(i) == Some(&b'R') && bytes.get(i + 1) == Some(&b'"') {
        let mut j = i + 2;
        while j < bytes.len() && bytes[j] != b'(' {
            j += 1;
        }
        if j >= bytes.len() {
            return None;
        }
        let delim = text.get(i + 2..j)?;
        let close = format!("){delim}");
        return raw_end(text, j + 1, &close);
    }
    c_literal_end(text, i)
}

/// The end of a Java literal, including the `"""` text block.
fn java_literal_end(text: &str, i: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    if bytes.get(i) == Some(&b'"') {
        if bytes.get(i + 1) == Some(&b'"') && bytes.get(i + 2) == Some(&b'"') {
            let tail = bytes.get(i + 3..)?;
            let at = tail.windows(3).position(|w| w == b"\"\"\"")? + i + 3;
            return Some(at + 3);
        }
        return quoted_end(text, i, b'"');
    }
    if bytes.get(i) == Some(&b'\'') {
        return quoted_end(text, i, b'\'');
    }
    None
}

/// A byte range replaced by new text.
struct Replacement {
    start: usize,
    end: usize,
    text: String,
}

/// The marker that stands in for an elided body.
///
/// It is a **string literal**, not a comment: the filter must survive being
/// run over its own output, and a comment would be stripped on the second
/// pass.
fn body_marker(first_line: usize, last_line: usize) -> String {
    format!("\"… body elided: lines {first_line}-{last_line} …\"")
}

/// Whether a body is already an elision marker.
fn is_body_marker(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed.starts_with('"') && trimmed.contains("body elided: lines")
}

/// The 1-based line number of a byte offset.
fn line_of(text: &str, offset: usize) -> usize {
    text.get(..offset)
        .map_or(1, |head| head.matches('\n').count() + 1)
}

/// The newlines inside a range, kept so the line numbering survives.
fn newlines_of(text: &str) -> String {
    text.chars().filter(|ch| *ch == '\n').collect()
}

/// Apply the comment removals and the body replacements to `text`.
///
/// A comment inside an elided body goes with the body: the body's own
/// newlines already keep the line count right.
fn apply_edits(text: &str, comments: &[Span], replacements: &[Replacement]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0usize;
    let mut index = 0usize;
    for replacement in replacements {
        while index < comments.len() && comments[index].end <= replacement.start {
            let comment = comments[index];
            out.push_str(&text[cursor..comment.start]);
            out.push_str(&newlines_of(&text[comment.start..comment.end]));
            cursor = comment.end;
            index += 1;
        }
        while index < comments.len() && comments[index].start < replacement.end {
            index += 1;
        }
        out.push_str(&text[cursor..replacement.start]);
        out.push_str(&replacement.text);
        cursor = replacement.end;
    }
    while index < comments.len() {
        let comment = comments[index];
        out.push_str(&text[cursor..comment.start]);
        out.push_str(&newlines_of(&text[comment.start..comment.end]));
        cursor = comment.end;
        index += 1;
    }
    out.push_str(&text[cursor..]);
    out
}

/// A forward cursor over the protected spans (literals and comments).
///
/// Braces and parentheses inside a literal or a comment are not code, so the
/// body finder walks past them instead of counting them.
struct Protected<'a> {
    spans: &'a [Span],
    index: usize,
}

impl<'a> Protected<'a> {
    fn new(spans: &'a [Span]) -> Self {
        Protected { spans, index: 0 }
    }

    /// The end of the protected span covering `i`, if there is one.
    ///
    /// A failed match leaves the cursor ahead of the scan position, so the
    /// cursor is moved back when that happens: otherwise a literal could be
    /// read as code.
    fn end_at(&mut self, i: usize) -> Option<usize> {
        while self.index + 1 < self.spans.len() && self.spans[self.index + 1].start <= i {
            self.index += 1;
        }
        while self.index > 0 && self.spans[self.index].start > i {
            self.index -= 1;
        }
        let span = self.spans.get(self.index)?;
        if span.start <= i && i < span.end {
            Some(span.end)
        } else {
            None
        }
    }
}

/// Find the function bodies in a brace language.
fn find_bodies(text: &str, lang: Lang, spans: &[Span]) -> Vec<Replacement> {
    if lang == Lang::Python {
        return python_bodies(text);
    }
    let mut out = Vec::new();
    let mut prot = Protected::new(spans);
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if let Some(end) = prot.end_at(i) {
            i = end;
            continue;
        }
        if bytes[i] != b'(' {
            i += 1;
            continue;
        }
        let Some(close) = matching_paren(text, i, &mut prot) else {
            i += 1;
            continue;
        };
        let ident = identifier_before(text, i);
        let Some((brace, arrow)) = body_brace(text, close + 1, &mut prot) else {
            i += 1;
            continue;
        };
        if !is_definition(lang, text, &ident, arrow) {
            i += 1;
            continue;
        }
        let Some(end_brace) = matching_brace(text, brace, &mut prot) else {
            i += 1;
            continue;
        };
        let start = brace + 1;
        let end = end_brace;
        let body = &text[start..end];
        // An empty body has nothing to elide, and a body that is already a
        // marker is left alone so the filter stays idempotent.
        if body.trim().is_empty() || is_body_marker(body) {
            i = end_brace + 1;
            continue;
        }
        let trimmed_start = start + (body.len() - body.trim_start().len());
        let trimmed_end = end - (body.len() - body.trim_end().len());
        let first_line = line_of(text, trimmed_start);
        let last_line = line_of(text, trimmed_end);
        let replacement = body_replacement(text, start, end, first_line, last_line);
        // A body shorter than its marker is not worth eliding: the filter must
        // never make the payload bigger.
        if replacement.len() < end - start {
            out.push(Replacement {
                start,
                end,
                text: replacement,
            });
        }
        i = end_brace + 1;
    }
    out
}

/// The replacement text for an elided body.
///
/// The body's newlines are kept so every line after the body keeps its
/// number.
fn body_replacement(
    text: &str,
    start: usize,
    end: usize,
    first_line: usize,
    last_line: usize,
) -> String {
    let body = &text[start..end];
    let newlines = body.matches('\n').count();
    let leading = body
        .find(|ch: char| !ch.is_whitespace())
        .map_or(0, |at| body[..at].matches('\n').count());
    let mut out = String::new();
    out.push_str(&"\n".repeat(leading));
    out.push_str(&body_marker(first_line, last_line));
    out.push_str(&"\n".repeat(newlines.saturating_sub(leading)));
    out
}

/// The offset of the `)` that closes the `(` at `open`.
fn matching_paren(text: &str, open: usize, prot: &mut Protected<'_>) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 0i32;
    let mut i = open;
    while i < bytes.len() {
        if let Some(end) = prot.end_at(i) {
            i = end;
            continue;
        }
        match bytes[i] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// The offset of the `}` that closes the `{` at `open`.
fn matching_brace(text: &str, open: usize, prot: &mut Protected<'_>) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 0i32;
    let mut i = open;
    while i < bytes.len() {
        if let Some(end) = prot.end_at(i) {
            i = end;
            continue;
        }
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// The `{` that opens a body after a parameter list, and whether an `=>`
/// arrow was seen on the way.
///
/// The tokens allowed between the `)` and the `{` are the ones that appear in
/// a signature tail: a return type, `const`, `noexcept`, `throws`, a Rust
/// `where` clause. An `=` stops the scan, so `if let Some(x) = y {` is not
/// mistaken for a definition.
fn body_brace(text: &str, after: usize, prot: &mut Protected<'_>) -> Option<(usize, bool)> {
    let bytes = text.as_bytes();
    let mut i = after;
    let mut newlines = 0usize;
    let mut steps = 0usize;
    let mut arrow = false;
    while i < bytes.len() && steps < 256 {
        steps += 1;
        if let Some(end) = prot.end_at(i) {
            i = end;
            continue;
        }
        let ch = text.get(i..)?.chars().next()?;
        if ch == '{' {
            return Some((i, arrow));
        }
        if ch == '\n' {
            newlines += 1;
            if newlines > 1 {
                return None;
            }
            i += 1;
            continue;
        }
        if ch == '=' && bytes.get(i + 1) == Some(&b'>') {
            arrow = true;
            i += 2;
            continue;
        }
        if ch.is_alphanumeric()
            || matches!(
                ch,
                '_' | ':'
                    | '<'
                    | '>'
                    | ','
                    | '.'
                    | '&'
                    | '*'
                    | '-'
                    | '?'
                    | '!'
                    | '\''
                    | ' '
                    | '\t'
                    | '\r'
            )
        {
            i += ch.len_utf8();
            continue;
        }
        return None;
    }
    None
}

/// The identifier that precedes a position, as (start, name).
struct Ident {
    start: usize,
    name: String,
}

/// The identifier immediately before `pos`, skipping a generic argument list.
fn identifier_before(text: &str, pos: usize) -> Ident {
    let bytes = text.as_bytes();
    let mut end = pos;
    if end > 0 && bytes[end - 1] == b'>' {
        let mut depth = 0i32;
        let mut i = end;
        while i > 0 {
            i -= 1;
            match bytes[i] {
                b'>' => depth += 1,
                b'<' => {
                    depth -= 1;
                    if depth == 0 {
                        end = i;
                        break;
                    }
                }
                b';' | b'{' | b'}' => break,
                _ => {}
            }
        }
    }
    let mut start = end;
    while start > 0 && (bytes[start - 1] == b'_' || bytes[start - 1].is_ascii_alphanumeric()) {
        start -= 1;
    }
    Ident {
        start,
        name: text[start..end].to_string(),
    }
}

/// Keywords whose parameter list never introduces a function body.
const CONTROL_KEYWORDS: &[&str] = &[
    "if",
    "for",
    "while",
    "switch",
    "catch",
    "do",
    "else",
    "loop",
    "match",
    "case",
    "return",
    "sizeof",
    "typeof",
    "throw",
    "using",
    "lock",
    "fixed",
    "synchronized",
    "try",
    "finally",
    "foreach",
    "when",
    "unless",
    "until",
    "with",
    "assert",
    "new",
    "delete",
    "await",
    "yield",
    "instanceof",
    "in",
    "of",
    "defer",
    "unsafe",
    "struct",
    "enum",
    "union",
    "class",
    "namespace",
    "module",
    "package",
    "trait",
    "impl",
    "interface",
    "record",
    "func",
    "fn",
    "def",
    "function",
    "let",
    "var",
    "const",
    "public",
    "private",
    "protected",
    "static",
    "override",
    "final",
    "abstract",
    "virtual",
    "inline",
    "extern",
    "export",
    "default",
    "extends",
    "implements",
    "type",
];

/// Whether a parameter list opens a function body.
///
/// Rust and Go require their declaration keyword, because there a bare
/// `name(...) {` is far more likely to be a call than a definition; the
/// C-family languages take the shape alone.
fn is_definition(lang: Lang, text: &str, ident: &Ident, arrow: bool) -> bool {
    if ident.name.is_empty() {
        return arrow;
    }
    if CONTROL_KEYWORDS.contains(&ident.name.as_str()) {
        return false;
    }
    match lang {
        Lang::Rust => has_word_before(text, ident.start, "fn"),
        Lang::Go => has_word_before(text, ident.start, "func"),
        _ => true,
    }
}

/// Whether `word` appears as a whole word before `pos`, without crossing a
/// statement boundary.
fn has_word_before(text: &str, pos: usize, word: &str) -> bool {
    let bytes = text.as_bytes();
    let word_bytes = word.as_bytes();
    let last = *word_bytes.last().unwrap_or(&0);
    let limit = pos.saturating_sub(256);
    let mut i = pos;
    while i > limit {
        i -= 1;
        let byte = bytes[i];
        if byte == b';' || byte == b'{' || byte == b'}' {
            return false;
        }
        if byte != last {
            continue;
        }
        if i + 1 < word_bytes.len() {
            continue;
        }
        let at = i + 1 - word_bytes.len();
        if at < limit || text.get(at..i + 1) != Some(word) {
            continue;
        }
        let before_ok = at == 0 || !is_word_byte(bytes[at - 1]);
        if before_ok {
            return true;
        }
    }
    false
}

/// Whether a byte can be part of an identifier.
fn is_word_byte(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphanumeric()
}

/// A source line with its byte offset.
struct SrcLine<'a> {
    start: usize,
    content: &'a str,
}

/// Split `text` into lines, keeping each line's offset.
fn source_lines(text: &str) -> Vec<SrcLine<'_>> {
    let mut out = Vec::new();
    let mut start = 0usize;
    for chunk in text.split_inclusive('\n') {
        out.push(SrcLine {
            start,
            content: chunk.strip_suffix('\n').unwrap_or(chunk),
        });
        start += chunk.len();
    }
    out
}

/// Find the function bodies in Python source.
///
/// Python has no braces: a body is the indented block that follows the `def`
/// header.
fn python_bodies(text: &str) -> Vec<Replacement> {
    let mut out = Vec::new();
    let lines = source_lines(text);
    let mut i = 0usize;
    while i < lines.len() {
        let trimmed = lines[i].content.trim_start();
        if !(trimmed.starts_with("def ") || trimmed.starts_with("async def ")) {
            i += 1;
            continue;
        }
        let indent = lines[i].content.len() - trimmed.len();
        // The header may span lines. It ends at the colon that closes the
        // parameter list.
        let mut header = i;
        let mut depth = 0i32;
        loop {
            let mut ended = false;
            for ch in lines[header].content.chars() {
                match ch {
                    '(' | '[' | '{' => depth += 1,
                    ')' | ']' | '}' => depth -= 1,
                    ':' if depth <= 0 => {
                        ended = true;
                        break;
                    }
                    _ => {}
                }
            }
            if ended || header + 1 >= lines.len() {
                break;
            }
            header += 1;
        }
        // The body is the indented block after the header.
        let mut first = header + 1;
        while first < lines.len() && lines[first].content.trim().is_empty() {
            first += 1;
        }
        let mut last = first;
        while last < lines.len() {
            let content = lines[last].content;
            if content.trim().is_empty() {
                last += 1;
                continue;
            }
            let line_indent = content.len() - content.trim_start().len();
            if line_indent <= indent {
                break;
            }
            last += 1;
        }
        // Blank lines at the end of the block are not part of it.
        while last > first && lines[last - 1].content.trim().is_empty() {
            last -= 1;
        }
        if first >= last {
            i = header + 1;
            continue;
        }
        let start = lines[first].start;
        let end = if last < lines.len() {
            lines[last].start
        } else {
            text.len()
        };
        let body = &text[start..end];
        if is_body_marker(body) {
            i = last;
            continue;
        }
        let body_indent = lines[first].content.len() - lines[first].content.trim_start().len();
        let mut replacement = String::new();
        replacement.push_str(&lines[first].content[..body_indent]);
        replacement.push_str(&body_marker(first + 1, last));
        replacement.push_str(&"\n".repeat(body.matches('\n').count()));
        if replacement.len() < end - start {
            out.push(Replacement {
                start,
                end,
                text: replacement,
            });
        }
        i = last;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::evidence::types::{Origin, Subject};

    fn filt(text: &str, level: CodeLevel, language: Option<&str>) -> Option<String> {
        filter(text, level, language)
    }

    /// The filter must never return something longer than it was given.
    fn assert_never_larger(input: &str, output: Option<&str>) {
        let Some(out) = output else {
            return;
        };
        assert!(
            out.len() <= input.len(),
            "filter grew {} bytes to {}: {out}",
            input.len(),
            out.len()
        );
    }

    #[test]
    fn minimal_strips_a_line_comment() {
        let code =
            "// header comment\nfn add(a: i32, b: i32) -> i32 {\n    // the sum\n    a + b\n}\n";
        let out = filt(code, CodeLevel::Minimal, Some("src/main.rs")).expect("strips");
        assert!(!out.contains("header comment"), "got {out}");
        assert!(out.contains("fn add"), "got {out}");
        assert_never_larger(code, Some(&out));
    }

    #[test]
    fn a_comment_marker_inside_a_string_is_not_a_comment() {
        let code = "fn main() {\n    let url = \"https://example.com\"; // trailing\n    let tag = \"#not-a-comment\";\n    println!(\"{url}\");\n}\n";
        let out = filt(code, CodeLevel::Minimal, Some("main.rs")).expect("strips");
        assert!(
            out.contains("https://example.com"),
            "the string must survive: {out}"
        );
        assert!(
            out.contains("#not-a-comment"),
            "the string must survive: {out}"
        );
        assert!(!out.contains("trailing"), "the comment must go: {out}");
        assert_never_larger(code, Some(&out));
    }

    #[test]
    fn a_block_comment_is_stripped_and_the_surrounding_content_survives() {
        let code = "/* lead */ let x = 1; /* tail */\nlet y = 2;\n";
        let out = filt(code, CodeLevel::Minimal, Some("a.rs")).expect("strips");
        assert!(out.contains("let x = 1"), "got {out}");
        assert!(out.contains("let y = 2"), "got {out}");
        assert!(!out.contains("lead"), "got {out}");
        assert!(!out.contains("tail"), "got {out}");
        assert_never_larger(code, Some(&out));
    }

    #[test]
    fn a_doc_comment_is_stripped_but_the_declaration_is_kept() {
        let code = "/// Add two numbers.\n/// Returns the sum.\npub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n";
        let out = filt(code, CodeLevel::Minimal, Some("lib.rs")).expect("strips");
        assert!(out.contains("pub fn add"), "got {out}");
        assert!(!out.contains("Add two numbers"), "got {out}");
        assert_never_larger(code, Some(&out));
    }

    #[test]
    fn aggressive_keeps_the_signature_and_names_the_body_lines() {
        let code = "fn compute(a: i32) -> i32 {\n    let inner = a * 2;\n    let mut out = inner;\n    out += 1;\n    out\n}\n";
        let out = filt(code, CodeLevel::Aggressive, Some("math.rs")).expect("strips");
        assert!(
            out.contains("fn compute(a: i32) -> i32"),
            "the signature must survive: {out}"
        );
        assert!(
            out.contains("body elided: lines"),
            "the marker must name the range: {out}"
        );
        assert!(
            !out.contains("let inner = a * 2;"),
            "the body must go: {out}"
        );
        assert_never_larger(code, Some(&out));
    }

    #[test]
    fn a_non_code_input_is_left_alone() {
        let prose = "This is a paragraph about reading code.\nIt has no annotations at all.\n";
        assert!(filt(prose, CodeLevel::Minimal, None).is_none());
        assert!(filt(prose, CodeLevel::Aggressive, None).is_none());
    }

    #[test]
    fn an_empty_payload_is_left_alone() {
        assert!(filt("", CodeLevel::Minimal, Some("a.rs")).is_none());
        assert!(filt("\n", CodeLevel::Minimal, Some("a.rs")).is_none());
    }

    #[test]
    fn the_language_is_sniffed_when_no_path_is_given() {
        // The evidence layer hands the payload over as text with a tool name,
        // so the content is the only signal there is.
        let code = "fn main() {\n    // a comment\n    let x = 1; // trailing\n}\n";
        let out = filt(code, CodeLevel::Minimal, None).expect("strips");
        assert!(!out.contains("a comment"), "got {out}");
        assert!(!out.contains("trailing"), "got {out}");
        assert!(out.contains("let x = 1;"), "got {out}");
        assert_never_larger(code, Some(&out));
    }

    #[test]
    fn python_is_sniffed_from_the_content() {
        let code = "def f():\n    # a comment\n    return 1\n";
        let out = filt(code, CodeLevel::Minimal, None).expect("strips");
        assert!(!out.contains("a comment"), "got {out}");
        assert!(out.contains("return 1"), "got {out}");
        assert_never_larger(code, Some(&out));
    }

    #[test]
    fn python_comments_outside_strings_are_stripped() {
        let code = "# module doc\ndef f():\n    \"a docstring\"\n    return 1  # inline\n";
        let out = filt(code, CodeLevel::Minimal, Some("m.py")).expect("strips");
        assert!(!out.contains("module doc"), "got {out}");
        assert!(!out.contains("inline"), "got {out}");
        assert!(
            out.contains("\"a docstring\""),
            "the string must survive: {out}"
        );
        assert_never_larger(code, Some(&out));
    }

    #[test]
    fn python_aggressive_keeps_the_def_and_marks_the_body() {
        let code = "def outer(a):\n    def inner(b):\n        return b\n    return a\n";
        let out = filt(code, CodeLevel::Aggressive, Some("m.py")).expect("strips");
        assert!(
            out.contains("def outer(a):"),
            "the signature must survive: {out}"
        );
        assert!(out.contains("body elided: lines"), "got {out}");
        assert_never_larger(code, Some(&out));
    }

    #[test]
    fn the_output_never_grows_across_languages_and_levels() {
        let cases: [(&str, CodeLevel, &str); 4] = [
            ("fn f() {\n    // c\n    1\n}\n", CodeLevel::Minimal, "f.rs"),
            (
                "fn f() {\n    // c\n    1\n}\n",
                CodeLevel::Aggressive,
                "f.rs",
            ),
            ("def f():\n    # c\n    pass\n", CodeLevel::Minimal, "f.py"),
            (
                "function f() {\n    // c\n    return 1;\n}\n",
                CodeLevel::Aggressive,
                "f.js",
            ),
        ];
        for (code, level, path) in cases {
            assert_never_larger(code, filt(code, level, Some(path)).as_deref());
        }
    }

    #[test]
    fn the_methods_declare_the_honest_contract() {
        let methods: [&dyn Method; 2] = [&CodeFilter, &CodeFilterAggressive];
        for method in methods {
            let info = method.info();
            assert!(info.lossy, "{} must declare itself lossy", method.name());
            assert_eq!(info.recovery, Recovery::Recall);
            assert_eq!(info.consumes, &[ContentFormat::Code]);
        }
        assert_eq!(CodeFilter.name(), "code-filter");
        assert_eq!(CodeFilterAggressive.name(), "code-filter-aggressive");
    }

    #[test]
    fn apply_skips_a_payload_with_nothing_to_strip() {
        let origin = Origin::default();
        let block = Block::new(
            "fn f() {}\n",
            Subject::ToolResult,
            ContentFormat::Code,
            &origin,
        );
        let ctx = MethodCtx::new("aft_read");
        assert!(matches!(CodeFilter.apply(&block, &ctx), Outcome::Skip));
    }
}
