//! **No source file may claim an isolation layer that does not exist in this
//! workspace.**
//!
//! Two comments asserted that real isolation lived in `operant-runtime`'s
//! sandbox backends (landlock, bubblewrap, firejail, docker, `SandboxBackend`):
//! `crates/operant-core/src/blast_radius.rs:25-26` and, much worse,
//! `crates/operant-core/src/approval.rs:654-657`, which sits **inside
//! `check_tool_approval`** — the enforced entry point every tool call passes
//! through, and the one place an engineer debugging an approval decision will
//! read.
//!
//! All of those claims were false. `operant-runtime` was deleted in iter-892 and
//! is absent from `Cargo.lock`; `SandboxBackend` matches nothing anywhere in the
//! tree; and `rg -ci seccomp|seatbelt|apparmor crates/` returns **0 files**
//! each. The only `landlock` and `bubblewrap` hits in the workspace were the two
//! comments asserting they existed. A reader was told a second enforcement layer
//! protected them when none did, in the file they would open to check.
//!
//! The defect was not the missing sandbox — the operator has not asked for one,
//! and this test does not demand one. The defect was a **documented false safety
//! claim on the security surface**. That is what this guard pins.
//!
//! ## The trap this guard hit while it was being written (worth reading)
//!
//! The first version analysed **one line at a time**, and when it was proven
//! against a restored copy of the original `approval.rs` defect it **passed**.
//! The original claim was line-wrapped:
//!
//! ```text
//! // `operant_runtime`'s landlock/bubblewrap/firejail/docker backends for
//! // real isolation.
//! ```
//!
//! The mechanism name and the promise sat on *different* lines, so no single
//! line satisfied the guard's conjunction and the exact defect was waved
//! through. The same guard DID catch the `blast_radius.rs` mutation — only
//! because that phrasing happened to keep name and promise on one line. A
//! checker that catches one instance of a defect class and misses another is
//! worse than none, because it manufactures false confidence.
//!
//! The unit of analysis is therefore the **contiguous comment block**, joined
//! into one string. Both restored defects fail `block_promises_sandbox`.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::{Path, PathBuf};

fn crate_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// This very file quotes the offending phrases, so scanning itself would fail on
/// its own documentation (which it did on the first run). Excluded by name.
fn is_self(path: &Path) -> bool {
    path.file_name().and_then(|n| n.to_str()) == Some("no_phantom_isolation_layer.rs")
}

/// Every `.rs` file under `crate_dir`, sorted for deterministic output. Walk, do
/// not guess: the defect lived in two files that share no module path, so naming
/// the sites would have missed one.
fn rust_files(crate_dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![crate_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            // A directory we cannot read is not a place a claim can hide.
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") && !is_self(&path) {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

/// True when a comment BLOCK affirmatively promises that a sandbox backend
/// provides isolation.
///
/// Three conditions, all required:
/// 1. it names a real sandbox mechanism (so ordinary prose about "defense in
///    depth" is not flagged);
/// 2. it asserts that mechanism exists / provides isolation;
/// 3. it does not, anywhere in the same block, retract the claim.
///
/// Condition 3 matters because the corrections legitimately walk through the
/// retracted claim to explain what was wrong. A block that both names and
/// retracts is a correction, not a promise.
fn block_promises_sandbox(block: &str) -> bool {
    let lowered = block.to_ascii_lowercase();

    let names_sandbox = [
        "landlock",
        "bubblewrap",
        "firejail",
        "sandbox-exec",
        "sandbox backend",
        "sandboxbackend",
        "seccomp",
        "apparmor",
        "seatbelt",
    ]
    .iter()
    .any(|needle| lowered.contains(needle));
    if !names_sandbox {
        return false;
    }

    let asserts_presence = [
        "real isolation",
        "is real",
        "are real",
        "exists in",
        "exists under",
        "provided by",
        "enforced by",
        "lives in",
        "lives under",
    ]
    .iter()
    .any(|d| lowered.contains(d));

    let denies_or_retracts = [
        "no os-level sandbox",
        "does not exist",
        "was deleted",
        "is gone",
        "no such backend",
        "an earlier",
        "a previous",
        "retracted",
        "never existed",
        "not a type here",
        "deleted crate",
    ]
    .iter()
    .any(|d| lowered.contains(d));

    asserts_presence && !denies_or_retracts
}

/// Split source text into contiguous runs of `//` / `///` / `//!` comment lines,
/// each joined with a single space and tagged with its first line number.
///
/// A run ends at the first non-comment line. The line-wrapped defect spanned two
/// adjacent comment lines with nothing between them, so a run boundary at code
/// is exactly the right cut — and joining is what makes the wrap visible.
fn comment_blocks(text: &str) -> Vec<(usize, String)> {
    let mut blocks: Vec<(usize, String)> = Vec::new();
    let mut current: Vec<String> = Vec::new();
    let mut start = 0usize;

    for (idx, raw) in text.lines().enumerate() {
        let trimmed = raw.trim_start();
        let body = if let Some(rest) = trimmed.strip_prefix("///") {
            Some(rest.trim())
        } else if let Some(rest) = trimmed.strip_prefix("//!") {
            Some(rest.trim())
        } else {
            trimmed.strip_prefix("//").map(str::trim)
        };

        match body {
            Some(content) => {
                if current.is_empty() {
                    start = idx + 1;
                }
                current.push(content.to_string());
            }
            None => {
                if !current.is_empty() {
                    blocks.push((start, current.join(" ")));
                    current.clear();
                }
            }
        }
    }
    if !current.is_empty() {
        blocks.push((start, current.join(" ")));
    }
    blocks
}

#[test]
fn no_source_file_may_promise_a_sandbox_backend_that_does_not_exist() {
    let mut offenders = Vec::new();
    for path in rust_files(crate_root()) {
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(_) => continue,
        };
        for (lineno, block) in comment_blocks(&text) {
            if block_promises_sandbox(&block) {
                offenders.push(format!("{}:{lineno}: {block}", path.display()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "these comments promise an isolation layer that does not exist — either the backend \
         is real (then cite its type and file) or the claim must be retracted:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn blast_radius_module_docs_state_that_no_sandbox_exists() {
    // The module header is the authority a reader consults about this gate's
    // scope. It must say plainly that there is no OS-level sandbox, so nobody
    // downstream invents one to fill the gap.
    let docs = std::fs::read_to_string(crate_root().join("src/blast_radius.rs"))
        .expect("blast_radius.rs must be readable");
    assert!(
        docs.contains("There is no OS-level sandbox in operant"),
        "blast_radius.rs must state that no OS-level sandbox exists; the gate \
         operates on command text and a reader deserves to know that is all of it"
    );
}

#[test]
fn approval_entry_point_does_not_promise_a_deleted_crate_as_isolation() {
    // `check_tool_approval` is the enforced seam. It may RECORD that a stale
    // claim existed (the correction does), but it must never ASK a reader to
    // rely on a crate that is gone. Same discriminator as the workspace walk, so
    // the two assertions cannot drift apart.
    let src = std::fs::read_to_string(crate_root().join("src/approval.rs"))
        .expect("approval.rs must be readable");
    let offenders: Vec<(usize, String)> = comment_blocks(&src)
        .into_iter()
        .filter(|(_, block)| block_promises_sandbox(block))
        .collect();
    assert!(
        offenders.is_empty(),
        "approval.rs promises an isolation layer that does not exist at {:?} — the enforced \
         entry point must not send a debugging engineer after a crate deleted in iter-892",
        offenders
            .iter()
            .map(|(l, b)| format!("approval.rs:{l}: {b}"))
            .collect::<Vec<_>>()
    );
}

/// The line-wrap lesson, pinned as a unit test so the guard's own reasoning
/// cannot silently regress to line-local analysis.
#[test]
fn a_line_wrapped_claim_must_still_be_detected() {
    // Exactly the shape of the original approval.rs defect: mechanism name and
    // promise on two different lines.
    let wrapped = "operant_runtime`'s landlock/bubblewrap/firejail/docker backends for\n\
                   // real isolation.";
    assert!(
        block_promises_sandbox(wrapped),
        "the guard must see a claim whose name and promise span two lines; \
         this is the exact shape that escaped the first version"
    );

    // And the corrected prose must NOT trip it.
    let corrected = "There is no OS-level sandbox in operant, and there never was one here. \
                     An earlier version of this comment named operant-runtime's sandbox \
                     backends as real isolation. That crate was deleted and no such backend \
                     exists anywhere in this workspace";
    assert!(
        !block_promises_sandbox(corrected),
        "the guard must not flag a correction that names the mechanism while retracting it"
    );
}
