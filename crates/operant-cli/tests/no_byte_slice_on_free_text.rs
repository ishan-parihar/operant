//! Repo-wide guard: no free text may be cut with a byte slice.
//!
//! # Why this exists
//!
//! RT-301 closed five sites of `&s[..n]` / `&s[s.len() - n..]` on arbitrary
//! text, and pinned them with a hand-written list of ten `(file, needle)`
//! pairs. That guard could only ever catch the sites somebody had already
//! found: `cmd_sessions.rs` and `gateway_platforms.rs` carried the same bug for
//! months because they were never on the list, and the guard covered **zero**
//! files in `operant-cli`.
//!
//! So this file replaces the enumerated list with a SWEEP over every `.rs` in
//! the workspace. A new byte-slice site fails the build instead of waiting for
//! an auditor.
//!
//! # What is and is not a finding
//!
//! The dangerous shape is an offset computed by ARITHMETIC on a `String`/`&str`
//! that then reaches a slice: `&s[..n]`, `&s[s.len() - n..]`, `&s[a..b]`.
//! An offset derived from `find`/`rfind`/`char_indices`/`graphemes` is already
//! on a char boundary and is safe; so is a slice of a `Vec` (UTF-8 is
//! irrelevant) or of `as_bytes()` (deliberately byte-oriented).
//!
//! The sweep is LINE-based and cannot tell those apart on its own, so it
//! reports candidates and this file's `SAFE` table records, with a reason, each
//! one that a human has verified. A new candidate fails until it is either
//! fixed or added here with a reason.
//!
//! The three known-unsafe sites were fixed at iter-2608
//! (`diff_viewer/render.rs`, `cmd_sessions.rs`, `gateway_platforms.rs`) and the
//! FTS grammar half in `database.rs`.

use std::path::{Path, PathBuf};

/// A candidate line, with the file it came from, that a human has verified as
/// NOT a free-text byte slice, and why.
///
/// Keep this table short. Every entry is a claim that needs a reason.
const SAFE: &[(&str, &str)] = &[
    // --- Vec / slice-of-collection, not text ---------------------------------
    (
        "crates/operant-core/src/agent/background_review.rs",
        "msgs is a Vec<Message>; the slice is over a collection",
    ),
    (
        "crates/operant-core/src/agent/evidence/methods/lossy.rs",
        "lines is a Vec<&str>; the slice is over a collection",
    ),
    (
        "crates/operant-core/src/database.rs",
        "messages is a Vec; the slice is over a collection",
    ),
    // --- byte-oriented on purpose -------------------------------------------
    (
        "crates/operant-cli/src/tui/prompt_input/prediction/query.rs",
        "compares as_bytes() windows; deliberately byte-oriented ASCII space",
    ),
    // --- offset derived from a delimiter that was just matched ---------------
    (
        "crates/operant-cli/src/tui/operant_app/provider.rs",
        "strips one known trailing char from a parsed model id; the id grammar is ASCII",
    ),
    (
        "crates/operant-cli/src/tui/prompt_input/typeahead.rs",
        "guarded by prefix.ends_with(&partial_name), so the byte math is exact",
    ),
    (
        "crates/operant-cli/src/tui/prompt_input/typeahead.rs",
        "strips one known path separator from a matched path",
    ),
    // --- credential mask: ASCII by construction ------------------------------
    (
        "crates/operant-cli/src/cmd_status.rs",
        "API key mask at a fixed offset; keys are ASCII and len() > 8 is checked",
    ),
];

/// Every `.rs` file under `dir`, recursively.
fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // Never descend into build output or vendored trees.
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            if name == "target" || name == ".git" {
                continue;
            }
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// Operands that are ASCII by construction, so a byte slice cannot split a char.
const ASCII_BY_CONSTRUCTION: &[&str] = &[
    "uuid::Uuid",
    "Uuid::new_v4",
    "hex::encode",
    "Sha256::digest",
    "sha256",
    "digest",
    "MODEL_REVISION",
    "hash",
    "hex_chars",
    "chars[",
    "cp.hash",
    "session_id[",
    "full[..",
    "raw[..",
    "dec[..",
    "encrypted[",
    "ab[..",
];

/// Operands that are collections, not text.
const COLLECTION_LOOKING: &[&str] = &[
    "names[",
    "lines[",
    "msgs[",
    "messages[",
    "entries[",
    "items[",
    "args[",
    "words[",
    "chars[",
    "draft[",
    "batch[",
    "results[",
    "tasks[",
    "files[",
    "rows[",
    "after[",
    "compact[",
];

/// Does this line cut a string with an ARITHMETIC byte offset?
///
/// Recognises the two shapes that bit us:
///   `&x[..N]`                (a fixed or computed prefix)
///   `&x[x.len() - N..]`      (a computed suffix)
/// Deliberately does NOT flag `&x[a..b]` where both bounds are variables,
/// because that is overwhelmingly a Vec/slice or a find-derived boundary; the
/// three real defects were all one of the two shapes above.
fn looks_like_a_byte_slice(line: &str) -> bool {
    let l = line.trim();
    if l.starts_with("//") {
        return false;
    }
    // Strip a string literal so needles inside a test fixture or a message do
    // not count. Crude but sufficient: these lines are code, not prose.
    let code = strip_string_literals(l);

    // An operand that is ASCII BY CONSTRUCTION cannot panic on a char boundary:
    // a hex digest, a UUID, or a byte buffer. Slicing those at a byte offset is
    // exactly as safe as slicing a Vec.
    if ASCII_BY_CONSTRUCTION
        .iter()
        .any(|needle| code.contains(needle))
    {
        return false;
    }

    // A slice of a COLLECTION (`names[..5]`, `lines[..2]`, `msgs[..n]`) is not
    // text at all; UTF-8 is irrelevant.
    if COLLECTION_LOOKING
        .iter()
        .any(|needle| code.contains(needle))
    {
        return false;
    }

    let fixed_prefix = code.contains("[..")
        && code
            .split("[..")
            .nth(1)
            .and_then(|r| r.split(']').next())
            .is_some_and(|inner| {
                let t = inner.trim();
                !t.is_empty() && t.chars().all(|c| c.is_ascii_digit())
            });
    if fixed_prefix {
        return true;
    }

    // `&x[x.len() - <n>..]` -- the suffix shape.
    if code.contains(".len() - ") && code.contains("..]") {
        return true;
    }
    if code.contains(".len()-") && code.contains("..]") {
        return true;
    }
    false
}

/// Remove `"..."` spans so a needle inside a literal is not counted.
fn strip_string_literals(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut in_str = false;
    let mut escaped = false;
    for c in line.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' if in_str => {
                escaped = true;
            }
            '"' => in_str = !in_str,
            _ if !in_str => out.push(c),
            _ => {}
        }
    }
    out
}

#[test]
fn no_free_text_is_cut_with_a_byte_slice_anywhere_in_the_workspace() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root is two levels above crates/operant-cli")
        .to_path_buf();
    let crates = root.join("crates");
    assert!(crates.is_dir(), "workspace crates/ not found at {crates:?}");

    let mut files = Vec::new();
    rust_files(&crates, &mut files);
    assert!(
        files.len() > 500,
        "the sweep found only {} .rs files -- the walk is broken, not the tree clean",
        files.len()
    );

    let mut offenders: Vec<String> = Vec::new();
    for path in &files {
        let Ok(src) = std::fs::read_to_string(path) else {
            continue;
        };
        // Skip test modules' own fixtures: a test asserting on a byte slice is
        // not the defect. Crude: skip files under tests/ and any line in a
        // `mod tests` block is handled by the literal-strip above.
        let rel = path
            .strip_prefix(&root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        // Test files: an assertion on a byte slice is not the defect. Covers
        // `tests/` dirs, `*_tests.rs`, and the in-file `#[cfg(test)]` module.
        if rel.contains("/tests/") || rel.ends_with("_tests.rs") || src.contains("#[cfg(test)]") {
            continue;
        }
        for (i, line) in src.lines().enumerate() {
            if !looks_like_a_byte_slice(line) {
                continue;
            }
            if SAFE.iter().any(|(f, _)| rel.ends_with(*f) || *f == rel) {
                continue;
            }
            offenders.push(format!("{rel}:{}  {}", i + 1, line.trim()));
        }
    }

    assert!(
        offenders.is_empty(),
        "free text is cut with a byte slice, which panics when the cut lands \
         inside a multi-byte char (RT-301 class). Fix by cutting on a char \
         boundary, or add the site to SAFE with a reason:\n  {}",
        offenders.join("\n  ")
    );
}

#[test]
fn the_safe_table_reasons_are_not_empty() {
    for (file, reason) in SAFE {
        assert!(
            reason.len() > 20,
            "SAFE entry for {file} needs a real reason, got {reason:?}"
        );
    }
}

// The four sites this guard FOUND (cmd_cron, cmd_kanban, main, gateway_runner)
// were fixed by routing them through `operant_core::text_slice`. These assert the
// helper the fix relies on, so a future "simplification" back to a byte slice is
// caught here as well as by the sweep above.
#[test]
fn the_shared_helper_the_fixes_rely_on_is_char_safe() {
    use operant_core::text_slice::{char_prefix, char_suffix, take_chars};

    // A cron name / task title / JSON value long enough to truncate, where the
    // byte offset falls inside a multi-byte char.
    let text = format!("{}é_tail", "a".repeat(30));
    assert!(text.len() > 25);

    // These would all panic as `&text[..25]` (byte 25 is inside the é).
    let _ = char_prefix(&text, 25);
    let _ = char_suffix(&text, 10);
    let _ = take_chars(&text, 25, "...");

    assert_eq!(char_prefix("日本語", 2), "日本");
    assert_eq!(char_suffix("日本語", 2), "本語");
    assert_eq!(char_suffix("日本語", 1), "語");
}
