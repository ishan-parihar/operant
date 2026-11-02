#![allow(clippy::expect_used, clippy::unwrap_used)]

//! The RT-209 row's own claims must stay true.
//!
//! Prose lag is not a one-off in this row: it recurred FOUR times in eleven
//! iterations. iter-1642 shipped a claim of "delivered to every subscriber"
//! while both subscribers read only `end.session_id`; iter-1647 corrected docs
//! that described the pre-1645 kill-switch world; iter-1653 found the row still
//! saying "the emission site the plan specified and nobody built" four
//! iterations AFTER the emission shipped; and iter-1657 -- this pass -- found
//! the row still citing a line number (`lib.rs:130`) the module had moved from
//! and a 409 LOC count the file had grown well past.
//!
//! Every one of those was a green-test-run defect: `cargo test` says nothing
//! about a sentence, and no other gate reads the ledger. So the fix is the same
//! shape as the orphan guard in the module itself -- an explicit check on the
//! exact artifact, run by the same command the agent already runs.
//!
//! Each assertion below names the claim it pins and the reason it is true. A
//! claim that becomes FALSE is a defect, so these fail loudly; a claim that
//! becomes true (a consumer lands, the module shrinks) is updated here in the
//! same iteration, which is the point -- the doc cannot silently drift again.

use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/operant-core is two levels under the repository root")
        .to_path_buf()
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(workspace_root().join(rel))
        .unwrap_or_else(|e| panic!("cannot read {rel}: {e}"))
}

/// No doc may cite the pre-move export line.
///
/// `lib.rs:130` was correct when RT-209 was opened and wrong by iter-1657: the
/// module's `pub mod` declaration sits elsewhere now, next to `turn_end`. A
/// hard-coded line number in prose is a promise the tree will not keep, so the
/// claim is the MODULE PATH, which is stable, and any file that re-states the
/// stale number is corrected.
#[test]
fn no_doc_cites_the_modules_stale_export_line() {
    for rel in [
        "docs/audit/2026-06-19-BUGS.md",
        "docs/CHANGELOG.md",
        "AGENTS.md",
        "README.md",
    ] {
        let text = read(rel);
        assert!(
            !text.contains("lib.rs:130"),
            "{rel} still says `lib.rs:130`: the module's export line moved. Cite the \
             module path (`crates/operant-core/src/turn_attribution.rs`), not a line \
             number, and check the RT-209 row and CHANGELOG's attribution entry both."
        );
    }
}

/// The ledger row must not claim an emission site that was never built.
///
/// iter-1631 shipped the emission at turn END; iter-1653 found the row's
/// closing sentence still reading "the emission site the plan specified and
/// nobody built". A reader would conclude from the verdict that nothing
/// happened, which is the overclaim this row has already produced three times.
///
/// The check is for the LIVE claim, not a quotation of it. The ledger
/// legitimately quotes the phrase -- once in the iter-1653 correction that
/// records it was removed, and in any future correction -- so a bare substring
/// ban fails on its own history. The first exemption I wrote (an immediately
/// following `"`) handled only one of the two real quoting forms: the ledger's
/// actual quotation continues `nobody built" after iter-1631`, with no period.
/// The rule is now explicit and mutation-proved in BOTH directions --
/// reinstating the live claim fails the test, and adding a quoted form passes.
#[test]
fn the_ledger_does_not_claim_the_emission_was_never_built() {
    let bugs = read("docs/audit/2026-06-19-BUGS.md");
    let tail = "the emission site the plan specified and nobody built";
    let live = bugs
        .match_indices(tail)
        .filter(|(i, m)| {
            let after = bugs[i + m.len()..].trim_start().chars().next();
            // A quotation continues with a closing quote or a comma; a live
            // claim ends the sentence, so a period follows.
            !matches!(after, Some('"') | Some(','))
        })
        .count();
    assert_eq!(
        live, 0,
        "the RT-209 row still says the emission was never built, OUTSIDE a quotation. \
         iter-1631 shipped it; the open half is the CONSUMER, and it must be named as such."
    );
}

/// Neither the ledger nor the CHANGELOG may claim the value is delivered.
///
/// "Delivered to every subscriber" was the iter-1642 overclaim, corrected, and
/// the word is a trap: `TurnEnd::attribution` IS delivered into every emitted
/// event, and was read by none of them. iter-1661 closed the seam half, so the
/// claim is narrower now and the guard follows it -- a sentence saying the
/// value is consumed without naming what still is NOT consumed is the same
/// false promise in a new shape.
///
/// Three accepted phrasings, because all three say the same thing and the
/// ledger legitimately reworded when the seam landed: "no subscriber reads"
/// (pre-1661), "no SUBSCRIBER reads" / "no consumer reads" (post-1661). The
/// point is that the DEFERRED PART IS NAMED, not the exact words.
#[test]
fn the_docs_still_name_the_deferred_consumer() {
    const NAMED: [&str; 4] = [
        "no subscriber reads",
        "no SUBSCRIBER reads",
        "no consumer reads",
        "every subscriber reads only",
    ];
    for rel in ["docs/audit/2026-06-19-BUGS.md", "docs/CHANGELOG.md"] {
        let text = read(rel);
        assert!(
            NAMED.iter().any(|n| text.contains(n)),
            "{rel} no longer names the deferred consumer (looked for: {NAMED:?}). The capture is \
             live and the consumer is the deferred half; a reader must be told that, not left to \
             infer it from a green test run."
        );
    }
}

/// The RT-209 row must not present the field-read check as the last layer.
///
/// iter-1643 added the field-read check and described the gap as fully
/// characterised; iter-1655 then found the two public METHODS that would render
/// the value also had no caller. The row now records all three variants. If a
/// future pass deletes that history, the next agent re-learns it the same way.
#[test]
fn the_ledger_records_all_three_orphan_variants() {
    let bugs = read("docs/audit/2026-06-19-BUGS.md");
    for variant in ["is_exclusive", "render", "no subscriber reads"] {
        assert!(
            bugs.contains(variant),
            "the RT-209 row no longer records the method-level orphan variant `{variant}`. \
                 iter-1655 found it after iter-1643 had called the field-read check the last \
                 layer; the three variants (zero-caller type, unread field, uncalled method) \
                 are the reason the check now covers public methods too."
        );
    }
}

/// The example config must advertise the kill switch.
///
/// iter-1645 threaded `[agent] show_turn_attribution` through `BehaviorSettings`
/// and advertised it in `operant.example.toml`. The switch was unreachable for
/// four iterations because the key did not exist, so if it disappears from the
/// template the same regression is one config edit away.
#[test]
fn the_example_config_advertises_the_kill_switch() {
    let example = read("operant.example.toml");
    assert!(
        example.contains("show_turn_attribution"),
        "operant.example.toml no longer advertises `show_turn_attribution`. iter-1645 \
         threaded it through `BehaviorSettings` after four iterations in which the switch \
         existed only as a hardcoded `true`; an unadvertised key is the same defect."
    );
}
