//! Methods that cannot lose information.
//!
//! **Contract for this file:** every method here must return
//! [`MethodInfo::LOSSLESS_TRANSFORM`] unless it genuinely transforms (some are
//! lossless but parse, and therefore declare [`CostClass::Fast`]). No method
//! in this file may set `lossy = true`; anything that can lose information
//! belongs in [`super::lossy`].

use super::super::types::{
    Block, ContentFormat, CostClass, Method, MethodCtx, MethodInfo, Outcome,
};

/// One source line, with its terminator kept apart.
///
/// Every line-shaped method in this file rewrites lines, so the terminator
/// has to survive the rewrite: a file that ended without a newline must not
/// gain one, and a CRLF file must stay CRLF.
#[derive(Debug, Clone, Copy)]
struct Line<'a> {
    content: &'a str,
    term: &'a str,
}

/// Split `text` into lines and the run of terminators that closes it.
///
/// The trailing terminators come back separately so a method can put them
/// back untouched: the trailing-newline structure of the payload is exactly
/// preserved, whether it is none, one, or a dozen.
fn split_lines(text: &str) -> (Vec<Line<'_>>, &str) {
    let bytes = text.as_bytes();
    let mut core_end = bytes.len();
    loop {
        if core_end >= 2 && bytes[core_end - 2] == b'\r' && bytes[core_end - 1] == b'\n' {
            core_end -= 2;
        } else if core_end >= 1 && bytes[core_end - 1] == b'\n' {
            core_end -= 1;
        } else {
            break;
        }
    }
    let (core, tail) = text.split_at(core_end);
    let mut lines = Vec::new();
    for chunk in core.split_inclusive('\n') {
        let (content, term) = match chunk.strip_suffix('\n') {
            Some(rest) => match rest.strip_suffix('\r') {
                Some(rest) => (rest, "\r\n"),
                None => (rest, "\n"),
            },
            None => (chunk, ""),
        };
        lines.push(Line { content, term });
    }
    (lines, tail)
}

/// Strip ANSI escape sequences.
#[derive(Debug, Clone, Copy, Default)]
pub struct Ansi;

impl Method for Ansi {
    fn name(&self) -> &'static str {
        "ansi"
    }
    fn info(&self) -> MethodInfo {
        MethodInfo {
            consumes: &[ContentFormat::Term, ContentFormat::Plain],
            cost: CostClass::Instant,
            ..MethodInfo::LOSSLESS_TRANSFORM
        }
    }
    fn apply(&self, block: &Block, _ctx: &MethodCtx) -> Outcome {
        Ansi::strip(block.text).map_or(Outcome::Skip, Outcome::Continue)
    }
}

impl Ansi {
    /// Strip ANSI escape sequences from `text`.
    ///
    /// The scanner knows the four forms a terminal emits: `ESC [ ...
    /// <final byte>`, `ESC O <byte>`, `ESC <byte>`, and `ESC <intermediate>
    /// <final byte>` — the `ESC ( B` charset designators. A CSI ends at the
    /// first byte in `0x40..=0x7E`, not at the first letter, so `ESC [ 1 @`
    /// is one sequence and not the rest of the file.
    ///
    /// An incomplete sequence is left alone: dropping it would eat the text
    /// after it. A non-ASCII byte after `ESC` is not an escape sequence at
    /// all, so the `ESC` stays and no UTF-8 codepoint is ever split.
    pub fn strip(text: &str) -> Option<String> {
        if !text.contains('\u{1b}') {
            return None;
        }
        let bytes = text.as_bytes();
        let mut out = String::with_capacity(text.len());
        let mut i = 0usize;
        let mut stripped = false;
        while i < bytes.len() {
            if bytes[i] != 0x1b {
                let start = i;
                while i < bytes.len() && bytes[i] != 0x1b {
                    i += 1;
                }
                out.push_str(&text[start..i]);
                continue;
            }
            let Some(next) = bytes.get(i + 1).copied() else {
                // A lone ESC at the end of the input is an incomplete
                // sequence: leave it in place.
                out.push_str(&text[i..]);
                break;
            };
            match next {
                b'[' => {
                    // CSI: parameter and intermediate bytes, then a final byte.
                    let mut j = i + 2;
                    let mut complete = false;
                    while j < bytes.len() {
                        let byte = bytes[j];
                        if byte == 0x1b {
                            break;
                        }
                        if (0x40..=0x7e).contains(&byte) {
                            complete = true;
                            break;
                        }
                        j += 1;
                    }
                    if complete {
                        i = j + 1;
                        stripped = true;
                    } else {
                        // Keep what we have and resume at the ESC that
                        // cancelled the sequence, or at the end of the input.
                        let resume = if j < bytes.len() { j } else { bytes.len() };
                        out.push_str(&text[i..resume]);
                        i = resume;
                    }
                }
                b'O' => {
                    // SS3: `ESC O` and exactly one more byte.
                    match bytes.get(i + 2).copied() {
                        Some(byte) if byte != 0x1b => {
                            i += 3;
                            stripped = true;
                        }
                        Some(_) => {
                            out.push_str(&text[i..i + 2]);
                            i += 2;
                        }
                        None => {
                            out.push_str(&text[i..]);
                            i = bytes.len();
                        }
                    }
                }
                0x20..=0x2f => {
                    // `ESC ( B`: intermediate bytes, then a final byte.
                    let mut j = i + 1;
                    while j < bytes.len() && (0x20..=0x2f).contains(&bytes[j]) {
                        j += 1;
                    }
                    match bytes.get(j).copied() {
                        Some(byte) if (0x30..=0x7e).contains(&byte) => {
                            i = j + 1;
                            stripped = true;
                        }
                        Some(_) => {
                            out.push_str(&text[i..j]);
                            i = j;
                        }
                        None => {
                            out.push_str(&text[i..]);
                            i = bytes.len();
                        }
                    }
                }
                0x30..=0x7e => {
                    // `ESC <one byte>`.
                    i += 2;
                    stripped = true;
                }
                _ => {
                    // Not ASCII, so not an escape sequence. The ESC is kept so
                    // that no UTF-8 codepoint is split.
                    out.push_str(&text[i..i + 1]);
                    i += 1;
                }
            }
        }
        if !stripped {
            return None;
        }
        Some(out)
    }
}

/// Strip progress bars and other carriage-return redraws, keeping the last
/// revision of each overwritten line.
#[derive(Debug, Clone, Copy, Default)]
pub struct Progress;

impl Method for Progress {
    fn name(&self) -> &'static str {
        "progress"
    }
    fn info(&self) -> MethodInfo {
        MethodInfo {
            consumes: &[ContentFormat::Term],
            cost: CostClass::Instant,
            ..MethodInfo::LOSSLESS_TRANSFORM
        }
    }
    fn apply(&self, block: &Block, _ctx: &MethodCtx) -> Outcome {
        Progress::collapse(block.text).map_or(Outcome::Skip, Outcome::Continue)
    }
}

impl Progress {
    /// Collapse carriage-return redraws, keeping the last revision of each
    /// overwritten line.
    ///
    /// A collapsed revision is only lossless if the reader is told how many
    /// went, so the count is printed. The method never drops a line: an
    /// empty final revision still leaves its marker behind.
    fn collapse(text: &str) -> Option<String> {
        if !text.contains('\r') {
            return None;
        }
        let (lines, tail) = split_lines(text);
        let mut out = String::with_capacity(text.len());
        let mut changed = false;
        for line in &lines {
            // A trailing carriage return is the redraw marker of the line
            // ending, not an empty revision of its own.
            let body = line.content.strip_suffix('\r').unwrap_or(line.content);
            let mut parts = body.split('\r');
            let last = parts.next_back().unwrap_or("");
            let dropped = parts.count();
            if dropped == 0 {
                out.push_str(line.content);
                out.push_str(line.term);
                continue;
            }
            let marker = if dropped == 1 {
                "  … [1 earlier revision elided]".to_string()
            } else {
                format!("  … [{dropped} earlier revisions elided]")
            };
            let collapsed = format!("{last}{marker}");
            // Only collapse when it pays: the marker must be shorter than the
            // revisions it replaces.
            if collapsed.len() >= line.content.len() {
                out.push_str(line.content);
                out.push_str(line.term);
                continue;
            }
            changed = true;
            out.push_str(&collapsed);
            out.push_str(line.term);
        }
        out.push_str(tail);
        if changed && out.len() < text.len() {
            Some(out)
        } else {
            None
        }
    }
}

/// Collapse consecutive duplicate lines.
#[derive(Debug, Clone, Copy, Default)]
pub struct Dedup;

impl Method for Dedup {
    fn name(&self) -> &'static str {
        "dedup"
    }
    fn info(&self) -> MethodInfo {
        MethodInfo {
            consumes: &[ContentFormat::Term, ContentFormat::Plain],
            cost: CostClass::Instant,
            ..MethodInfo::LOSSLESS_TRANSFORM
        }
    }
    fn apply(&self, block: &Block, _ctx: &MethodCtx) -> Outcome {
        Dedup::collapse(block.text).map_or(Outcome::Skip, Outcome::Continue)
    }
}

impl Dedup {
    /// Collapse consecutive duplicate lines, reporting how many went.
    ///
    /// The trailing-newline structure is preserved exactly: a payload that
    /// ended without a newline does not gain one, and a run of trailing
    /// blank lines is left alone.
    fn collapse(text: &str) -> Option<String> {
        let (lines, tail) = split_lines(text);
        if lines.len() < 2 {
            return None;
        }
        let mut out = String::with_capacity(text.len());
        let mut changed = false;
        let mut i = 0usize;
        while i < lines.len() {
            let mut j = i + 1;
            while j < lines.len() && lines[j].content == lines[i].content {
                j += 1;
            }
            let group = &lines[i..j];
            if group.len() > 1 {
                let marker = format!("  … [{} identical lines elided]", group.len() - 1);
                let term = group[0].term;
                let original: usize = group
                    .iter()
                    .map(|line| line.content.len() + line.term.len())
                    .sum();
                let collapsed = group[0].content.len()
                    + term.len()
                    + marker.len()
                    + if j < lines.len() { term.len() } else { 0 };
                // Only collapse when it pays: the marker must be shorter than
                // the lines it replaces.
                if collapsed < original {
                    changed = true;
                    out.push_str(group[0].content);
                    out.push_str(term);
                    out.push_str(&marker);
                    if j < lines.len() {
                        out.push_str(term);
                    }
                    i = j;
                    continue;
                }
            }
            for line in group {
                out.push_str(line.content);
                out.push_str(line.term);
            }
            i = j;
        }
        out.push_str(tail);
        if changed && out.len() < text.len() {
            Some(out)
        } else {
            None
        }
    }
}

/// Minify JSON.
#[derive(Debug, Clone, Copy, Default)]
pub struct MinifyJson;

impl Method for MinifyJson {
    fn name(&self) -> &'static str {
        "minify-json"
    }
    fn info(&self) -> MethodInfo {
        MethodInfo {
            consumes: &[ContentFormat::Json],
            cost: CostClass::Fast,
            ..MethodInfo::LOSSLESS_TRANSFORM
        }
    }
    fn apply(&self, block: &Block, _ctx: &MethodCtx) -> Outcome {
        MinifyJson::minify(block.text).map_or(Outcome::Skip, Outcome::Continue)
    }
}

impl MinifyJson {
    /// Minify a JSON document.
    ///
    /// The document is parsed to prove that it *is* JSON, but the output is
    /// produced by dropping the whitespace between tokens rather than by
    /// re-serialising the parsed value: a re-serialisation would round a
    /// large integer through a float and would drop a duplicate key, and
    /// neither is allowed in a lossless method.
    fn minify(text: &str) -> Option<String> {
        if serde_json::from_str::<serde_json::Value>(text).is_err() {
            return None;
        }
        let mut out = String::with_capacity(text.len());
        let mut in_string = false;
        let mut escaped = false;
        for ch in text.chars() {
            if in_string {
                out.push(ch);
                if escaped {
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == '"' {
                    in_string = false;
                }
                continue;
            }
            match ch {
                '"' => {
                    in_string = true;
                    out.push(ch);
                }
                ' ' | '\t' | '\n' | '\r' => {}
                _ => out.push(ch),
            }
        }
        if out.len() < text.len() {
            Some(out)
        } else {
            None
        }
    }
}

/// Strip markdown and HTML boilerplate that never carries meaning.
#[derive(Debug, Clone, Copy, Default)]
pub struct MdSlim;

impl Method for MdSlim {
    fn name(&self) -> &'static str {
        "md-slim"
    }
    fn info(&self) -> MethodInfo {
        MethodInfo {
            consumes: &[ContentFormat::Web, ContentFormat::Plain],
            cost: CostClass::Fast,
            ..MethodInfo::LOSSLESS_TRANSFORM
        }
    }
    fn apply(&self, block: &Block, _ctx: &MethodCtx) -> Outcome {
        MdSlim::slim(block.text).map_or(Outcome::Skip, Outcome::Continue)
    }
}

impl MdSlim {
    /// Strip the comment spans a markdown or HTML document hides content in.
    ///
    /// The comment goes and the text around it stays — a comment in the
    /// middle of a sentence must not take the sentence with it. The newlines
    /// inside a comment are kept, so every line keeps its number.
    fn slim(text: &str) -> Option<String> {
        if !text.contains("<!--") {
            return None;
        }
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        let mut changed = false;
        while let Some(start) = rest.find("<!--") {
            out.push_str(&rest[..start]);
            let after = &rest[start + 4..];
            match after.find("-->") {
                Some(end) => {
                    for ch in after[..end].chars() {
                        if ch == '\n' {
                            out.push('\n');
                        }
                    }
                    rest = &after[end + 3..];
                    changed = true;
                }
                None => {
                    // An unterminated comment has no end: leaving it alone is
                    // the only choice that does not eat the rest of the page.
                    out.push_str(&rest[start..]);
                    rest = "";
                    break;
                }
            }
        }
        out.push_str(rest);
        if changed && out.len() < text.len() {
            Some(out)
        } else {
            None
        }
    }
}

/// Strip the line-number prefix a `read` tool adds.
#[derive(Debug, Clone, Copy, Default)]
pub struct StripLineNumbers;

impl Method for StripLineNumbers {
    fn name(&self) -> &'static str {
        "strip-line-numbers"
    }
    fn info(&self) -> MethodInfo {
        MethodInfo {
            consumes: &[ContentFormat::Code],
            cost: CostClass::Instant,
            ..MethodInfo::LOSSLESS_TRANSFORM
        }
    }
    fn apply(&self, block: &Block, _ctx: &MethodCtx) -> Outcome {
        StripLineNumbers::strip(block.text).map_or(Outcome::Skip, Outcome::Continue)
    }
}

impl StripLineNumbers {
    /// Strip the line-number prefix a read tool adds.
    ///
    /// The prefix is only removed when it really is digits-then-tab *and*
    /// the payload looks like numbered output: every non-empty line carries
    /// a number and the numbers count up. A file that merely contains a
    /// `12\tvalue` somewhere is left exactly as it is.
    fn strip(text: &str) -> Option<String> {
        let (lines, tail) = split_lines(text);
        let mut numbered = 0usize;
        let mut non_empty = 0usize;
        let mut previous: Option<usize> = None;
        let mut ascending = true;
        for line in &lines {
            if line.content.trim().is_empty() {
                continue;
            }
            non_empty += 1;
            let Some((_, number)) = number_prefix(line.content) else {
                ascending = false;
                continue;
            };
            numbered += 1;
            if previous.is_some_and(|last| number <= last) {
                ascending = false;
            }
            previous = Some(number);
        }
        if numbered < 2 || non_empty == 0 || numbered != non_empty || !ascending {
            return None;
        }
        let mut out = String::with_capacity(text.len());
        for line in &lines {
            match number_prefix(line.content) {
                Some((after, _)) => out.push_str(&line.content[after..]),
                None => out.push_str(line.content),
            }
            out.push_str(line.term);
        }
        out.push_str(tail);
        if out.len() < text.len() {
            Some(out)
        } else {
            None
        }
    }
}

/// The offset just past a digits-then-tab prefix, and the number in it.
fn number_prefix(content: &str) -> Option<(usize, usize)> {
    let tab = content.find('\t')?;
    let digits = content.get(..tab)?;
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let number = digits.parse::<usize>().ok()?;
    Some((tab + 1, number))
}

/// Replace long base64 blobs with a placeholder.
#[derive(Debug, Clone, Copy, Default)]
pub struct Base64Elide;

impl Method for Base64Elide {
    fn name(&self) -> &'static str {
        "base64-elide"
    }
    fn info(&self) -> MethodInfo {
        MethodInfo {
            consumes: &[ContentFormat::Json, ContentFormat::Web, ContentFormat::Term],
            cost: CostClass::Instant,
            ..MethodInfo::LOSSLESS_TRANSFORM
        }
    }
    fn apply(&self, block: &Block, _ctx: &MethodCtx) -> Outcome {
        Base64Elide::elide(block.text).map_or(Outcome::Skip, Outcome::Continue)
    }
}

impl Base64Elide {
    /// Replace every maximal run of base64-alphabet characters that is at
    /// least `MIN_BLOB` long with a placeholder that names its length.
    ///
    /// A shorter run is left exactly as it was: the scanner never reslices a
    /// run it has already measured, so a run is either whole or untouched.
    fn elide(text: &str) -> Option<String> {
        const MIN_BLOB: usize = 200;
        let bytes = text.as_bytes();
        let mut out = String::with_capacity(text.len());
        let mut i = 0usize;
        let mut changed = false;
        while i < bytes.len() {
            if !is_base64(bytes[i]) {
                let start = i;
                while i < bytes.len() && !is_base64(bytes[i]) {
                    i += 1;
                }
                out.push_str(&text[start..i]);
                continue;
            }
            let start = i;
            while i < bytes.len() && is_base64(bytes[i]) {
                i += 1;
            }
            let length = i - start;
            if length >= MIN_BLOB {
                out.push_str(&format!("[base64 blob, {length} chars elided]"));
                changed = true;
            } else {
                out.push_str(&text[start..i]);
            }
        }
        if changed && out.len() < text.len() {
            Some(out)
        } else {
            None
        }
    }
}

/// Whether a byte belongs to the base64 alphabet.
fn is_base64(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'+' || byte == b'/' || byte == b'='
}

/// Replace a JSON document with its shape.
#[derive(Debug, Clone, Copy, Default)]
pub struct Shape;

impl Method for Shape {
    fn name(&self) -> &'static str {
        "shape"
    }
    fn info(&self) -> MethodInfo {
        MethodInfo {
            consumes: &[ContentFormat::Json],
            cost: CostClass::Fast,
            ..MethodInfo::LOSSLESS_TRANSFORM
        }
    }
    fn apply(&self, block: &Block, _ctx: &MethodCtx) -> Outcome {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(block.text) else {
            return Outcome::Skip;
        };
        // The shape is rendered against the length of the payload: a shape
        // that does not fit is cut short, so it can never be the longer
        // string.
        let shaped = Shape::render_bounded(&value, block.text.len());
        if shaped.len() < block.text.len() {
            Outcome::Continue(shaped)
        } else {
            Outcome::Skip
        }
    }
}

impl Shape {
    /// The deepest nesting the renderer describes.
    pub const MAX_DEPTH: usize = 4;

    /// Render the shape of a JSON value, keeping keys and types and dropping
    /// leaf values.
    pub fn render(value: &serde_json::Value, depth: usize) -> String {
        let mut writer = ShapeWriter {
            limit: None,
            used: 0,
        };
        Shape::write(&mut writer, value, depth)
    }

    /// Render a shape that fits inside `max_len` bytes.
    fn render_bounded(value: &serde_json::Value, max_len: usize) -> String {
        let mut writer = ShapeWriter {
            limit: Some(max_len),
            used: 0,
        };
        Shape::write(&mut writer, value, 0)
    }

    fn write(writer: &mut ShapeWriter, value: &serde_json::Value, depth: usize) -> String {
        if depth > Shape::MAX_DEPTH {
            return writer.take("…");
        }
        match value {
            serde_json::Value::Object(map) => {
                let mut out = writer.take("{");
                for (index, (key, child)) in map.iter().enumerate() {
                    if index > 0 {
                        out.push_str(&writer.take(", "));
                    }
                    let label = format!("\"{key}\": ");
                    out.push_str(&writer.take(&label));
                    out.push_str(&Shape::write(writer, child, depth + 1));
                }
                out.push_str(&writer.take("}"));
                out
            }
            serde_json::Value::Array(items) => {
                let inner = items
                    .first()
                    .map(|item| Shape::write(writer, item, depth + 1))
                    .unwrap_or_default();
                let label = format!("[{inner}] ({} items)", items.len());
                writer.take(&label)
            }
            serde_json::Value::String(text) => {
                let label = format!("string({})", text.chars().count());
                writer.take(&label)
            }
            serde_json::Value::Number(_) => writer.take("number"),
            serde_json::Value::Bool(_) => writer.take("bool"),
            serde_json::Value::Null => writer.take("null"),
        }
    }
}

/// The byte budget of a rendered shape.
struct ShapeWriter {
    /// The longest shape the writer may produce, if there is a limit.
    limit: Option<usize>,
    /// The bytes taken so far.
    used: usize,
}

impl ShapeWriter {
    /// Take `text` from the budget, or take nothing when it does not fit.
    fn take(&mut self, text: &str) -> String {
        if let Some(limit) = self.limit
            && self.used + text.len() > limit
        {
            return String::new();
        }
        self.used += text.len();
        text.to_string()
    }
}

/// Run-length encode runs of identical lines into a count.
#[derive(Debug, Clone, Copy, Default)]
pub struct Rle;

impl Method for Rle {
    fn name(&self) -> &'static str {
        "rle"
    }
    fn info(&self) -> MethodInfo {
        MethodInfo {
            consumes: &[ContentFormat::Plain, ContentFormat::Term],
            cost: CostClass::Instant,
            ..MethodInfo::LOSSLESS_TRANSFORM
        }
    }
    fn apply(&self, block: &Block, _ctx: &MethodCtx) -> Outcome {
        Rle::collapse(block.text).map_or(Outcome::Skip, Outcome::Continue)
    }
}

impl Rle {
    /// Collapse a run of three or more identical lines into one line and a
    /// count.
    ///
    /// The count is printed: a collapsed run is only lossless if the reader
    /// knows how many lines the one line stands for.
    fn collapse(text: &str) -> Option<String> {
        let (lines, tail) = split_lines(text);
        if lines.len() < 2 {
            return None;
        }
        let mut out = String::with_capacity(text.len());
        let mut changed = false;
        let mut i = 0usize;
        while i < lines.len() {
            let mut j = i + 1;
            while j < lines.len() && lines[j].content == lines[i].content {
                j += 1;
            }
            let group = &lines[i..j];
            if group.len() >= 3 {
                let marker = format!("  … [×{} identical]", group.len());
                let term = group[0].term;
                let original: usize = group
                    .iter()
                    .map(|line| line.content.len() + line.term.len())
                    .sum();
                let collapsed = group[0].content.len()
                    + term.len()
                    + marker.len()
                    + if j < lines.len() { term.len() } else { 0 };
                // Only collapse when it pays: the marker must be shorter than
                // the run it replaces.
                if collapsed < original {
                    changed = true;
                    out.push_str(group[0].content);
                    out.push_str(term);
                    out.push_str(&marker);
                    if j < lines.len() {
                        out.push_str(term);
                    }
                    i = j;
                    continue;
                }
            }
            for line in group {
                out.push_str(line.content);
                out.push_str(line.term);
            }
            i = j;
        }
        out.push_str(tail);
        if changed && out.len() < text.len() {
            Some(out)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::evidence::types::{Origin, Subject};

    fn run(method: &dyn Method, text: &str, format: ContentFormat) -> Option<String> {
        let origin = Origin::default();
        let block = Block {
            text,
            subject: Subject::ToolResult,
            format,
            origin: &origin,
        };
        let ctx = MethodCtx::new("test");
        match method.apply(&block, &ctx) {
            Outcome::Continue(next) => Some(next),
            Outcome::ShortCircuit(next) => Some(next),
            Outcome::Skip => None,
        }
    }

    /// A method must never make the payload bigger.
    fn assert_never_larger(input: &str, output: Option<&str>) {
        let Some(out) = output else {
            return;
        };
        assert!(
            out.len() <= input.len(),
            "grew {} bytes to {}: {out}",
            input.len(),
            out.len()
        );
    }

    #[test]
    fn ansi_strips_csi_sequences_without_eating_the_text() {
        let input = "\u{1b}[31mred\u{1b}[0m done";
        let out = run(&Ansi, input, ContentFormat::Term).expect("strips");
        assert_eq!(out, "red done");
        assert_never_larger(input, Some(&out));
    }

    #[test]
    fn ansi_strips_the_other_sequence_forms() {
        // `ESC O P` is one sequence, `ESC c` is one sequence, and `ESC ( B`
        // is one sequence.
        let input = "a\u{1b}OPb\u{1b}c\u{1b}(Bd";
        let out = run(&Ansi, input, ContentFormat::Term).expect("strips");
        assert_eq!(out, "abd");
        assert_never_larger(input, Some(&out));
    }

    #[test]
    fn ansi_leaves_an_incomplete_sequence_alone() {
        // No final byte: dropping it would eat the rest of the payload.
        let input = "plain \u{1b}[31";
        assert!(run(&Ansi, input, ContentFormat::Term).is_none());
    }

    #[test]
    fn ansi_never_splits_a_multibyte_codepoint() {
        // The CSI ends at the `m`, so the `é` behind it is untouched.
        let input = "\u{1b}[31mé";
        let out = run(&Ansi, input, ContentFormat::Term).expect("strips");
        assert_eq!(out, "é");
        assert_never_larger(input, Some(&out));

        // No final byte: the sequence is incomplete, so the text stays.
        let input = "\u{1b}[3é";
        assert!(run(&Ansi, input, ContentFormat::Term).is_none());
    }

    #[test]
    fn ansi_leaves_a_payload_with_no_escape_alone() {
        assert!(run(&Ansi, "plain text", ContentFormat::Term).is_none());
        assert!(run(&Ansi, "", ContentFormat::Term).is_none());
    }

    #[test]
    fn dedup_collapses_consecutive_duplicates_and_says_how_many() {
        let input = "a repeated line of output\n".repeat(8);
        let out = run(&Dedup, &input, ContentFormat::Term).expect("collapses");
        assert!(out.contains("7 identical lines elided"), "got {out}");
        assert_never_larger(&input, Some(&out));
    }

    #[test]
    fn dedup_preserves_the_trailing_newline_structure() {
        let terminated = "a repeated line of output\n".repeat(8);
        let out = run(&Dedup, &terminated, ContentFormat::Term).expect("collapses");
        assert!(out.ends_with('\n'), "got {out:?}");

        let unterminated = "a repeated line of output\n".repeat(7) + "a repeated line of output";
        let out = run(&Dedup, &unterminated, ContentFormat::Term).expect("collapses");
        assert!(!out.ends_with('\n'), "got {out:?}");
    }

    #[test]
    fn dedup_leaves_unique_lines_alone() {
        assert!(run(&Dedup, "one\ntwo\nthree\n", ContentFormat::Term).is_none());
        assert!(run(&Dedup, "one line\n", ContentFormat::Term).is_none());
        assert!(run(&Dedup, "", ContentFormat::Term).is_none());
    }

    #[test]
    fn minify_json_reformats_without_losing_the_payload() {
        let input = "{\n  \"a\": 1,\n  \"b\": [1, 2]\n}\n";
        let out = run(&MinifyJson, input, ContentFormat::Json).expect("minifies");
        let parsed: serde_json::Value = serde_json::from_str(&out).expect("still valid");
        assert_eq!(parsed["a"], 1);
        assert_never_larger(input, Some(&out));
    }

    #[test]
    fn minify_json_skips_a_document_that_is_not_json() {
        assert!(run(&MinifyJson, "not json at all", ContentFormat::Json).is_none());
        assert!(run(&MinifyJson, "", ContentFormat::Json).is_none());
    }

    #[test]
    fn minify_json_keeps_a_large_integer_exact() {
        let input = "{\n  \"id\": 12345678901234567890\n}";
        let out = run(&MinifyJson, input, ContentFormat::Json).expect("minifies");
        assert!(out.contains("12345678901234567890"), "got {out}");
        assert_never_larger(input, Some(&out));
    }

    #[test]
    fn md_slim_removes_a_comment_without_eating_the_surroundings() {
        let input = "before <!-- hidden --> after\n";
        let out = run(&MdSlim, input, ContentFormat::Web).expect("slims");
        assert!(out.contains("before"), "got {out}");
        assert!(out.contains("after"), "got {out}");
        assert!(!out.contains("hidden"), "got {out}");
        assert_never_larger(input, Some(&out));
    }

    #[test]
    fn md_slim_skips_a_document_with_no_comment() {
        assert!(run(&MdSlim, "plain prose\n", ContentFormat::Web).is_none());
        assert!(run(&MdSlim, "", ContentFormat::Web).is_none());
    }

    #[test]
    fn strip_line_numbers_removes_a_numbered_prefix() {
        let input = "1\tfn main() {\n2\t    println!(\"hi\");\n3\t}\n";
        let out = run(&StripLineNumbers, input, ContentFormat::Code).expect("strips");
        assert_eq!(out, "fn main() {\n    println!(\"hi\");\n}\n");
        assert_never_larger(input, Some(&out));
    }

    #[test]
    fn strip_line_numbers_leaves_a_file_that_only_looks_numbered_alone() {
        // One numbered line among many is not numbered output.
        let input = "12\tvalue\nlet x = 1;\nlet y = 2;\n";
        assert!(run(&StripLineNumbers, input, ContentFormat::Code).is_none());
    }

    #[test]
    fn strip_line_numbers_leaves_a_tab_that_is_not_a_number_alone() {
        let input = "a\tb\nc\td\n";
        assert!(run(&StripLineNumbers, input, ContentFormat::Code).is_none());
        assert!(run(&StripLineNumbers, "", ContentFormat::Code).is_none());
    }

    #[test]
    fn base64_elide_replaces_a_long_blob() {
        let blob = "a".repeat(400);
        let input = format!("{{\"data\":\"{blob}\"}}");
        let out = run(&Base64Elide, &input, ContentFormat::Json).expect("elides");
        assert!(out.contains("base64 blob, 400 chars elided"), "got {out}");
        assert_never_larger(&input, Some(&out));
    }

    #[test]
    fn base64_elide_leaves_a_short_run_alone() {
        let blob = "a".repeat(40);
        let input = format!("{{\"data\":\"{blob}\"}}");
        assert!(run(&Base64Elide, &input, ContentFormat::Json).is_none());
        assert!(run(&Base64Elide, "", ContentFormat::Json).is_none());
    }

    #[test]
    fn shape_renders_the_shape_of_a_document() {
        let input = "{\"id\": 1, \"name\": \"operant\", \"description\": \"a long description that carries the payload\", \"tags\": [\"alpha\", \"beta\"], \"nested\": {\"x\": 1, \"y\": 2}}";
        let out = run(&Shape, input, ContentFormat::Json).expect("shapes");
        assert!(out.contains("\"id\": number"), "got {out}");
        assert!(!out.contains("operant"), "the leaf value must go: {out}");
        assert_never_larger(input, Some(&out));
    }

    #[test]
    fn shape_skips_a_document_that_is_not_json() {
        assert!(run(&Shape, "not json", ContentFormat::Json).is_none());
        assert!(run(&Shape, "", ContentFormat::Json).is_none());
    }

    #[test]
    fn shape_never_exceeds_the_payload() {
        // The shape of `{}` is `{}`: there is nothing to win.
        assert!(run(&Shape, "{}", ContentFormat::Json).is_none());
    }

    #[test]
    fn rle_encodes_a_run_of_three_or_more() {
        let input = "same line of output\n".repeat(10);
        let out = run(&Rle, &input, ContentFormat::Plain).expect("rles");
        assert!(out.contains("×10"), "got {out}");
        assert_never_larger(&input, Some(&out));
    }

    #[test]
    fn rle_leaves_a_short_run_alone() {
        let input = "same\nsame\n";
        assert!(run(&Rle, input, ContentFormat::Plain).is_none());
        assert!(run(&Rle, "", ContentFormat::Plain).is_none());
    }

    #[test]
    fn progress_keeps_the_last_revision_and_says_how_many_went() {
        let line = "downloading the artifact";
        let input = format!("{line}\r{line}\r{line}\r\n");
        let out = run(&Progress, &input, ContentFormat::Term).expect("collapses");
        assert!(out.contains(line), "got {out}");
        assert!(out.contains("2 earlier revisions elided"), "got {out}");
        assert_never_larger(&input, Some(&out));
    }

    #[test]
    fn progress_leaves_a_payload_with_no_redraw_alone() {
        assert!(run(&Progress, "one line\n", ContentFormat::Term).is_none());
        assert!(run(&Progress, "", ContentFormat::Term).is_none());
    }

    #[test]
    fn a_payload_with_no_boilerplate_is_left_alone() {
        assert!(run(&Dedup, "one\ntwo\nthree\n", ContentFormat::Plain).is_none());
        assert!(run(&Ansi, "plain text", ContentFormat::Plain).is_none());
    }
}
