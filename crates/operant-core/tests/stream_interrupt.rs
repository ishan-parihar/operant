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
        reasoning_effort: String::new(),
        reasoning_effort_param: Default::default(),
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
        loop_detection_enabled: true,
        approval_mode: "off".to_string(),
        approval_allowlist: Vec::new(),
        approval_allowlist_path: None,
        record_trajectories: false,
        skill_nudge_interval: 0,
        memory_review_interval: 0,
        max_retries: 3,
        tool_search: Default::default(),
        guardrail_exempt_tools: Vec::new(),
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

    // (2026-10-14 batch, plan 04) this fixture's stream ENDS with the
    // partial as its final content, so the loop takes the normal
    // final-text exit: Ok with the partial as the turn's answer — and the
    // partial IS in the store (committed by the normal path), so the next
    // turn's request carries it.
    assert!(
        result.content.contains(HEAD),
        "the partial content received before the interrupt must be returned, got: {:?}",
        result.content
    );
    assert!(
        !result.content.contains(TAIL),
        "the chunk after the interrupt must not be consumed into the answer, got: {:?}",
        result.content
    );

    // Store consistency: the same partial is the committed assistant tail.
    let conv = agent.conversation().await;
    let partial = conv
        .iter()
        .rev()
        .find(|m| m.role == operant_core::client::Role::Assistant)
        .expect("the partial assistant message is committed");
    assert!(partial.content.contains(HEAD), "partial is in the store");

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

/// (plan 04 RED list, bullet 2) An assistant that issues tool calls, with
/// the interrupt landing during the batch: every unanswered call is paired
/// with a truthful synthetic result, the next request passes
/// message_safety with ZERO repairs, and no "Operation interrupted."
/// mutation is appended.
#[tokio::test]
async fn interrupt_during_tool_batch_pairs_every_call_without_repairs() -> Result<()> {
    use operant_core::schema::ToolSchema;
    use operant_core::tools::{OperantTool, ToolContext, ToolResult};
    use serde_json::{Value, json};

    /// A tool that blocks forever — the "tool ignoring cancellation" case
    /// (the real fix races the batch drain; completed calls keep results,
    /// blocked ones get the synthetic result).
    struct BlockingTool;
    #[async_trait]
    impl OperantTool for BlockingTool {
        fn name(&self) -> &str {
            "blocking_tool"
        }
        fn description(&self) -> &str {
            "blocks forever"
        }
        fn schema(&self) -> ToolSchema {
            ToolSchema::new("blocking_tool", "blocks forever", json!({"type": "object"}))
        }
        async fn execute(&self, _args: Value, _ctx: ToolContext) -> ToolResult {
            std::future::pending::<()>().await;
            #[expect(clippy::unreachable, reason = "pending never resolves")]
            unreachable!()
        }
    }

    let temp = tempfile::tempdir()?;
    let flag = InterruptFlag::new();
    let registry = ToolRegistry::new(Duration::from_secs(300));
    registry.register(BlockingTool).await?;

    /// Serves one assistant message with three tool calls, then blocks the
    /// stream forever on the second iteration.
    struct ToolBatchClient;
    #[async_trait]
    impl ModelClient for ToolBatchClient {
        fn provider_name(&self) -> &str {
            "tool-batch-fixture"
        }
        async fn chat(&self, _r: ChatRequest) -> Result<ChatResponse> {
            Err(Error::Agent("streaming fixture only".into()))
        }
        async fn chat_streaming(
            &self,
            _r: ChatRequest,
        ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
            // One iteration: an assistant with three tool calls, then the
            // stream ENDS (finish) — the run loop commits the assistant
            // and enters the tool batch, which is where the interrupt
            // lands (the spawned trigger fires 300 ms in, while all
            // three blocking calls are running).
            let chunk = StreamChunk::new(
                None,
                None,
                Some(vec![
                    operant_core::client::ToolCall {
                        id: "call-1".to_string(),
                        function: operant_core::client::ToolCallFunction {
                            name: "blocking_tool".to_string(),
                            arguments: "{\"n\":1}".to_string(),
                        },
                    },
                    operant_core::client::ToolCall {
                        id: "call-2".to_string(),
                        function: operant_core::client::ToolCallFunction {
                            name: "blocking_tool".to_string(),
                            arguments: "{\"n\":2}".to_string(),
                        },
                    },
                    operant_core::client::ToolCall {
                        id: "call-3".to_string(),
                        function: operant_core::client::ToolCallFunction {
                            name: "blocking_tool".to_string(),
                            arguments: "{\"n\":3}".to_string(),
                        },
                    },
                ]),
            );
            Ok(futures::stream::once(async move { Ok(chunk) }).boxed())
        }
    }

    let agent = OperantAgent::new(
        test_config(),
        Box::new(ToolBatchClient),
        registry,
        Arc::new(Database::init(
            temp.path().join("tool_batch_interrupt.sqlite"),
        )?),
    )
    .with_interrupt_flag(flag.clone());

    // Trigger the flag while the tool batch is running.
    let trigger_flag = flag.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        trigger_flag.trigger();
    });

    let result = agent.run("run the blocking tools".to_string()).await;
    let Err(Error::Interrupted) = result else {
        panic!("an interrupted tool-batch run must return Error::Interrupted");
    };

    // The store: 3 tool results, each "Cancelled…" — pairing holds.
    let conv = agent.conversation().await;
    let tool_results: Vec<_> = conv
        .iter()
        .filter(|m| m.role == operant_core::client::Role::Tool)
        .collect();
    assert_eq!(
        tool_results.len(),
        3,
        "every unanswered call gets a synthetic result (API pairing rule)"
    );
    for m in &tool_results {
        assert!(
            m.content.contains("Cancelled by the user"),
            "synthetic results are truthful, got: {:?}",
            m.content
        );
    }
    // No "Operation interrupted." mutation anywhere.
    assert!(
        conv.iter()
            .all(|m| !m.content.contains("Operation interrupted.")),
        "the store is never mutated with interruption filler"
    );
    // And the pairing is provider-valid: a repair pass finds nothing to
    // mutate (repairs == 0 — the plan's "without repairs" criterion).
    let mut for_repair = conv.clone();
    let repairs = operant_core::agent::message_safety::repair_message_sequence(&mut for_repair);
    assert_eq!(
        repairs, 0,
        "the interrupted store must pass the repair pass without changes"
    );

    Ok(())
}
