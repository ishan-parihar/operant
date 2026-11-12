//! Per-turn setup for `OperantAgent::run()` (the turn prologue).
//!
//! Ports the pattern from `hermes-agent/agent/turn_context.py`: all
//! once-per-turn setup — interrupt flag reset, session ID resolution,
//! evolution state hydration, user message dedup, DB session creation,
//! message building — runs before the tool-calling loop and produces
//! a fixed set of values the loop consumes.
//!
//! ## Design
//!
//! `TurnContext` captures the *locals* the loop reads back. The builder
//! mutates agent state (counters, DB) exactly as the inline code did —
//! those side effects are the point. The struct it returns carries only
//! the values the loop unpacks.
//!
//! This is a pure move-and-name refactor with no semantic change from
//! the original inline prologue in `run()`.

use crate::client::Message;
use crate::context_references;
use crate::error::Result;

use super::OperantAgent;

use tracing::{debug, warn};

/// Values produced by the turn prologue and consumed by the turn loop.
///
/// Extracted from the inline setup in `OperantAgent::run()` to make the
/// per-turn setup testable and to shrink the orchestrator by the full
/// prologue. Matches hermes-agent's `TurnContext` dataclass.
#[derive(Debug)]
pub struct TurnContext {
    /// Resolved session ID (persistent or freshly generated).
    pub session_id: String,
    /// Whether the user message was already in the conversation (dedup).
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "written by build_turn_context; read only by tests"
        )
    )]
    pub already_added: bool,
    /// Working message list for this turn (loop appends to it).
    pub messages: Vec<Message>,
}

/// A short, human-readable title for the interrupted-session marker.
///
/// The marker's title is what a resume notice shows without a database
/// lookup, so it must be the operator's own words — their first prompt —
/// and short enough to read at a glance.
fn interrupted_title(query: &str) -> String {
    const MAX: usize = 60;
    let trimmed = query.trim();
    if trimmed.chars().count() <= MAX {
        return trimmed.to_string();
    }
    let head: String = trimmed.chars().take(MAX).collect();
    format!("{}…", head.trim_end())
}

/// Clear the interrupted-session records for `session_id`.
///
/// (RT-511) The records are written when a turn STARTS, so a kill
/// mid-turn leaves them behind. A turn that ended CLEANLY must take
/// them back: the session was answered, and a leftover record would
/// offer a resume for work that is already done.
///
/// The file marker is a single slot shared by every session in the
/// home, so it is deleted only when it names THIS session — a clean
/// turn in one session must not delete another session's offer.
///
/// Called on the clean terminal-reply paths only. A turn that was
/// broken off (circuit breaker, interrupt, error, budget) leaves the
/// records in place: that session WAS cut off, and that is the answer
/// the operator needs.
pub async fn clear_interrupted_records(agent: &OperantAgent, session_id: &str) {
    if let Err(e) = agent.database.clear_session_interrupted(session_id) {
        warn!(error = %e, "Failed to clear the interrupted-session flag");
    }
    if !crate::session_interrupted::clear_marker_for(session_id) {
        debug!(
            session_id = %session_id,
            "No interrupted-session marker for this session to clear"
        );
    }
}

/// Run the once-per-turn setup and return the loop's input context.
///
/// Performs:
/// 1. Interrupt flag reset (prevents stale Ctrl-C from prior run)
/// 2. Session ID resolution (persistent or fresh UUID)
/// 3. Evolution state hydration from persisted metadata (Phase 4)
/// 4. User message dedup check
/// 5. DB session + user message persistence
/// 6. Message building (system prompt + preflight compression + eviction)
///
/// Behavior is identical to the original inline prologue; this is a
/// pure move-and-name refactor with no semantic change.
#[expect(
    clippy::expect_used,
    reason = "poisoned lock: panic is the intended recovery"
)]
pub async fn build_turn_context(agent: &OperantAgent, user_query: &str) -> Result<TurnContext> {
    // ── 0. @-reference expansion ─────────────────────────────────────
    // Expand `@file:`, `@folder:`, `@git:…`, `@url:…` tokens the user typed
    // BEFORE the message enters the conversation, so every surface (CLI, TUI,
    // gateway, cron, autonomous, sub-agents) gets identical expansion. Mirrors
    // hermes agent/context_references.py (R1). Warnings are surfaced to the
    // user via the AgentEvent channel below.
    let context_window = agent.effective_context_window();
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let ctx =
        context_references::preprocess_context_references(user_query, &cwd, context_window).await;
    let effective_query = ctx.message.clone();

    if !ctx.warnings.is_empty() && !ctx.blocked {
        let joined = ctx.warnings.join(" | ");
        warn!(warnings = %joined, "@ context references expanded with warnings");
        agent
            .emit(crate::agent::AgentEvent::Content {
                text: format!("⚠ {joined}"),
            })
            .await;
    }
    if ctx.blocked {
        warn!(
            injected_tokens = ctx.injected_tokens,
            "@ context injection refused (50% hard limit)"
        );
        agent
            .emit(crate::agent::AgentEvent::Content {
                text: "⚠ @ context injection refused: attached files exceed the 50% hard limit"
                    .to_string(),
            })
            .await;
    }

    // ── 1. Reset interrupt flag ──────────────────────────────────────
    // Without this, a Ctrl-C in run #1 permanently breaks run #2+
    // (the flag stays triggered and the loop exits immediately).
    agent.interrupt_flag.reset();

    // ── 2. Session ID resolution ─────────────────────────────────────
    // ONE id, resolved once and stored back on the agent.
    //
    // A host that owns session identity (TUI/WebSocket via
    // `with_persistent_session`, the gateway via `set_session_id`)
    // assigns it before the first turn, and every turn reuses it. Only
    // an unconfigured one-shot run mints here — and the mint is written
    // back, so a second turn on the same agent lands in the SAME
    // session instead of orphaning its trajectory under a fresh uuid.
    // Before this, every turn minted a new id and the whole tool
    // trajectory became unreachable: the gateway reloads its own
    // stable `gw_<hash>` id, which the per-turn `sess_<uuid>` rows never
    // appeared under.
    // Whether a HOST owns this session id (the TUI/WebSocket via
    // `with_persistent_session`, the gateway via `set_session_id`). Only a
    // host-owned session has a boot path that can offer it back, and the
    // file marker IS that offer — so it is written for those sessions and
    // not for a one-shot run that minted its own id (RT-511).
    let host_owned = agent.host_assigned_session();
    let session_id = match agent.session_id() {
        Some(id) => id,
        None => {
            let id = format!("sess_{}", uuid::Uuid::new_v4());
            agent.set_session_id(id.clone());
            // The prologue's OWN mint is not a host assignment, and
            // `set_session_id` cannot tell the two apart — so undo the
            // mark here. Read BEFORE the mint (above), for the same reason.
            agent.set_host_assigned_session(false);
            id
        }
    };

    // ── 3. Hydrate evolution state counters from session metadata ────
    // When a session is resumed, the in-memory counters start at 0.
    // Hydrate them from persisted metadata so the review cadence
    // continues where it left off. Matches hermes-agent's
    // _restore_memory_nudge_from_history pattern.
    //
    // The id is always assigned by now (step 2), so "has prior metadata"
    // is the only remaining condition — a first-turn session has none.
    if let Ok(metadata) = agent.database.get_all_session_metadata(&session_id)
        && !metadata.is_empty()
    {
        // In-process evolution_state lock; only held across a synchronous
        // hydrate_from_metadata call, never across await points, so a
        // poisoned guard is a programmer error.
        let mut evo = agent
            .evolution_state
            .lock()
            .expect("evolution_state lock poisoned");
        evo.hydrate_from_metadata(&metadata);
    }

    // ── 4. User message dedup check ──────────────────────────────────
    // Skip if the last message is already this exact query (happens
    // when run_with_healing retries run() — without this check, N
    // retries produce N duplicate user messages).
    // Deduplicate ONLY the retry immediately after a self-healing recovery
    // hint. A legitimate later turn with the same text (for example the
    // user says "continue" twice) must be a new user row and a new rewind
    // mark. The recovery hint is the exact marker between the original user
    // row and this retry.
    let already_added = {
        let conv = agent.conversation.read().await;
        match conv.last() {
            Some(last)
                if last.role == super::Role::System
                    && last.content.starts_with(super::HEALING_RETRY_HINT_PREFIX) =>
            {
                let prior_user = conv
                    .iter()
                    .rev()
                    .skip(1)
                    .find(|message| message.role == super::Role::User);
                prior_user.is_some_and(|message| message.content == effective_query)
            }
            _ => false,
        }
    };

    if !already_added {
        let conv_index = {
            let conv = agent.conversation.read().await;
            conv.len()
        };
        agent
            .add_message(Message::user(effective_query.clone()))
            .await;
        // (plan 05) Mark the real user prompt for the rewind selector.
        // Only this path records marks: nudge/steer/continue messages enter
        // through drain_steers mid-run and never pass build_turn_context,
        // so they are excluded structurally, not by content checks.
        let turn_id = agent.record_user_turn(effective_query.clone(), conv_index);
        agent
            .emit(crate::agent::AgentEvent::TurnStarted { turn_id })
            .await;
    }

    // ── 5. DB session + user message persistence ─────────────────────
    // Save session first (must exist before messages can reference it)
    let now = chrono::Utc::now().to_rfc3339();
    agent
        .database
        .save_session(&session_id, None, "agent", &now, &now)
        .map_err(|e| {
            warn!(error = %e, "Failed to save session metadata");
            e
        })?;
    // A turn in this session is the proof it is no longer interrupted: clear
    // the flag left by a previous process's exit. Without this every relaunch
    // would re-offer the same resume, which is the lie the flag exists to
    // avoid. Best-effort and deliberately AFTER save_session, so the row is
    // guaranteed to exist.
    if let Err(e) = agent.database.clear_session_interrupted(&session_id) {
        // Not fatal: a stale flag costs one extra resume notice, and failing a
        // turn over it would be a far worse trade.
        warn!(error = %e, "Failed to clear the interrupted-session flag");
    }

    // ── 5b. Record that THIS session is live RIGHT NOW (RT-511) ──────
    //
    // The interrupted-session records used to be written by ONE path —
    // the TUI's exit teardown — so a SIGKILL, a SIGTERM, a panic and
    // every gateway exit wrote NOTHING: the next launch offered no
    // resume notice, the session rendered as `Idle`, and
    // `operant sessions list --interrupted` printed "No interrupted
    // sessions" for a session that had been cut off mid-turn — the
    // question the feature exists to answer, answered wrongly.
    //
    // The turn START is the only moment guaranteed to run before the
    // work that can be interrupted, so that is where the records go:
    // the file marker (the one-generation resume offer) and the DB flag
    // (what the session list shows). The clear above must come FIRST —
    // it retires the previous process's record, and this one replaces
    // it.
    //
    // Both are best-effort: a turn must not fail over its own
    // bookkeeping. A record that could not be written costs one resume
    // offer; a turn that died over it costs the operator's work.
    let title = interrupted_title(&effective_query);
    if host_owned {
        let marker = crate::session_interrupted::SessionInterrupted::new(&session_id, &title);
        if let Err(e) = crate::session_interrupted::write_marker(&marker) {
            warn!(error = %e, "Failed to write the interrupted-session marker");
        }
    }
    if let Err(e) = agent.database.mark_session_interrupted(&session_id) {
        warn!(error = %e, "Failed to flag the session interrupted");
    }
    // The operator's employee tag: stamp the seat on the row (plan
    // 2026-10-28 iter-A). Personal mode leaves the column NULL. A separate
    // UPDATE rather than a wider `save_session` signature: three other
    // callers (llm_compressor, rewind forks' parent rows, tests) write
    // session metadata without a seat and keep the 5-arg shape.
    if let Some(seat) = agent.seat_id() {
        agent
            .database
            .set_session_employee(&session_id, &seat)
            .map_err(|e| {
                warn!(error = %e, "Failed to stamp session employee tag");
                e
            })?;
    }

    // Persist user message
    //
    // EXACTLY ONCE per user message. The dedup check above skips a
    // re-added query (the healing retry, the context-exhausted retry), and
    // this write used to run anyway — so a retried turn left the operator's
    // message in the transcript TWICE.
    if !already_added {
        agent
            .database
            .save_message(
                &session_id,
                "user",
                &effective_query,
                &chrono::Utc::now().to_rfc3339(),
            )
            .map_err(|e| {
                warn!(error = %e, "Failed to persist user message");
                e
            })?;
    }

    // ── 6. Message building (system prompt + preflight compression) ──
    // Pass the resolved session key so the context engine's DAG ingestion
    // uses the SAME key as the run loop's progressive/eager ingest (a
    // `"default"` fallback would duplicate every node under two session ids).
    let messages = agent.build_messages(&session_id).await?;

    debug!(
        session_id = %session_id,
        messages = messages.len(),
        user_query_len = effective_query.len(),
        already_added,
        "Turn context built"
    );

    Ok(TurnContext {
        session_id,
        already_added,
        messages,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_turn_context_struct_creation() {
        let ctx = TurnContext {
            session_id: "test-session".to_string(),
            already_added: false,
            messages: vec![Message::user("hello")],
        };
        assert_eq!(ctx.session_id, "test-session");
        assert!(!ctx.already_added);
        assert_eq!(ctx.messages.len(), 1);
    }

    #[test]
    fn test_turn_context_with_dedup() {
        let ctx = TurnContext {
            session_id: "test-session".to_string(),
            already_added: true,
            messages: vec![Message::user("hello")],
        };
        assert!(ctx.already_added);
    }

    #[test]
    fn test_turn_context_session_id_format() {
        let uuid_id = format!("sess_{}", uuid::Uuid::new_v4());
        assert!(uuid_id.starts_with("sess_"));
        assert!(uuid_id.len() > 5);

        let persistent_id = "my-persistent-session".to_string();
        assert!(!persistent_id.starts_with("sess_"));
    }

    #[test]
    fn test_turn_context_messages_preserve_order() {
        let messages = vec![
            Message::system("system prompt"),
            Message::user("hello"),
            Message::assistant("hi there"),
        ];
        let ctx = TurnContext {
            session_id: "test".to_string(),
            already_added: false,
            messages: messages.clone(),
        };
        assert_eq!(ctx.messages.len(), 3);
        use crate::client::Role;
        assert_eq!(ctx.messages[0].role, Role::System);
        assert_eq!(ctx.messages[1].role, Role::User);
        assert_eq!(ctx.messages[2].role, Role::Assistant);
    }

    #[test]
    fn test_preflight_constants_values() {
        use crate::agent::turn_finalizer::{
            PREFLIGHT_DECAY_CONSTANT, PREFLIGHT_DECAY_H50, PREFLIGHT_THRESHOLD_PERCENT,
        };
        assert_eq!(PREFLIGHT_THRESHOLD_PERCENT, 80);
        assert_eq!(PREFLIGHT_DECAY_H50, 100);
        assert!((PREFLIGHT_DECAY_CONSTANT - 20.0).abs() < f64::EPSILON);
    }

    // ── Session-namespace regression tests ─────────────────────────
    //
    // The live bug: `session_id` was minted per turn
    // (`unwrap_or_else(|| format!("sess_{}", uuid))`) unless a host had
    // set `persistent_session_id` at BUILD time. The Telegram gateway
    // never did — it derives its own stable `gw_<hash>` id, writes only
    // user/assistant text there, and reloads the last 20 rows of THAT id
    // on restart. So every turn's full tool trajectory landed in a
    // throwaway `sess_*` id that no reload would ever read, and a restart
    // rehydrated a text-only skeleton ("five prompts and no replies").
    //
    // These tests assert the post-fix property directly, through the same
    // `get_session_messages` query the gateway's reload path calls.

    mod session_namespace {
        use super::*;
        use crate::agent::clients::openai::OpenAIModelClient;
        use crate::agent::{AgentConfig, OperantAgent};
        use crate::client::OpenAIClient;
        use crate::database::Database;
        use crate::tools::ToolRegistry;
        use std::sync::Arc;
        use std::time::Duration;

        fn agent_with_db(db: &Arc<Database>) -> OperantAgent {
            OperantAgent::new(
                AgentConfig::default(),
                Box::new(OpenAIModelClient::new(OpenAIClient::new(
                    crate::client::ClientConfig::default(),
                ))),
                ToolRegistry::new(Duration::from_secs(1)),
                Arc::clone(db),
            )
        }

        /// (plan 05, step 1) every real user prompt gets exactly one mark,
        /// with the pre-add conversation index and the current epoch.
        #[test]
        fn user_turn_marks_record_one_per_prompt() {
            let db = crate::database::Database::init(
                std::env::temp_dir().join(format!("rewind_marks_{}.db", std::process::id())),
            )
            .expect("test db");
            let agent = agent_with_db(&Arc::new(db));
            assert!(agent.user_turns().is_empty(), "no marks before any turn");

            let id1 = agent.record_user_turn("first prompt".to_string(), 0);
            let id2 = agent.record_user_turn("second prompt".to_string(), 2);
            assert_ne!(id1, id2, "turn ids are unique");

            let turns = agent.user_turns();
            assert_eq!(turns.len(), 2);
            assert_eq!(turns[0].text, "first prompt");
            assert_eq!(turns[0].conv_index, 0);
            assert_eq!(turns[1].text, "second prompt");
            assert_eq!(turns[1].conv_index, 2);
            assert_eq!(turns[0].conv_epoch, agent.conv_epoch());
        }

        /// (plan 05, step 1) non-append mutations bump the epoch so stale
        /// marks know they need relocation by text.
        #[test]
        fn conv_epoch_bumps_on_non_append_mutations() {
            let db = crate::database::Database::init(
                std::env::temp_dir().join(format!("rewind_epoch_{}.db", std::process::id())),
            )
            .expect("test db");
            let agent = agent_with_db(&Arc::new(db));
            let before = agent.conv_epoch();
            agent.bump_conv_epoch();
            assert_eq!(agent.conv_epoch(), before + 1);
        }

        /// Two turns on one agent must land in ONE session, and that
        /// session must be the one the agent reports — so a host that
        /// reloads by `agent.session_id()` reads the full transcript.
        ///
        /// Before the fix this failed at the first assertion: two distinct
        /// `sess_<uuid>` ids.
        #[tokio::test]
        async fn two_turns_share_one_session_id() {
            let dir = tempfile::tempdir().expect("tempdir");
            let db = Arc::new(Database::init(dir.path().join("t.db")).expect("db init"));
            let agent = agent_with_db(&db);

            let first = build_turn_context(&agent, "first turn")
                .await
                .expect("turn 1");
            let second = build_turn_context(&agent, "second turn")
                .await
                .expect("turn 2");

            assert_eq!(
                first.session_id, second.session_id,
                "two turns on one agent must resolve to the SAME session id"
            );
            assert_eq!(
                agent.session_id().as_deref(),
                Some(first.session_id.as_str()),
                "the turn must not write to an id the agent does not report"
            );

            // What a restart reload reads: both turns, in order, under one id.
            let reloaded = db
                .get_session_messages(&first.session_id)
                .expect("reload query");
            let contents: Vec<&str> = reloaded.iter().map(|m| m.content.as_str()).collect();
            assert_eq!(contents, vec!["first turn", "second turn"]);
        }

        /// A legitimate repeated user turn is NOT a healing retry. The
        /// second "continue" must be a new user row and a new rewind mark.
        #[tokio::test]
        async fn a_repeated_user_turn_is_not_deduplicated() {
            let dir = tempfile::tempdir().expect("tempdir");
            let db =
                Arc::new(Database::init(dir.path().join("repeated-turn.db")).expect("db init"));
            let agent = agent_with_db(&db);

            let first = build_turn_context(&agent, "continue")
                .await
                .expect("first turn");
            assert!(!first.already_added);
            let second = build_turn_context(&agent, "continue")
                .await
                .expect("second turn");
            assert!(
                !second.already_added,
                "the same text in a later turn is a new user row"
            );

            let reloaded = db
                .get_session_messages(&first.session_id)
                .expect("reload query");
            assert_eq!(
                reloaded
                    .iter()
                    .filter(|message| message.role == "user")
                    .count(),
                2,
                "two intentional identical turns must persist twice"
            );
        }

        /// A healing retry adds a SYSTEM recovery hint after the original
        /// user row, then starts another turn with the same query. The
        /// original user row must remain the one persisted row: the retry
        /// may add the hint, never a second copy of the user's message.
        #[tokio::test]
        async fn a_healing_retry_does_not_persist_the_user_twice() {
            let dir = tempfile::tempdir().expect("tempdir");
            let db =
                Arc::new(Database::init(dir.path().join("healing-dedup.db")).expect("db init"));
            let agent = agent_with_db(&db);

            let first = build_turn_context(&agent, "same question")
                .await
                .expect("first turn");
            assert!(!first.already_added);
            agent
                .add_message(Message::system(
                    "Note: The previous attempt encountered an error: provider exploded. Please correct your approach and try again.",
                ))
                .await;
            let retry = build_turn_context(&agent, "same question")
                .await
                .expect("retry turn");
            assert!(
                retry.already_added,
                "the retry must recognize the existing user row across the recovery hint"
            );

            let reloaded = db
                .get_session_messages(&first.session_id)
                .expect("reload query");
            let user_rows = reloaded
                .iter()
                .filter(|message| message.role == "user")
                .count();
            assert_eq!(user_rows, 1, "the user message must persist exactly once");
        }

        /// The gateway seam: a host that owns session identity assigns its
        /// own stable id at runtime, and the turn's rows land THERE — in
        /// the namespace its reload query reads.
        ///
        /// Before the fix there was no runtime setter at all, and the
        /// gateway could only reach `with_persistent_session` at agent
        /// construction, which it never controls per chat.
        #[tokio::test]
        async fn host_assigned_session_id_receives_the_turn() {
            // (RT-511) This turn is host-assigned, so it writes the
            // interrupted-session marker. The home is therefore redirected
            // to the test's own temp dir under the crate's environment
            // lock: a test's marker must never land in the operator's real
            // home, and must never race another test's marker.
            let _env = crate::platform::test_env_lock().await;
            let dir = tempfile::tempdir().expect("tempdir");
            let previous_home = std::env::var("HERMES_HOME").ok();
            // SAFETY: serialised by the crate-wide environment lock.
            unsafe { std::env::set_var("HERMES_HOME", dir.path()) };
            let db = Arc::new(Database::init(dir.path().join("t.db")).expect("db init"));
            let agent = agent_with_db(&db);

            // The id the gateway derives at gateway_runner.rs:436.
            let gateway_id = "gw_1a2b3c4d5e6f";
            db.save_session(
                gateway_id,
                None,
                "gateway",
                "2026-01-01T00:00:00+00:00",
                "2026-01-01T00:00:00+00:00",
            )
            .expect("gateway session");
            agent.set_session_id(gateway_id);

            let ctx = build_turn_context(&agent, "hello telegram")
                .await
                .expect("turn");

            assert_eq!(ctx.session_id, gateway_id, "host-owned id must win");
            let reloaded = db.get_session_messages(gateway_id).expect("reload query");
            assert_eq!(reloaded.len(), 1);
            assert_eq!(reloaded[0].content, "hello telegram");

            // A second turn in the same chat keeps the same namespace.
            let next = build_turn_context(&agent, "and again")
                .await
                .expect("turn 2");
            assert_eq!(next.session_id, gateway_id);
            assert_eq!(
                db.get_session_messages(gateway_id).expect("reload").len(),
                2
            );

            // The build-time setter feeds the SAME slot — no second
            // namespace behind it (the WebSocket/TUI path depends on it).
            let ws = agent_with_db(&db).with_persistent_session("gw_ws".to_string());
            assert_eq!(ws.session_id().as_deref(), Some("gw_ws"));
            let ws_ctx = build_turn_context(&ws, "ws turn").await.expect("ws turn");
            assert_eq!(ws_ctx.session_id, "gw_ws");

            match previous_home {
                Some(v) => unsafe { std::env::set_var("HERMES_HOME", v) },
                None => unsafe { std::env::remove_var("HERMES_HOME") },
            }
        }

        /// RT-511: a turn START records the session as live, and a CLEAN
        /// end takes the record back.
        ///
        /// The records used to be written by ONE path — the TUI's exit
        /// teardown — so a SIGKILL, a SIGTERM, a panic and every gateway
        /// exit wrote NOTHING: the next launch offered no resume notice,
        /// the session rendered as `Idle`, and `operant sessions list
        /// --interrupted` printed "No interrupted sessions" for a session
        /// that had been cut off mid-turn — the question the feature
        /// exists to answer, answered wrongly.
        ///
        /// The marker file lives in the operant home, so the home is
        /// redirected for the test's duration under the crate's
        /// environment lock (the RT-99 pattern: prove the sandbox before
        /// writing into it).
        #[tokio::test]
        async fn a_turn_start_records_the_session_as_live_and_a_clean_end_takes_it_back() {
            let _env = crate::platform::test_env_lock().await;
            let home =
                std::env::temp_dir().join(format!("operant-turn-marker-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&home);
            std::fs::create_dir_all(&home).expect("temp home");
            let previous = std::env::var("HERMES_HOME").ok();
            // SAFETY: serialised by the crate-wide environment lock, and
            // restored before every assertion below.
            unsafe { std::env::set_var("HERMES_HOME", &home) };
            assert_eq!(
                crate::platform::operant_home(),
                home,
                "the marker must be written into the TEST's home, never the operator's"
            );

            let db = Arc::new(Database::init(home.join("test.db")).expect("test db"));
            // A HOST-owned session: only those have a boot path that can
            // offer the resume back, which is what the file marker is for.
            let agent = agent_with_db(&db).with_persistent_session("gw_marker".to_string());

            // The turn prologue saves the session row BEFORE it flags it,
            // so one turn is enough to leave both records behind.
            build_turn_context(&agent, "do the thing")
                .await
                .expect("turn");

            // The file marker: the one-generation resume offer.
            let marker = crate::session_interrupted::read_marker()
                .expect("the turn start must write the marker");
            assert_eq!(marker.session_id, "gw_marker");
            assert!(
                marker.title.contains("do the thing"),
                "the marker title must be the operator's prompt, got: {:?}",
                marker.title
            );

            // The DB flag: what `operant sessions list --interrupted` reads.
            assert!(
                db.interrupted_session_ids()
                    .expect("flag query")
                    .contains(&"gw_marker".to_string()),
                "the session must be flagged interrupted while the turn is live"
            );

            // A CLEAN end takes both back.
            clear_interrupted_records(&agent, "gw_marker").await;

            assert!(
                crate::session_interrupted::read_marker().is_none(),
                "a clean turn must not leave a resume offer behind"
            );
            assert!(
                !db.interrupted_session_ids()
                    .expect("flag query")
                    .contains(&"gw_marker".to_string()),
                "a clean turn must not leave the session flagged interrupted"
            );

            match previous {
                Some(v) => unsafe { std::env::set_var("HERMES_HOME", v) },
                None => unsafe { std::env::remove_var("HERMES_HOME") },
            }
        }
    }
}
