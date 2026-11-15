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

/// The recap prompt for one call: the built-in contract, plus whatever the
/// operator named in `[recap] instructions`.
///
/// Kept as a function rather than an inline `format!` so the default path is
/// byte-identical to the pre-knob behaviour — an empty instruction returns
/// the constant unchanged, not the constant with a trailing separator.
/// Token ceiling on the transcript snapshot one recap call may carry.
///
/// The recap is a ~40-word status line over a disposable copy of the
/// transcript, so it does not need the whole session — and a session longer
/// than the model's window made every recap fail rather than degrade. 8,000
/// tokens is far more than a 40-word summary can read, and matches the
/// disposable-copy budget the MoA reference models already get.
const RECAP_VIEW_TOKEN_BUDGET: usize = 8_000;

/// Cut a built advisory view to the NEWEST messages that fit the budget.
///
/// The view arrives oldest-first, so the tail is kept and the head dropped —
/// a recap is about where the session stands, and that is the recent turns.
/// The cut itself is the shared `fit_messages_to_token_budget`, so this names
/// only the recap's own ceiling: the policy ("keep the newest that fit") is one
/// policy and now lives in one place, which is what the truncator audit asked
/// for. A view that already fits comes back unchanged, byte for byte.
fn fit_recap_view(view: Vec<Message>) -> Vec<Message> {
    crate::context_management::fit_messages_to_token_budget(view, RECAP_VIEW_TOKEN_BUDGET)
}

fn recap_instruction(instructions: &str) -> String {
    let extra = instructions.trim();
    if extra.is_empty() {
        RECAP_INSTRUCTION.to_string()
    } else {
        format!("{RECAP_INSTRUCTION}\n\nAdditional instructions from the operator:\n{extra}")
    }
}

/// One generated recap, ready for the host to render/journal.
#[derive(Debug, Clone)]
pub struct Recap {
    /// Session the recap describes (from the `TurnEnd` that armed the timer).
    pub session_id: String,
    /// The recap text, already trimmed to a single line.
    pub text: String,
}

/// Handle for the host to signal user activity (re-arm/cancel the timer).
///
/// `None` is the DISABLED state: no engine was spawned, so there is no
/// receiver and `notify` is a no-op. An `Option` rather than a channel whose
/// receiver is dropped on purpose — the off state exists to cost nothing, and
/// allocating a channel to immediately break it was a lie about that cost.
#[derive(Clone)]
pub struct RecapActivity {
    tx: Option<mpsc::Sender<()>>,
}

impl RecapActivity {
    /// The handle for a DISABLED engine: `recap_idle_seconds = 0` is the
    /// documented off switch, so no engine is spawned and there is nothing
    /// to cancel. `notify` is still safe to call — the host notifies on
    /// every user message and must not have to know whether the engine is
    /// live.
    ///
    /// Before this existed the off switch was a lie: the host passed
    /// `Duration::from_secs(0)` straight into `sleep`, which is already
    /// elapsed, so a recap fired on EVERY turn end instead of never.
    pub fn disabled() -> Self {
        Self { tx: None }
    }

    /// Whether this handle is the disabled (off) one. The observable a host or
    /// a test needs: a disabled handle owns no sender, so no engine is running.
    pub fn is_disabled(&self) -> bool {
        self.tx.is_none()
    }

    /// Signal activity: cancels any pending recap and re-arms the idle
    /// timer. Cheap (one try_send); lossy is fine — activity is a flood
    /// signal, not a queue.
    pub fn notify(&self) {
        if let Some(tx) = &self.tx {
            let _ = tx.try_send(());
        }
    }
}

/// Configuration for the idle-recap engine (`[recap]` section, omp
/// `recap.*` parity).
///
/// The recap is a side-channel turn over a transcript snapshot — the SMOL
/// role in omp's model-roles: a cheap, fast model renders the status line
/// while the primary stays untouched. `model` is the slot for exactly that;
/// empty (default) keeps the historical behavior (the primary model on the
/// primary route).
#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RecapConfig {
    /// The recap model slot: provider/model/base_url/api_key overrides.
    /// Empty model = the primary model (historical default).
    #[serde(default)]
    pub model: crate::config::AuxiliaryModelConfig,
    /// Freeform instructions APPENDED to the recap prompt (audit iter-2345).
    ///
    /// The four background context engines are each supposed to be tunable on
    /// three axes — on/off, the model, and the response shape. RECAP had the
    /// first two and not the third: `RECAP_INSTRUCTION` was a `const`, so the
    /// only way to change what a recap says was a rebuild. Empty (the
    /// default) is byte-identical to before, which is the point — a session
    /// that sets nothing must produce exactly the prompt it always did.
    #[serde(default)]
    pub instructions: String,
    /// The on/off axis, IN THIS SECTION.
    ///
    /// RECAP's switch used to be `agent.recap_idle_seconds = 0` — a different
    /// section from the model and the instructions. An operator reading
    /// `[recap]` saw two knobs and no way to turn the engine off, and the live
    /// operator config on this host has no `[recap]` section at all, so recap
    /// ran on the PRIMARY model while the advisor beside it ran on
    /// `small-stack`.
    ///
    /// `None` (the default) means "follow the idle window", which is exactly the
    /// old behaviour: a non-zero window is on, a zero window is off. `Some(false)`
    /// turns it off regardless of the window; `Some(true)` with a zero window
    /// stays off, because there is no cadence to run on.
    #[serde(default)]
    pub enabled: Option<bool>,
}

impl RecapConfig {
    /// Is the engine on, given the idle window?
    ///
    /// One resolution, so the section's flag and the host's window cannot
    /// disagree: an absent flag defers to the window, an explicit `false` wins
    /// over it, and an explicit `true` still needs a window to run on.
    pub fn is_enabled(&self, idle: std::time::Duration) -> bool {
        match self.enabled {
            Some(false) => false,
            Some(true) | None => !idle.is_zero(),
        }
    }
}

/// Spawn the idle-recap engine. Call once per agent session; only call when
/// `recap_idle_seconds > 0` (the host's gate).
///
/// * `client` — model client for the side-channel call. When the `[recap]`
///   model slot pins base_url/api_key, the host passes the SLOT client;
///   otherwise the same client the agent uses (the call is non-streaming
///   and carries no tools).
/// * `model` — model name for the recap call (the slot's model, or the
///   session's primary model when the slot is empty).
/// * `idle` — quiet window before the recap fires.
/// * `turn_ends` — subscription to the agent's `TurnEndBus`.
/// * `snapshot` — host callback returning the current transcript snapshot
///   (a read lock on the agent's conversation). Called only when the timer
///   fires, never on every turn.
/// * `deliver` — host callback receiving the finished recap (TUI notice,
///   gateway send, journal).
/// * `instructions` — the `[recap] instructions` text, appended to the
///   built-in prompt. Empty keeps the historical prompt byte-for-byte.
///
/// Returns the activity handle. Dropping it stops the engine; the task
/// also exits when the bus subscription lags (agent dropped).
pub fn spawn_recap_engine(
    client: Arc<dyn ModelClient>,
    model: String,
    idle: Duration,
    instructions: String,
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
            // FIT THE VIEW TO A BUDGET. `build_advisory_messages` bounds tool
            // results only, so user and assistant text pass through whole: a
            // session longer than the window made EVERY recap fail
            // `context_length_exceeded`, log a warn and deliver nothing —
            // permanently, with no knob. The fix is to keep the TAIL (a recap
            // is "where the session stands", and the tail is where it stands)
            // and cut the view to the budget the main path already uses for an
            // injected block.
            let mut view = fit_recap_view(crate::moa::build_advisory_messages(&messages));
            // If the newest single message out-bust the budget (a huge pasted
            // log, one assistant turn echoing a file, full tool-call args), the
            // fit keeps NOTHING and the recap would run on an empty view with
            // only the instruction — a confident fabrication. Skip the recap:
            // "nothing delivered" is safer than "a fabricated recap the operator
            // will believe".
            if view.is_empty() {
                tracing::warn!(
                    session = %session_id,
                    "recap view emptied by the budget — the newest message is larger than the whole recap budget; skipped rather than fabricate"
                );
                continue;
            }
            view.push(Message::user(recap_instruction(&instructions)));
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
            // Enforce the promised shape: one line, no markdown. The model
            // is INSTRUCTED to produce "40 words, 1-2 plain sentences, no
            // markdown", but a multi-line reply would ride to the host and
            // the gateway splits at the first continuation tag — so an
            // injected tag anywhere inside a multi-line recap would truncate
            // the operator's view. Collapse to a single line here, at the
            // one place the promise is made.
            let text = text
                .replace(crate::advisor::CONTINUATION_TAG, "[advisor-continuation]")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            if text.is_empty() {
                continue;
            }
            tracing::debug!(session = %session_id, "recap delivered to host callback");
            deliver(Recap { session_id, text });
        }
    });
    RecapActivity { tx: Some(act_tx) }
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
            String::new(),
            rx,
            || vec![Message::user("do the thing")],
            move |r| d2.lock().push(r),
        );
        bus.emit("sess-1", 3, 5, "partial result text", None);
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

    /// A transcript whose newest single message out-busts the whole recap
    /// budget empties the fitted view. The engine must SKIP rather than send
    /// the instruction alone and let the model fabricate a recap.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_empty_fitted_view_skips_the_recap() {
        let bus = crate::turn_end::TurnEndBus::new();
        let rx = bus.subscribe();
        let delivered = Arc::new(Mutex::new(Vec::<Recap>::new()));
        let d2 = delivered.clone();
        let huge = "x".repeat(RECAP_VIEW_TOKEN_BUDGET * 4 + 1);
        let engine = spawn_recap_engine(
            Arc::new(FixedClient("goal: never; task: never; next: never")),
            "m".to_string(),
            Duration::from_millis(10),
            String::new(),
            rx,
            move || vec![Message::user(huge.clone())],
            move |r| d2.lock().push(r),
        );
        bus.emit("sess-1", 3, 5, "partial result text", None);
        for _ in 0..50 {
            if !delivered.lock().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            delivered.lock().is_empty(),
            "a recap whose view is empty must not be delivered — it would be fabricated"
        );
        drop(engine);
    }

    /// The delivered recap is ONE line: the model is instructed to produce
    /// 1-2 plain sentences, but a multi-line reply must not ride to the host
    /// as a multi-line block (the gateway splits advisor summaries at the
    /// first continuation tag, and a tag inside a multi-line recap would
    /// truncate it).
    #[tokio::test(flavor = "multi_thread")]
    async fn the_delivered_recap_is_one_line() {
        let bus = crate::turn_end::TurnEndBus::new();
        let rx = bus.subscribe();
        let delivered = Arc::new(Mutex::new(Vec::<Recap>::new()));
        let d2 = delivered.clone();
        let engine = spawn_recap_engine(
            Arc::new(FixedClient(
                "goal: testing\nwith a newline\ntask: one line\nnext: verify",
            )),
            "m".to_string(),
            Duration::from_millis(10),
            String::new(),
            rx,
            || vec![Message::user("do the thing")],
            move |r| d2.lock().push(r),
        );
        bus.emit("sess-1", 3, 5, "partial result text", None);
        for _ in 0..50 {
            if !delivered.lock().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let got = delivered.lock();
        assert_eq!(got.len(), 1);
        assert!(
            !got[0].text.contains('\n'),
            "recap must be a single line, got {:?}",
            got[0].text
        );
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
            String::new(),
            rx,
            || vec![Message::user("x")],
            move |r| d2.lock().push(r),
        );
        bus.emit("sess-2", 1, 0, "done", None);
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
            String::new(),
            rx,
            || vec![Message::user("x")],
            move |r| d2.lock().push(r),
        );
        // Notify BEFORE the turn ends — the queued activity is stale by the
        // time the engine arms at the turn end below.
        engine.notify();
        bus.emit("sess-3", 1, 0, "done", None);
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
            String::new(),
            rx,
            Vec::new, // empty conversation
            move |r| d2.lock().push(r),
        );
        bus.emit("sess-3", 0, 0, "", None);
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(delivered.lock().is_empty());
        drop(engine);
    }

    /// THE OFF SWITCH, driven through the real seam.
    ///
    /// `recap_idle_seconds = 0` is documented as disabled in the config and in
    /// this module's own doc, but the engine was handed the duration verbatim:
    /// `sleep(Duration::ZERO)` is already elapsed, so a recap fired after EVERY
    /// turn instead of never — the opposite of off. Proven by probe before the
    /// fix: one recap inside 150 ms with `Duration::from_secs(0)`.
    ///
    /// A transcript longer than the budget is cut to the NEWEST turns.
    ///
    /// Before this the whole snapshot went to the model: `build_advisory_messages`
    /// bounds tool results only, so a session longer than the window made every
    /// recap fail `context_length_exceeded` and deliver nothing, permanently.
    #[test]
    fn a_long_snapshot_is_cut_to_the_newest_turns() {
        let view: Vec<Message> = (0..400)
            .map(|i| Message::user(format!("message {i} {}", "x".repeat(400))))
            .collect();
        let fitted = fit_recap_view(view.clone());
        let total: usize = fitted
            .iter()
            .map(|m| crate::context_management::estimate_tokens(&m.content))
            .sum();
        assert!(total <= RECAP_VIEW_TOKEN_BUDGET, "fitted to {total}");
        assert!(fitted.len() < view.len(), "must actually cut");
        // The TAIL survives: the recap is about where the session stands.
        assert_eq!(fitted.last().unwrap().content, view.last().unwrap().content);
        assert_eq!(
            fitted[0].content,
            view[view.len() - fitted.len()].content,
            "the kept window must be a contiguous newest slice"
        );
    }

    /// A view under the budget comes back byte-identical, not rebuilt.
    #[test]
    fn a_short_view_is_returned_unchanged() {
        let view = vec![Message::user("one"), Message::assistant("two")];
        let fitted = fit_recap_view(view.clone());
        assert_eq!(fitted.len(), view.len());
        assert_eq!(fitted[0].content, view[0].content);
        assert_eq!(fitted[1].content, view[1].content);
    }

    /// The guard lives in `attach_recap_engine`, so the observable is the
    /// HANDLE it returns: a live engine owns a broadcast receiver and a
    /// disabled one has dropped it. Asserting on `RecapActivity::disabled()`
    /// alone would pass with the guard deleted — mutation-proven: removing
    /// the `idle.is_zero()` early return left that weaker test green.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_zero_idle_window_attaches_a_disabled_handle() {
        let mut agent = crate::agent::tests_support::agent_for_recap_test();
        let handle = agent.attach_recap_engine(
            std::time::Duration::from_secs(0),
            RecapConfig::default(),
            crate::client::ClientConfig::default(),
        );
        // A live engine's `notify` reaches a live receiver; a disabled
        // handle's receiver is gone, so `try_send` fails. That difference is
        // the whole contract.
        assert!(
            handle.is_disabled(),
            "a zero idle window must attach a DISABLED handle — a live engine here is the \
             zero-second timer that fired a recap after every turn"
        );
        // And the host can still notify unconditionally.
        handle.notify();
    }

    /// A non-zero window still spawns a live engine — the guard must not
    /// disable the feature it guards.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_real_idle_window_attaches_a_live_handle() {
        let mut agent = crate::agent::tests_support::agent_for_recap_test();
        let handle = agent.attach_recap_engine(
            std::time::Duration::from_secs(240),
            RecapConfig::default(),
            crate::client::ClientConfig::default(),
        );
        assert!(
            !handle.is_disabled(),
            "a 240s window must attach a live engine, or the recap never runs at all"
        );
    }
    /// The resolution rule: an ABSENT flag defers to the window, so a config
    /// that sets nothing behaves exactly as it did before this field existed.
    #[test]
    fn an_absent_flag_follows_the_window() {
        let cfg = RecapConfig::default();
        assert!(
            cfg.is_enabled(Duration::from_secs(240)),
            "the default config with a window is on, exactly as before"
        );
        assert!(
            !cfg.is_enabled(Duration::ZERO),
            "a zero window is still off, exactly as before"
        );
    }

    /// An explicit `false` wins over a live window — the point of the field: an
    /// operator can stop the engine from INSIDE `[recap]`.
    #[test]
    fn an_explicit_false_disables_the_engine() {
        let cfg = RecapConfig {
            enabled: Some(false),
            ..Default::default()
        };
        assert!(
            !cfg.is_enabled(Duration::from_secs(240)),
            "`[recap] enabled = false` must win over recap_idle_seconds = 240"
        );
    }

    /// An explicit `true` still needs a window: there is no cadence to run on
    /// with a zero idle, so `enabled = true` alone must not produce a recap
    /// after every turn.
    #[test]
    fn an_explicit_true_still_needs_a_window() {
        let cfg = RecapConfig {
            enabled: Some(true),
            ..Default::default()
        };
        assert!(cfg.is_enabled(Duration::from_secs(240)));
        assert!(
            !cfg.is_enabled(Duration::ZERO),
            "enabled = true with no window has no cadence, so it stays off"
        );
    }

    /// The flag parses from TOML in the section the operator edits.
    #[test]
    fn the_flag_parses_from_the_recap_section() {
        let cfg: RecapConfig = toml::from_str("enabled = false\n").expect("parse");
        assert_eq!(cfg.enabled, Some(false));
        let cfg: RecapConfig = toml::from_str("").expect("parse");
        assert_eq!(cfg.enabled, None, "absent is None, not Some(true)");
    }
}
