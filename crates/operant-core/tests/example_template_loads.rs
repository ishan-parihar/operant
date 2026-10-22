#![allow(clippy::expect_used, clippy::unwrap_used)]

//! Every commented example in `operant.example.toml` must name a field the
//! config loader accepts.
//!
//! The template is the operator's menu: the commented blocks are the examples
//! they uncomment. The settings structs use `deny_unknown_fields`, so ONE
//! unknown key rejects the WHOLE file — a template that advertises a key the
//! binary does not know hands every operator who follows it a config that
//! will not load.
//!
//! That is not hypothetical. The template carried `fallbacks = [...]` (the
//! live field is `fallback_models`, a list of strings), an
//! `[agent] max_tool_result_chars` knob that never existed in the live config
//! (it lived in the retired schema aggregate), and a whole `[channels.*]`
//! section for a crate deleted in iter-892. None of them load.
//!
//! This test uncomments every commented statement — alone, and together with
//! the commented table header that encloses it — and hands the result to the
//! REAL loader (`parse_config_str`, the function `load_app_config` uses). A
//! statement is accepted when at least one placement loads without an
//! unknown-field verdict.

use std::path::{Path, PathBuf};

/// The repository root (this crate lives at `<root>/crates/operant-core`).
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/operant-core is two levels under the repository root")
        .to_path_buf()
}

/// The text after a line's comment markers (`""` for a line that is not a
/// comment).
fn body(line: &str) -> &str {
    let trimmed = line.trim_start();
    let hashes = trimmed.chars().take_while(|c| *c == '#').count();
    if hashes == 0 {
        return "";
    }
    trimmed[hashes..].trim()
}

/// How many comment markers the line carries (`0` for a live line).
fn comment_depth(line: &str) -> usize {
    line.trim_start().chars().take_while(|c| *c == '#').count()
}

/// Whether the fragment is a table header (`[table]` or `[[array]]`) and not
/// prose that merely starts with a bracket.
fn is_table_header(fragment: &str) -> bool {
    fragment.starts_with('[') && fragment.ends_with(']')
}

/// The line with its comment markers removed.
fn uncommented(line: &str) -> String {
    let trimmed = line.trim_start();
    let indent = &line[..line.len() - trimmed.len()];
    let hashes = trimmed.chars().take_while(|c| *c == '#').count();
    let after = &trimmed[hashes..];
    let after = after.strip_prefix(' ').unwrap_or(after);
    format!("{indent}{after}")
}

/// Net bracket depth of a fragment, ignoring brackets inside string literals.
fn bracket_depth(text: &str) -> i32 {
    let mut depth = 0i32;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' | '\'' => {
                for inner in chars.by_ref() {
                    if inner == c {
                        break;
                    }
                }
            }
            '[' | '{' => depth += 1,
            ']' | '}' => depth -= 1,
            _ => {}
        }
    }
    depth
}

/// `key = ...` at the start of the fragment (spaces before `=` are the
/// template's style).
fn is_key_assignment(fragment: &str) -> bool {
    let mut chars = fragment.chars().peekable();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    while let Some(&c) = chars.peek() {
        if c == '=' {
            return true;
        }
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' || c == ' ' || c == '\t' {
            chars.next();
            continue;
        }
        return false;
    }
    false
}

/// One commented TOML statement in the template.
struct Statement {
    /// Index of the first line.
    start: usize,
    /// Index of the last line (inclusive) — a multi-line value spans several.
    end: usize,
    /// Whether the statement is a table header rather than a key.
    is_header: bool,
}

/// Every commented TOML statement in the template.
fn statements(lines: &[&str]) -> Vec<Statement> {
    let mut out: Vec<Statement> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let fragment = body(lines[i]);
        if fragment.is_empty() {
            i += 1;
            continue;
        }
        let is_header = is_table_header(fragment);
        if !(is_header || is_key_assignment(fragment)) {
            i += 1;
            continue;
        }
        let mut end = i;
        if !is_header && bracket_depth(fragment) > 0 {
            // A value that opens a bracket continues onto the following lines
            // (a multi-line array in the template). The continuation stops at
            // the first line that is not a comment.
            let mut depth = bracket_depth(fragment);
            let mut j = i + 1;
            while j < lines.len() && depth > 0 && comment_depth(lines[j]) > 0 {
                depth += bracket_depth(body(lines[j]));
                end = j;
                j += 1;
            }
        }
        out.push(Statement {
            start: i,
            end,
            is_header,
        });
        i = end + 1;
    }
    out
}

/// The nearest preceding commented table header (the block the statement sits
/// in when the operator uncomments the whole example).
fn enclosing_header(lines: &[&str], start: usize) -> Option<usize> {
    (0..start).rev().find(|&j| is_table_header(body(lines[j])))
}

/// The template with the given lines uncommented.
fn candidate_text(lines: &[&str], uncomment: &[usize]) -> String {
    let mut out: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    for &i in uncomment {
        out[i] = uncommented(lines[i]);
    }
    out.join("\n")
}

/// The loader's verdict for a candidate: the field it rejects, if any.
fn unknown_field(text: &str) -> Option<String> {
    let marker = "unknown field `";
    let start = text.find(marker)? + marker.len();
    let rest = &text[start..];
    let end = rest.find('`')?;
    Some(rest[..end].to_string())
}

/// The loader's error text for a candidate (empty when it loads).
fn load(text: &str) -> String {
    match operant_core::config::parse_config_str(text, Path::new("memory://template")) {
        Ok(_) => String::new(),
        Err(error) => error.to_string(),
    }
}

/// Every commented example the loader rejects, in both placements.
fn unknown_fields_in(lines: &[&str]) -> Vec<String> {
    let mut findings = Vec::new();
    for statement in statements(lines) {
        let range: Vec<usize> = (statement.start..=statement.end).collect();
        let alone = load(&candidate_text(lines, &range));
        let with_header = if statement.is_header {
            alone.clone()
        } else {
            match enclosing_header(lines, statement.start) {
                Some(header) => {
                    let mut idxs = range.clone();
                    idxs.push(header);
                    load(&candidate_text(lines, &idxs))
                }
                None => alone.clone(),
            }
        };
        if let (Some(alone_field), Some(header_field)) =
            (unknown_field(&alone), unknown_field(&with_header))
        {
            findings.push(format!(
                "line {}: the loader rejects `{alone_field}` (and `{header_field}` inside its table) — `{}`",
                statement.start + 1,
                body(lines[statement.start])
            ));
        }
    }
    findings
}

#[test]
fn every_commented_example_in_the_template_names_a_field_the_loader_accepts() {
    let template = repo_root().join("operant.example.toml");
    let raw = std::fs::read_to_string(&template).expect("the template is readable");
    let lines: Vec<&str> = raw.lines().collect();

    let statements = statements(&lines);
    assert!(
        statements.len() > 10,
        "the template has almost no commented examples left — this test would be \
         vacuous, so either the examples moved or the test should go"
    );

    let findings = unknown_fields_in(&lines);
    assert!(
        findings.is_empty(),
        "operant.example.toml advertises example(s) the config loader REJECTS \
         (one unknown key rejects the whole file, so following the example \
         breaks the operator's config):\n  {}",
        findings.join("\n  ")
    );
}

#[test]
fn the_template_gate_catches_an_unknown_key_and_an_unknown_table() {
    // The gate must have teeth: the two defects this row is about are injected
    // back into a copy of the template, and the gate must report both.
    let template = repo_root().join("operant.example.toml");
    let raw = std::fs::read_to_string(&template).expect("the template is readable");
    let mut lines: Vec<String> = raw.lines().map(|l| l.to_string()).collect();

    // `fallbacks` under `[agent]` — the key the README once advertised.
    let agent = lines
        .iter()
        .position(|l| l.trim() == "[agent]")
        .expect("the template has an [agent] table");
    lines.insert(agent + 1, "# fallbacks = [\"gpt-4o-mini\"]".to_string());
    // `[channels.telegram]` — the table of the crate deleted in iter-892.
    lines.push("# [channels.telegram]".to_string());

    let borrowed: Vec<&str> = lines.iter().map(|l| l.as_str()).collect();
    let findings = unknown_fields_in(&borrowed);
    assert!(
        findings.iter().any(|f| f.contains("fallbacks")),
        "the gate did not catch the retired `fallbacks` key: {findings:?}"
    );
    assert!(
        findings.iter().any(|f| f.contains("channels")),
        "the gate did not catch the deleted `[channels.telegram]` table: {findings:?}"
    );
}

// ── RT-154: the template must not promise encryption it does not do ──────────

/// Phrases that claim config-file secrets are protected at rest. RT-154 is
/// exactly this promise made twice by `ModelProviderConfig`'s own doc comments
/// ("Stored via the OS keyring when possible", "each element is encrypted on
/// save"), and the only code that honoured the marker never ran. The live write
/// path writes the value as given and the loader never decrypts.
///
/// The template is the layer this test can actually check: an operator reads
/// the commented examples, so a promise here is a promise to the person most
/// likely to rely on it.
const ENCRYPTION_PROMISES: &[&str] = &[
    "stored via the os keyring",
    "stored in the os keyring",
    "encrypted on save",
    "encrypted at rest",
    "stored encrypted",
    "never write",
];

#[test]
fn the_template_does_not_promise_encryption_for_secret_config_values() {
    let template = repo_root().join("operant.example.toml");
    let raw = std::fs::read_to_string(&template).expect("the template is readable");

    let mut promises = Vec::new();
    for (index, line) in raw.lines().enumerate() {
        let lower = line.to_lowercase();
        for phrase in ENCRYPTION_PROMISES {
            if lower.contains(phrase) {
                promises.push(format!(
                    "line {}: promises `{phrase}` — the config file stores secrets in \
                     PLAINTEXT in a 0600 file. State that instead (see RT-154).",
                    index + 1
                ));
            }
        }
    }
    assert!(
        promises.is_empty(),
        "operant.example.toml makes an encryption promise the loader cannot keep:\n  {}",
        promises.join("\n  ")
    );
}

#[test]
fn the_plaintext_gate_catches_a_promise_the_template_must_not_make() {
    // The gate must have teeth: an encryption promise is injected into a copy
    // of the template and the test above must report it. Without this, a
    // template that regains the RT-154 wording passes silently.
    let template = repo_root().join("operant.example.toml");
    let raw = std::fs::read_to_string(&template).expect("the template is readable");
    let planted =
        format!("{raw}\n# api_key = \"...\"  # Stored via the OS keyring when possible\n");

    let mut hits = Vec::new();
    for line in planted.lines() {
        let lower = line.to_lowercase();
        if ENCRYPTION_PROMISES.iter().any(|p| lower.contains(p)) {
            hits.push(line.trim().to_string());
        }
    }
    assert!(
        hits.iter().any(|h| h.contains("keyring")),
        "the gate did not catch an injected keyring promise: {hits:?}"
    );
}
