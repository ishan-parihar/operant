//! Regression test for the `/stop`-mid-stream guard in
//! `agent/stream.rs::process_stream`.
//!
//! ## What is pinned
//!
//! `process_stream`'s SSE consume loop checks the interrupt flag at the top
//! of every iteration and `break`s when it is set. Before that guard existed
//! the loop had no interrupt check at all, so `/stop` could not stop a
//! response that was *already streaming* — a stop only took effect between
//! tool calls. That is the reported "`/stop` did not stop the streaming"
//! symptom.
//!
//! The `break` is what makes the partial answer visible: it falls through to
//! the flush that `process_stream` runs on every exit path, so the chunks
//! that already arrived are returned rather than discarded. The property
//! under test is therefore two-sided, and the test asserts both halves:
//!
//! 1. the content received *before* the interrupt is returned, and
//! 2. the content that would have arrived *after* it is not.
//!
//! Either half alone is a weaker test: (1) alone passes against a fix that
//! returned an empty body, and (2) alone passes against a fix that truncated
//! the answer to nothing.
//!
//! ## How the interrupt is triggered
//!
//! From the stream itself, between the two chunks. That is the real
//! ordering — the user hits `/stop` while bytes are still arriving, so the
//! flag is already set by the time the consumer asks for the *next* item.
//! Triggering it before `run()` would not exercise the guard at all: the
//! loop's own pre-iteration check (`run.rs:414`) would exit the turn first.
//!
//! ## Harness
//!
//! Copied from `tests/turn_anomaly_retry.rs` / `tests/loop_recovery_paths.rs`:
//! a real `OperantAgent` over a temp `Database`, driven by a scripted
//! `ModelClient`. The one difference is `stream: true` plus a client that
//! actually serves `StreamChunk`s, because the guard lives on the
//! streaming-only path. The siblings set `stream: false` and stub
//! `chat_streaming` with an error.
//!
//! No tool is registered: this turn never executes one, and an empty
//! registry keeps the tool-result anomaly retry (which needs trailing
//! `Role::Tool` messages) out of the picture.
//!
//! No `#[allow]` anywhere — the test is written with `?` and atomics so it
//! needs no lint escape hatch.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use futures::{StreamExt, stream::BoxStream};

use operant_core::agent::{AgentConfig, ChatRequest, ModelClient, OperantAgent, StreamChunk};
use operant_core::client::ChatResponse;
use operant_core::context_management::DEFAULT_MAX_TOOL_RESULT_SHARE;
use operant_core::database::Database;
use operant_core::error::{Error, Result};
use operant_core::interrupt::InterruptFlag;
use operant_core::tools::ToolRegistry;

/// The text that had already arrived when `/stop` landed. Must survive.
const HEAD: &str = "PART-1";

/// The text that was still in flight when `/stop` landed. Must NOT survive —
/// receiving it is the bug.
const TAIL: &str = "TAIL-THAT-MUST-NOT-APPEAR";

/// Serves one streaming turn of two text chunks, tripping the interrupt flag
/// from inside the stream in between. `polls` counts how many times the
/// consumer actually pulled from the stream, so "stopped early" is measured
/// and not just inferred from the returned text.
struct InterruptingStreamClient {
    flag: InterruptFlag,
    polls: Arc<AtomicUsize>,
}

#[async_trait]
impl ModelClient for InterruptingStreamClient {
    fn provider_name(&self) -> &str {
        "interrupting-stream"
    }

    /// Unreachable under this fixture: the config sets `stream: true`, so the
    /// loop only ever takes the streaming branch. Errors rather than
    /// returning a plausible-looking answer, so a config regression shows up
    /// as a failure instead of a silently-passing test.
    async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse> {
        Err(Error::Agent(
            "this fixture serves the streaming path only".into(),
        ))
    }

    async fn chat_streaming(
        &self,
        _request: ChatRequest,
    ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
        let (flag, polls) = (self.flag.clone(), Arc::clone(&self.polls));
        Ok(
            futures::stream::unfold((0usize, flag, polls), |(step, flag, polls)| async move {
                polls.fetch_add(1, Ordering::SeqCst);
                match step {
                    0 => Some((Ok(text_chunk(HEAD)), (1, flag, polls))),
                    1 => {
                        // The user hits `/stop` here, mid-stream. The chunk is
                        // still handed to the consumer — the guard, not the
                        // producer, is what has to cut the loop.
                        flag.trigger();
                        Some((Ok(text_chunk(TAIL)), (2, flag, polls)))
                    }
                    _ => None,
                }
            })
            .boxed(),
        )
    }
}

/// A plain text chunk. No `finish_reason` on purpose: the truncation
/// continuation (`should_treat_stop_as_truncated`) is keyed on
/// `Some("length")` or a GLM-shaped `Some("stop")`, so leaving it `None`
/// keeps this turn on the plain text-response exit and the returned content
/// is the stream's own accumulator, unmodified.
fn text_chunk(content: &str) -> StreamChunk {
    StreamChunk::new(Some(content.to_string()), None, None)
}

fn test_config() -> AgentConfig {
    AgentConfig {
        model: "demo-stream-interrupt".to_string(),
        max_iterations: 3,
        tool_timeout: Duration::from_secs(5),
        request_timeout: Duration::from_secs(10),
        system_prompt: Some("You are a test agent.".to_string()),
        // The one field that differs from the sibling harnesses: the guard
        // under test is on the streaming-only path.
        stream: true,
        context_window: 8000,
        max_tool_result_share: DEFAULT_MAX_TOOL_RESULT_SHARE,
        max_healing_attempts: 1,
        fallback_models: Vec::new(),
        fallback_on_errors: false,
        approval_mode: "off".to_string(),
        approval_allowlist: Vec::new(),
        approval_allowlist_path: None,
        record_trajectories: false,
        skill_nudge_interval: 0,
        memory_review_interval: 0,
        max_retries: 3,
        tool_search: Default::default(),
    }
}

/// `/stop` during a live stream returns the partial answer and stops pulling
/// from the stream.
///
/// The mutation this catches: delete the `if self.interrupt_flag
/// .is_triggered() { break; }` guard at the top of `process_stream`'s
/// consume loop. The stream is then drained to exhaustion, the tail is
/// appended to the accumulator, and this test fails on the second assertion
/// (`polls` reads 3 instead of 2).
#[tokio::test]
async fn interrupt_mid_stream_returns_partial_content_and_stops_consuming() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let flag = InterruptFlag::new();
    let polls = Arc::new(AtomicUsize::new(0));

    // `OperantAgent::new` mints its own flag, so the flag the stream trips is
    // wired in afterwards with `with_interrupt_flag` — the same injection a
    // TUI/CLI does for Ctrl-C. Both sides then share one `InterruptFlag`.
    let agent = OperantAgent::new(
        test_config(),
        Box::new(InterruptingStreamClient {
            flag: flag.clone(),
            polls: Arc::clone(&polls),
        }),
        // No tool registered: this turn never executes one.
        ToolRegistry::new(Duration::from_secs(5)),
        Arc::new(Database::init(temp.path().join("stream_interrupt.sqlite"))?),
    )
    .with_interrupt_flag(flag);

    let result = agent.run("stream a long answer".to_string()).await?;

    // The stop actually landed, mid-flight, on the flag the guard reads.
    assert!(
        agent.interrupt_triggered(),
        "the fixture must have tripped the agent's interrupt flag mid-stream"
    );

    // Half 1: what arrived before the stop is returned, not discarded. The
    // flush `process_stream` runs after the `break` is what makes this
    // possible — a guard that returned early *before* accumulating would
    // hand back an empty body and still satisfy half 2.
    assert!(
        result.content.contains(HEAD),
        "the partial content received before the interrupt must be returned, got: {:?}",
        result.content
    );

    // Half 2: what was still in flight is not.
    assert!(
        !result.content.contains(TAIL),
        "the chunk after the interrupt must not be consumed into the answer \
         (this is the /stop-did-not-stop-streaming bug), got: {:?}",
        result.content
    );

    // And the stream was genuinely left unconsumed: the consumer pulled the
    // head, pulled the tail, and stopped — it never asked for the third item
    // that would have ended the stream. A guard that kept pulling but
    // filtered the text afterwards would pass the two assertions above and
    // fail here.
    assert_eq!(
        polls.load(Ordering::SeqCst),
        2,
        "the loop must break on the pull that carried the interrupt, not \
         drain the stream to exhaustion"
    );

    Ok(())
}
