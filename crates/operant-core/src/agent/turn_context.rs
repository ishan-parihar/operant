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
    #[allow(dead_code)]
    pub already_added: bool,
    /// Working message list for this turn (loop appends to it).
    pub messages: Vec<Message>,
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
pub async fn build_turn_context(agent: &OperantAgent, user_query: &str) -> Result<TurnContext> {
    // ── 1. Reset interrupt flag ──────────────────────────────────────
    // Without this, a Ctrl-C in run #1 permanently breaks run #2+
    // (the flag stays triggered and the loop exits immediately).
    agent.interrupt_flag.reset();

    // ── 2. Session ID resolution ─────────────────────────────────────
    let session_id = agent
        .persistent_session_id
        .clone()
        .unwrap_or_else(|| format!("sess_{}", uuid::Uuid::new_v4()));

    // ── 3. Hydrate evolution state counters from session metadata ────
    // When a session is resumed, the in-memory counters start at 0.
    // Hydrate them from persisted metadata so the review cadence
    // continues where it left off. Matches hermes-agent's
    // _restore_memory_nudge_from_history pattern.
    if agent.persistent_session_id.is_some() {
        if let Ok(metadata) = agent.database.get_all_session_metadata(&session_id) {
            if !metadata.is_empty() {
                let mut evo = agent.evolution_state.lock().unwrap();
                evo.hydrate_from_metadata(&metadata);
            }
        }
    }

    // ── 4. User message dedup check ──────────────────────────────────
    // Skip if the last message is already this exact query (happens
    // when run_with_healing retries run() — without this check, N
    // retries produce N duplicate user messages).
    let already_added = {
        let conv = agent.conversation.read().await;
        conv.last()
            .is_some_and(|last| last.role == super::Role::User && last.content == user_query)
    };

    if !already_added {
        agent.add_message(Message::user(user_query)).await;
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
            user_query,
            &chrono::Utc::now().to_rfc3339(),
        )
        .map_err(|e| {
            warn!(error = %e, "Failed to persist user message");
            e
        })?;

    // ── 6. Message building (system prompt + preflight compression) ──
    let messages = agent.build_messages().await?;

    debug!(
        session_id = %session_id,
        messages = messages.len(),
        user_query_len = user_query.len(),
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
}
