//! Scripted-model tests for three loop-RECOVERY paths in `agent/run.rs` —
//! the branches that turn a wasted LLM call into another LLM call instead of
//! ending the turn.
//!
//! | path | site | what the test pins |
//! |---|---|---|
//! | context overflow → compress → inline retry | `run.rs:784-806` | the retry succeeded AND the wasted iteration was refunded |
//! | truncated stop → continuation prompt → re-loop | `run.rs:878-928` | `continuation_prompt()` reached the model AND the iteration was refunded |
//! | empty content ladder → fallback model | `run.rs:995-1032` | the fallback model was really requested AND the turn ended on the exhausted shape |
//!
//! ## Why the refund is asserted through the diagnostics log
//!
//! The refund (`IterationBudget::refund`) is NOT observable through the
//! loop's terminal shape, and this is worth stating precisely because it
//! decides how the tests are written:
//!
//! - `IterationBudget::new(config.max_iterations)` (builders.rs:93) makes
//!   `budget.max_total == config.max_iterations`, and every loop pass does
//!   one `consume()` then one `iteration += 1` (run.rs:377, 407). So
//!   `used == iteration - refunds`.
//! - The budget gate (`run.rs:377`) and the `iteration > max_iterations`
//!   gate (`run.rs:447`) therefore trip on the SAME pass when nothing was
//!   refunded, and the `iteration` gate trips FIRST when something was —
//!   refunds only ever delay the budget gate past a gate that has already
//!   fired.
//! - Both gates land on `attempt_grace_call`, so both terminal shapes are
//!   `Ok(<grace summary>)` (or `Err(MaxIterationsExceeded)`), identical with
//!   and without the refund.
//!
//! The one observable that does move is `TurnDiagnostics::budget_used`,
//! which the loop logs on the final-response exit (`run.rs:1179-1190`) as
//! `Turn ended: reason=… model=… api_calls=1/1 budget=0/1 …`. The tests
//! capture the tracing output and read `budget=` off the line for the turn's
//! own (unique) model name. Delete any one `refund()` call and the expected
//! number changes: that is the mutation these tests are written to catch.
//! (Exit reasons render snake_case — `reason=text_response`,
//! `reason=budget_exhausted` — per `TurnExitReason`'s Display.)
//!
//! ## Two facts the scripts depend on (found while reading the branches)
//!
//! 1. `Error::ContextLengthExceeded` does **not** reach the compression
//!    branch. `Failover::classify` (fallback.rs:93) only sets
//!    `should_compress` for `Error::Provider` bodies that match
//!    `CONTEXT_OVERFLOW_PATTERNS`; the bare `ContextLengthExceeded` variant
//!    falls into the catch-all arm with `should_compress: false`. So the
//!    script raises a real provider error body.
//! 2. The empty ladder's `EmptyExhausted` sentinel (turn_rules.rs:101) is
//!    **not** wired into `run.rs` — only its own unit tests call it. The
//!    exhausted terminal shape is therefore `Ok(<assistant message with an
//!    empty body>)` and the diagnostic reports `reason=TextResponse …
//!    response_len=0`, not an error. Test 3 pins the shape that actually
//!    exists and asserts it is distinguishable from both `Ok(<breaker abort>)`
//!    and `Ok(<grace summary>)` (see `tests/circuit_breaker_abort.rs` for
//!    those two).
//!
//! ## Harness
//!
//! Mirrors `tests/turn_anomaly_retry.rs`: a real `OperantAgent` driven by a
//! scripted `ModelClient` that records every request's model name and
//! message list, so both the injected prompts and the fallback switch are
//! assertable from the model's point of view. No tool is registered — none of
//! these three paths executes one, and an empty registry keeps the
//! tool-result anomaly retry (which needs trailing `Role::Tool` messages)
//! out of the picture entirely.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use futures::stream::BoxStream;

use operant_core::agent::{AgentConfig, ChatRequest, ModelClient, OperantAgent, StreamChunk};
use operant_core::client::{ChatResponse, Choice, MessageDelta, Role, Usage};
use operant_core::database::Database;
use operant_core::error::{Error, Result};
use operant_core::tools::ToolRegistry;

// ── Scripted model ──────────────────────────────────────────────

/// One scripted turn. The variants exist so each test can name the exact
/// provider behaviour it is pinning instead of hiding it behind a builder.
enum Step {
    /// A normal answer: non-empty content, `finish_reason="stop"`.
    Text(&'static str),
    /// An empty assistant turn — no content, no tool calls. Drives the
    /// empty-content ladder.
    Empty,
    /// A cut-off answer: `finish_reason="length"`, no tool calls. Drives the
    /// truncation continuation.
    Truncated(&'static str),
    /// A provider context-overflow error. Drives the compress-and-retry
    /// branch (see the module docs on why this cannot be
    /// `Error::ContextLengthExceeded`).
    ContextOverflow,
}

struct RecordedRequest {
    model: String,
    messages: Vec<String>,
}

struct ScriptedClient {
    steps: Mutex<Vec<Step>>,
    seen: Mutex<Vec<RecordedRequest>>,
}

impl ScriptedClient {
    fn new(steps: Vec<Step>) -> Self {
        Self {
            steps: Mutex::new(steps),
            seen: Mutex::new(Vec::new()),
        }
    }

    /// How many times the loop asked the model for a response.
    fn requests(&self) -> usize {
        self.seen.lock().unwrap().len()
    }

    /// The model name every request was addressed to, in order. The
    /// empty-ladder fallback flips this mid-turn, so it is the assertion
    /// that proves the switch happened rather than merely being intended.
    fn models(&self) -> Vec<String> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.model.clone())
            .collect()
    }

    fn messages_of(&self, index: usize) -> Vec<String> {
        self.seen.lock().unwrap()[index].messages.clone()
    }
}

#[async_trait]
impl ModelClient for ScriptedClient {
    fn provider_name(&self) -> &str {
        "scripted"
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
        self.seen.lock().unwrap().push(RecordedRequest {
            model: request.model.clone(),
            messages: request.messages.iter().map(|m| m.content.clone()).collect(),
        });
        let mut guard = self.steps.lock().unwrap();
        let step = guard.remove(0);
        drop(guard);
        match step {
            Step::Text(content) => Ok(response(content, "stop")),
            Step::Empty => Ok(response("", "stop")),
            Step::Truncated(content) => Ok(response(content, "length")),
            Step::ContextOverflow => Err(Error::Provider {
                status: 400,
                body: "This model's maximum context length is 8192 tokens, however you \
                       requested 9017 tokens."
                    .to_string(),
                retry_after: None,
            }),
        }
    }

    async fn chat_streaming(
        &self,
        _request: ChatRequest,
    ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
        Err(Error::Agent("streaming not used in this test".into()))
    }
}

fn response(content: &str, finish_reason: &str) -> ChatResponse {
    ChatResponse {
        id: "resp".to_string(),
        object: "chat.completion".to_string(),
        created: 0,
        model: "demo".to_string(),
        choices: vec![Choice {
            index: 0,
            message: MessageDelta {
                role: Some(Role::Assistant),
                content: Some(content.to_string()),
                reasoning_content: None,
                tool_calls: None,
            },
            finish_reason: Some(finish_reason.to_string()),
        }],
        usage: Usage {
            prompt_tokens: 1,
            completion_tokens: 1,
            total_tokens: 2,
        },
    }
}

// ── Turn-end diagnostic capture ────────────────────────────────

/// Process-wide tracing sink. The loop's `TurnDiagnostics` line is the only
/// place `budget_used` is visible from outside the crate.
#[derive(Clone)]
struct LogSink(Arc<Mutex<Vec<String>>>);

/// Holds the sink lock for the whole of one event's write, so two tests
/// running in parallel can never interleave halves of a log line.
struct EventLine {
    sink: Arc<Mutex<Vec<String>>>,
    buf: Vec<u8>,
}

impl std::io::Write for EventLine {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.buf.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Drop for EventLine {
    fn drop(&mut self) {
        let text = String::from_utf8_lossy(&self.buf).into_owned();
        if !text.trim().is_empty() {
            // Poison-tolerant: one failing assertion must not blind the
            // other two tests to their own diagnostics.
            self.sink
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(text);
        }
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogSink {
    type Writer = EventLine;

    fn make_writer(&'a self) -> Self::Writer {
        EventLine {
            sink: Arc::clone(&self.0),
            buf: Vec::new(),
        }
    }
}

/// Installs the process-global tracing sink (idempotent) and hands back the
/// buffer. Every test calls this BEFORE `run()`: a global subscriber can only
/// be installed once per process, so the first test to ask owns the
/// installation and the other two merely read the same buffer.
fn install_log_capture() -> Arc<Mutex<Vec<String>>> {
    static SINK: OnceLock<Arc<Mutex<Vec<String>>>> = OnceLock::new();
    SINK.get_or_init(|| {
        let sink = Arc::new(Mutex::new(Vec::new()));
        let _ = tracing::subscriber::set_global_default(
            tracing_subscriber::fmt()
                .with_writer(LogSink(Arc::clone(&sink)))
                .with_ansi(false)
                .with_max_level(tracing::Level::INFO)
                .finish(),
        );
        Arc::clone(&sink)
    })
    .clone()
}

/// The `Turn ended:` diagnostic the loop logged for `model`, or a panic.
///
/// Each test uses a unique model name, so the filter stays exact even though
/// the subscriber and its buffer are process-global and shared by all three.
fn turn_end_log(model: &str) -> String {
    let sink = install_log_capture();

    let needle = format!("model={model} ");
    let guard = sink.lock().unwrap_or_else(|e| e.into_inner());
    let found = guard
        .iter()
        .rev()
        .find(|line| line.contains("Turn ended:") && line.contains(&needle))
        .cloned();
    let tail: Vec<String> = guard
        .iter()
        .rev()
        .take(12)
        .cloned()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    drop(guard);
    found.unwrap_or_else(|| {
        panic!(
            "no `Turn ended:` diagnostic captured for model {model}; the turn-end log is \
             what carries budget_used. Last captured lines: {tail:?}"
        )
    })
}

// ── Agent construction ──────────────────────────────────────────

fn test_config(model: &str, max_iterations: usize) -> AgentConfig {
    AgentConfig {
        model: model.to_string(),
        max_iterations,
        tool_timeout: Duration::from_secs(5),
        request_timeout: Duration::from_secs(10),
        system_prompt: Some("You are a test agent.".to_string()),
        // Non-streaming: all three recovery branches live on the shared
        // response path, which both modes enter.
        stream: false,
        context_window: 8000,
        max_tool_result_share: operant_core::context_management::DEFAULT_MAX_TOOL_RESULT_SHARE,
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

async fn build_agent(
    client: Arc<ScriptedClient>,
    config: AgentConfig,
    db_path: std::path::PathBuf,
) -> OperantAgent {
    // No tool registered: none of these paths executes one, and an empty
    // registry keeps the tool-result anomaly detector (which needs trailing
    // Role::Tool messages) from ever arming.
    let registry = ToolRegistry::new(Duration::from_secs(5));
    OperantAgent::new(
        config,
        Box::new(ScriptedClientHandle(client)),
        registry,
        Arc::new(Database::init(db_path).unwrap()),
    )
}

/// Lets the shared [`Arc<ScriptedClient>`] survive the `Box<dyn ModelClient>`
/// hand-off so the test can inspect recorded requests afterwards.
struct ScriptedClientHandle(Arc<ScriptedClient>);

#[async_trait]
impl ModelClient for ScriptedClientHandle {
    fn provider_name(&self) -> &str {
        self.0.provider_name()
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
        self.0.chat(request).await
    }

    async fn chat_streaming(
        &self,
        request: ChatRequest,
    ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
        self.0.chat_streaming(request).await
    }
}

// ── 1. Context overflow → compress → refund → inline retry ──────

/// The provider rejects iteration 1 with a context-overflow body. The loop
/// classifies it, compresses the conversation, **refunds** the wasted
/// iteration (run.rs:792) and re-issues the request inline (run.rs:801-806).
/// The turn then ends on the retry's answer.
///
/// `max_iterations: 1` is the point: a single iteration is allowed, and the
/// `budget=0/1` in the turn-end diagnostic can only be produced by the
/// refund. Without `refund()` the counter is left at 1 and the line reads
/// `budget=1/1` — the assertion fails, it does not silently pass.
#[tokio::test]
async fn context_overflow_compresses_refunds_and_retries_inline() {
    const MODEL: &str = "demo-overflow-refund";
    let _capture = install_log_capture();
    let temp = tempfile::tempdir().unwrap();
    let client = Arc::new(ScriptedClient::new(vec![
        Step::ContextOverflow,
        Step::Text("answer served after the compression retry"),
    ]));
    let agent = build_agent(
        Arc::clone(&client),
        test_config(MODEL, 1),
        temp.path().join("overflow_retry.sqlite"),
    )
    .await;

    let result = agent
        .run("summarise the long thread".to_string())
        .await
        .expect("the compression retry must recover the turn, not propagate the error");

    assert_eq!(
        result.content, "answer served after the compression retry",
        "the turn must end on the post-compression retry's answer"
    );
    // Two model calls: the rejected one and the inline retry that replaced
    // it. A third would mean the loop went around again.
    assert_eq!(
        client.requests(),
        2,
        "the overflow must cost exactly one wasted call plus one retry"
    );
    // The retry is a rebuilt request against the compressed history, not a
    // replay of the original — and it still addresses the same model.
    assert!(
        !client.messages_of(1).is_empty(),
        "the retry must be issued with a rebuilt message list"
    );
    assert_eq!(
        client.models(),
        vec![MODEL.to_string(), MODEL.to_string()],
        "compression must not switch the model"
    );

    let diag = turn_end_log(MODEL);
    assert!(
        diag.contains("reason=text_response"),
        "the recovered turn must end as a text response, got: {diag}"
    );
    assert!(
        diag.contains("api_calls=1/1"),
        "the retry happened inside iteration 1, not in a new one, got: {diag}"
    );
    // THE assertion: the overflow iteration was paid for and then given
    // back. Delete run.rs:792 and this reads `budget=1/1`.
    assert!(
        diag.contains("budget=0/1"),
        "the context-overflow iteration must be refunded, got: {diag}"
    );
}

// ── 2. Truncated stop → continuation prompt → refund → re-loop ───

/// The provider cuts the answer off (`finish_reason="length"`). The loop
/// appends `continuation_prompt()` as a user message, refunds the wasted
/// iteration (run.rs:925) and re-loops; the second response is the finished
/// answer.
///
/// `max_iterations: 2` because the continuation is a real second iteration —
/// unlike the inline retry above, this path goes around the loop. The
/// `budget=1/2` in the diagnostic is one consume for the truncated turn and
/// one for the continuation, i.e. the refund happened; without it the
/// truncated iteration would still be charged and the line reads
/// `budget=2/2`.
#[tokio::test]
async fn truncated_stop_continues_with_prompt_and_refunds_the_iteration() {
    const MODEL: &str = "demo-truncation-refund";
    const CONTINUATION: &str = "[System: Your previous response was truncated by the output";
    let _capture = install_log_capture();
    let temp = tempfile::tempdir().unwrap();
    let client = Arc::new(ScriptedClient::new(vec![
        Step::Truncated("Here is the first half of the answer, and then the"),
        Step::Text(" second half, so the whole answer is finally complete."),
    ]));
    let agent = build_agent(
        Arc::clone(&client),
        test_config(MODEL, 2),
        temp.path().join("truncation_refund.sqlite"),
    )
    .await;

    let result = agent
        .run("write a long answer".to_string())
        .await
        .expect("a truncated answer must be continued, not surfaced or errored");

    assert_eq!(
        result.content, " second half, so the whole answer is finally complete.",
        "the turn must end on the continuation's answer, not the truncated one"
    );
    assert_eq!(
        client.requests(),
        2,
        "the truncation must cost exactly one truncated call plus one continuation"
    );
    // The continuation reached the model as a user message on the retry.
    assert!(
        client
            .messages_of(1)
            .iter()
            .any(|m| m.contains(CONTINUATION)),
        "the continuation request must carry the truncation prompt, got: {:?}",
        client.messages_of(1)
    );
    assert_eq!(
        agent.metrics().snapshot().truncation_continuations,
        1,
        "the truncation continuation must be recorded exactly once"
    );

    let diag = turn_end_log(MODEL);
    assert!(
        diag.contains("api_calls=2/2"),
        "the continuation is a real second iteration, got: {diag}"
    );
    // THE assertion: only the continuation's own iteration is charged.
    // Delete run.rs:925 and this reads `budget=2/2`.
    assert!(
        diag.contains("budget=1/2"),
        "the truncated iteration must be refunded, got: {diag}"
    );
}

// ── 3. Empty content ladder → fallback model → exhausted exit ───

/// Four consecutive empty assistant turns walk the whole ladder: three
/// nudges (`EmptyResponseCounter`, run.rs:995) and then, with the counter
/// spent, the fallback-model switch (run.rs:1016-1032). A fifth empty turn
/// is needed to produce a terminal shape at all — the loop only stops
/// giving the model a chance once the fallback has also come back empty.
///
/// Asserts the fallback was genuinely requested (`models()` flips mid-turn),
/// and that the terminal is the exhausted-empty shape — `Ok` with an empty
/// body, `reason=TextResponse`, `response_len=0` — which is neither the
/// `Ok(⚠️ I stopped early: …)` breaker abort nor the `Ok(<summary>)` grace
/// call that `tests/circuit_breaker_abort.rs` pins.
#[tokio::test]
async fn empty_ladder_tries_fallback_before_ending_on_the_exhausted_shape() {
    const MODEL: &str = "demo-empty-ladder";
    const FALLBACK: &str = "demo-empty-ladder-alt";
    /// The breaker abort body from `tests/circuit_breaker_abort.rs`.
    const ABORT_PREFIX: &str = "⚠️ I stopped early:";

    let _capture = install_log_capture();
    let temp = tempfile::tempdir().unwrap();
    let client = Arc::new(ScriptedClient::new(vec![
        Step::Empty,
        Step::Empty,
        Step::Empty,
        Step::Empty,
        Step::Empty,
    ]));
    let mut config = test_config(MODEL, 8);
    config.fallback_models = vec![FALLBACK.to_string()];
    config.fallback_on_errors = true;
    let agent = build_agent(
        Arc::clone(&client),
        config,
        temp.path().join("empty_ladder.sqlite"),
    )
    .await;

    let result = agent
        .run("say something".to_string())
        .await
        .expect("the exhausted empty ladder still returns Ok — see the module docs");

    // THE assertion: the fallback switch (run.rs:1029) actually reached a
    // request. Delete `set_model` and the last request is still addressed to
    // the primary, so this fails instead of quietly passing.
    let models = client.models();
    assert_eq!(
        models.len(),
        5,
        "3 nudges + the fallback switch + the turn that ends it"
    );
    assert!(
        models[..4].iter().all(|m| m == MODEL),
        "the primary must serve the three retries and the switch itself, got: {models:?}"
    );
    assert_eq!(
        models[4], FALLBACK,
        "the fifth turn must be served by the fallback model, got: {models:?}"
    );

    // The exhausted terminal shape, distinct from both Ok shapes above.
    assert!(
        result.content.is_empty(),
        "the exhausted ladder has no content to return, got: {:?}",
        result.content
    );
    assert!(
        !result.content.starts_with(ABORT_PREFIX),
        "this must not be a circuit-breaker abort, got: {}",
        result.content
    );
    assert_eq!(
        agent.metrics().snapshot().empty_content_retries,
        3,
        "the nudge budget is max_retries=3, all three spent"
    );

    let diag = turn_end_log(FALLBACK);
    assert!(
        diag.contains("response_len=0"),
        "the exhausted exit returns an empty body, got: {diag}"
    );
    assert!(
        !diag.contains("reason=budget_exhausted"),
        "this is the exhausted-empty exit, not the grace-summary exit, got: {diag}"
    );
    // 5 iterations ran but only 1 is charged: the three nudges and the
    // fallback switch each refunded themselves (run.rs:1011, 1030). Delete
    // either refund and this reads `budget=2/8` or higher.
    assert!(
        diag.contains("budget=1/8"),
        "each wasted empty iteration must be refunded, got: {diag}"
    );
}
