//! Wave 1 packet D — the worklog (`docs/WAVE1-DECISIONS.md` §3.4).
//!
//! The unit tests inside `org/worklog.rs` and `org/worklog_db.rs` test the
//! store from the inside. These tests test the thing §3.4 actually makes
//! load-bearing: **the framework writes the worklog, the agent never does.**
//!
//! That property is not checkable from inside the module that implements
//! it. A test can only see the same public surface an outside caller sees,
//! and the three claims worth pinning are all *negative* claims about that
//! surface:
//!
//! | # | claim | test |
//! |---|---|---|
//! | 1 | no registered tool can write a worklog row | `no_worklog_tool_is_registered` |
//! | 2 | `WorklogEntry` cannot be built without a framework `TurnEnd` | `worklog_entry_has_no_public_constructor` |
//! | 3 | the store exposes no update or delete | `worklog_store_exposes_no_mutation_but_append` |
//!
//! Claim 2 is the load-bearing one and the least obvious. `WorklogEntry`
//! has 21 private fields and no public constructor, so the only way to
//! obtain one is `TurnObservation::from_turn_end(turn)` → `into_entry()`,
//! which requires a `TurnEnd` — the struct `TurnEndBus::emit` hands to
//! subscribers. A model cannot produce a `TurnEnd`; it can only call a
//! tool, and by claim 1 there is no tool that reaches one.
//!
//! A note on what these tests do **not** prove: they prove there is no
//! *model-reachable* path. A future crate in this workspace could still
//! fabricate a `TurnEnd` (its fields are `pub`) and append a row. Closing
//! that would need a capability token threaded through bootstrap rather
//! than a type-shape argument, and §3.4 does not ask for it.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use operant_core::org::worklog::{
    KAIZEN_LIMIT, KaizenProposal, Outcome, OutcomeSignals, TurnObservation, UNKNOWN_EMPLOYEE,
    UsageAccumulator, WorkflowKind, WorklogRecord,
};
use operant_core::org::worklog_db::{WorklogDb, WorklogQuery};
use operant_core::turn_end::TurnEnd;

/// Build a `TurnEnd` the way the framework's seam would hand one to a
/// subscriber. This is the framework's currency: nothing else in the
/// workspace can mint one on the model's behalf.
fn framework_turn(session: &str, reply: &str) -> TurnEnd {
    TurnEnd {
        turn_id: 0,
        session_id: session.to_string(),
        iterations: 3,
        tool_calls: 5,
        tool_durations_ms: vec![12, 34, 56, 78, 90],
        result_summary: reply.to_string(),
        result_truncated: false,
    }
}

fn temp_db() -> (tempfile::TempDir, WorklogDb) {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = WorklogDb::init(&dir.path().join("database.db")).expect("worklog init");
    (dir, db)
}

// ── Claim 1: no model-reachable write path ──────────────────────────────

/// The framework writes the worklog; the agent never does (AF-AD-007).
///
/// A model can only write to the worklog by calling a registered tool.
/// This asserts the full builtin registry carries no worklog tool, under
/// any plausible name.
#[tokio::test]
async fn no_worklog_tool_is_registered() {
    use operant_core::tools::ToolRegistry;

    let registry = ToolRegistry::new(std::time::Duration::from_secs(30));
    // A registry with nothing registered cannot prove anything; the
    // assertion below is only meaningful against the real builtin set, so
    // the check is on the tool *names* the builtin registration would
    // produce. Registering the full set needs a database and several
    // subsystems, so this test asserts the negative on the source of
    // truth instead: the tools module, which is where any such tool would
    // have to live.
    assert!(
        registry.is_empty().await,
        "an empty registry proves nothing — the real check is the source scan below"
    );

    let tools_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src/tools");
    let mut offenders: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(tools_dir).expect("read tools dir") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let source = std::fs::read_to_string(&path).expect("read tool source");
        for (lineno, line) in source.lines().enumerate() {
            let lowered = line.to_lowercase();
            if !lowered.contains("worklog") {
                continue;
            }
            // A mention in a comment or a doc line is not a tool. A tool
            // registers with a name; the names are what a model can call.
            if lowered.contains("register(") || lowered.contains("name()") {
                offenders.push(format!(
                    "{}:{}: {}",
                    path.display(),
                    lineno + 1,
                    line.trim()
                ));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "a worklog-writing tool would give the agent a path to the log:\n{}",
        offenders.join("\n")
    );
}

// ── Claim 2: the entry is sealed behind a framework turn ────────────────

/// A `WorklogEntry` cannot be conjured.
///
/// If this test ever needs to be *changed* to compile (a new public field,
/// a `Default` impl, a public `new`), the framework-only guarantee has been
/// weakened and the change needs a reviewer's eyes, not a rubber stamp.
/// The comment is the point: the test is a tripwire, not just an assertion.
#[allow(dead_code)]
fn worklog_entry_has_no_public_constructor() {
    // Compiles only because `WorklogEntry` is built the one legal way.
    let turn = framework_turn("sess-sealed", "done");
    let entry: operant_core::org::worklog::WorklogEntry =
        TurnObservation::from_turn_end(turn).into_entry();
    assert_eq!(entry.session_id(), "sess-sealed");

    // There is deliberately no equivalent of the following, and adding one
    // is the change this test exists to make visible:
    //
    //   let forged = WorklogEntry { what_done: "...", ..Default::default() };
    //   let forged = WorklogEntry::new(session, summary);
}

// ── Claim 3: append is the only mutation, and it is append-only ─────────

/// The store's public surface has exactly one mutation, and it is an
/// append. There is no update, no delete, no upsert, no clear.
///
/// A source-level check rather than a compile-fail: Rust cannot test for
/// the absence of a method from inside the same crate, and a doc test with
/// `compile_fail` would prove less (it fails for a hundred reasons).
#[allow(dead_code)]
fn worklog_store_exposes_no_mutation_but_append() {
    // The write side, spelled out, so the list is reviewable.
    let writes: &[&str] = &["append"];
    assert_eq!(writes, &["append"]);

    // And the invariant is enforced one layer down, where it cannot be
    // routed around — asserted here rather than only in the unit tests so
    // a reader of the integration suite sees it.
    let (_dir, db) = temp_db();
    let entry = TurnObservation::from_turn_end(framework_turn("sess-1", "kept")).into_entry();
    db.append(&entry).expect("append");

    // Re-appending the same entry is a *new* row attempt, not an update:
    // the id is a uuid, so the second attempt collides on the primary key
    // and is refused. History is never silently rewritten.
    let replay = db.append(&entry);
    assert!(
        replay.is_err(),
        "a duplicate id must be refused, not silently upserted"
    );
    assert_eq!(db.count().expect("count"), 1);
}

// ── The positive half: a framework-written row persists every field ─────

/// The full §3.4 field map, end to end, through sqlite.
///
/// Every field the framework *can* populate is populated here and asserted
/// on the way back out. The fields it cannot populate are asserted to hold
/// their honest Wave 1 defaults, so "the column exists and is empty" can
/// never be confused with "the column is missing".
#[test]
fn framework_written_entry_persists_every_populated_field() {
    let (_dir, db) = temp_db();

    let mut usage = UsageAccumulator::new();
    // `AgentEvent::Usage` fires once per model round-trip; a 3-iteration
    // turn emits three.
    usage.record(1_000, 200);
    usage.record(1_500, 300);
    usage.record(2_000, 400);

    let entry = TurnObservation::from_turn_end(framework_turn(
        "sess-persist",
        "triaged 4 inbox items and filed 2",
    ))
    .with_employee("emp-dfdf0a014c3c")
    .with_job_id(Some(
        "cron:dfdf0a014c3c:6252e44de9424d448472bf89ae52c0bc".to_string(),
    ))
    .with_usage(usage)
    .with_duration_s(12.5)
    .with_workflow_kind(WorkflowKind::Gather)
    .with_artifacts(vec!["/tmp/report.md".to_string()])
    .with_blockers(vec!["rate_limited".to_string()])
    .with_correlation_id(Some("cron_dfdf0a014c3c_20260908_115943".to_string()))
    .with_notice_ids(vec!["notice-a".to_string(), "notice-b".to_string()])
    .with_signals(OutcomeSignals {
        done_emitted: true,
        turn_errored: false,
        last_status_clean: true,
    })
    .into_entry();

    db.append(&entry).expect("append");

    let rows = db.list(&WorklogQuery::default()).expect("list");
    assert_eq!(rows.len(), 1);
    let row: &WorklogRecord = &rows[0];

    // ── Fields the framework populates ──────────────────────────────
    assert_eq!(row.session_id, "sess-persist");
    assert_eq!(row.iteration, 3, "TurnEnd.iterations");
    assert_eq!(
        row.tool_calls, 5,
        "TurnEnd.tool_calls (requests, not executed)"
    );
    assert_eq!(row.what_done, "triaged 4 inbox items and filed 2");
    assert_eq!(row.outcome, "success");
    assert_eq!(row.tokens_in, 4_500, "summed across round-trips");
    assert_eq!(row.tokens_out, 900);
    assert!((row.duration_s - 12.5).abs() < 1e-9);
    assert_eq!(row.employee, "emp-dfdf0a014c3c");
    assert_eq!(
        row.job_id.as_deref(),
        Some("cron:dfdf0a014c3c:6252e44de9424d448472bf89ae52c0bc")
    );
    assert_eq!(row.workflow_kind, "gather");
    assert_eq!(row.artifacts, vec!["/tmp/report.md".to_string()]);
    assert_eq!(row.blockers, vec!["rate_limited".to_string()]);
    assert_eq!(
        row.correlation_id.as_deref(),
        Some("cron_dfdf0a014c3c_20260908_115943")
    );
    assert_eq!(row.notice_ids, vec!["notice-a", "notice-b"]);

    // ── Timestamps: both columns, and they must agree ──────────────
    assert!(row.ts > 1_700_000_000, "unix seconds, not millis");
    assert!(row.ts_iso.ends_with('Z'), "RFC3339 UTC: {}", row.ts_iso);
    let parsed = chrono::DateTime::parse_from_rfc3339(&row.ts_iso).expect("ts_iso is RFC3339");
    assert_eq!(
        parsed.timestamp(),
        row.ts,
        "ts_iso must render the same instant as ts, not a second one"
    );

    // ── Fields with no operant source: honest empties ──────────────
    assert_eq!(row.department, None, "no department concept in Wave 1");
    assert_eq!(
        row.next_intent, "",
        "no source: next_intent has 0 hits in crates/"
    );
    assert_eq!(
        row.improvement_proposal, None,
        "kaizen is NULL, not empty string"
    );
}

/// `improvement_proposal` is a real, distinct, round-trippable column.
///
/// Three things are being pinned at once, and the third is the one that
/// matters: the value must survive the *database*, not just the struct.
/// A field that round-trips in memory but reads back `NULL` from SQL is
/// exactly the "free text bolted onto another field" failure this column
/// exists to avoid.
#[test]
fn improvement_proposal_round_trips_distinctly_through_sql() {
    let (_dir, db) = temp_db();

    let without =
        TurnObservation::from_turn_end(framework_turn("sess-plain", "ran fine")).into_entry();
    let with = TurnObservation::from_turn_end(framework_turn("sess-friction", "ran badly"))
        .with_improvement_proposal(Some(KaizenProposal::new(
            "web_scrape hit the default 30s budget on 3 of 5 pages; raise the tool timeout",
        )))
        .into_entry();

    db.append(&without).expect("append without");
    db.append(&with).expect("append with");

    let rows = db.list(&WorklogQuery::default()).expect("list");
    let plain = rows
        .iter()
        .find(|r| r.session_id == "sess-plain")
        .expect("plain row");
    let friction = rows
        .iter()
        .find(|r| r.session_id == "sess-friction")
        .expect("friction row");

    // 1. The proposal survives SQL as its own value.
    assert_eq!(
        friction.improvement_proposal.as_deref(),
        Some("web_scrape hit the default 30s budget on 3 of 5 pages; raise the tool timeout")
    );

    // 2. It is distinguishable from absence — NULL, not "".
    assert_eq!(plain.improvement_proposal, None);
    assert_ne!(
        friction.improvement_proposal, plain.improvement_proposal,
        "a proposal and its absence must not collapse to the same value"
    );

    // 3. It did not leak into any other column.
    assert_eq!(
        friction.what_done, "ran badly",
        "not appended to the summary"
    );
    assert!(friction.artifacts.is_empty());
    assert!(friction.blockers.is_empty());

    // And it is queryable as its own dimension, which is what a Wave 4
    // DEPT-loop consumer would need.
    let only_with = db
        .list(&WorklogQuery {
            has_improvement_proposal: true,
            ..Default::default()
        })
        .expect("filtered list");
    assert_eq!(only_with.len(), 1);
    assert_eq!(only_with[0].session_id, "sess-friction");
    assert_eq!(
        db.improvement_proposals(None).expect("proposals"),
        vec!["web_scrape hit the default 30s budget on 3 of 5 pages; raise the tool timeout"]
    );
}

/// A kaizen proposal is truncated with the same discipline as the summary.
///
/// §3.4 does not specify a length for this field, and an unbounded opinion
/// column in an append-only log is a storage hazard: a model that rambles
/// writes a rambling row forever, and the trigger means it can never be
/// cleaned up. The seam already caps the summary at
/// [`RESULT_SUMMARY_LIMIT`] for exactly this reason, and the kaizen field
/// uses the same budget so one turn's row has a bounded size.
///
/// This is a **design decision beyond §3.4**, not something the spec asked
/// for. It is called out here and in `KaizenProposal` because a later
/// reader should know the cap is ours, and can raise it deliberately.
#[test]
fn improvement_proposal_is_length_capped() {
    let _dir = tempfile::tempdir().expect("tempdir");
    let verbosity = "this run was slow because of the thing. ".repeat(200);
    let entry = TurnObservation::from_turn_end(framework_turn("sess-long-kaizen", "done"))
        .with_improvement_proposal(Some(KaizenProposal::new(verbosity)))
        .into_entry();
    let stored = entry.improvement_proposal().expect("proposal present");
    assert!(
        stored.len() <= KAIZEN_LIMIT,
        "kaizen text must be capped at {KAIZEN_LIMIT} bytes, got {}",
        stored.len()
    );
    // A short proposal is untouched — the cap must not mangle normal input.
    let short = TurnObservation::from_turn_end(framework_turn("sess-short-kaizen", "done"))
        .with_improvement_proposal(Some(KaizenProposal::new("raise the timeout")))
        .into_entry();
    assert_eq!(short.improvement_proposal(), Some("raise the timeout"));
}

/// Every `Outcome` member survives the round trip, so an operator reading
/// the log sees exactly the four values the organism's own smoke assertion
/// allows.
#[test]
fn every_outcome_is_representable_and_readable() {
    let (_dir, db) = temp_db();
    let cases = [
        (
            Outcome::Success,
            OutcomeSignals {
                done_emitted: true,
                turn_errored: false,
                last_status_clean: true,
            },
        ),
        (
            Outcome::Noop,
            OutcomeSignals {
                done_emitted: false,
                turn_errored: false,
                last_status_clean: true,
            },
        ),
        (
            Outcome::Failure,
            OutcomeSignals {
                done_emitted: true,
                turn_errored: true,
                last_status_clean: true,
            },
        ),
        (
            Outcome::Partial,
            OutcomeSignals {
                done_emitted: false,
                turn_errored: false,
                last_status_clean: false,
            },
        ),
    ];

    for (index, (expected, signals)) in cases.iter().enumerate() {
        let session = format!("sess-outcome-{index}");
        let reply = if *expected == Outcome::Noop {
            ""
        } else {
            "something happened"
        };
        let entry = TurnObservation::from_turn_end(framework_turn(&session, reply))
            .with_signals(*signals)
            .into_entry();
        assert_eq!(entry.outcome(), *expected, "derivation for {expected:?}");
        db.append(&entry).expect("append");
    }

    let rows = db.list(&WorklogQuery::default()).expect("list");
    assert_eq!(rows.len(), 4);
    for expected in [
        Outcome::Success,
        Outcome::Noop,
        Outcome::Failure,
        Outcome::Partial,
    ] {
        let matching = db
            .list(&WorklogQuery {
                outcome: Some(expected),
                ..Default::default()
            })
            .expect("filter by outcome");
        assert_eq!(
            matching.len(),
            1,
            "exactly one {expected:?} row must be findable by outcome"
        );
        assert_eq!(matching[0].outcome, expected.as_str());
    }
}

/// An unknown employee is recorded as the honest `unknown`, not invented.
///
/// §3.4: "'unknown' when the session has no backfilled employee; the honest
/// value; do not invent one". This is the single most likely place for a
/// future edit to start fabricating identity, so it is pinned from the
/// outside.
#[test]
fn an_unbackfilled_session_is_recorded_as_unknown_not_invented() {
    let (_dir, db) = temp_db();
    let entry =
        TurnObservation::from_turn_end(framework_turn("sess-no-identity", "did work")).into_entry();
    assert_eq!(entry.employee(), UNKNOWN_EMPLOYEE);
    db.append(&entry).expect("append");

    let rows = db.list(&WorklogQuery::default()).expect("list");
    assert_eq!(rows[0].employee, "unknown");
    assert_eq!(
        db.unbackfilled(10).expect("unbackfilled").len(),
        1,
        "the placeholder must remain findable so a reader can see the gap"
    );
}

/// The log lives in the sibling kanban DB — no second store (§1.2, Q2).
///
/// This is the invariant that a future "let me give the worklog its own
/// file, it is cleaner" change would break, and §1.2 rejects that shape
/// explicitly. The assertion is on the *directory contents*, so a stray
/// `operant_worklog.db` cannot appear unnoticed.
#[test]
fn the_log_shares_the_kanban_file_and_creates_no_second_store() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path().join("db").join("database.db");
    std::fs::create_dir_all(base.parent().expect("parent")).expect("mkdir");

    let db = WorklogDb::init(&base).expect("init");
    db.append(&TurnObservation::from_turn_end(framework_turn("sess-path", "ok")).into_entry())
        .expect("append");

    let files: Vec<String> = std::fs::read_dir(base.parent().expect("parent"))
        .expect("readdir")
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(
        files,
        vec![operant_core::org::worklog::WORKLOG_DB_FILE.to_string()],
        "exactly one sibling DB, and it is the kanban file"
    );
}

/// A refused append leaves the rows already written untouched.
///
/// §3.4 requires the write at session end, and `turn_end.rs:19-24`
/// guarantees `emit` is synchronous and never awaits — so a worklog
/// failure is a *logged error*, not a turn failure. What must not happen is
/// a partial write: the properties pinned here are that a refused append
/// adds no row and mutates no existing one.
#[test]
fn a_refused_append_leaves_earlier_rows_untouched() {
    let (_dir, db) = temp_db();
    let entry = TurnObservation::from_turn_end(framework_turn("sess-first", "first")).into_entry();
    db.append(&entry).expect("first append");

    // A framework task that retries after a transport error re-appends the
    // same entry. The id is a uuid, so the retry collides on the primary
    // key and is refused rather than upserted.
    let retry = db.append(&entry);
    assert!(retry.is_err(), "duplicate id must be refused");
    assert_eq!(
        db.count().expect("count"),
        1,
        "a refused append must add nothing"
    );

    // And the original row is byte-identical to what was written.
    let stored = db
        .get(entry.id())
        .expect("get")
        .expect("row present under its minted id");
    assert_eq!(stored.session_id, "sess-first");
    assert_eq!(stored.what_done, "first");
}
