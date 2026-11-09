//! RT-435: the learning annotation's documented kind list must match the
//! kinds the emitters actually write.
//!
//! The note used to name `skill_changes`, `learnings` and `proposals`. The
//! live kinds are `skill_changes`, `learnings` and `learning_changes` — the
//! knowledge-base edits land under the last one — and nothing writes
//! `proposals` at all. A consumer keying on the old list missed the
//! knowledge-base edits and waited for proposals that never arrive.
//!
//! This contract derives the kinds from the EMITTERS and compares them with
//! the note, so the note cannot drift from them again.

use std::collections::BTreeSet;
use std::path::Path;

/// Every `record_learning_annotation(` call site in the crate, and the kind
/// literal each one passes.
fn emitted_kinds() -> BTreeSet<String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut stack = vec![root];
    let mut kinds = BTreeSet::new();
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            // A call with a string-literal kind: record_learning_annotation(
            //     &run_dir, "kind", ...
            let mut rest = text.as_str();
            while let Some(i) = rest.find("record_learning_annotation(") {
                // Skip the DEFINITION site (the fn itself) — only call sites
                // name a kind.
                let before = &rest[..i];
                if before.trim_end().ends_with("fn") {
                    rest = &rest[i + "record_learning_annotation(".len()..];
                    continue;
                }
                let tail = &rest[i + "record_learning_annotation(".len()..];
                let Some(quote) = tail.find('"') else {
                    break;
                };
                let after = &tail[quote + 1..];
                let Some(end) = after.find('"') else { break };
                kinds.insert(after[..end].to_string());
                rest = &tail[quote + 1 + end..];
            }
        }
    }
    kinds
}

/// The kinds the note claims, read out of the note itself.
fn documented_kinds() -> BTreeSet<String> {
    let org = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("org")
            .join("mod.rs"),
    )
    .expect("org/mod.rs exists");
    // The note is the doc comment on record_learning_annotation.
    let start = org
        .find("/// W6 (audit O-22, invariant 4)")
        .expect("the note exists");
    let note = &org[start..];
    let end = note
        .find("pub fn record_learning_annotation")
        .unwrap_or(note.len());
    let note = &note[..end];
    // Only the LIVE-KIND sentence is parsed — the note's later prose names
    // other kinds to say they are NOT live.
    let list_start = note
        .find("The LIVE kinds are")
        .expect("the note names its live kinds");
    let list_end = note[list_start..]
        .find('.')
        .map(|i| list_start + i)
        .unwrap_or(note.len());
    let note = &note[list_start..list_end];
    let mut kinds = BTreeSet::new();
    let mut rest = note;
    while let Some(i) = rest.find('`') {
        let tail = &rest[i + 1..];
        let Some(end) = tail.find('`') else { break };
        let candidate = &tail[..end];
        // A kind is a snake_case word in the note's list.
        if candidate
            .chars()
            .all(|c| c.is_ascii_lowercase() || c == '_')
            && candidate.len() > 4
            && !candidate.contains(' ')
        {
            kinds.insert(candidate.to_string());
        }
        rest = &tail[end + 1..];
    }
    kinds
}

#[test]
fn the_documented_annotation_kinds_are_the_kinds_the_emitters_write() {
    let emitted = emitted_kinds();
    assert!(
        !emitted.is_empty(),
        "the scan found no emitters — the scan itself is broken"
    );
    let documented = documented_kinds();
    assert_eq!(
        documented, emitted,
        "the note's kind list must be exactly the kinds the emitters write"
    );
    // And the kind the old list named instead of the live one is named
    // explicitly, so a reader knows it is not coming.
    let proposals = "proposals";
    assert!(
        !emitted.contains(proposals),
        "nothing writes `proposals` — the note must not list it as live"
    );
}
