//! Integration tests for scheduled-run autocompaction and its compaction floor.
//!
//! §4.5 requires two things that pull in opposite directions: every persistent
//! session must compact after its scheduled run, and short sessions must not be
//! reduced to a lossy summary-of-a-summary. The floor is what reconciles them,
//! so the floor is what these tests are mostly about.
//!
//! The headline case is the first test: a session under the floor is left
//! completely alone. If the floor is deleted, that test fails. The rest pin the
//! properties a compaction must not break — the session survives, the loss is
//! recorded, and repeating the operation does not degrade the session further.
//!
//! Every test drives the real [`Autocompactor`] over a real [`SessionStore`] and
//! a real [`Database`], through the same public API the scheduler will use.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use operant_core::client::{Message, Role};
use operant_core::context_management::estimate_total_tokens;
use operant_core::database::Database;
use operant_core::session::autocompact::{
    AutocompactConfig, Autocompactor, COMPACTION_FLOOR_TOKENS, COMPACTION_MARKER,
    CompactionOutcome, KEEP_HEAD_MESSAGES, KEEP_TAIL_TOKENS,
};
use operant_core::session::{SessionKey, SessionStore, SessionStoreConfig};

/// An autocompactor over its own temporary database.
struct Fixture {
    _dir: tempfile::TempDir,
    db: Arc<Database>,
    store: Arc<SessionStore>,
    compactor: Autocompactor,
}

impl Fixture {
    fn new() -> Self {
        Self::with_config(AutocompactConfig::default())
    }

    fn with_config(config: AutocompactConfig) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Arc::new(Database::init(dir.path().join("autocompact.sqlite")).expect("init"));
        let store = Arc::new(SessionStore::with_config(
            Arc::clone(&db),
            SessionStoreConfig {
                warm_capacity: 8,
                // Long enough that a seeded transcript is rehydrated whole, so
                // the store's own tail window is not what these tests measure.
                rehydrate_tail: 1000,
            },
        ));
        let compactor =
            Autocompactor::with_config(Arc::clone(&store), config).with_database(Arc::clone(&db));
        Self {
            _dir: dir,
            db,
            store,
            compactor,
        }
    }

    /// Seed `session` with `contents` and make it resident, the way a finished
    /// scheduled run leaves it: turns persisted under the session id, and the
    /// store holding the live copy.
    fn seed(&self, session: &str, contents: &[Message]) -> SessionKey {
        let key = SessionKey::new(session);
        self.db
            .save_session(session, None, "employee", "t0", "t0")
            .expect("save session");
        for (i, m) in contents.iter().enumerate() {
            self.db
                .save_message(session, m.role.as_str(), &m.content, &format!("t{i:04}"))
                .expect("save message");
        }
        self.store.acquire(&key);
        self.store.persist(&key, contents);
        key
    }

    /// The resident transcript's contents, in order.
    fn contents(&self, key: &SessionKey) -> Vec<String> {
        self.store
            .peek(key)
            .into_iter()
            .map(|m| m.content)
            .collect()
    }
}

/// A user turn of roughly `tokens` estimated tokens.
fn turn(label: &str, tokens: usize) -> Message {
    Message::user(format!("{label} {}", "x".repeat(tokens * 4)))
}

/// A transcript of `n` user/assistant pairs whose middle is comfortably
/// compressible: well over the floor, and long enough that the tail budget
/// cannot swallow all of it.
fn long_transcript(pairs: usize, per_turn_tokens: usize) -> Vec<Message> {
    let mut out = vec![Message::system("You are Victor, Head of Engineering.")];
    for i in 0..pairs {
        out.push(turn(&format!("prompt{i}"), per_turn_tokens));
        out.push(Message::assistant(format!(
            "reply{i} {}",
            "y".repeat(per_turn_tokens * 4)
        )));
    }
    out
}

/// A transcript that stays under the floor: real opening material, no bulk.
fn short_transcript() -> Vec<Message> {
    vec![
        Message::system("You are Victor, Head of Engineering."),
        Message::user("status?"),
        Message::assistant("green"),
    ]
}

// ---------------------------------------------------------------------------
// The floor
// ---------------------------------------------------------------------------

/// THE HEADLINE CASE. A session below the floor keeps its verbatim history,
/// untouched, and says so.
#[test]
fn a_session_below_the_floor_is_not_compacted() {
    let f = Fixture::new();
    let messages = short_transcript();
    let key = f.seed("emp-short", &messages);
    let before = f.contents(&key);

    let outcome = f.compactor.compact(&key);

    assert_eq!(
        outcome,
        CompactionOutcome::SkippedByFloor {
            tokens: estimate_total_tokens(&messages),
            floor: COMPACTION_FLOOR_TOKENS,
        },
        "a short session must be reported as floor-skipped, got: {}",
        outcome.describe()
    );
    assert!(
        !outcome.compacted(),
        "the floor must not report itself as a compaction"
    );
    assert_eq!(
        f.contents(&key),
        before,
        "a session under the floor keeps its verbatim history"
    );
    assert!(
        !f.contents(&key)
            .iter()
            .any(|c| c.contains(COMPACTION_MARKER)),
        "an uncompacted session must not gain a marker"
    );
    assert_eq!(f.compactor.stats().compactions, 0, "nothing compacted");
    assert_eq!(f.compactor.stats().skipped_by_floor, 1, "floor stopped it");
}

/// The floor's boundary is exact: a session pushed just over it compacts, and
/// the skip is reported with the numbers that made the decision.
///
/// The tail budget is shrunk to 1 000 tokens *on purpose*. The default tail is
/// larger than the default floor, so at defaults a transcript can sit above the
/// floor while being wholly tail-protected — "nothing to compact" would then be
/// the honest answer, and this test would be measuring the tail rather than the
/// floor. Holding the tail below the floor isolates the floor as the single
/// binding constraint, which is what this test is about.
#[test]
fn the_floor_boundary_is_exact() {
    let mut config = AutocompactConfig::default();
    config.keep_tail_tokens = 1_000;
    let f = Fixture::with_config(config);
    let key = f.seed("emp-edge", &long_transcript(12, 200));
    let tokens = estimate_total_tokens(&f.store.peek(&key));
    assert!(
        tokens > COMPACTION_FLOOR_TOKENS,
        "fixture must be over the floor to test the boundary, got {tokens}"
    );

    let outcome = f.compactor.compact(&key);

    assert!(
        outcome.compacted(),
        "a session over the floor must compact, got: {}",
        outcome.describe()
    );
}

/// At the **default** config the tail is larger than the floor, so a transcript
/// above the floor can still be entirely head/tail protected. That is not a
/// floor skip and must not be reported as one: it is its own outcome, and it is
/// the case the idempotence rules exist for.
#[test]
fn above_the_floor_but_fully_protected_is_not_a_floor_skip() {
    let f = Fixture::new();
    // Comfortably over the 4 000 floor, comfortably under the 8 000 tail.
    let messages = long_transcript(12, 180);
    let key = f.seed("emp-protected", &messages);
    let tokens = estimate_total_tokens(&f.store.peek(&key));
    assert!(
        tokens > COMPACTION_FLOOR_TOKENS,
        "fixture must be over the floor, got {tokens}"
    );

    let outcome = f.compactor.compact(&key);

    assert_eq!(
        outcome,
        CompactionOutcome::SkippedNothingToCompact {
            messages: messages.len(),
        },
        "a wholly protected transcript must report nothing-to-compact, not a \
         floor skip, got: {}",
        outcome.describe()
    );
    assert!(
        !matches!(outcome, CompactionOutcome::SkippedByFloor { .. }),
        "the floor was not the binding constraint here"
    );
}

// ---------------------------------------------------------------------------
// Compaction happens, and the session survives
// ---------------------------------------------------------------------------

/// A session above the floor compacts: the transcript gets smaller, the middle
/// is dropped, and the loss is recorded rather than silent.
#[test]
fn a_session_above_the_floor_is_compacted() {
    let f = Fixture::new();
    let messages = long_transcript(20, 300);
    let key = f.seed("emp-long", &messages);

    let outcome = f.compactor.compact(&key);

    assert!(outcome.compacted(), "got: {}", outcome.describe());
    let CompactionOutcome::Compacted {
        messages_before,
        messages_after,
        messages_dropped,
        tokens_before,
        tokens_after,
        summarised,
        ..
    } = outcome
    else {
        panic!("expected Compacted, got {outcome:?}");
    };

    assert!(messages_after < messages_before, "the transcript shrank");
    assert!(messages_dropped > 0, "something was actually dropped");
    assert!(tokens_after < tokens_before, "the token count fell");
    assert!(
        !summarised,
        "the deterministic path replaces the middle with a marker, not a summary"
    );

    let after = f.contents(&key);
    assert_eq!(after.len(), messages_after, "the store holds the new shape");
    assert_eq!(
        after
            .iter()
            .filter(|c| c.contains(COMPACTION_MARKER))
            .count(),
        1,
        "exactly one marker records the compaction"
    );
    let marker = after
        .iter()
        .find(|c| c.contains(COMPACTION_MARKER))
        .expect("marker present");
    assert!(
        marker.contains("emp-long"),
        "the marker names the session it compacted"
    );
    assert!(
        marker.contains(&messages_dropped.to_string()),
        "the marker states how many messages were dropped, got: {marker}"
    );
}

/// The session survives compaction: the same id, a usable transcript, and a
/// cold acquire still finds it.
#[test]
fn the_session_id_and_a_usable_transcript_survive_compaction() {
    let f = Fixture::new();
    let messages = long_transcript(20, 300);
    let key = f.seed("emp-survivor", &messages);
    let id_before = key.id().to_string();

    let outcome = f.compactor.compact(&key);
    assert!(outcome.compacted(), "got: {}", outcome.describe());
    assert_eq!(key.id(), id_before, "the key's id is unchanged");

    // The resident copy is usable: it still has the system prompt and the most
    // recent turn, so the next scheduled run has something to work with.
    let resident = f.store.peek(&key);
    assert!(
        !resident.is_empty(),
        "compaction must not empty the session"
    );
    assert_eq!(
        resident[0].content, messages[0].content,
        "the system prompt survives as the first message"
    );
    assert_eq!(
        resident.last().map(|m| m.content.clone()),
        messages.last().map(|m| m.content.clone()),
        "the most recent turn survives verbatim"
    );

    // A cold rehydrate still resolves to this session and still finds a usable
    // transcript, so a restart does not come back to an empty slot.
    assert!(
        f.store.evict(&key),
        "the session was resident and is now cold"
    );
    let rehydrated = f.store.acquire(&key);
    assert!(
        rehydrated > 0,
        "a cold acquire of a compacted session must return a transcript, not 0"
    );
}

/// Head and tail are protected, and only the middle is dropped. A compaction
/// must not be able to remove the system prompt or the opening exchange.
#[test]
fn compaction_preserves_the_system_prompt_and_the_most_recent_turns() {
    let f = Fixture::new();
    let messages = long_transcript(20, 300);
    let key = f.seed("emp-preserve", &messages);

    let outcome = f.compactor.compact(&key);
    assert!(outcome.compacted(), "got: {}", outcome.describe());

    let after = f.store.peek(&key);
    assert_eq!(
        after[0].role,
        Role::System,
        "the system prompt is still first"
    );
    assert_eq!(
        after[0].content, messages[0].content,
        "the system prompt is byte-identical, not summarised"
    );

    // The opening exchange is inside the protected head.
    for i in 1..=KEEP_HEAD_MESSAGES {
        assert_eq!(
            after[i].content, messages[i].content,
            "opening message {i} is inside the protected head"
        );
    }

    // The most recent turn survives verbatim at the end.
    assert_eq!(
        after.last().map(|m| m.content.clone()),
        messages.last().map(|m| m.content.clone()),
        "the last turn is inside the protected tail"
    );

    // And the marker sits after the head, not before it.
    let marker_at = after
        .iter()
        .position(|m| m.content.contains(COMPACTION_MARKER))
        .expect("marker present");
    assert!(
        marker_at >= KEEP_HEAD_MESSAGES,
        "the marker replaces the middle, so it comes after the head"
    );
}

/// Compaction is idempotent: running it again on an already-compacted session
/// changes nothing, and does not stack markers or shrink it further.
///
/// Three passes, not two, because a third is where a session that grew back over
/// the floor has to be re-compacted *legitimately* — and that new compaction must
/// not erase the previous record.
#[test]
fn compaction_is_idempotent() {
    let f = Fixture::new();
    let key = f.seed("emp-twice", &long_transcript(20, 300));

    let first = f.compactor.compact(&key);
    assert!(first.compacted(), "first pass: {}", first.describe());
    let after_first = f.contents(&key);

    let second = f.compactor.compact(&key);
    let after_second = f.contents(&key);

    assert!(
        !second.compacted(),
        "a second pass must not compact again, got: {}",
        second.describe()
    );
    assert_eq!(
        after_second, after_first,
        "a second compaction must not degrade the session further"
    );
    assert_eq!(
        after_second
            .iter()
            .filter(|c| c.contains(COMPACTION_MARKER))
            .count(),
        1,
        "the first compaction's record must survive the second pass"
    );
    assert_eq!(f.compactor.stats().compactions, 1, "exactly one compaction");

    // A third pass on an unchanged session is still a no-op.
    let third = f.compactor.compact(&key);
    assert!(
        !third.compacted(),
        "a third pass must not compact either, got: {}",
        third.describe()
    );
    assert_eq!(f.contents(&key), after_first, "still unchanged");
    assert_eq!(f.compactor.stats().compactions, 1, "still exactly one");
    assert_eq!(
        f.compactor.stats().invocations,
        3,
        "all three passes observable"
    );
}

/// The tail walk stops at the newest marker, and that stop keeps the marker
/// inside the tail instead of letting it become droppable middle. This test
/// isolates that rule from the marker-only check that sits beside it.
///
/// The fixture is shaped so the two rules disagree. Reading backward, the tail
/// budget (1 000) is large enough that the walk reaches the record, and there
/// are **two** droppable messages between the head and the record — more than one,
/// so the marker-only check cannot fire. What separates the behaviours is only
/// where the walk stops:
///
/// - stopping **at** the marker leaves the two messages as the middle; the record
///   stays in the tail, so both records survive;
/// - stopping **after** it leaves the two messages *and the record* as the
///   middle; the pass replaces the record, and the transcript ends up claiming
///   one compaction when two happened.
///
/// Both outcomes are legitimate compactions — the material genuinely grew back.
/// The record surviving is the part that is not negotiable.
#[test]
fn an_existing_record_survives_a_later_compaction() {
    let mut config = AutocompactConfig::default();
    config.keep_tail_tokens = 1_000;
    let f = Fixture::with_config(config);

    // Head, large enough to clear the floor on its own.
    let mut messages = vec![Message::system("You are Victor, Head of Engineering.")];
    for i in 0..KEEP_HEAD_MESSAGES {
        messages.push(turn(&format!("head{i}"), 1_400));
    }
    // Regrown material in front of the record: two messages, so the middle is
    // longer than one and the marker-only rule cannot rescue the record.
    messages.push(turn("regrown-a", 200));
    messages.push(turn("regrown-b", 200));
    // The earlier record.
    messages.push(Message::system(format!(
        "{COMPACTION_MARKER} This session was compacted after a scheduled run.\n\
         Session: emp-marker-mid\n\
         Dropped: 30 messages, ~9000 tokens\n\
         Compaction: #1"
    )));
    // A tail smaller than the budget, so the walk reaches the record.
    for i in 0..3 {
        messages.push(turn(&format!("tail{i}"), 120));
    }
    let key = f.seed("emp-marker-mid", &messages);
    let tokens = estimate_total_tokens(&f.store.peek(&key));
    assert!(
        tokens > COMPACTION_FLOOR_TOKENS,
        "fixture must be over the floor, got {tokens}"
    );

    let outcome = f.compactor.compact(&key);

    assert!(
        outcome.compacted(),
        "the regrown material must compact, got: {}",
        outcome.describe()
    );
    let records = f
        .contents(&key)
        .into_iter()
        .filter(|c| c.contains(COMPACTION_MARKER))
        .count();
    assert_eq!(
        records, 2,
        "the earlier record must survive alongside the new one; a count of 1 \
         means the old record was summarised away"
    );
    assert_eq!(
        f.compactor.stats().compactions,
        1,
        "exactly one compaction, recorded as generation 2"
    );
}

/// A session that genuinely grows past the floor again compacts again — and the
/// second compaction is recorded as a new generation rather than erasing the
/// first. This is the growth path, distinct from idempotence.
#[test]
fn a_session_that_grows_again_compacts_again_and_keeps_the_earlier_record() {
    let f = Fixture::new();
    let key = f.seed("emp-regrow", &long_transcript(20, 300));

    let first = f.compactor.compact(&key);
    assert!(first.compacted(), "first: {}", first.describe());
    let marker_first = f
        .contents(&key)
        .into_iter()
        .find(|c| c.contains(COMPACTION_MARKER))
        .expect("marker after the first compaction");

    // The next scheduled run appends real work.
    let mut grown = f.store.peek(&key);
    for i in 0..20 {
        grown.push(turn(&format!("followup{i}"), 300));
        grown.push(Message::assistant(format!("done{i} {}", "z".repeat(1200))));
    }
    f.store.persist(&key, &grown);

    let second = f.compactor.compact(&key);
    assert!(second.compacted(), "second: {}", second.describe());

    let after = f.contents(&key);
    let markers: Vec<&String> = after
        .iter()
        .filter(|c| c.contains(COMPACTION_MARKER))
        .collect();
    assert_eq!(
        markers.len(),
        1,
        "a re-compaction replaces the old marker with a new one rather than \
         stacking, got {markers:?}"
    );
    assert_ne!(
        *markers[0], marker_first,
        "the surviving marker is the new record, not the old one"
    );
    assert!(
        markers[0].contains("Compaction: #2"),
        "the new record names its generation, got: {}",
        markers[0]
    );
    assert_eq!(f.compactor.stats().compactions, 2, "two real compactions");
}

/// The floor default is pinned. Changing it silently is a behaviour change that
/// must fail a test, not slide through.
#[test]
fn the_floor_default_is_pinned() {
    assert_eq!(
        COMPACTION_FLOOR_TOKENS, 4_000,
        "the compaction floor default changed — §4.5 fixes it at 4 000 tokens \
         and any new value needs a fresh derivation and an owner decision"
    );

    // The default config must actually carry that value, not a copy of it.
    let config = AutocompactConfig::default();
    assert_eq!(config.floor_tokens, COMPACTION_FLOOR_TOKENS);
    assert_eq!(config.keep_tail_tokens, KEEP_TAIL_TOKENS);
    assert_eq!(config.keep_head_messages, KEEP_HEAD_MESSAGES);
    assert!(config.enabled, "compaction is on by default");

    // The idempotence argument depends on the tail budget exceeding the floor:
    // a compacted transcript must fall inside the next pass's tail walk, or a
    // second pass would find a fresh middle and compact again.
    assert!(
        KEEP_TAIL_TOKENS > COMPACTION_FLOOR_TOKENS,
        "idempotence depends on the tail budget ({KEEP_TAIL_TOKENS}) exceeding \
         the floor ({COMPACTION_FLOOR_TOKENS})"
    );
}

// ---------------------------------------------------------------------------
// Observability
// ---------------------------------------------------------------------------

/// Every pass reports something. A caller can always tell a compaction from a
/// floor skip from a "nothing to compact" — silence is not an option.
#[test]
fn every_pass_reports_an_outcome() {
    let f = Fixture::new();
    let short = f.seed("emp-report-short", &short_transcript());
    let long = f.seed("emp-report-long", &long_transcript(20, 300));

    let skipped = f.compactor.compact(&short);
    let compacted = f.compactor.compact(&long);
    let repeat = f.compactor.compact(&long);

    for outcome in [&skipped, &compacted, &repeat] {
        let described = outcome.describe();
        assert!(
            !described.is_empty(),
            "every outcome must describe itself: {outcome:?}"
        );
        assert!(
            described.contains("not compacted") || described.contains("compacted"),
            "the report must state whether compaction happened: {described}"
        );
    }

    assert!(matches!(skipped, CompactionOutcome::SkippedByFloor { .. }));
    assert!(compacted.compacted());
    assert!(matches!(
        repeat,
        CompactionOutcome::SkippedByFloor { .. }
            | CompactionOutcome::SkippedNothingToCompact { .. }
    ));

    let stats = f.compactor.stats();
    assert_eq!(stats.invocations, 3, "every pass is counted");
    assert_eq!(stats.compactions, 1);
    assert!(stats.skipped_by_floor + stats.skipped_nothing_to_compact >= 1);
}

/// A disabled compactor says so rather than doing nothing quietly.
#[test]
fn a_disabled_compactor_reports_disabled() {
    let config = AutocompactConfig {
        enabled: false,
        ..Default::default()
    };
    let f = Fixture::with_config(config);
    let messages = long_transcript(20, 300);
    let key = f.seed("emp-off", &messages);

    let outcome = f.compactor.compact(&key);

    assert_eq!(outcome, CompactionOutcome::Disabled);
    assert_eq!(
        f.contents(&key),
        messages_contents(&messages),
        "nothing changed"
    );
    assert_eq!(f.compactor.stats().compactions, 0);
}

/// A zero floor is the documented "unconditionally compact" setting, so the
/// deviation from §4.5 is one config field rather than a code change.
#[test]
fn a_zero_floor_makes_compaction_unconditional() {
    let config = AutocompactConfig {
        floor_tokens: 0,
        ..Default::default()
    };
    let f = Fixture::with_config(config);
    let key = f.seed("emp-unconditional", &long_transcript(20, 300));

    let outcome = f.compactor.compact(&key);

    assert!(outcome.compacted(), "got: {}", outcome.describe());
    assert_eq!(
        f.compactor.floor_tokens(),
        0,
        "the floor is honoured as set"
    );
}

/// Compaction is a change of view, not of history: the persisted rows are still
/// there, so a lossy post-run step never silently destroys an employee's work.
#[test]
fn compaction_does_not_delete_persisted_history() {
    let f = Fixture::new();
    let messages = long_transcript(20, 300);
    let key = f.seed("emp-history", &messages);

    let outcome = f.compactor.compact(&key);
    assert!(outcome.compacted(), "got: {}", outcome.describe());

    let persisted =
        f.db.get_session_messages("emp-history")
            .expect("read persisted messages");
    assert_eq!(
        persisted.len(),
        messages.len(),
        "every persisted turn is still on disk after compaction"
    );
}

/// Compaction is scoped to the session it was asked about. One employee's
/// compaction must not touch a colleague's transcript.
#[test]
fn compaction_does_not_cross_sessions() {
    let f = Fixture::new();
    let mine = f.seed("emp-mine", &long_transcript(20, 300));
    let theirs = f.seed("emp-theirs", &long_transcript(20, 300));
    let theirs_before = f.contents(&theirs);

    let outcome = f.compactor.compact(&mine);
    assert!(outcome.compacted(), "got: {}", outcome.describe());

    assert_eq!(
        f.contents(&theirs),
        theirs_before,
        "the other session is untouched"
    );
    assert!(
        !f.contents(&theirs)
            .iter()
            .any(|c| c.contains(COMPACTION_MARKER)),
        "the other session gained no marker"
    );
}

fn messages_contents(messages: &[Message]) -> Vec<String> {
    messages.iter().map(|m| m.content.clone()).collect()
}
