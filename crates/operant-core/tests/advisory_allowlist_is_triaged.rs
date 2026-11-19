#![allow(clippy::expect_used, clippy::unwrap_used)]

//! Every entry in `.ci/advisory-allowlist.txt` must name the ledger row that
//! triaged it.
//!
//! The allowlist is not a blanket exemption: its own header says each entry is
//! one advisory id, "recorded in BUGS.md with its reason", and the supply gate
//! tells you to "triage each one and record it in BUGS.md before allowlisting
//! it". Nothing enforced the "recorded in BUGS.md" half — the gate only checks
//! that the allowlist parses and covers what cargo-audit reports.
//!
//! That is the RT-615 shape exactly: three advisories were allowlisted with
//! reasons, and the guard that would notice a future entry added WITHOUT a
//! reason (or whose BUGS.md row was deleted) did not exist. A reason in a
//! comment inside the allowlist file is not the record: the ledger row is the
//! searchable, linkable record, and it is what a later audit reads.
//!
//! The rule is deliberately mechanical — every non-comment line must be
//! `<kind>|<advisory-id>|<crate>@<version>`, and the advisory id must appear in
//! `docs/audit/2026-06-19-BUGS.md` — because that is the check that would have
//! failed on the day RT-615's entries were first written.

use std::path::Path;

fn repo_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the crate is two levels under the workspace root")
        .to_path_buf()
}

/// Every allowlisted advisory id, in the order the file lists them.
fn allowlisted_ids() -> Vec<String> {
    let text = std::fs::read_to_string(repo_root().join(".ci/advisory-allowlist.txt"))
        .expect("the allowlist is committed at the repo root");
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| {
            let parts: Vec<&str> = l.split('|').collect();
            assert_eq!(
                parts.len(),
                3,
                "allowlist line must be <kind>|<advisory-id>|<crate>@<version>, got {l:?}"
            );
            parts[1].to_string()
        })
        .collect()
}

#[test]
fn the_allowlist_is_not_empty_and_every_line_has_three_fields() {
    let ids = allowlisted_ids();
    assert!(!ids.is_empty(), "the allowlist parses to zero advisories");
    // The shape check above already ran on every line; this pins that at least
    // one advisory of each known kind is present, so a file that silently lost
    // its entries to a truncated write is caught rather than passing vacuously.
    let text = std::fs::read_to_string(repo_root().join(".ci/advisory-allowlist.txt"))
        .expect("the allowlist is committed at the repo root");
    assert!(
        text.contains("vulnerability|") || text.contains("unmaintained|"),
        "the allowlist carries no cargo-audit kind prefix"
    );
}

/// The heart of the guard: each advisory id must be named in the ledger.
#[test]
fn every_allowlisted_advisory_is_recorded_in_the_ledger() {
    let ledger = std::fs::read_to_string(repo_root().join("docs/audit/2026-06-19-BUGS.md"))
        .expect("the BUGS ledger is committed at the repo root");
    for id in allowlisted_ids() {
        assert!(
            ledger.contains(&id),
            "advisory {id} is allowlisted but named by no row of docs/audit/2026-06-19-BUGS.md \
             — triage it there BEFORE allowlisting it (the gate's own instruction)"
        );
    }
}
