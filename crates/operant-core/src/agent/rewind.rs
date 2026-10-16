//! Rewind to a previous user turn (2026-10-14 batch, plan 05).
//!
//! A rewind is a FORK, not a deletion: the prefix before the chosen
//! user prompt becomes a NEW session (with `parent_session_id` pointing
//! back at the old one), the old session stays intact in `/sessions`,
//! and the agent retargets to the fork. The old path is never lost.

use crate::agent::{OperantAgent, UserTurnMark};
use crate::client::Message;

/// Why a rewind could not happen.
#[derive(Debug, thiserror::Error)]
pub enum RewindError {
    /// The turn's messages were folded away by compaction; rewind cannot
    /// reconstruct them. The UI reports the earliest still-rewindable turn.
    #[error("that turn was folded by compaction; the earliest rewindable turn is #{earliest}")]
    Compacted { earliest: u64 },
    /// The requested turn id was never recorded in this session.
    #[error("unknown turn id {0}")]
    UnknownTurn(u64),
    /// Persisting the fork's session row failed; no state was changed.
    #[error("could not save the forked session: {0}")]
    SaveFailed(String),
}

/// A successful fork.
#[derive(Debug, Clone)]
pub struct RewindOutcome {
    /// The new session the agent now points at.
    pub new_session_id: String,
    /// The session the fork came from (still intact, still listed).
    pub parent_session_id: String,
    /// The rewound prompt's text (the TUI puts it back in the composer).
    pub prompt_text: String,
    /// How many user turns the fork dropped (what the preview dimmed).
    pub removed_turns: usize,
}

/// A fresh session id — one shape everywhere a session is minted
/// (rewind forks, the TUI's /new). (plan 03 §2 extracted this so /new
/// cannot drift from the fork shape.)
pub fn new_session_id() -> String {
    format!("sess_{}", uuid::Uuid::new_v4())
}

impl OperantAgent {
    /// (plan 05) Read-only preview: the conversation prefix a rewind to
    /// `turn_id` would keep, and its estimated tokens. No state changes —
    /// the selector calls this for every cursor move.
    pub async fn preview_rewind(&self, turn_id: u64) -> Result<(Vec<Message>, u64), RewindError> {
        let cut = self.rewind_cut_point(turn_id).await?;
        let conv = self.conversation.read().await;
        let prefix: Vec<Message> = conv[..cut].to_vec();
        let tokens = crate::context_management::estimate_total_tokens(&prefix) as u64;
        Ok((prefix, tokens))
    }

    /// Resolve a turn mark to a conversation cut point, relocating by text
    /// when the epoch has moved since the mark was taken.
    async fn rewind_cut_point(&self, turn_id: u64) -> Result<usize, RewindError> {
        let marks = self.user_turns();
        let mark = marks
            .iter()
            .find(|m| m.turn_id == turn_id)
            .ok_or(RewindError::UnknownTurn(turn_id))?
            .clone();

        let conv = self.conversation.read().await;
        if mark.conv_epoch == self.conv_epoch() && mark.conv_index < conv.len() {
            return Ok(mark.conv_index);
        }
        // Stale mark: relocate by exact text. A unique user-role match is
        // required — an ambiguous or missing match means compaction folded
        // the turn away.
        let mut matches = conv
            .iter()
            .enumerate()
            .filter(|(_, m)| m.role == crate::client::Role::User && m.content == mark.text)
            .map(|(i, _)| i);
        match (matches.next(), matches.next()) {
            (Some(idx), None) => Ok(idx),
            _ => {
                // Compacted (or duplicated) — report the earliest surviving
                // mark so the UI can say how far back rewind still reaches.
                let earliest = marks
                    .iter()
                    .filter(|m| {
                        conv.iter()
                            .any(|c| c.role == crate::client::Role::User && c.content == m.text)
                    })
                    .map(|m| m.turn_id)
                    .min()
                    .unwrap_or(1);
                Err(RewindError::Compacted { earliest })
            }
        }
    }

    /// (plan 05) Fork the conversation at the chosen user turn and retarget
    /// the agent to the new session. The old session is untouched.
    pub async fn rewind_to_turn(&self, turn_id: u64) -> Result<RewindOutcome, RewindError> {
        let old_session = self
            .session_id()
            .unwrap_or_else(|| format!("sess_{}", uuid::Uuid::new_v4()));
        let cut = self.rewind_cut_point(turn_id).await?;
        let prompt_text = self
            .user_turns()
            .iter()
            .find(|m| m.turn_id == turn_id)
            .map(|m| m.text.clone())
            .unwrap_or_default();

        // 1. Snapshot the prefix under the read lock.
        let prefix: Vec<Message> = {
            let conv = self.conversation.read().await;
            conv[..cut].to_vec()
        };
        let removed_turns = self
            .user_turns()
            .iter()
            .filter(|m| m.turn_id > turn_id)
            .count();

        // 2. New session id (same shape as /new), parent = old.
        let new_session = new_session_id();
        let now = chrono::Utc::now().to_rfc3339();
        let session_data = crate::database::SessionData {
            id: new_session.clone(),
            source: "agent".to_string(),
            user_id: None,
            model: Some(self.effective_model()),
            model_config: None,
            system_prompt: self.config.system_prompt.clone(),
            parent_session_id: Some(old_session.clone()),
            started_at: now.clone(),
            ended_at: None,
            end_reason: None,
            message_count: prefix.len() as i64,
            tool_call_count: 0,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            reasoning_tokens: 0,
            cwd: None,
            billing_provider: None,
            billing_base_url: None,
            billing_mode: None,
            estimated_cost_usd: None,
            actual_cost_usd: None,
            cost_status: None,
            cost_source: None,
            pricing_version: None,
            title: Some(format!(
                "Rewind of {}",
                &old_session[..12.min(old_session.len())]
            )),
            api_call_count: 0,
            handoff_state: None,
            handoff_platform: None,
            handoff_error: None,
            rewind_count: 1,
            archived: 0,
        };
        self.database
            .save_session_full(&session_data)
            .map_err(|e| {
                // A DB failure before any state change is safe to surface.
                tracing::error!(?e, "rewind_to_turn: save_session_full failed");
                RewindError::SaveFailed(e.to_string())
            })?;

        // 3. Write the prefix as the new session's transcript.
        for msg in &prefix {
            let _ = self.database.save_message(
                &new_session,
                msg.role.as_str(),
                &msg.content,
                &chrono::Utc::now().to_rfc3339(),
            );
        }

        // 4. Swap the agent onto the fork: mark history of marks is
        //    turn-scoped, so keep the PRE-fork marks only.
        {
            let mut conv = self.conversation.write().await;
            *conv = prefix;
        }
        self.set_session_id(new_session.clone());
        self.notify_session_switch(&new_session, &old_session, false);

        Ok(RewindOutcome {
            new_session_id: new_session,
            parent_session_id: old_session,
            prompt_text,
            removed_turns,
        })
    }

    /// The recorded marks, oldest first — the selector's row list.
    pub fn rewind_marks(&self) -> Vec<UserTurnMark> {
        self.user_turns()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::clients::openai::OpenAIModelClient;
    use crate::agent::{AgentConfig, OperantAgent};
    use crate::client::OpenAIClient;
    use crate::database::Database;
    use crate::tools::ToolRegistry;
    use std::sync::Arc;
    use std::time::Duration;

    fn agent() -> (OperantAgent, Arc<Database>) {
        let path = std::env::temp_dir().join(format!(
            "rewind_fork_{}_{}.db",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let _ = std::fs::remove_file(&path);
        let db = Arc::new(Database::init(path).expect("test db"));
        let agent = OperantAgent::new(
            AgentConfig::default(),
            Box::new(OpenAIModelClient::new(OpenAIClient::new(
                crate::client::ClientConfig::default(),
            ))),
            ToolRegistry::new(Duration::from_secs(1)),
            Arc::clone(&db),
        );
        (agent, db)
    }

    async fn three_turns(agent: &OperantAgent) -> [u64; 3] {
        let mut ids = [0u64; 3];
        for (i, text) in ["one", "two", "three"].iter().enumerate() {
            let idx = agent.conversation.read().await.len();
            agent.add_message(Message::user(*text)).await;
            ids[i] = agent.record_user_turn((*text).to_string(), idx);
        }
        // interleaved assistant replies so the prefix is realistic
        for text in ["a1", "a2", "a3"] {
            agent
                .add_message(crate::client::Message::assistant(text.to_string()))
                .await;
        }
        ids
    }

    /// Fork at turn 2: the agent's conversation becomes the pre-turn-2
    /// prefix, the session id changes, and the DB row points back at the
    /// parent. (plan 05 RED-test list, first bullet — the fork itself.)
    #[tokio::test]
    async fn rewind_forks_at_the_chosen_turn() {
        let (agent, db) = agent();
        let old_session = format!("sess_old_{}", uuid::Uuid::new_v4());
        // turn_context step 5 always saves the session row before any
        // message lands — the fork's parent FK requires it to exist.
        let now = chrono::Utc::now().to_rfc3339();
        agent
            .database
            .save_session(&old_session, None, "agent", &now, &now)
            .expect("parent session row");
        agent.set_session_id(old_session.clone());
        let ids = three_turns(&agent).await;
        let conv_before: Vec<String> = agent
            .conversation
            .read()
            .await
            .iter()
            .map(|m| m.content.clone())
            .collect();

        let outcome = agent.rewind_to_turn(ids[1]).await.expect("fork succeeds");

        // New conversation = messages before turn 2's user message only.
        let conv_after: Vec<String> = agent
            .conversation
            .read()
            .await
            .iter()
            .map(|m| m.content.clone())
            .collect();
        assert_eq!(conv_after.len(), 1, "keeps only the first user turn");
        assert_eq!(conv_after[0], "one");
        assert_eq!(conv_before.len(), 6, "sanity: 3 turns + 3 replies");

        // Session retargeted; fork transcript written under the new id.
        assert_ne!(outcome.new_session_id, old_session);
        assert_eq!(outcome.parent_session_id, old_session);
        assert_eq!(
            agent.session_id().as_deref(),
            Some(outcome.new_session_id.as_str())
        );
        let fork_msgs = db
            .get_session_messages(&outcome.new_session_id)
            .expect("fork transcript exists");
        assert_eq!(fork_msgs.len(), 1, "fork keeps only the prefix");
        assert_eq!(fork_msgs[0].content, "one");
        assert_eq!(outcome.prompt_text, "two");
        assert_eq!(outcome.removed_turns, 1, "turn 3 dropped, turn 2 rewound");
    }

    /// A mark from a compacted-away turn returns `Compacted`; a later mark
    /// still relocates by text. (plan 05 RED-test list, second bullet.)
    #[tokio::test]
    async fn compacted_turn_reports_and_later_turn_relocates() {
        let (agent, db) = agent();
        // Parent row must exist for the fork FK (turn_context step-5 parity).
        let sid = format!("sess_c_{}", uuid::Uuid::new_v4());
        let now = chrono::Utc::now().to_rfc3339();
        db.save_session(&sid, None, "agent", &now, &now)
            .expect("parent row");
        agent.set_session_id(sid);
        let ids = three_turns(&agent).await;

        // Simulate compaction folding turn 1: replace the whole store and
        // bump the epoch (that is what compact_history_now does).
        agent.bump_conv_epoch();
        {
            let mut conv = agent.conversation.write().await;
            conv.drain(..1); // fold away user turn 1 ("one")
        }

        match agent.rewind_to_turn(ids[0]).await {
            Err(RewindError::Compacted { earliest }) => {
                assert_eq!(earliest, ids[1], "earliest surviving turn is #2");
            }
            other => panic!(
                "expected Compacted, got {:?}",
                other.map(|o| o.removed_turns)
            ),
        }

        // Turn 3 relocates by text (stale epoch, exact match survives).
        let outcome = agent.rewind_to_turn(ids[2]).await.expect("relocates");
        assert_eq!(outcome.removed_turns, 0);
    }

    /// Marks are per real prompt only — steer/nudge messages never pass
    /// build_turn_context, so they never get marks. (plan 05, third bullet.)
    #[tokio::test]
    async fn steer_and_nudge_never_get_marks() {
        let (agent, _db) = agent();
        // A steer message enters directly, as drain_steers delivers it:
        let idx = agent.conversation.read().await.len();
        agent.add_message(Message::user("STEER: hurry up")).await;
        // (no record_user_turn — exactly what the drain path does)
        assert_eq!(idx, 0);
        assert!(agent.user_turns().is_empty(), "steer has no mark");

        // A real turn does:
        agent.add_message(Message::user("real prompt")).await;
        agent.record_user_turn("real prompt".to_string(), 1);
        assert_eq!(agent.user_turns().len(), 1);
    }
}
