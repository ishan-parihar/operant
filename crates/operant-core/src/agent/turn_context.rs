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

#[expect(
    clippy::expect_used,
    reason = "poisoned lock: panic is the intended recovery"
)]
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
pub async fn build_turn_context(agent: &OperantAgent, user_query: &str) -> Result<TurnContext> {
    // ── 0. @-reference expansion ─────────────────────────────────────
    // Expand `@file:`, `@folder:`, `@git:…`, `@url:…` tokens the user typed
    // BEFORE the message enters the conversation, so every surface (CLI, TUI,
    // gateway, cron, autonomous, sub-agents) gets identical expansion. Mirrors
    // hermes agent/context_references.py (R1). Warnings are surfaced to the
    // user via the AgentEvent channel below.
    let context_window = agent.config.context_window;
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
    let session_id = match agent.session_id() {
        Some(id) => id,
        None => {
            let id = format!("sess_{}", uuid::Uuid::new_v4());
            agent.set_session_id(id.clone());
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
    let already_added = {
        let conv = agent.conversation.read().await;
        conv.last()
            .is_some_and(|last| last.role == super::Role::User && last.content == effective_query)
    };

    if !already_added {
        agent
            .add_message(Message::user(effective_query.clone()))
            .await;
    }

    // ── 5. DB session + user message persistence ─────────────────────
    // Save session first (must exist before messages can reference it)
    agent
        .database
        .save_session(
            &session_id,
            None,
            "agent",
            &chrono::Utc::now().to_rfc3339(),
            &chrono::Utc::now().to_rfc3339(),
        )
        .map_err(|e| {
            warn!(error = %e, "Failed to save session metadata");
            e
        })?;

    // Persist user message
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

        /// The gateway seam: a host that owns session identity assigns its
        /// own stable id at runtime, and the turn's rows land THERE — in
        /// the namespace its reload query reads.
        ///
        /// Before the fix there was no runtime setter at all, and the
        /// gateway could only reach `with_persistent_session` at agent
        /// construction, which it never controls per chat.
        #[tokio::test]
        async fn host_assigned_session_id_receives_the_turn() {
            let dir = tempfile::tempdir().expect("tempdir");
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
        }
    }
}
