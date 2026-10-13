//! Idle recap — "where the session stands", written while the user was away.
//!
//! omp parity (`recap.*`, `recap-user.md`, `RecapNotice`): after a completed
//! turn, when no further activity arrives for `recap_idle_seconds`, one
//! ephemeral side-channel LLM call renders a ~40-word status line over a
//! SNAPSHOT of the transcript. The live agent context is never touched —
//! the snapshot is built through `crate::moa::build_advisory_messages`
//! (tool calls folded to text, head+tail previews, always ends on a user
//! turn), which is exactly the strict-provider-safe disposable copy the MoA
//! reference models already receive.
//!
//! Cancellation: any activity (a new turn start, a steer, a user message)
//! arms a fresh timer; the recap only fires on a quiet window. A recap that
//! is mid-flight when activity lands is still delivered — the snapshot it
//! read is a consistent prefix of the session, never a torn one — but its
//! `session_id` lets the host drop it if the session has already moved on.
//!
//! Journaling: recaps are journaled per-session via the host callback so the
//! TUI transcript and gateway history both treat a recap as a first-class
//! line (a dim `※ recap:` notice), not model-visible context.

use crate::agent::{ChatRequest, ModelClient};
use crate::client::Message;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

/// omp's recap prompt (`recap-user.md`), verbatim contract: 40 words,
/// goal → task → next action, no markdown, no root-cause narrative.
const RECAP_INSTRUCTION: &str = "User stepped away; returning. Recap: <40 words, 1-2 plain \
sentences, no markdown. Lead: overall goal, current task; then one next action. Skip: \
root-cause narrative, fix internals, secondary to-dos, em-dash tangents.";

/// One generated recap, ready for the host to render/journal.
#[derive(Debug, Clone)]
pub struct Recap {
    /// Session the recap describes (from the `TurnEnd` that armed the timer).
    pub session_id: String,
    /// The recap text, already trimmed to a single line.
    pub text: String,
}

/// Handle for the host to signal user activity (re-arm/cancel the timer).
#[derive(Clone)]
pub struct RecapActivity {
    tx: mpsc::Sender<()>,
}

impl RecapActivity {
    /// Signal activity: cancels any pending recap and re-arms the idle
    /// timer. Cheap (one try_send); lossy is fine — activity is a flood
    /// signal, not a queue.
    pub fn notify(&self) {
        let _ = self.tx.try_send(());
    }
}

/// Spawn the idle-recap engine. Call once per agent session; only call when
/// `recap_idle_seconds > 0` (the host's gate).
///
/// * `client` — model client for the side-channel call (the same client the
///   agent uses; the call is non-streaming and carries no tools).
/// * `model` — model name for the recap call (the session's model).
/// * `idle` — quiet window before the recap fires.
/// * `turn_ends` — subscription to the agent's `TurnEndBus`.
/// * `snapshot` — host callback returning the current transcript snapshot
///   (a read lock on the agent's conversation). Called only when the timer
///   fires, never on every turn.
/// * `deliver` — host callback receiving the finished recap (TUI notice,
///   gateway send, journal).
///
/// Returns the activity handle. Dropping it stops the engine; the task
/// also exits when the bus subscription lags (agent dropped).
pub fn spawn_recap_engine(
    client: Arc<dyn ModelClient>,
    model: String,
    idle: Duration,
    mut turn_ends: tokio::sync::broadcast::Receiver<crate::turn_end::TurnEnd>,
    snapshot: impl FnMut() -> Vec<Message> + Send + 'static,
    deliver: impl FnMut(Recap) + Send + 'static,
) -> RecapActivity {
    let (act_tx, mut act_rx) = mpsc::channel::<()>(8);
    let mut snapshot = snapshot;
    let mut deliver = deliver;
    tokio::spawn(async move {
        loop {
            // Wait for a turn to end, or for the agent to go away.
            let end = match turn_ends.recv().await {
                Ok(e) => e,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break, // channel closed: agent dropped, engine exits
            };
            let session_id = end.session_id.clone();
            // Drain stale activity before arming: a notify that queued while
            // the turn was RUNNING (e.g. the gateway's turn-start cancel for
            // this very turn) must not cancel the window that opens at this
            // turn's end. Only activity arriving AFTER the arm cancels.
            // (Observed live: turn-start notify queued at 18:40:53, engine
            // armed at 18:40:56, buffered notify cancelled the window 9ms
            // in — the recap never fired once across three E2E windows.)
            while act_rx.try_recv().is_ok() {}
            // Idle window: fire only if no activity lands inside it.
            let quiet = tokio::time::sleep(idle);
            tokio::select! {
                () = quiet => {}
                Some(()) = act_rx.recv() => {
                    tracing::debug!(session = %session_id, "recap cancelled by activity");
                    continue; // activity: re-arm on next turn end
                }
                else => continue,
            }
            // Snapshot the transcript now — consistent prefix, read once.
            let messages = snapshot();
            if messages.is_empty() {
                continue;
            }
            let mut view = crate::moa::build_advisory_messages(&messages);
            view.push(Message::user(RECAP_INSTRUCTION));
            let request = ChatRequest::new(model.clone(), view).with_stream(false);
            let reply = match client.chat(request).await {
                Ok(r) => r,
                Err(e) => {
                    // Side turn: degrade to silence, never block the session —
                    // but say WHY (route flakes are the top silent-kill cause
                    // for the recap chain; without this line the E2E symptom
                    // is just "nothing arrived").
                    tracing::warn!(error = %e, session = %session_id, "recap side-turn call failed");
                    continue;
                }
            };
            let text = reply
                .choices
                .iter()
                .find_map(|c| c.message.content.clone())
                .unwrap_or_default();
            let text = text.trim().to_string();
            if text.is_empty() {
                continue;
            }
            tracing::debug!(session = %session_id, "recap delivered to host callback");
            deliver(Recap { session_id, text });
        }
    });
    RecapActivity { tx: act_tx }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::StreamChunk;
    use crate::error::{Error, Result};
    use async_trait::async_trait;
    use futures::stream::BoxStream;
    use parking_lot::Mutex;

    /// A client whose chat() always returns the same single-line reply.
    struct FixedClient(&'static str);

    #[async_trait]
    impl ModelClient for FixedClient {
        fn provider_name(&self) -> &str {
            "fixed"
        }
        async fn chat(&self, _req: ChatRequest) -> Result<crate::client::ChatResponse> {
            let payload = serde_json::json!({
                "id": "chatcmpl-fixed",
                "object": "chat.completion",
                "created": 0,
                "model": "fixed",
                "choices": [{"message": {"role": "assistant", "content": self.0}}],
                "usage": {"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0}
            });
            let resp: crate::client::ChatResponse = serde_json::from_value(payload)
                .map_err(|e| Error::ParseResponse(format!("{e}")))?;
            Ok(resp)
        }

        async fn chat_streaming(
            &self,
            _req: ChatRequest,
        ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
            Err(Error::Agent("streaming not mocked".into()))
        }
    }

    /// End-to-end: turn ends → quiet window → snapshot → side call → deliver.
    /// Uses a 10ms idle so the test does not sleep the real 240s.
    #[tokio::test(flavor = "multi_thread")]
    async fn recap_fires_after_idle_window() {
        let bus = crate::turn_end::TurnEndBus::new();
        let rx = bus.subscribe();
        let delivered = Arc::new(Mutex::new(Vec::<Recap>::new()));
        let d2 = delivered.clone();
        let engine = spawn_recap_engine(
            Arc::new(FixedClient(
                "goal: testing; task: recap engine; next: verify",
            )),
            "m".to_string(),
            Duration::from_millis(10),
            rx,
            || vec![Message::user("do the thing")],
            move |r| d2.lock().push(r),
        );
        bus.emit("sess-1", 3, 5, "partial result text");
        // The engine needs a moment to run its side call.
        for _ in 0..50 {
            if !delivered.lock().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let got = delivered.lock();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].session_id, "sess-1");
        assert!(got[0].text.contains("recap engine"));
        drop(engine);
    }

    /// Activity inside the idle window cancels the recap: nothing delivered.
    #[tokio::test(flavor = "multi_thread")]
    async fn activity_cancels_the_pending_recap() {
        let bus = crate::turn_end::TurnEndBus::new();
        let rx = bus.subscribe();
        let delivered = Arc::new(Mutex::new(Vec::<Recap>::new()));
        let d2 = delivered.clone();
        let engine = spawn_recap_engine(
            Arc::new(FixedClient("never delivered")),
            "m".to_string(),
            Duration::from_millis(200),
            rx,
            || vec![Message::user("x")],
            move |r| d2.lock().push(r),
        );
        bus.emit("sess-2", 1, 0, "done");
        // Let the engine arm its 200ms window first — a notify that lands
        // BEFORE the arm is a stale pre-turn notification, drained by the
        // engine (see stale_notify_does_not_cancel). Only activity arriving
        // INSIDE the window cancels.
        tokio::time::sleep(Duration::from_millis(50)).await;
        engine.notify();
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert!(delivered.lock().is_empty(), "activity must cancel");
        drop(engine);
    }

    /// A notification that queued while the turn was still RUNNING (the
    /// gateway notifies at turn START — its cancel contract targets the
    /// PREVIOUS window, never the one this turn's end opens) must not
    /// cancel the fresh window. Observed live 2026-10-13: turn-start
    /// notify queued, engine armed 3s later, buffered notify cancelled the
    /// recap 9ms in — the recap never fired across three E2E windows.
    #[tokio::test(flavor = "multi_thread")]
    async fn stale_notify_does_not_cancel() {
        let bus = crate::turn_end::TurnEndBus::new();
        let rx = bus.subscribe();
        let delivered = Arc::new(Mutex::new(Vec::<Recap>::new()));
        let d2 = delivered.clone();
        let engine = spawn_recap_engine(
            Arc::new(FixedClient("goal: g; task: t; next: n")),
            "m".to_string(),
            Duration::from_millis(100),
            rx,
            || vec![Message::user("x")],
            move |r| d2.lock().push(r),
        );
        // Notify BEFORE the turn ends — the queued activity is stale by the
        // time the engine arms at the turn end below.
        engine.notify();
        bus.emit("sess-3", 1, 0, "done");
        tokio::time::sleep(Duration::from_millis(300)).await;
        let got = delivered.lock();
        assert_eq!(got.len(), 1, "stale pre-turn notify must NOT cancel");
        drop(got);
        drop(engine);
    }

    /// The snapshot view must be the advisory render (tool calls folded),
    /// end on the recap instruction, and never leak the raw tool-role
    /// messages — asserted by construction: build_advisory_messages is the
    /// same function MoA tests cover; here we only pin the engine's contract
    /// that an empty session produces no side call.
    #[tokio::test(flavor = "multi_thread")]
    async fn empty_session_skips_the_side_call() {
        let bus = crate::turn_end::TurnEndBus::new();
        let rx = bus.subscribe();
        let delivered = Arc::new(Mutex::new(Vec::<Recap>::new()));
        let d2 = delivered.clone();
        let engine = spawn_recap_engine(
            Arc::new(FixedClient("should not be called")),
            "m".to_string(),
            Duration::from_millis(10),
            rx,
            Vec::new, // empty conversation
            move |r| d2.lock().push(r),
        );
        bus.emit("sess-3", 0, 0, "");
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(delivered.lock().is_empty());
        drop(engine);
    }
}
