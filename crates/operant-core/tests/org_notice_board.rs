//! Integration tests for the org notice board (`docs/WAVE1-DECISIONS.md` §3.3).
//!
//! Covers: post → inbox round-trip, typed-recipient routing (agent / dept /
//! team must not cross-deliver), ack semantics, TTL expiry, correlation
//! grouping, and batched GC (expired-only, cap-respecting, idempotent).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use operant_core::org::notice::{
    DEFAULT_MAX_WEEKLY_SUMMARIES_PER_TICK, InboxQuery, NoticeIdentity, PostNotice, Recipient,
    RetentionLimits, WEEK_SECONDS, WEEKLY_SUMMARY_TAG,
};
use operant_core::org::notice_db::NoticeBoard;
use std::path::PathBuf;
use tempfile::TempDir;

/// A notice board on a throwaway sqlite file named exactly like the real
/// sibling DB, so a test that passed here cannot pass by accident against a
/// differently-named file.
fn board() -> (NoticeBoard, TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path: PathBuf = dir.path().join("operant_kanban.db");
    let board = NoticeBoard::init(path).expect("NoticeBoard::init");
    (board, dir)
}

fn all(query: &InboxQuery) -> InboxQuery {
    query.clone()
}

/// Insert a row with an explicit `created_at`/`epoch` so retention tests do
/// not have to sleep. Writes through the board's own connection and returns
/// the id.
fn backdate(board: &NoticeBoard, id: &str, created_at: &str, epoch: i64, extra: &[(&str, &str)]) {
    let conn = board.conn().lock().expect("lock");
    let recipients = extra
        .iter()
        .find(|(k, _)| *k == "recipients")
        .map(|(_, v)| *v)
        .unwrap_or("[\"broadcast\"]");
    let correlation = extra
        .iter()
        .find(|(k, _)| *k == "correlation_id")
        .map(|(_, v)| *v);
    let tags = extra
        .iter()
        .find(|(k, _)| *k == "tags")
        .map(|(_, v)| *v)
        .unwrap_or("[\"status\"]");
    let pinned: i64 = extra
        .iter()
        .find(|(k, _)| *k == "pinned")
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0);

    // `Params` is implemented for fixed-size arrays and for `&[&dyn ToSql]`,
    // but *not* for a `&[Value]` slice, so a heterogeneous list of any length
    // goes through `params_from_iter` — which is what `params!` uses
    // internally, and what `Value: ToSql` makes possible here.
    let params = rusqlite_params(id, created_at, epoch, recipients, correlation, tags, pinned);
    conn.execute(
        "INSERT INTO notices (id, created_at, epoch, sender, from_dept, recipients, subject, body,
                              correlation_id, ack_required, acked_by, acked_at, ttl_expires_at,
                              tags, thread_id, pinned, reason, metadata)
         VALUES (?1, ?2, ?3, 'test-sender', 'test-dept', ?4, 's', 'b', ?5, 0, '[]', NULL, NULL, ?6, NULL, ?7, 'backdated', NULL)",
        rusqlite::params_from_iter(params),
    )
    .expect("backdated insert");
}

/// `conn.execute` takes `Params`, and the test wants a heterogeneous tuple, so
/// box the values to one type. rusqlite has no `Params` impl for a `&[Value]`
/// slice, so the caller wraps the result in `params_from_iter` — which works
/// because `Value: ToSql`.
fn rusqlite_params(
    id: &str,
    created_at: &str,
    epoch: i64,
    recipients: &str,
    correlation: Option<&str>,
    tags: &str,
    pinned: i64,
) -> Vec<rusqlite::types::Value> {
    use rusqlite::types::Value as SqlValue;
    vec![
        SqlValue::Text(id.to_string()),
        SqlValue::Text(created_at.to_string()),
        SqlValue::Integer(epoch),
        SqlValue::Text(recipients.to_string()),
        match correlation {
            Some(c) => SqlValue::Text(c.to_string()),
            None => SqlValue::Null,
        },
        SqlValue::Text(tags.to_string()),
        SqlValue::Integer(pinned),
    ]
}

fn rfc3339_at(epoch: i64) -> String {
    operant_core::org::notice::rfc3339(
        chrono::DateTime::from_timestamp(epoch, 0).expect("representable epoch"),
    )
}

/// An epoch `weeks` behind `now_epoch`, snapped back to a week boundary.
///
/// GC groups raw rows by `epoch / WEEK_SECONDS`, so a test that wants "two
/// rows in one week" has to actually place both in one week. Two arbitrary
/// offsets 1 day apart straddle a boundary roughly five times in seven, which
/// silently turns a one-week assertion into a two-week one.
fn week_aligned(now_epoch: i64, weeks_ago: i64) -> i64 {
    let week = WEEK_SECONDS;
    let now_week_start = now_epoch.div_euclid(week) * week;
    now_week_start - weeks_ago * week
}

// ---------------------------------------------------------------- round-trip

#[test]
fn org_notice_post_to_inbox_round_trip() {
    let (board, _dir) = board();

    let posted = board
        .post(&{
            let mut p = PostNotice::new(
                "emp-1",
                vec![Recipient::Agent("emp-2".into())],
                "deploy is green",
                "status update",
            );
            p.from_dept = Some("platform-infra".into());
            p.subject = Some("deploy".into());
            p.tags = vec!["status".into()];
            p
        })
        .expect("post");

    assert!(posted.id.starts_with("n_"), "id must be n_ prefixed");
    assert!(posted.epoch > 1_700_000_000, "epoch must be unix seconds");
    assert_eq!(posted.sender, "emp-1");
    assert_eq!(posted.from_dept.as_deref(), Some("platform-infra"));
    assert_eq!(posted.recipients, vec![Recipient::Agent("emp-2".into())]);

    // Read back through the recipient's inbox.
    let reader = NoticeIdentity::new("emp-2");
    let inbox = board
        .inbox(&reader, &all(&InboxQuery::default()))
        .expect("inbox");
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0], posted, "round trip must be lossless");

    // And through get().
    let fetched = board.get(&posted.id).expect("get").expect("row present");
    assert_eq!(fetched, posted);
}

#[test]
fn org_notice_provenance_defaults_to_unknown_when_absent() {
    let (board, _dir) = board();
    let posted = board
        .post(&PostNotice::new(
            "user",
            vec![Recipient::Broadcast],
            "hello",
            "human said hi",
        ))
        .expect("post");
    // `post` returns the pre-read row: storage keeps `from_dept` NULL.
    assert_eq!(posted.from_dept, None);
    // The read path COALESCEs it to AD-013 §2's `unknown` without a backfill.
    let fetched = board.get(&posted.id).expect("get").expect("row");
    assert_eq!(fetched.from_dept.as_deref(), Some("unknown"));
}

// --------------------------------------------------------------- recipients

#[test]
fn org_notice_typed_recipients_do_not_cross_deliver() {
    let (board, _dir) = board();

    board
        .post(&PostNotice::new(
            "emp-1",
            vec![Recipient::Agent("emp-agent".into())],
            "for one agent",
            "agent-scoped",
        ))
        .expect("post agent");
    board
        .post(&PostNotice::new(
            "emp-1",
            vec![Recipient::Dept("dept-a".into())],
            "for a dept",
            "dept-scoped",
        ))
        .expect("post dept");
    board
        .post(&PostNotice::new(
            "emp-1",
            vec![Recipient::Team("team-a".into())],
            "for a team",
            "team-scoped",
        ))
        .expect("post team");
    board
        .post(&PostNotice::new(
            "emp-1",
            vec![Recipient::Role("reviewer".into())],
            "for a role",
            "role-scoped",
        ))
        .expect("post role");

    // An employee in dept-a, on team-a, holding the reviewer role.
    let reader = NoticeIdentity::new("emp-agent")
        .in_dept("dept-a")
        .on_teams(vec!["team-a".to_string()])
        .with_roles(vec!["reviewer".to_string()]);
    let inbox = board
        .inbox(&reader, &all(&InboxQuery::default()))
        .expect("inbox");
    assert_eq!(inbox.len(), 4, "all four selectors address this reader");

    // An employee in a different dept, different team, no matching role,
    // and a different id: nothing should reach them.
    let stranger = NoticeIdentity::new("emp-stranger")
        .in_dept("dept-b")
        .on_teams(vec!["team-b".to_string()]);
    let inbox = board
        .inbox(&stranger, &all(&InboxQuery::default()))
        .expect("inbox");
    assert!(inbox.is_empty(), "stranger must not receive anything");

    // Cross-kind non-delivery, one kind at a time. A `dept:` notice must not
    // reach an employee who is only named by an unrelated team.
    let only_team = NoticeIdentity::new("emp-team-only").on_teams(vec!["team-a".to_string()]);
    let inbox = board
        .inbox(&only_team, &all(&InboxQuery::default()))
        .expect("inbox");
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0].body, "for a team");
}

#[test]
fn org_notice_broadcast_reaches_every_reader() {
    let (board, _dir) = board();
    board
        .post(&PostNotice::new(
            "system",
            vec![Recipient::Broadcast],
            "org-wide announcement",
            "announce",
        ))
        .expect("post");
    // An empty recipient list is broadcast too (axe_lib.py:191-217).
    let normalised = board
        .post(&PostNotice::new(
            "system",
            vec![],
            "implicit broadcast",
            "announce",
        ))
        .expect("post");
    assert_eq!(normalised.recipients, vec![Recipient::Broadcast]);

    let inbox = board
        .inbox(
            &NoticeIdentity::new("whoever"),
            &all(&InboxQuery::default()),
        )
        .expect("inbox");
    assert_eq!(inbox.len(), 2);
}

#[test]
fn org_notice_recipients_resolve_at_read_time_not_write_time() {
    let (board, _dir) = board();
    // Posted BEFORE the reader joined the department — the whole point of
    // storing selectors verbatim (§3.3).
    board
        .post(&PostNotice::new(
            "emp-1",
            vec![Recipient::Dept("late-joiner-dept".into())],
            "posted before you joined",
            "read-time resolution",
        ))
        .expect("post");

    let newcomer = NoticeIdentity::new("emp-new").in_dept("late-joiner-dept");
    let inbox = board
        .inbox(&newcomer, &all(&InboxQuery::default()))
        .expect("inbox");
    assert_eq!(inbox.len(), 1, "dept notice must reach a later joiner");
}

#[test]
fn org_notice_recipient_parser_normalises_legacy_forms() {
    let (board, _dir) = board();
    // The organism's normaliser: a bare string becomes agent:<s>
    // (axe_lib.py:191-217). Measured: 3 such rows exist in the organism.
    let bare = Recipient::parse("axe-test2").expect("parses");
    assert_eq!(bare, Recipient::Agent("axe-test2".into()));
    board
        .post(&PostNotice::new(
            "emp-1",
            vec![bare],
            "legacy bare selector",
            "legacy normalisation",
        ))
        .expect("post");

    let inbox = board
        .inbox(
            &NoticeIdentity::new("axe-test2"),
            &all(&InboxQuery::default()),
        )
        .expect("inbox");
    assert_eq!(inbox.len(), 1);

    // An unknown kind is rejected at post time rather than silently
    // delivering to nobody.
    assert!(Recipient::parse("guild:ops").is_err());
}

// --------------------------------------------------------------------- ack

#[test]
fn org_notice_ack_required_semantics() {
    let (board, _dir) = board();

    let request = board
        .post(&{
            let mut p = PostNotice::new(
                "emp-1",
                vec![Recipient::Agent("emp-2".into())],
                "please review",
                "request",
            );
            p.ack_required = true;
            p.correlation_id = Some("corr-1".into());
            p
        })
        .expect("post");

    let reader = NoticeIdentity::new("emp-2");
    let pending = board.pending_acks(&reader, "emp-2").expect("pending");
    assert_eq!(pending.len(), 1, "unacked request is pending");

    let acked = board.ack(&request.id, "emp-2").expect("ack");
    assert_eq!(acked.acked_by, vec!["emp-2".to_string()]);
    assert!(acked.acked_at.is_some());

    let pending = board.pending_acks(&reader, "emp-2").expect("pending");
    assert!(pending.is_empty(), "acked request leaves the pending set");

    // The ack is also a board notice in the same correlation chain, tagged
    // `ack` (§3.3's request → ack → result protocol).
    let chain = board.correlation_chain("corr-1").expect("chain");
    assert_eq!(chain.len(), 2);
    assert!(
        chain.iter().any(|n| n.is_ack()),
        "chain must contain the ack"
    );
    assert_eq!(
        chain.iter().filter(|n| n.is_ack()).count(),
        1,
        "one ack notice per acker"
    );

    // Repeated ack by the same employee is idempotent.
    board.ack(&request.id, "emp-2").expect("second ack");
    assert_eq!(board.correlation_chain("corr-1").expect("chain").len(), 2);

    // A notice with ack_required unset never enters the pending set.
    let plain = board
        .post(&PostNotice::new(
            "emp-1",
            vec![Recipient::Agent("emp-2".into())],
            "fyi",
            "no ack needed",
        ))
        .expect("post");
    assert!(!plain.ack_required);
    assert_eq!(
        board.pending_acks(&reader, "emp-2").expect("pending").len(),
        0
    );
}

#[test]
fn org_notice_ack_records_every_acker() {
    let (board, _dir) = board();
    let request = board
        .post(&{
            let mut p = PostNotice::new(
                "emp-1",
                vec![Recipient::Dept("dept-a".into())],
                "dept-wide request",
                "request",
            );
            p.ack_required = true;
            p
        })
        .expect("post");

    board.ack(&request.id, "emp-2").expect("ack by 2");
    let after_two = board.ack(&request.id, "emp-3").expect("ack by 3");
    assert_eq!(
        after_two.acked_by,
        vec!["emp-2".to_string(), "emp-3".to_string()]
    );

    // emp-3 is done; emp-4 still has not acked.
    let emp3 = NoticeIdentity::new("emp-3").in_dept("dept-a");
    assert_eq!(
        board.pending_acks(&emp3, "emp-3").expect("pending").len(),
        0
    );
    let emp4 = NoticeIdentity::new("emp-4").in_dept("dept-a");
    assert_eq!(
        board.pending_acks(&emp4, "emp-4").expect("pending").len(),
        1
    );
}

#[test]
fn org_notice_ack_of_missing_notice_is_an_error_not_a_panic() {
    let (board, _dir) = board();
    assert!(board.ack("n_does_not_exist", "emp-1").is_err());
}

// --------------------------------------------------------------------- ttl

#[test]
fn org_notice_ttl_expiry_hides_notice_from_inbox_but_keeps_history() {
    let (board, _dir) = board();

    let short = board
        .post(&{
            let mut p = PostNotice::new(
                "emp-1",
                vec![Recipient::Agent("emp-2".into())],
                "already expired",
                "ttl test",
            );
            // `std::time::Duration` cannot be negative, so a genuinely
            // expired row needs the absolute-instant escape hatch. (The first
            // draft of this test tried `Duration::from_secs(0) - from_secs(60)`
            // and panicked with "overflow when subtracting durations".)
            p.ttl_expires_at = Some(chrono::Utc::now() - chrono::Duration::minutes(5));
            p.thread_id = Some("t1".into());
            p
        })
        .expect("post");

    let live = board
        .post(&{
            let mut p = PostNotice::new(
                "emp-1",
                vec![Recipient::Agent("emp-2".into())],
                "still live",
                "ttl test",
            );
            p.ttl = Some(std::time::Duration::from_secs(3600));
            p
        })
        .expect("post");

    let reader = NoticeIdentity::new("emp-2");
    let inbox = board
        .inbox(&reader, &all(&InboxQuery::default()))
        .expect("inbox");
    assert_eq!(inbox.len(), 1, "expired notice must not be delivered");
    assert_eq!(inbox[0].id, live.id);

    // A thread read is an audit read: TTL governs delivery, not history.
    assert_eq!(board.thread("t1", None).expect("thread").len(), 1);
    assert!(
        board
            .get(&short.id)
            .expect("get")
            .expect("row")
            .ttl_expires_at
            .is_some()
    );
}

// ------------------------------------------------------------- correlation

#[test]
fn org_notice_correlation_groups_a_request_ack_result_chain() {
    let (board, _dir) = board();

    let request = board
        .post(&{
            let mut p = PostNotice::new(
                "emp-1",
                vec![Recipient::Agent("emp-2".into())],
                "do the thing",
                "request",
            );
            p.correlation_id = Some("corr-chain".into());
            p.thread_id = Some("thread-a".into());
            p.ack_required = true;
            p
        })
        .expect("post request");
    board.ack(&request.id, "emp-2").expect("ack");

    // The result lands in the same chain but a different thread — that is
    // the point of a correlation id as opposed to a thread id.
    board
        .post(&{
            let mut p = PostNotice::new(
                "emp-2",
                vec![Recipient::Agent("emp-1".into())],
                "done",
                "result",
            );
            p.correlation_id = Some("corr-chain".into());
            p.thread_id = Some("thread-b".into());
            p
        })
        .expect("post result");

    let chain = board.correlation_chain("corr-chain").expect("chain");
    assert_eq!(chain.len(), 3, "request + ack + result");
    assert_eq!(chain[0].body, "do the thing");
    assert!(chain[1].is_ack());
    assert_eq!(chain[2].body, "done");

    // The ack inherits the parent's thread, which is what makes a thread read
    // a self-contained conversation. The *correlation* chain is the wider
    // view: it also spans the result notice in `thread-b`, which is the whole
    // point of having a correlation id as well as a thread id.
    assert_eq!(
        board.thread("thread-a", None).expect("a").len(),
        2,
        "request + ack"
    );
    assert_eq!(
        board.thread("thread-b", None).expect("b").len(),
        1,
        "result only"
    );

    // An unrelated correlation id groups nothing.
    assert!(
        board
            .correlation_chain("corr-unrelated")
            .expect("chain")
            .is_empty()
    );
}

#[test]
fn org_notice_thread_read_returns_ordered_thread() {
    let (board, _dir) = board();
    for i in 0..3 {
        board
            .post(&{
                let mut p = PostNotice::new(
                    "emp-1",
                    vec![Recipient::Agent("emp-2".into())],
                    format!("reply {i}"),
                    "thread reply",
                );
                p.thread_id = Some("t-order".into());
                p
            })
            .expect("post");
    }
    let thread = board.thread("t-order", Some(2)).expect("thread");
    assert_eq!(thread.len(), 2, "limit applies");
    let epochs: Vec<i64> = thread.iter().map(|n| n.epoch).collect();
    assert!(epochs.windows(2).all(|w| w[0] <= w[1]), "oldest first");
}

// ---------------------------------------------------------------------- GC

#[test]
fn org_notice_gc_deletes_only_expired_and_respects_cap() {
    let (board, _dir) = board();
    let now = chrono::Utc::now();
    let now_epoch = now.timestamp();
    let day = 86_400;

    // 10 expired raw rows.
    for i in 0..10 {
        backdate(
            &board,
            &format!("n_old_{i}"),
            &rfc3339_at(now_epoch - 20 * day),
            now_epoch - 20 * day,
            &[("tags", "[\"status\"]")],
        );
    }
    // 3 fresh rows inside the retention window.
    for i in 0..3 {
        backdate(
            &board,
            &format!("n_new_{i}"),
            &rfc3339_at(now_epoch - 2 * day),
            now_epoch - 2 * day,
            &[("tags", "[\"status\"]")],
        );
    }
    // 2 pinned rows that are old — pins must survive.
    for i in 0..2 {
        backdate(
            &board,
            &format!("n_pinned_{i}"),
            &rfc3339_at(now_epoch - 30 * day),
            now_epoch - 30 * day,
            &[("tags", "[\"status\"]"), ("pinned", "1")],
        );
    }
    assert_eq!(board.count().expect("count"), 15);

    // Cap of 4 raw rows: the pass must delete at most 4, not all 8.
    //
    // `max_weekly_summaries_per_tick` is left at its default so summarization
    // runs: a week is only collectable once its summary exists, so a pass with
    // summarization switched off collects nothing at all (see
    // `org_notice_gc_without_summarization_collects_nothing`).
    let limits = RetentionLimits {
        max_rows_per_pass: 4,
        ..RetentionLimits::default()
    };
    let report = board.retention_gc(now, limits).expect("gc");
    assert_eq!(report.raw_deleted, 4, "cap must bound the delete");
    assert!(report.more_work_pending, "hitting the cap means more work");
    // 15 starting rows, minus 4 deleted, plus the summaries the same pass wrote.
    //
    // Exactly 2 summaries: the 10 old rows all share one timestamp, so they
    // fall in a single week despite being 20 days old, and the 2 pinned rows
    // sit in a second week. Summarization covers that pinned week (it only
    // excludes pins from the *delete*), so it gets a summary even though none
    // of its rows are collectable.
    assert_eq!(report.summaries_written, 2, "one summary per distinct week");
    assert_eq!(board.count().expect("count"), 13, "15 - 4 + 2 summaries");
    assert!(
        board.get("n_pinned_0").expect("get").is_some(),
        "pins survive"
    );
    assert!(
        board.get("n_new_0").expect("get").is_some(),
        "fresh rows survive"
    );
}

#[test]
fn org_notice_gc_drains_the_backlog_over_several_ticks() {
    let (board, _dir) = board();
    let now = chrono::Utc::now();
    let now_epoch = now.timestamp();
    let day = 86_400;

    for i in 0..7 {
        backdate(
            &board,
            &format!("n_old_{i}"),
            &rfc3339_at(now_epoch - 20 * day),
            now_epoch - 20 * day,
            &[("tags", "[\"status\"]")],
        );
    }
    let limits = RetentionLimits {
        max_rows_per_pass: 2,
        ..RetentionLimits::default()
    };

    let mut ticks = 0;
    loop {
        let report = board.retention_gc(now, limits).expect("gc");
        ticks += 1;
        assert!(ticks <= 10, "gc must drain, not loop forever");
        if !report.more_work_pending {
            break;
        }
    }
    assert_eq!(ticks, 4, "7 rows at 2 per tick");
    // Every one of the 7 raw rows is gone; what remains is the single weekly
    // summary that replaced the week they all shared.
    assert_eq!(board.count().expect("count"), 1, "one summary survives");
}

#[test]
fn org_notice_gc_without_summarization_collects_nothing() {
    let (board, _dir) = board();
    let now = chrono::Utc::now();
    let now_epoch = now.timestamp();

    // A week whose rows are old enough to collect.
    let w = week_aligned(now_epoch, 3);
    backdate(
        &board,
        "n_old",
        &rfc3339_at(w),
        w,
        &[("tags", "[\"status\"]")],
    );

    // Turning summarization off means no week ever gets a summary, and a week
    // is only collectable once its summary exists. The board therefore holds
    // its raw rows indefinitely rather than losing them.
    //
    // This is the deliberate cost of not copying §3.3's unconditional raw
    // delete. §3.3 would collect this row immediately — and would also
    // destroy any week the cap had not summarized yet, silently. Holding the
    // row is the recoverable failure; losing it is not. A caller that wants
    // collection must let summarization run, which is the default.
    let limits = RetentionLimits {
        max_rows_per_pass: 500,
        max_weekly_summaries_per_tick: 0,
    };
    let report = board.retention_gc(now, limits).expect("gc");
    assert_eq!(report.summaries_written, 0);
    assert_eq!(report.raw_deleted, 0, "no summary, no collection");
    assert!(board.get("n_old").expect("get").is_some());

    // The default drains the same row.
    let report = board
        .retention_gc(now, RetentionLimits::default())
        .expect("gc 2");
    assert_eq!(report.summaries_written, 1);
    assert_eq!(report.raw_deleted, 1);
}

#[test]
fn org_notice_gc_is_idempotent() {
    let (board, _dir) = board();
    let now = chrono::Utc::now();
    let now_epoch = now.timestamp();
    let day = 86_400;

    backdate(
        &board,
        "n_old",
        &rfc3339_at(now_epoch - 40 * day),
        now_epoch - 40 * day,
        &[("tags", "[\"status\"]")],
    );
    let limits = RetentionLimits::default();

    let first = board.retention_gc(now, limits).expect("gc 1");
    assert_eq!(first.summaries_written, 1, "the old week is summarized");
    assert_eq!(first.raw_deleted, 1, "and its raw row is pruned");

    // Every subsequent pass is a no-op: the deterministic summary id plus
    // `INSERT OR IGNORE` means a retried pass writes nothing new.
    let second = board.retention_gc(now, limits).expect("gc 2");
    assert_eq!(second.raw_deleted, 0);
    assert_eq!(second.summaries_written, 0, "no duplicate summaries");
    assert_eq!(second.summaries_deleted, 0);
    assert!(
        !second.more_work_pending,
        "a pass with nothing to do reports a drained backlog"
    );

    let after_second = board.count().expect("count");
    let third = board.retention_gc(now, limits).expect("gc 3");
    assert_eq!(third.summaries_written, 0);
    assert_eq!(
        board.count().expect("count"),
        after_second,
        "row count stable"
    );

    // Exactly one summary row exists, three passes later.
    let summaries = board
        .query_inbox(&InboxQuery {
            selectors: None,
            ..Default::default()
        })
        .expect("all")
        .into_iter()
        .filter(|n| n.tags.iter().any(|t| t == WEEKLY_SUMMARY_TAG))
        .count();
    assert_eq!(summaries, 1, "one summary per completed week");
}

#[test]
fn org_notice_gc_keeps_a_live_correlation_chain_whole() {
    let (board, _dir) = board();
    let now = chrono::Utc::now();
    let now_epoch = now.timestamp();
    let day = 86_400;
    let limits = RetentionLimits {
        max_rows_per_pass: 500,
        ..RetentionLimits::default()
    };

    // An old request in a chain whose result is still live. §3.3's promise is
    // "a chain is kept whole or not at all".
    backdate(
        &board,
        "n_chain_request",
        &rfc3339_at(now_epoch - 30 * day),
        now_epoch - 30 * day,
        &[("tags", "[\"request\"]"), ("correlation_id", "corr-live")],
    );
    backdate(
        &board,
        "n_chain_result",
        &rfc3339_at(now_epoch - day),
        now_epoch - day,
        &[("tags", "[\"result\"]"), ("correlation_id", "corr-live")],
    );
    // An old notice whose whole chain is expired — collectable.
    backdate(
        &board,
        "n_dead_chain",
        &rfc3339_at(now_epoch - 30 * day),
        now_epoch - 30 * day,
        &[("tags", "[\"request\"]"), ("correlation_id", "corr-dead")],
    );

    let report = board.retention_gc(now, limits).expect("gc");
    assert_eq!(report.raw_deleted, 1, "only the fully expired chain");

    assert!(
        board.get("n_chain_request").expect("get").is_some(),
        "a chain with a live member is kept whole"
    );
    assert!(board.get("n_chain_result").expect("get").is_some());
    assert!(
        board.get("n_dead_chain").expect("get").is_none(),
        "a chain whose members are all expired is collectable"
    );

    // Once the live member of the chain also ages out, the whole chain becomes
    // collectable at once. The sibling crossed the 14-day window on this pass,
    // so its week is now eligible for summarization too and both members go
    // together.
    let later = now + chrono::Duration::days(40);
    let report = board.retention_gc(later, limits).expect("gc 2");
    assert_eq!(report.raw_deleted, 2, "the chain goes as a unit");
    assert!(board.get("n_chain_request").expect("get").is_none());
    assert!(board.get("n_chain_result").expect("get").is_none());
}

#[test]
fn org_notice_gc_summarizes_before_it_prunes() {
    let (board, _dir) = board();
    let now = chrono::Utc::now();
    let now_epoch = now.timestamp();
    let day = 86_400;

    // Two expired rows in ONE week, placed off the boundary. Anchoring to the
    // week grid is what makes "one summary" the honest expectation: raw rows
    // are grouped by `epoch / WEEK_SECONDS`, so rows 1 day apart usually sit in
    // two different weeks.
    let w = week_aligned(now_epoch, 3);
    backdate(
        &board,
        "n_a",
        &rfc3339_at(w),
        w,
        &[("tags", "[\"spawn\"]"), ("correlation_id", "c1")],
    );
    backdate(
        &board,
        "n_b",
        &rfc3339_at(w + day),
        w + day,
        &[("tags", "[\"spawn\"]"), ("correlation_id", "c2")],
    );

    let report = board
        .retention_gc(now, RetentionLimits::default())
        .expect("gc");
    // The summary must carry the real count. §3.3's literal step order
    // (delete → summarize) would record 0 here because the delete runs first.
    assert_eq!(report.summaries_written, 1);
    assert_eq!(report.raw_deleted, 2, "raw rows pruned in the same pass");

    let all = board
        .query_inbox(&InboxQuery {
            selectors: None,
            ..Default::default()
        })
        .expect("all");
    assert_eq!(all.len(), 1, "only the summary row survives");
    let summary = &all[0];
    assert!(summary.is_weekly_summary());

    let meta = summary.metadata.as_ref().expect("summary carries metadata");
    assert_eq!(
        meta["entry_count"].as_i64().expect("count"),
        2,
        "the summary must record the rows it replaced, not zero"
    );
    assert_eq!(
        meta["tag_distribution"]["spawn"]
            .as_i64()
            .expect("tag dist"),
        2
    );
    assert_eq!(
        meta["agent_distribution"]["test-sender"]
            .as_i64()
            .expect("agent dist"),
        2
    );
    assert_eq!(meta["outcomes"].as_object().expect("outcomes").len(), 3);

    // The organism's summary shape (AD-006): a 7-day window and a
    // deterministic id derived from the window start.
    assert_eq!(
        meta["week_end"].as_i64().expect("week_end")
            - meta["week_start"].as_i64().expect("week_start"),
        WEEK_SECONDS
    );
    assert_eq!(
        summary.id,
        format!(
            "n_weekly_{}",
            meta["week_start"].as_i64().expect("week_start")
        )
    );
    assert!(summary.body.contains("WEEKLY SUMMARY"));
    assert!(summary.body.contains("Total notices: 2"));
    assert!(summary.body.contains("spawn(2)"));
    assert!(!summary.is_ack());
}

#[test]
fn org_notice_gc_prunes_summaries_past_the_16_week_window() {
    let (board, _dir) = board();
    let now = chrono::Utc::now();
    let now_epoch = now.timestamp();
    let week = WEEK_SECONDS;

    // A summary row from 20 weeks ago: past SUMMARY_RETENTION_WEEKS (16).
    backdate(
        &board,
        "n_weekly_ancient",
        &rfc3339_at(now_epoch - 20 * week),
        now_epoch - 20 * week,
        &[("tags", "[\"weekly-summary\"]")],
    );
    // A recent summary row: inside the window.
    backdate(
        &board,
        "n_weekly_recent",
        &rfc3339_at(now_epoch - 2 * week),
        now_epoch - 2 * week,
        &[("tags", "[\"weekly-summary\"]")],
    );

    let report = board
        .retention_gc(now, RetentionLimits::default())
        .expect("gc");
    assert_eq!(report.summaries_deleted, 1, "only the 20-week-old summary");
    assert!(board.get("n_weekly_ancient").expect("get").is_none());
    assert!(board.get("n_weekly_recent").expect("get").is_some());
}

#[test]
fn org_notice_gc_caps_weekly_summaries_per_tick() {
    let (board, _dir) = board();
    let now = chrono::Utc::now();
    let now_epoch = now.timestamp();

    // Five distinct completed weeks, snapped to the week grid so each row is
    // unambiguously in its own week. Three weeks back is already past the
    // 14-day raw window.
    for w in 0..5i64 {
        let epoch = week_aligned(now_epoch, 3 + w);
        backdate(
            &board,
            &format!("n_w{w}"),
            &rfc3339_at(epoch),
            epoch,
            &[
                ("tags", "[\"status\"]"),
                ("correlation_id", &format!("cw{w}")),
            ],
        );
    }

    // Per-tag: 4 per tick, the organism's parameter.
    let limits = RetentionLimits {
        max_rows_per_pass: 500,
        max_weekly_summaries_per_tick: DEFAULT_MAX_WEEKLY_SUMMARIES_PER_TICK,
    };
    let report = board.retention_gc(now, limits).expect("gc");
    assert_eq!(
        report.summaries_written, 4,
        "weekly summaries must be capped at 4 per tick"
    );
    assert!(
        report.more_work_pending,
        "a capped summary pass means more work"
    );
    assert_eq!(
        report.raw_deleted, 4,
        "all four summarized weeks are collectable; the fifth has no summary yet"
    );

    // The next tick drains the fifth week.
    let report2 = board.retention_gc(now, limits).expect("gc 2");
    assert_eq!(report2.summaries_written, 1);
    // The fifth week's raw row was *not* deleted on tick 1. §3.3 runs the raw
    // delete unconditionally, which would have destroyed the only evidence
    // tick 2 needs to build that summary — the capped week would be lost
    // outright, and the cap would be a data-loss knob rather than a throttle.
    assert_eq!(report2.raw_deleted, 1, "the fifth week drains on tick 2");
    assert!(!report2.more_work_pending, "backlog is now drained");
    assert_eq!(
        board.count().expect("count"),
        5,
        "5 summary rows, 0 raw rows"
    );
    // A third pass finds only already-summarized weeks: it must report a
    // drained backlog rather than claiming there is more to do, so a caller
    // looping on `more_work_pending` terminates.
    let report3 = board.retention_gc(now, limits).expect("gc 3");
    assert_eq!(report3.summaries_written, 0);
    assert!(!report3.more_work_pending, "nothing new to summarize");
}

#[test]
fn org_notice_gc_never_eats_its_own_weekly_summaries() {
    let (board, _dir) = board();
    let now = chrono::Utc::now();
    let now_epoch = now.timestamp();
    let day = 86_400;

    backdate(
        &board,
        "n_old",
        &rfc3339_at(now_epoch - 20 * day),
        now_epoch - 20 * day,
        &[("tags", "[\"status\"]"), ("correlation_id", "c1")],
    );
    let limits = RetentionLimits::default();

    assert_eq!(
        board
            .retention_gc(now, limits)
            .expect("gc 1")
            .summaries_written,
        1
    );

    // A summary row is stamped with the week it summarizes, so it is always
    // past the 14-day raw window by construction. If the raw delete did not
    // exclude it, the *next* tick would delete the summary and the 16-week
    // prune would never get a chance to run — the board would keep churning
    // empty summaries forever. Run several ticks and prove the summary lives.
    for tick in 2..=6 {
        let report = board.retention_gc(now, limits).expect("gc tick");
        assert_eq!(
            report.raw_deleted, 0,
            "tick {tick} must not delete the summary row"
        );
        assert_eq!(report.summaries_written, 0, "tick {tick} rewrote nothing");
    }

    let all = board
        .query_inbox(&InboxQuery {
            selectors: None,
            ..Default::default()
        })
        .expect("all");
    assert_eq!(all.len(), 1, "the summary survives repeated GC passes");
    assert!(all[0].is_weekly_summary());
}

#[test]
fn org_notice_gc_chain_protection_actually_protects() {
    let (board, _dir) = board();
    let now = chrono::Utc::now();
    let now_epoch = now.timestamp();
    let day = 86_400;
    let limits = RetentionLimits {
        max_rows_per_pass: 500,
        ..RetentionLimits::default()
    };

    // One old member of a two-row chain whose sibling is still live.
    backdate(
        &board,
        "n_old_member",
        &rfc3339_at(now_epoch - 30 * day),
        now_epoch - 30 * day,
        &[
            ("tags", "[\"request\"]"),
            ("correlation_id", "shared-chain"),
        ],
    );
    backdate(
        &board,
        "n_live_sibling",
        &rfc3339_at(now_epoch - day),
        now_epoch - day,
        &[("tags", "[\"result\"]"), ("correlation_id", "shared-chain")],
    );

    let report = board.retention_gc(now, limits).expect("gc");
    assert_eq!(
        report.raw_deleted, 0,
        "a chain with a live member must survive intact"
    );
    assert!(board.get("n_old_member").expect("get").is_some());

    // A chain of one expired row is collectable, which is what proves the
    // filter is discriminating rather than just refusing to delete anything.
    backdate(
        &board,
        "n_lonely",
        &rfc3339_at(now_epoch - 30 * day),
        now_epoch - 30 * day,
        &[
            ("tags", "[\"request\"]"),
            ("correlation_id", "lonely-chain"),
        ],
    );
    let report = board.retention_gc(now, limits).expect("gc 2");
    assert_eq!(
        report.raw_deleted, 1,
        "a fully expired chain is collectable"
    );
    assert!(board.get("n_lonely").expect("get").is_none());
    assert!(board.get("n_live_sibling").expect("get").is_some());
}

#[test]
fn org_notice_gc_finds_new_weeks_after_a_summary_exists() {
    let (board, _dir) = board();
    let now = chrono::Utc::now();
    let now_epoch = now.timestamp();

    // Five weeks, all eligible. The first pass writes 4 summaries.
    for w in 0..5i64 {
        let epoch = week_aligned(now_epoch, 3 + w);
        backdate(
            &board,
            &format!("n_w{w}"),
            &rfc3339_at(epoch),
            epoch,
            &[
                ("tags", "[\"status\"]"),
                ("correlation_id", &format!("cw{w}")),
            ],
        );
    }
    let limits = RetentionLimits::default();
    let first = board.retention_gc(now, limits).expect("gc 1");
    assert_eq!(first.summaries_written, 4);

    // The fifth week must still be findable now that four summaries exist.
    //
    // This is the regression that matters: the candidate select asks "does
    // this week's summary row exist?". Written with an unqualified `epoch`
    // inside the `NOT EXISTS` subquery, SQLite resolves it to the *summary
    // row's* epoch, so the predicate becomes "does any summary row's own week
    // have a summary" — true for the first summary and false for every row
    // after it. The select then returns zero candidates forever and the GC
    // deadlocks: the backlog silently stops draining, with no error and
    // `more_work_pending` stuck at whatever the last non-zero pass reported.
    let second = board.retention_gc(now, limits).expect("gc 2");
    assert_eq!(
        second.summaries_written, 1,
        "a new week must still be found after summaries exist"
    );
    assert!(
        !second.more_work_pending,
        "the backlog is now fully drained"
    );
    assert_eq!(board.count().expect("count"), 5, "5 summaries, 0 raw rows");
}

// ------------------------------------------------------------ no second store

#[test]
fn org_notice_board_creates_no_database_file_beyond_the_one_it_was_given() {
    let (board, dir) = board();

    board
        .post(&PostNotice::new(
            "emp-1",
            vec![Recipient::Broadcast],
            "hello",
            "store check",
        ))
        .expect("post");
    board
        .retention_gc(chrono::Utc::now(), RetentionLimits::default())
        .expect("gc");

    // Exactly one .db file, and it is the kanban sibling DB. No notice.db,
    // no notice-board.jsonl, no sidecar of any kind.
    let mut entries: Vec<String> = std::fs::read_dir(dir.path())
        .expect("read dir")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    entries.sort();
    assert_eq!(
        entries,
        vec!["operant_kanban.db".to_string()],
        "the board must not create a second store: saw {entries:?}"
    );
}

#[test]
fn org_notice_board_shares_the_kanban_file_rather_than_opening_its_own() {
    // Prove the table can live inside a DB another subsystem already opened:
    // one connection, the kanban table and the notices table side by side.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("operant_kanban.db");
    let conn = rusqlite::Connection::open(&path).expect("open");
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS tasks (id TEXT PRIMARY KEY, title TEXT NOT NULL);
         INSERT INTO tasks (id, title) VALUES ('t1', 'pre-existing kanban row');",
    )
    .expect("seed kanban table");

    let board = NoticeBoard::from_connection(conn).expect("attach board");
    board
        .post(&PostNotice::new(
            "emp-1",
            vec![Recipient::Broadcast],
            "coexisting with kanban",
            "shared file",
        ))
        .expect("post");

    // The kanban row survived, and the notices table landed in the same file.
    let conn = board.conn().lock().expect("lock");
    let title: String = conn
        .query_row("SELECT title FROM tasks WHERE id = 't1'", [], |r| r.get(0))
        .expect("kanban row intact");
    assert_eq!(title, "pre-existing kanban row");
    let notices: i64 = conn
        .query_row("SELECT COUNT(*) FROM notices", [], |r| r.get(0))
        .expect("notices in same file");
    assert_eq!(notices, 1);
}

#[test]
fn org_notice_board_does_not_bump_the_file_wide_schema_version() {
    // `PRAGMA user_version` is file-wide; the board must leave kanban's
    // version alone or kanban's next open hard-fails with "refusing to
    // downgrade" (migrations.rs's documented INVARIANT, from R39-7).
    let (board, _dir) = board();
    let conn = board.conn().lock().expect("lock");
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .expect("user_version");
    assert_eq!(version, 0, "the board must not claim a migration version");
}
