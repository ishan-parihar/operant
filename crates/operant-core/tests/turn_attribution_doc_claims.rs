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
//! **iter-2689 adds a fifth, and inverts one.** The subscriber half of RT-209
//! closed, so `the_docs_still_name_the_deferred_consumer` became a guard for a
//! claim that is no longer true -- it asserted the docs SAY no subscriber reads
//! the value. It is replaced, not deleted (deleting is how this row drifted
//! before), by three checks: the stale phrasing must be gone from the live
//! claim, the closing must be recorded, and the SOURCE must actually read
//! `end.attribution`. The last one matters most: prose can be corrected while
//! the read is absent, which is exactly the state this iteration inherited.
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

/// Neither the ledger nor the CHANGELOG may claim the value is still unread.
///
/// The history of this row is a sequence of TRUE statements that each became
/// false: "delivered to every subscriber" (iter-1642 overclaim, both then read
/// only `session_id`), then the accurate "no subscriber reads" / "no SUBSCRIBER
/// reads" / "no consumer reads" from iter-1661. iter-2689 closed that last
/// half, so the guard INVERTS rather than being deleted -- deleting it is how
/// the row's prose drifted four times in eleven iterations.
///
/// Two things are pinned. (1) The deferred-consumer phrasing must be GONE from
/// the live claim in both docs. (2) The closing must be RECORDED, so a reader
/// who greps for the row learns it is finished.
///
/// The banned list is two entries, not the four this guard used to accept.
/// `no consumer reads` is dropped because it is a TRUE sentence about a
/// different row (RT-413-FOLLOW's OAuth scope field); `every subscriber reads
/// only` is dropped because iter-2689 made it false in a way the first entry
/// already covers. Banning a phrase the docs use correctly elsewhere is the
/// substring trap this module has hit five times. The exemption for quotations is
/// the same shape as `the_ledger_does_not_claim_the_emission_was_never_built`:
/// the ledger legitimately quotes the old phrasing while recording that it was
/// corrected, so a bare substring ban would fail on its own history.
#[test]
fn the_docs_no_longer_claim_no_subscriber_reads() {
    for rel in ["docs/audit/2026-06-19-BUGS.md", "docs/CHANGELOG.md"] {
        let text = read(rel);
        // ONLY the phrases that actually describe this row's deferred consumer.
        // `no consumer reads` is deliberately absent: it is a live claim about a
        // DIFFERENT row (RT-413-FOLLOW's OAuth scope field), so banning it here
        // would fail on an unrelated true sentence -- which is how the first
        // draft of this guard was wrong.
        for phrase in ["no subscriber reads", "no SUBSCRIBER reads"] {
            let live = text
                .match_indices(phrase)
                .filter(|(i, m)| {
                    // A quotation is followed by a closing quote or a comma; a
                    // live claim ends the sentence. Same rule the sibling guard
                    // uses, for the same reason.
                    let after = text[i + m.len()..].trim_start().chars().next();
                    !matches!(after, Some('"') | Some(','))
                })
                .count();
            assert_eq!(
                live, 0,
                "{rel} still claims `{phrase}` OUTSIDE a quotation. iter-2689 made a subscriber read the value -- recap.rs renders it to the operator. Re-derive before re-adding this claim."
            );
        }
    }
}

/// Both docs must RECORD that the subscriber gap closed.
///
/// A guard that only bans the stale phrasing passes just as happily on a doc
/// that never mentions the gap at all, which is the silent-pass failure this
/// module has shipped twice. So the close is asserted positively too.
#[test]
fn the_docs_record_that_the_subscriber_gap_closed() {
    for rel in ["docs/audit/2026-06-19-BUGS.md", "docs/CHANGELOG.md"] {
        let text = read(rel);
        assert!(
            text.contains("iter-2689"),
            "{rel} does not record iter-2689 (the iteration that made a subscriber read the carried attribution). Without it a reader greps for RT-209, finds the old 'no subscriber reads' sentence, and re-opens a closed row."
        );
    }
}

/// The source must actually READ the carried value off the event.
///
/// The two guards above pin PROSE. This one pins the CODE, because the prose
/// can be corrected while the read is absent -- which is precisely the state
/// iter-2689 inherited. The needle is a call-site shape
/// (`end.attribution.as_ref()`), not the bare field name, for the reason the
/// module has learned five times: a bare `attribution` matches the struct
/// definition, the `emit` parameter, and the `log_attribution` method, so
/// deleting the read would leave the check green.
#[test]
fn a_subscriber_reads_the_attribution_off_the_event() {
    let recap = read("crates/operant-core/src/recap.rs");
    assert!(
        recap.contains("end.attribution.as_ref()"),
        "recap.rs no longer reads `end.attribution` off the delivered TurnEnd. RT-209's whole open half was that both subscribers read only `session_id`; the value is carried, logged at the seam, and dropped. If the read moved, point this guard at its new home rather than deleting the check."
    );
}

/// The RT-209 row must not present the field-read check as the last layer.
///
/// iter-1643 added the field-read check and described the gap as fully
/// characterised; iter-1655 then found the two public METHODS that would render
/// the value also had no caller. The row now records all three variants. If a
/// future pass deletes that history, the next agent re-learns it the same way.
///
/// NOTE (iter-2689): the "no subscriber reads" entry is kept DELIBERATELY even
/// though that half is now closed. It is history, and the loop below checks the
/// row still REMEMBERS having recorded it -- a closed row that forgets its own
/// variants is how the next agent re-opens the question.
#[test]
fn the_ledger_records_all_three_orphan_variants() {
    let bugs = read("docs/audit/2026-06-19-BUGS.md");
    // Case-insensitive: iter-2689 date-stamped the row's stale phrasing, and the
    // one that survives is the upper-case `no SUBSCRIBER reads` form. The guard
    // pins that the row REMEMBERS recording the variant, not its capitalisation.
    let lowered = bugs.to_lowercase();
    // `no subscriber reads` is NOT required here: iter-2689's inverted guard
    // above forbids that phrasing outside a quotation, and this guard used to
    // require it -- the two contradicted each other for one iteration. The
    // history is pinned by the closing note instead (see the next assert).
    for variant in ["is_exclusive", "render"] {
        assert!(
            lowered.contains(variant),
            "the RT-209 row no longer records the method-level orphan variant `{variant}`. \
                 iter-1655 found it after iter-1643 had called the field-read check the last \
                 layer; the three variants (zero-caller type, unread field, uncalled method) \
                 are the reason the check now covers public methods too."
        );
    }
}

/// The row must still RECORD that the subscriber half was once deferred.
///
/// The variants loop above dropped `no subscriber reads` because the inverted
/// guard forbids that phrasing; this pins the same history in the form the row
/// now uses, so dropping the history entirely is still caught.
#[test]
fn the_row_records_the_subscriber_was_once_deferred() {
    let bugs = read("docs/audit/2026-06-19-BUGS.md").to_lowercase();
    assert!(
        bugs.contains("no subscriber read") || bugs.contains("no subscriber reads"),
        "the RT-209 row no longer records that a subscriber once failed to read the carried value. The gap is closed, but a row that forgets it was open is how the next agent re-opens the question."
    );
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
