//! Methods that can lose information.
//!
//! **Contract for this file:** every method here declares
//! [`MethodInfo::LOSSY_RECALLABLE`] (or a `Recovery` that names how the full
//! text comes back). A method that cannot honour that contract belongs in
//! [`super::lossless`] instead.
//!
//! The caller is responsible for writing the **full, pre-compression text**
//! to the recall store before one of these runs, and for printing the
//! provenance marker with the compressed output. A lossy method that runs
//! without a recall record is a bug, not a trade-off.

use std::collections::HashSet;

use super::super::types::{
    Block, ContentFormat, Method, MethodCtx, MethodInfo, Outcome, Produces, Recovery,
};

/// Whether `out` is genuinely smaller than `input`.
///
/// The layer's contract is that no compression ever grows a result. A method
/// that cannot shrink the payload has nothing to offer, so it skips rather
/// than handing back a longer string.
fn shrinks(out: &str, input: &str) -> bool {
    out.len() < input.len() && out.chars().count() < input.chars().count()
}

/// Encode a tabular JSON document as TOON.
///
/// A table of uniform objects becomes a header plus comma-separated rows.
/// Anything else — a nested value, a mixed key set, an empty array — is
/// declined rather than half-encoded: a broken TOON document is worse than
/// no TOON document.
#[derive(Debug, Clone, Copy, Default)]
pub struct Toon;

impl Method for Toon {
    fn name(&self) -> &'static str {
        "toon"
    }
    fn info(&self) -> MethodInfo {
        MethodInfo {
            lossy: true,
            recovery: Recovery::Recall,
            consumes: &[ContentFormat::Json],
            produces: Produces::Format(ContentFormat::Plain),
        }
    }
    fn apply(&self, block: &Block, _ctx: &MethodCtx) -> Outcome {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(block.text) else {
            return Outcome::Skip;
        };
        let Some(encoded) = Toon::encode(&value) else {
            return Outcome::Skip;
        };
        if !shrinks(&encoded, block.text) {
            return Outcome::Skip;
        }
        // Short-circuit: a later cap must never cut a converted document
        // into invalid syntax.
        Outcome::ShortCircuit(encoded)
    }
}

impl Toon {
    /// Encode a JSON value as TOON, or `None` when it is not a uniform table.
    ///
    /// Two document shapes are accepted: a root array of uniform objects,
    /// and a single-key object whose value is such an array (`{"tasks":[…]}`),
    /// which names the table.
    pub fn encode(value: &serde_json::Value) -> Option<String> {
        let (name, items): (Option<&str>, &[serde_json::Value]) = match value {
            serde_json::Value::Array(items) => (None, items.as_slice()),
            serde_json::Value::Object(map) if map.len() == 1 => {
                let (name, items) = map.iter().next()?;
                (Some(name.as_str()), items.as_array()?.as_slice())
            }
            _ => return None,
        };
        // An empty array has no columns to declare, so there is no table to
        // encode.
        let first = items.first()?.as_object()?;
        let columns: Vec<&str> = first.keys().map(String::as_str).collect();
        if columns.is_empty() {
            return None;
        }
        // Every row must carry every column, with scalar values.
        for item in items {
            let obj = item.as_object()?;
            if obj.len() != columns.len() {
                return None;
            }
            for column in &columns {
                let cell = obj.get(*column)?;
                if !is_scalar(cell) {
                    return None;
                }
            }
        }

        let mut out = String::new();
        if let Some(name) = name {
            out.push_str(&toon_cell(name));
        }
        let header: Vec<String> = columns.iter().copied().map(toon_cell).collect();
        out.push_str(&format!("[{}]{{{}}}:", items.len(), header.join(",")));
        for item in items {
            let obj = item.as_object()?;
            let mut cells: Vec<String> = Vec::with_capacity(columns.len());
            for column in &columns {
                let cell = obj.get(*column)?;
                cells.push(toon_value(cell));
            }
            out.push('\n');
            out.push_str("  ");
            out.push_str(&cells.join(","));
        }
        Some(out)
    }
}

/// Whether a JSON value can live in a TOON cell.
fn is_scalar(value: &serde_json::Value) -> bool {
    matches!(
        value,
        serde_json::Value::String(_)
            | serde_json::Value::Number(_)
            | serde_json::Value::Bool(_)
            | serde_json::Value::Null
    )
}

/// Render one cell value.
fn toon_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => toon_cell(text),
        serde_json::Value::Null => "null".to_string(),
        other => other.to_string(),
    }
}

/// Render a string as a TOON cell, quoting it only when a bare copy would
/// be read back as something else.
fn toon_cell(text: &str) -> String {
    if !needs_quotes(text) {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if (ch as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

/// Whether a string has to be quoted to survive a round trip.
fn needs_quotes(text: &str) -> bool {
    text.is_empty()
        || text.trim() != text
        || text.contains([',', '"', '\\', '\n', '\r', '{', '}', '[', ']'])
        // A bare `1` would be read back as a number and a bare `true` as a
        // boolean: quoting keeps the value a string.
        || text.parse::<f64>().is_ok()
        || matches!(text, "true" | "false" | "null")
}

/// Group search matches by the file they came from.
#[derive(Debug, Clone, Copy, Default)]
pub struct GrepGroup;

impl Method for GrepGroup {
    fn name(&self) -> &'static str {
        "grep-group"
    }
    fn info(&self) -> MethodInfo {
        MethodInfo {
            lossy: true,
            recovery: Recovery::Recall,
            consumes: &[ContentFormat::Matches],
            ..MethodInfo::LOSSY_RECALLABLE
        }
    }
    fn apply(&self, block: &Block, _ctx: &MethodCtx) -> Outcome {
        let mut groups: Vec<(String, Vec<String>)> = Vec::new();
        for line in block.text.lines() {
            let line = line.trim_end();
            if line.trim().is_empty() {
                continue;
            }
            // A line with no usable path is a path-less match: it belongs to
            // no file, so there is nothing to group it under.
            let Some((path, matched)) = split_match(line) else {
                continue;
            };
            // A path is not hundreds of characters long; a longer prefix is
            // content that merely looks like a path.
            if path.is_empty() || path.len() > 260 {
                continue;
            }
            match groups.iter_mut().find(|(file, _)| file == path) {
                Some((_, matches)) => matches.push(matched.to_string()),
                None => groups.push((path.to_string(), vec![matched.to_string()])),
            }
        }
        if groups.is_empty() {
            return Outcome::Skip;
        }
        let mut out = String::new();
        for (path, matches) in &groups {
            out.push_str(path);
            out.push_str(&format!(" ({} matches)\n", matches.len()));
            for matched in matches {
                out.push_str("  ");
                out.push_str(matched);
                out.push('\n');
            }
        }
        if !shrinks(&out, block.text) {
            return Outcome::Skip;
        }
        Outcome::Continue(out)
    }
}

/// Split a match line into its path and its matched text.
///
/// A match line is `path:line:content` or `path:content`. The path may
/// itself contain a colon (`C:\src\a.rs`), so a boundary is only accepted
/// where it is unambiguous: a numeric line number after the colon, or a
/// colon that cannot be a Windows drive letter. A line with neither is a
/// path-less match and is left ungrouped.
fn split_match(line: &str) -> Option<(&str, &str)> {
    if let Some(split) = split_numbered(line) {
        return Some(split);
    }
    // A path never contains whitespace, so a colon whose prefix does is not
    // a path boundary.
    let usable: Vec<usize> = line
        .char_indices()
        .filter(|(_, ch)| *ch == ':')
        .map(|(idx, _)| idx)
        .filter(|idx| !line[..*idx].chars().any(char::is_whitespace))
        .collect();
    // A Windows path carries a colon of its own, so its boundary is the
    // last one; everywhere else the first one is the boundary.
    let colon = if is_drive_path(line) {
        usable.last().copied()
    } else {
        usable.first().copied()
    }?;
    let path = &line[..colon];
    if path.is_empty() || is_drive_letter(path) {
        return None;
    }
    Some((path, &line[colon + 1..]))
}

/// Split a `path:line:content` line, where the line number proves the shape.
fn split_numbered(line: &str) -> Option<(&str, &str)> {
    let mut from = 0usize;
    while let Some(pos) = line[from..].find(':') {
        let colon = from + pos;
        let path = &line[..colon];
        let rest = &line[colon + 1..];
        if !path.is_empty() && !path.chars().any(char::is_whitespace) && is_line_number(rest) {
            return Some((path, rest));
        }
        from = colon + 1;
    }
    None
}

/// Whether the text after a colon starts with a line number.
fn is_line_number(text: &str) -> bool {
    let Some((digits, _)) = text.split_once(':') else {
        return false;
    };
    !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
}

/// Whether the line starts with a Windows drive letter (`C:\src\a.rs`).
fn is_drive_path(line: &str) -> bool {
    let mut chars = line.chars();
    match (chars.next(), chars.next()) {
        (Some(first), Some(':')) => first.is_ascii_alphabetic(),
        _ => false,
    }
}

/// Whether the text is a bare drive letter, which is never a whole path.
fn is_drive_letter(path: &str) -> bool {
    path.len() == 1
        && path
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_alphabetic())
}

/// Cap a payload, keeping the tail.
///
/// Shell errors live at the end of the output, so the tail is the part worth
/// keeping; the head is dropped first.
#[derive(Debug, Clone, Copy, Default)]
pub struct Cap;

impl Method for Cap {
    fn name(&self) -> &'static str {
        "cap"
    }
    fn info(&self) -> MethodInfo {
        MethodInfo {
            lossy: true,
            recovery: Recovery::Recall,
            consumes: &[ContentFormat::Term],
            ..MethodInfo::LOSSY_RECALLABLE
        }
    }
    fn apply(&self, block: &Block, _ctx: &MethodCtx) -> Outcome {
        const CAP_LINES: usize = 200;
        let lines: Vec<&str> = block.text.lines().collect();
        if lines.len() <= CAP_LINES {
            return Outcome::Skip;
        }
        // The count is exact: every line before the tail is named in the
        // marker, so the reader knows exactly what the payload is missing.
        let dropped = lines.len() - CAP_LINES;
        let mut out = String::new();
        out.push_str(&format!("[… {dropped} earlier lines elided]\n"));
        for line in &lines[lines.len() - CAP_LINES..] {
            out.push_str(line);
            out.push('\n');
        }
        if !shrinks(&out, block.text) {
            return Outcome::Skip;
        }
        Outcome::Continue(out)
    }
}

/// Truncate a payload to a character budget, keeping its head and tail.
#[derive(Debug, Clone, Copy, Default)]
pub struct Truncate;

impl Method for Truncate {
    fn name(&self) -> &'static str {
        "truncate"
    }
    fn info(&self) -> MethodInfo {
        MethodInfo {
            lossy: true,
            recovery: Recovery::Recall,
            consumes: &[
                ContentFormat::Plain,
                ContentFormat::Term,
                ContentFormat::Code,
            ],
            ..MethodInfo::LOSSY_RECALLABLE
        }
    }
    fn apply(&self, block: &Block, _ctx: &MethodCtx) -> Outcome {
        let total = block.text.chars().count();
        if total <= Truncate::BUDGET {
            return Outcome::Skip;
        }
        // Two thirds of the budget from the head, one third from the tail:
        // the head carries the setup, the tail carries the outcome.
        let head = Truncate::BUDGET * 2 / 3;
        let tail = Truncate::BUDGET - head;
        // `chars()`, never bytes: a cut in the middle of a codepoint is not a
        // shorter string, it is a broken one.
        let mut out = String::new();
        out.extend(block.text.chars().take(head));
        out.push_str(&format!(
            "\n\n[… {total} chars total, {} elided]\n\n",
            total - Truncate::BUDGET
        ));
        out.extend(block.text.chars().skip(total - tail));
        if !shrinks(&out, block.text) {
            return Outcome::Skip;
        }
        Outcome::Continue(out)
    }
}

impl Truncate {
    /// The character budget a truncated payload keeps, head plus tail.
    pub const BUDGET: usize = 4000;
}

/// Reduce an HTML page to its readable text.
#[derive(Debug, Clone, Copy, Default)]
pub struct WebMd;

impl Method for WebMd {
    fn name(&self) -> &'static str {
        "web-md"
    }
    fn info(&self) -> MethodInfo {
        MethodInfo {
            lossy: true,
            recovery: Recovery::Recall,
            consumes: &[ContentFormat::Web],
            produces: Produces::Format(ContentFormat::Plain),
        }
    }
    fn apply(&self, block: &Block, _ctx: &MethodCtx) -> Outcome {
        let Some(text) = WebMd::extract(block.text) else {
            return Outcome::Skip;
        };
        if !shrinks(&text, block.text) {
            return Outcome::Skip;
        }
        Outcome::Continue(text)
    }
}

impl WebMd {
    /// The readable text of an HTML page, or `None` when there is none.
    pub fn extract(html: &str) -> Option<String> {
        if !html.contains('<') {
            // Not markup: there is nothing to reduce.
            return None;
        }
        let bytes = html.as_bytes();
        let mut out = String::with_capacity(html.len());
        let mut index = 0usize;
        while index < bytes.len() {
            if bytes[index] != b'<' {
                let start = index;
                while index < bytes.len() && bytes[index] != b'<' {
                    index += 1;
                }
                out.push_str(&html[start..index]);
                continue;
            }
            // A comment carries no document text.
            if html[index..].starts_with("<!--") {
                index = match html[index + 4..].find("-->") {
                    Some(end) => index + 4 + end + 3,
                    None => bytes.len(),
                };
                continue;
            }
            let Some(end) = find_tag_end(bytes, index) else {
                // An unterminated tag: the rest of the page is text, unless
                // it is the body of a script or a style block.
                let name = tag_name(&html[index + 1..]);
                if name != "script" && name != "style" {
                    out.push_str(&html[index..]);
                }
                break;
            };
            let name = tag_name(&html[index + 1..end]);
            index = end + 1;
            if name == "script" || name == "style" {
                // The content of a script or a style block is not the
                // document: it goes with the tags.
                index = skip_raw_element(html, index, &name);
                continue;
            }
            if is_block_tag(&name) {
                out.push(' ');
            }
        }
        let decoded = decode_entities(&out);
        let text = decoded.split_whitespace().collect::<Vec<_>>().join(" ");
        if text.is_empty() {
            return None;
        }
        Some(text)
    }
}

/// The index of the `>` that closes a tag, honouring quoted attributes.
fn find_tag_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut index = start + 1;
    let mut quote: Option<u8> = None;
    while index < bytes.len() {
        let byte = bytes[index];
        match quote {
            Some(open) => {
                if byte == open {
                    quote = None;
                }
            }
            None => {
                if byte == b'"' || byte == b'\'' {
                    quote = Some(byte);
                } else if byte == b'>' {
                    return Some(index);
                }
            }
        }
        index += 1;
    }
    None
}

/// The index just after the closing tag of a script or a style element.
fn skip_raw_element(html: &str, from: usize, name: &str) -> usize {
    let bytes = html.as_bytes();
    let mut index = from;
    while index + 1 < bytes.len() {
        if bytes[index] == b'<' && bytes[index + 1] == b'/' {
            let after = index + 2;
            let end = after + name.len();
            if end <= bytes.len() && name_matches(&bytes[after..end], name) {
                let closes = bytes
                    .get(end)
                    .is_none_or(|byte| byte.is_ascii_whitespace() || *byte == b'>');
                if closes {
                    return find_tag_end(bytes, index).map_or(bytes.len(), |pos| pos + 1);
                }
            }
        }
        index += 1;
    }
    bytes.len()
}

/// Whether the bytes are the tag name, ignoring case.
fn name_matches(bytes: &[u8], name: &str) -> bool {
    bytes.len() == name.len()
        && bytes
            .iter()
            .zip(name.bytes())
            .all(|(byte, expected)| byte.eq_ignore_ascii_case(&expected))
}

/// The name of a tag, lowercased.
fn tag_name(tag: &str) -> String {
    tag.trim_start_matches('/')
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase()
}

/// The tags that separate words in the rendered text.
fn is_block_tag(name: &str) -> bool {
    matches!(
        name,
        "address"
            | "article"
            | "aside"
            | "blockquote"
            | "br"
            | "dd"
            | "div"
            | "dl"
            | "dt"
            | "fieldset"
            | "figcaption"
            | "figure"
            | "footer"
            | "form"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "header"
            | "hr"
            | "li"
            | "main"
            | "nav"
            | "ol"
            | "p"
            | "pre"
            | "section"
            | "table"
            | "tbody"
            | "td"
            | "tfoot"
            | "th"
            | "thead"
            | "tr"
            | "ul"
    )
}

/// Decode the entities a readable page actually uses.
///
/// The entity names are held without their delimiters, and the pass is
/// single: an escaped ampersand decodes once, so a doubly escaped entity
/// stays escaped.
fn decode_entities(text: &str) -> String {
    const ENTITIES: [(&str, char); 6] = [
        ("amp", '&'),
        ("lt", '<'),
        ("gt", '>'),
        ("quot", '"'),
        ("#39", '\''),
        ("nbsp", ' '),
    ];
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut index = 0usize;
    while let Some(offset) = text[index..].find('&') {
        let start = index + offset;
        out.push_str(&text[index..start]);
        let rest = &text[start + 1..];
        let mut matched = false;
        for (name, character) in ENTITIES {
            if rest.starts_with(name) && rest[name.len()..].starts_with(';') {
                out.push(character);
                index = start + 1 + name.len() + 1;
                matched = true;
                break;
            }
        }
        if !matched {
            out.push('&');
            index = start + 1;
        }
    }
    out.push_str(&text[index..]);
    out
}

/// Reduce a dependency lockfile to its dependency names and versions.
#[derive(Debug, Clone, Copy, Default)]
pub struct Lockfile;

impl Method for Lockfile {
    fn name(&self) -> &'static str {
        "lockfile"
    }
    fn info(&self) -> MethodInfo {
        MethodInfo {
            lossy: true,
            recovery: Recovery::Recall,
            consumes: &[ContentFormat::Lockfile],
            ..MethodInfo::LOSSY_RECALLABLE
        }
    }
    fn apply(&self, block: &Block, _ctx: &MethodCtx) -> Outcome {
        let entries = Lockfile::entries(block.text);
        if entries.is_empty() {
            return Outcome::Skip;
        }
        let mut out = String::new();
        out.push_str(&format!("[{} dependencies kept]\n", entries.len()));
        for (name, version) in &entries {
            out.push_str(name);
            out.push(' ');
            out.push_str(version);
            out.push('\n');
        }
        if !shrinks(&out, block.text) {
            return Outcome::Skip;
        }
        Outcome::Continue(out)
    }
}

impl Lockfile {
    /// The dependency names and versions a lockfile records.
    ///
    /// The same dependency is often listed twice — once in a lockfile's
    /// package map and once in its dependency map — so an entry that has
    /// already been seen is not repeated.
    pub fn entries(text: &str) -> Vec<(String, String)> {
        let lines: Vec<&str> = text.lines().collect();
        let mut entries: Vec<(String, String)> = Vec::new();
        let mut seen: HashSet<(String, String)> = HashSet::new();
        for (index, line) in lines.iter().enumerate() {
            let trimmed = line.trim();
            let next = lines.get(index + 1).copied();
            // The look-ahead for a version field is bounded: a dependency
            // object is a handful of lines, and the scan must not walk the
            // whole file for every key.
            let ahead = &lines[(index + 1).min(lines.len())..(index + 21).min(lines.len())];
            let entry = cargo_entry(trimmed, next)
                .or_else(|| at_version_entry(trimmed))
                .or_else(|| versioned_object_entry(line, ahead));
            if let Some((name, version)) = entry
                && seen.insert((name.clone(), version.clone()))
            {
                entries.push((name, version));
            }
        }
        entries
    }
}

/// A Cargo.lock `key = "value"` entry.
///
/// A Cargo.lock names a package on one line and versions it on the next,
/// so the two are read together.
fn cargo_entry(line: &str, next: Option<&str>) -> Option<(String, String)> {
    let (key, value) = line.split_once('=')?;
    let key = key.trim();
    let value = quoted(value.trim())?;
    if key == "name" {
        let next = next?.trim();
        let (version_key, version) = next.split_once('=')?;
        if version_key.trim() != "version" {
            return None;
        }
        let version = quoted(version.trim())?;
        return Some((value.to_string(), version.to_string()));
    }
    if !is_package_name(key) {
        return None;
    }
    Some((key.to_string(), value.to_string()))
}

/// A `"name@version":` entry, the shape pnpm-lock.yaml uses.
fn at_version_entry(line: &str) -> Option<(String, String)> {
    let (key, after_colon) = match quoted_key(line) {
        Some((key, rest)) => {
            let after = rest.trim_start().strip_prefix(':')?;
            (key, after)
        }
        None => {
            // An unquoted YAML mapping key, the shape pnpm-lock.yaml uses for
            // a package that needs no quoting.
            let trimmed = line.trim_start();
            let colon = trimmed.find(':')?;
            let key = trimmed[..colon].trim_end();
            if key.is_empty() || key.chars().any(char::is_whitespace) {
                return None;
            }
            (key.to_string(), &trimmed[colon + 1..])
        }
    };
    // A mapping key, not a value.
    let after = after_colon.trim_start();
    if !(after.is_empty() || after.starts_with('{')) {
        return None;
    }
    // The last `@` separates the name from the version, so a scoped name
    // (`@scope/pkg@1.0.0`) still splits in the right place.
    let at = key.rfind('@')?;
    let name = &key[..at];
    let version = &key[at + 1..];
    if name.is_empty() || version.is_empty() {
        return None;
    }
    Some((dependency_name(name), version.to_string()))
}

/// A `"name": { "version": "x" }` entry, the shape package-lock.json uses.
fn versioned_object_entry(line: &str, ahead: &[&str]) -> Option<(String, String)> {
    let (key, rest) = quoted_key(line)?;
    if key.is_empty() {
        // The root project is not a dependency.
        return None;
    }
    let rest = rest.trim_start().strip_prefix(':')?.trim_start();
    let body = rest.strip_prefix('{')?;
    // The version on the same line as the key.
    if let Some(version) = first_field_version(body) {
        return Some((dependency_name(&key), version));
    }
    // Otherwise the version is a field of the object below the key. The
    // field has to belong to this object: a deeper line is a field of a
    // nested object, and a shallower one ends the object.
    let key_indent = indent_of(line);
    let mut field_indent: Option<usize> = None;
    for next in ahead {
        let trimmed = next.trim();
        if trimmed.is_empty() {
            continue;
        }
        let indent = indent_of(next);
        if indent <= key_indent {
            return None;
        }
        let field = *field_indent.get_or_insert(indent);
        // A deeper line is a field of a nested object, not of this one.
        if indent != field {
            continue;
        }
        if let Some(version) = first_field_version(trimmed) {
            return Some((dependency_name(&key), version));
        }
    }
    None
}

/// The value of the first `"version": "x"` field of an object body.
fn first_field_version(body: &str) -> Option<String> {
    let rest = body.trim_start().strip_prefix("\"version\"")?;
    let rest = rest.trim_start().strip_prefix(':')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// The quoted key at the start of a line, with the text after it.
fn quoted_key(line: &str) -> Option<(String, &str)> {
    let trimmed = line.trim_start();
    let quote = trimmed.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let body = &trimmed[1..];
    let end = body.find(quote)?;
    Some((body[..end].to_string(), &body[end + 1..]))
}

/// The text inside a pair of quotes, if it is quoted.
fn quoted(text: &str) -> Option<&str> {
    if text.len() < 2 || !text.starts_with('"') || !text.ends_with('"') {
        return None;
    }
    Some(&text[1..text.len() - 1])
}

/// Whether a key is a package name rather than lockfile metadata.
fn is_package_name(key: &str) -> bool {
    if key.is_empty() || key.len() > 200 {
        return false;
    }
    if matches!(
        key,
        "version" | "source" | "checksum" | "dependencies" | "replace" | "metadata"
    ) {
        return false;
    }
    key.chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '+'))
}

/// The package a lockfile key refers to.
fn dependency_name(key: &str) -> String {
    match key.rfind("node_modules/") {
        Some(index) => key[index + "node_modules/".len()..].to_string(),
        None => key.to_string(),
    }
}

/// The width of a line's leading whitespace, in bytes.
fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start().len()
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

    /// An HTML entity, built so that this file never carries a literal one.
    fn entity(name: &str) -> String {
        format!("&{name};")
    }

    /// A method never hands back a longer payload than it was given.
    fn assert_not_larger(out: &str, input: &str) {
        assert!(
            out.len() <= input.len(),
            "grew from {} to {} bytes",
            input.len(),
            out.len()
        );
        assert!(
            out.chars().count() <= input.chars().count(),
            "grew from {} to {} characters",
            input.chars().count(),
            out.chars().count()
        );
    }

    #[test]
    fn toon_encodes_a_uniform_table() {
        assert_lossy_contract(&Toon);
        let json = r#"[{"id":1,"name":"a"},{"id":2,"name":"b"}]"#;
        let out = run(&Toon, json, ContentFormat::Json).expect("encodes");
        assert!(out.starts_with("[2]{id,name}:"), "got {out}");
        assert!(out.contains("\n  1,a"), "got {out}");
        assert_not_larger(&out, json);
    }

    #[test]
    fn toon_names_a_table_from_its_key() {
        let json = r#"{"tasks":[{"id":"1","title":"Fix auth bug","status":"open"}]}"#;
        let out = run(&Toon, json, ContentFormat::Json).expect("encodes");
        // The column order follows the JSON map's own iteration order, and
        // serde_json's default `Map` is a `BTreeMap` — so the columns come
        // out ALPHABETICALLY, not in the order the document wrote them. The
        // row follows the header, so it does too.
        assert!(out.starts_with("tasks[1]{id,status,title}:"), "got {out}");
        assert!(out.contains(r#"  "1",open,Fix auth bug"#), "got {out}");
        assert_not_larger(&out, json);
    }

    #[test]
    fn toon_declines_anything_that_is_not_a_uniform_table() {
        // A nested value, a mixed key set, an empty array, no array at all.
        assert!(run(&Toon, r#"[{"id":{"nested":1}}]"#, ContentFormat::Json).is_none());
        assert!(
            run(
                &Toon,
                r#"[{"id":1},{"id":2,"extra":true}]"#,
                ContentFormat::Json
            )
            .is_none()
        );
        assert!(run(&Toon, "[]", ContentFormat::Json).is_none());
        assert!(run(&Toon, "{}", ContentFormat::Json).is_none());
        assert!(run(&Toon, "", ContentFormat::Json).is_none());
        assert!(run(&Toon, "not json", ContentFormat::Json).is_none());
    }

    #[test]
    fn toon_quotes_the_values_that_would_change_meaning() {
        let json = r#"[{"text":"a,b"},{"text":"line\nbreak"}]"#;
        let out = run(&Toon, json, ContentFormat::Json).expect("encodes");
        assert!(out.contains(r#""a,b""#), "got {out}");
        assert!(out.contains(r#""line\nbreak""#), "got {out}");
        assert_not_larger(&out, json);
    }

    #[test]
    fn toon_keeps_unicode_intact() {
        let json = "[\n  {\"name\": \"日本語\"},\n  {\"name\": \"中文\"}\n]";
        let out = run(&Toon, json, ContentFormat::Json).expect("encodes");
        assert!(out.contains("日本語"), "got {out}");
        assert!(out.contains("中文"), "got {out}");
        assert_not_larger(&out, json);
    }

    #[test]
    fn grep_group_groups_matches_by_file() {
        assert_lossy_contract(&GrepGroup);
        let path = "crates/operant-core/src/agent/evidence/methods/lossy.rs";
        let text =
            format!("{path}:1:x\n{path}:2:x\n{path}:3:x\n{path}:4:x\n{path}:5:x\n{path}:6:x");
        let out = run(&GrepGroup, &text, ContentFormat::Matches).expect("groups");
        assert!(out.contains(&format!("{path} (6 matches)")), "got {out}");
        assert!(out.contains("  3:x"), "got {out}");
        assert_not_larger(&out, &text);
    }

    #[test]
    fn grep_group_keeps_a_windows_path_intact() {
        let path = "C:\\src\\operant\\crates\\operant-core\\src\\agent\\evidence.rs";
        let text = format!("{path}:1:fn a() {{}}\n{path}:9:fn b() {{}}");
        let out = run(&GrepGroup, &text, ContentFormat::Matches).expect("groups");
        assert!(out.contains(path), "the path must not be mangled: {out}");
        assert!(!out.contains("C (2 matches)"), "got {out}");
        assert_not_larger(&out, &text);
    }

    #[test]
    fn grep_group_groups_a_match_without_a_line_number() {
        let path = "crates/operant-core/src/agent/evidence/methods/lossy.rs";
        let text = format!("{path}:fn a() {{}}\n{path}:fn b() {{}}");
        let out = run(&GrepGroup, &text, ContentFormat::Matches).expect("groups");
        assert!(out.contains(&format!("{path} (2 matches)")), "got {out}");
        assert_not_larger(&out, &text);
    }

    #[test]
    fn grep_group_skips_a_payload_it_cannot_group() {
        assert!(run(&GrepGroup, "", ContentFormat::Matches).is_none());
        assert!(
            run(
                &GrepGroup,
                "output with no path\nnor here",
                ContentFormat::Matches
            )
            .is_none()
        );
        assert!(run(&GrepGroup, "single match:12", ContentFormat::Matches).is_none());
    }

    #[test]
    fn cap_keeps_the_tail_and_counts_the_dropped_lines() {
        assert_lossy_contract(&Cap);
        let text: String = (0..500).map(|i| format!("line {i}\n")).collect();
        let out = run(&Cap, &text, ContentFormat::Term).expect("caps");
        assert!(out.contains("[… 300 earlier lines elided]"), "got {out}");
        assert!(out.contains("line 499"), "the tail must survive");
        assert!(!out.contains("line 0\n"), "the head must be dropped");
        assert_not_larger(&out, &text);
    }

    #[test]
    fn cap_leaves_a_short_payload_alone() {
        assert!(run(&Cap, "one line\n", ContentFormat::Term).is_none());
        assert!(run(&Cap, "", ContentFormat::Term).is_none());
    }

    /// The contract a lossy method has to declare.
    fn assert_lossy_contract(method: &dyn Method) {
        let info = method.info();
        assert!(info.lossy, "{} must declare itself lossy", method.name());
        assert_eq!(
            info.recovery,
            Recovery::Recall,
            "{} must name how the lost text comes back",
            method.name()
        );
        assert!(
            !info.consumes.is_empty(),
            "{} must say which formats it consumes",
            method.name()
        );
    }

    #[test]
    fn truncate_keeps_two_thirds_and_one_third() {
        assert_lossy_contract(&Truncate);
        let text: String = (0..4000).map(|i| format!("{i} ")).collect();
        let out = run(&Truncate, &text, ContentFormat::Plain).expect("truncates");
        let total = text.chars().count();
        assert!(out.contains(&format!("{total} chars total")), "got {out}");
        assert!(out.starts_with("0 1 2 "), "got {out}");
        assert_not_larger(&out, &text);
    }

    #[test]
    fn truncate_counts_characters_not_bytes() {
        let text: String = "日本語".repeat(2000);
        let out = run(&Truncate, &text, ContentFormat::Plain).expect("truncates");
        let head = Truncate::BUDGET * 2 / 3;
        let tail = Truncate::BUDGET - head;
        let kept_head: String = text.chars().take(head).collect();
        let kept_tail: String = text.chars().skip(text.chars().count() - tail).collect();
        assert_eq!(head + tail, Truncate::BUDGET);
        assert!(
            out.starts_with(&kept_head),
            "the head must be exactly the budgeted characters"
        );
        assert!(
            out.ends_with(&kept_tail),
            "the tail must be exactly the budgeted characters"
        );
        assert_not_larger(&out, &text);
    }

    #[test]
    fn truncate_leaves_a_short_payload_alone() {
        assert!(run(&Truncate, "short", ContentFormat::Plain).is_none());
        assert!(run(&Truncate, "", ContentFormat::Plain).is_none());
    }

    #[test]
    fn web_md_drops_script_and_style_content() {
        assert_lossy_contract(&WebMd);
        let html = "<html><head><style>body { color: red; }</style><script>var x = 1;</script></head><body><h1>Title</h1><p>Hello <b>world</b>.</p></body></html>";
        let out = run(&WebMd, html, ContentFormat::Web).expect("reduces");
        assert!(out.contains("Title"), "got {out}");
        assert!(out.contains("Hello world."), "got {out}");
        assert!(
            !out.contains("var x"),
            "the script is not the document: {out}"
        );
        assert!(
            !out.contains("color: red"),
            "the style is not the document: {out}"
        );
        assert_not_larger(&out, html);
    }

    #[test]
    fn web_md_decodes_entities_and_drops_comments() {
        let html = format!(
            "<p>Tom {0} Jerry {1}3</p><!-- a comment --><p>He said {2}hi{3}</p>",
            entity("amp"),
            entity("lt"),
            entity("quot"),
            entity("quot")
        );
        let out = run(&WebMd, &html, ContentFormat::Web).expect("reduces");
        assert!(out.contains("Tom & Jerry <3"), "got {out}");
        assert!(out.contains("He said \"hi\""), "got {out}");
        assert!(!out.contains("comment"), "got {out}");
        assert_not_larger(&out, &html);
    }

    #[test]
    fn web_md_handles_awkward_markup() {
        let html = "<a title=\"a > b\">link</a><br/><img src=\"x.png\"/><p>tail";
        let out = run(&WebMd, html, ContentFormat::Web).expect("reduces");
        assert!(out.contains("link"), "got {out}");
        assert!(out.contains("tail"), "got {out}");
        assert!(!out.contains("title"), "got {out}");
        assert_not_larger(&out, html);
    }

    #[test]
    fn web_md_skips_a_payload_with_no_tags() {
        assert!(run(&WebMd, "plain text, no markup at all", ContentFormat::Web).is_none());
        assert!(run(&WebMd, "", ContentFormat::Web).is_none());
    }

    #[test]
    fn lockfile_reads_a_cargo_lock() {
        assert_lossy_contract(&Lockfile);
        let text = concat!(
            "# This file is automatically @generated by Cargo.\n",
            "version = 3\n",
            "\n",
            "[[package]]\n",
            "name = \"serde\"\n",
            "version = \"1.0.197\"\n",
            "source = \"registry+https://github.com/rust-lang/crates.io-index\"\n",
            "checksum = \"abc123\"\n",
            "\n",
            "[[package]]\n",
            "name = \"itoa\"\n",
            "version = \"1.0.11\"\n",
        );
        let out = run(&Lockfile, text, ContentFormat::Lockfile).expect("reduces");
        assert!(out.contains("serde 1.0.197"), "got {out}");
        assert!(out.contains("itoa 1.0.11"), "got {out}");
        assert!(out.contains("[2 dependencies kept]"), "got {out}");
        assert!(
            !out.contains("source"),
            "the metadata is not a dependency: {out}"
        );
        assert_not_larger(&out, text);
    }

    #[test]
    fn lockfile_reads_a_package_lock() {
        let text = concat!(
            "{\n",
            "  \"name\": \"app\",\n",
            "  \"lockfileVersion\": 3,\n",
            "  \"packages\": {\n",
            "    \"\": {\n",
            "      \"name\": \"app\",\n",
            "      \"version\": \"1.0.0\"\n",
            "    },\n",
            "    \"node_modules/left-pad\": {\n",
            "      \"version\": \"1.3.0\",\n",
            "      \"resolved\": \"https://example.com/left-pad\",\n",
            "      \"integrity\": \"sha512-abc\"\n",
            "    }\n",
            "  },\n",
            "  \"dependencies\": {\n",
            "    \"left-pad\": {\n",
            "      \"version\": \"1.3.0\"\n",
            "    }\n",
            "  }\n",
            "}\n",
        );
        let out = run(&Lockfile, text, ContentFormat::Lockfile).expect("reduces");
        assert!(out.contains("left-pad 1.3.0"), "got {out}");
        assert!(out.contains("[1 dependencies kept]"), "got {out}");
        assert!(
            !out.contains("app"),
            "the root project is not a dependency: {out}"
        );
        assert_not_larger(&out, text);
    }

    #[test]
    fn lockfile_reads_a_pnpm_lock() {
        let text = concat!(
            "lockfileVersion: '9.0'\n",
            "\n",
            "packages:\n",
            "\n",
            "  '@babel/core@7.26.0':\n",
            "    resolution: {integrity: sha512-abc}\n",
            "\n",
            "  left-pad@1.3.0:\n",
            "    resolution: {integrity: sha512-def}\n",
        );
        let out = run(&Lockfile, text, ContentFormat::Lockfile).expect("reduces");
        assert!(out.contains("@babel/core 7.26.0"), "got {out}");
        assert!(out.contains("left-pad 1.3.0"), "got {out}");
        assert!(out.contains("[2 dependencies kept]"), "got {out}");
        assert_not_larger(&out, text);
    }

    #[test]
    fn lockfile_skips_a_payload_with_no_dependencies() {
        assert!(run(&Lockfile, "", ContentFormat::Lockfile).is_none());
        assert!(run(&Lockfile, "name = \"serde\"\n", ContentFormat::Lockfile).is_none());
    }

    #[test]
    fn every_lossy_method_declares_its_recovery() {
        let methods = super::super::LOSSY;
        assert!(!methods.is_empty(), "the lossy registry must not be empty");
        for method in methods {
            assert_lossy_contract(*method);
        }
    }

    #[test]
    fn every_lossy_method_skips_an_empty_payload() {
        for method in super::super::LOSSY {
            assert!(
                run(*method, "", ContentFormat::Plain).is_none(),
                "{} must skip an empty payload",
                method.name()
            );
        }
    }
}
