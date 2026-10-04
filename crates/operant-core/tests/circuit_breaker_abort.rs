//! Scripted-model tests for the two HARD-ABORT circuit breakers in
//! `agent/run.rs` — the only two places the loop returns an abort message
//! instead of a model answer:
//!
//! | breaker | nudge | abort | site |
//! |---|---|---|---|
//! | R35 identical-call guard | `identical_streak == 4` | `identical_streak >= 6` | `run.rs:1332-1358` |
//! | degenerate all-failure loop | `consecutive_failed_iters == 3` | `consecutive_failed_iters >= 6` | `run.rs:1359-1384` |
//!
//! Both breakers abort by `return Ok(Message::assistant("⚠️ I stopped
//! early: …"))`. That is a THIRD terminal shape, and the whole point of
//! this file is to keep it distinguishable from the other two exits the
//! loop can take:
//!
//! 1. `Ok(abort)` — a breaker fired. Body is the canned `⚠️ I stopped
//!    early:` text; no model turn produced it.
//! 2. `Ok(summary)` — the iteration budget / `max_iterations` ran out and
//!    the grace call (toolless summarisation request) SUCCEEDED. Body is
//!    whatever the model wrote.
//! 3. `Err(MaxIterationsExceeded)` — exhaustion AND the grace call failed.
//!
//! A breaker must never be mistaken for a graceful summary (it would look
//! like the agent answered) and a graceful summary must never be mistaken
//! for a hard error. Each test below pins the exact body it expects and
//! asserts the other two shapes are absent.
//!
//! ## Harness
//!
//! Mirrors `tests/turn_anomaly_retry.rs`: a real `OperantAgent` driven by a
//! scripted `ModelClient` that records every request's message list, so
//! the injected nudges can be asserted from the model's point of view.
//!
//! ## Interacting paths these scripts are written around
//!
//! - **R4 tool guardrail** (`tool_guardrails.rs`, called from
//!   `execute_tools`) skips a side-effecting tool's 3rd+ identical call and
//!   returns a FAILING synthetic result. The R35 test therefore produces a
//!   few failed iterations on its way to the streak of 6 — harmless,
//!   because the R35 check (run.rs:1332) runs *before* the degenerate
//!   check (run.rs:1359) and the asserted body proves which branch fired.
//!   The degenerate test dodges the guardrail entirely by varying the
//!   arguments on every call, which also keeps the R35 signature distinct.
//! - **Tool-result anomaly retry** (`turn_end_heuristics`) is gated on a
//!   tool-call-free response (`run.rs:870`), so a script that only ever
//!   emits tool calls never reaches it.
//! - **Iteration budget** is `IterationBudget::new(max_iterations)`, and
//!   its `consume()` gate (run.rs:371) sits *before* the
//!   `iteration > max_iterations` gate (run.rs:441). Both land on the same
//!   `attempt_grace_call`, so the exhaustion tests exercise shapes 2 and 3.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::stream::BoxStream;

use operant_core::agent::{AgentConfig, ChatRequest, ModelClient, OperantAgent, StreamChunk};
use operant_core::client::{
    ChatResponse, Choice, MessageDelta, Role, ToolCallDelta, ToolCallFunction, Usage,
};
use operant_core::database::Database;
use operant_core::error::{Error, Result};
use operant_core::schema::ToolSchema;
use operant_core::tools::{OperantTool, ToolContext, ToolRegistry, ToolResult};

/// The shared prefix of every breaker abort body. Nothing the model writes
/// starts with this, so it is the reliable discriminator between shape 1
/// (breaker) and shape 2 (grace summary).
const ABORT_PREFIX: &str = "⚠️ I stopped early:";

/// Body fragment unique to the R35 identical-call abort.
const R35_ABORT_FRAGMENT: &str = "I repeated the same tool call 6 times in a row";

/// Body fragment unique to the degenerate all-failure abort.
const DEGENERATE_ABORT_FRAGMENT: &str = "my last 6 tool iterations kept failing";

/// Fragment of the R35 nudge injected at `identical_streak == 4`.
const R35_NUDGE_FRAGMENT: &str = "exact same tool call 4 times in a row";

/// Fragment of the degenerate nudge injected at `consecutive_failed_iters == 3`.
const DEGENERATE_NUDGE_FRAGMENT: &str = "Your last 3 tool-call iterations ALL failed";

/// A tool that always succeeds — used by the R35 test, whose whole premise
/// is that identical calls keep SUCCEEDING (the failure-only breaker can
/// never see this, which is exactly why R35 exists).
struct OkTool;

#[async_trait]
impl OperantTool for OkTool {
    fn name(&self) -> &str {
        "ok_probe"
    }

    fn description(&self) -> &str {
        "Always succeeds with a normal body (test fixture)."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "ok_probe",
            "Always succeeds with a normal body (test fixture).",
            serde_json::json!({"type": "object", "properties": {}}),
        )
    }

    async fn execute(&self, _args: serde_json::Value, _context: ToolContext) -> ToolResult {
        ToolResult {
            tool_call_id: String::new(),
            name: "ok_probe".to_string(),
            success: true,
            content: "42".to_string(),
            error: None,
            timed_out: false,
        }
    }
}

/// A tool that always FAILS — drives the degenerate all-failure breaker.
/// Side-effecting by name (not in `NO_EFFECT_TOOL_NAMES`), so with distinct
/// arguments the R4 guardrail never skips it and every iteration yields a
/// genuine failing result.
struct FailingTool;

#[async_trait]
impl OperantTool for FailingTool {
    fn name(&self) -> &str {
        "fail_probe"
    }

    fn description(&self) -> &str {
        "Always fails (test fixture for the degenerate-loop breaker)."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "fail_probe",
            "Always fails (test fixture for the degenerate-loop breaker).",
            serde_json::json!({"type": "object", "properties": {}}),
        )
    }

    async fn execute(&self, _args: serde_json::Value, _context: ToolContext) -> ToolResult {
        ToolResult {
            tool_call_id: String::new(),
            name: "fail_probe".to_string(),
            success: false,
            content: String::new(),
            error: Some("probe failure (test fixture)".to_string()),
            timed_out: false,
        }
    }
}

/// Scripted non-streaming client. Each call pops the next response and
/// records the request's message contents so a test can assert what the
/// model actually saw — including the breakers' injected nudges.
struct ScriptedClient {
    responses: Mutex<Vec<ChatResponse>>,
    seen: Mutex<Vec<Vec<String>>>,
}

impl ScriptedClient {
    fn new(responses: Vec<ChatResponse>) -> Self {
        Self {
            responses: Mutex::new(responses),
            seen: Mutex::new(Vec::new()),
        }
    }

    /// How many times the loop asked the model for a response.
    fn requests(&self) -> usize {
        self.seen.lock().unwrap().len()
    }

    /// True when ANY recorded request carried a message containing
    /// `fragment` — i.e. the loop actually injected it and the model saw it.
    fn saw_fragment(&self, fragment: &str) -> bool {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .flatten()
            .any(|m| m.contains(fragment))
    }
}

#[async_trait]
impl ModelClient for ScriptedClient {
    fn provider_name(&self) -> &str {
        "scripted"
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
        self.seen
            .lock()
            .unwrap()
            .push(request.messages.iter().map(|m| m.content.clone()).collect());
        let mut guard = self.responses.lock().unwrap();
        if guard.is_empty() {
            return Ok(text_response("default"));
        }
        Ok(guard.remove(0))
    }

    async fn chat_streaming(
        &self,
        _request: ChatRequest,
    ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
        Err(Error::Agent("streaming not used in this test".into()))
    }
}

fn text_response(content: &str) -> ChatResponse {
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
            finish_reason: Some("stop".to_string()),
        }],
        usage: Usage {
            prompt_tokens: 1,
            completion_tokens: 1,
            total_tokens: 2,
        },
    }
}

/// A well-formed response with NO choices. `process_response` rejects this
/// with `Error::ParseResponse`, which is how the grace call can be made to
/// fail and surface `Error::MaxIterationsExceeded` to the caller.
fn empty_choices_response() -> ChatResponse {
    ChatResponse {
        id: "resp".to_string(),
        object: "chat.completion".to_string(),
        created: 0,
        model: "demo".to_string(),
        choices: vec![],
        usage: Usage {
            prompt_tokens: 1,
            completion_tokens: 0,
            total_tokens: 1,
        },
    }
}

fn tool_call_response(tool: &str, arguments: &str) -> ChatResponse {
    ChatResponse {
        id: "resp".to_string(),
        object: "chat.completion".to_string(),
        created: 0,
        model: "demo".to_string(),
        choices: vec![Choice {
            index: 0,
            message: MessageDelta {
                role: Some(Role::Assistant),
                content: Some(String::new()),
                reasoning_content: None,
                tool_calls: Some(vec![ToolCallDelta {
                    index: 0,
                    id: Some(format!("call_{tool}")),
                    call_type: Some("function".to_string()),
                    function: Some(ToolCallFunction {
                        name: tool.to_string(),
                        arguments: arguments.to_string(),
                    }),
                }]),
            },
            finish_reason: Some("tool_calls".to_string()),
        }],
        usage: Usage {
            prompt_tokens: 1,
            completion_tokens: 1,
            total_tokens: 2,
        },
    }
}

/// `n` identical calls — the R35 script.
fn identical_calls(tool: &str, n: usize) -> Vec<ChatResponse> {
    (0..n).map(|_| tool_call_response(tool, "{}")).collect()
}

/// `n` calls whose ARGUMENTS differ on every iteration. Keeps the R35
/// signature (`name` + raw `arguments`, hashed at run.rs:1294) distinct so
/// the degenerate breaker is exercised in isolation.
fn varying_calls(tool: &str, n: usize) -> Vec<ChatResponse> {
    (1..=n)
        .map(|i| tool_call_response(tool, &format!("{{\"i\":{i}}}")))
        .collect()
}

fn test_config(max_iterations: usize) -> AgentConfig {
    AgentConfig {
        model: "demo".to_string(),
        max_iterations,
        tool_timeout: Duration::from_secs(5),
        request_timeout: Duration::from_secs(10),
        system_prompt: Some("You are a test agent.".to_string()),
        // Non-streaming: both breakers live on the shared response path,
        // which both modes enter.
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

async fn build_agent<T: OperantTool + 'static>(
    client: Arc<ScriptedClient>,
    tool: T,
    max_iterations: usize,
    db_path: std::path::PathBuf,
) -> OperantAgent {
    let registry = ToolRegistry::new(Duration::from_secs(5));
    registry.register(tool).await.unwrap();
    OperantAgent::new(
        test_config(max_iterations),
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

// ── 1. R35 identical-call guard ───────────────────────────────────

/// R35 fires at `identical_streak >= 6` and aborts the turn with
/// `Ok(<canned message>)` — even though every call SUCCEEDED, which is
/// precisely the case the failure-only breaker cannot see.
#[tokio::test]
async fn r35_identical_call_guard_aborts_the_turn() {
    let temp = tempfile::tempdir().unwrap();
    // 6 == the abort threshold, and == max_iterations so the budget gate
    // (which runs first) cannot pre-empt it.
    let client = Arc::new(ScriptedClient::new(identical_calls("ok_probe", 6)));
    let agent = build_agent(
        Arc::clone(&client),
        OkTool,
        6,
        temp.path().join("r35_abort.sqlite"),
    )
    .await;

    let result = agent
        .run("call the probe repeatedly".to_string())
        .await
        .expect("the R35 breaker aborts with Ok, not Err");

    // Shape 1: the breaker body, produced by the loop and not the model.
    assert!(
        result.content.starts_with(ABORT_PREFIX),
        "expected a breaker abort body, got: {result:?}"
    );
    assert!(
        result.content.contains(R35_ABORT_FRAGMENT),
        "expected the R35 identical-call abort, got: {}",
        result.content
    );
    // The R35 branch is checked FIRST (run.rs:1332, before the degenerate
    // branch at 1359), so seeing its body proves which one won.
    assert!(
        !result.content.contains(DEGENERATE_ABORT_FRAGMENT),
        "the R35 abort must not be the degenerate abort, got: {}",
        result.content
    );

    // The turn ended at the 6th identical call — not one iteration later.
    assert_eq!(
        client.requests(),
        6,
        "the abort must stop the turn at streak 6"
    );

    // The `identical_streak == 4` nudge reached the model one turn earlier.
    assert!(
        client.saw_fragment(R35_NUDGE_FRAGMENT),
        "the R35 nudge at streak 4 must be injected into the message list"
    );
}

// ── 2. Degenerate all-failure loop breaker ───────────────────────

/// The failure-only breaker nudges at 3 consecutive all-failing iterations
/// and aborts at 6. Arguments vary each iteration so the R4 guardrail
/// never skips and the R35 signature never repeats — this test therefore
/// isolates the degenerate branch.
#[tokio::test]
async fn degenerate_all_failure_breaker_aborts_the_turn() {
    let temp = tempfile::tempdir().unwrap();
    let client = Arc::new(ScriptedClient::new(varying_calls("fail_probe", 6)));
    let agent = build_agent(
        Arc::clone(&client),
        FailingTool,
        6,
        temp.path().join("degenerate_abort.sqlite"),
    )
    .await;

    let result = agent
        .run("keep failing".to_string())
        .await
        .expect("the degenerate breaker aborts with Ok, not Err");

    assert!(
        result.content.starts_with(ABORT_PREFIX),
        "expected a breaker abort body, got: {result:?}"
    );
    assert!(
        result.content.contains(DEGENERATE_ABORT_FRAGMENT),
        "expected the degenerate all-failure abort, got: {}",
        result.content
    );
    // Varying the arguments kept R35 at streak 1, so it cannot have fired.
    assert!(
        !result.content.contains("repeated the same tool call"),
        "R35 must not fire when arguments differ each iteration, got: {}",
        result.content
    );

    assert_eq!(
        client.requests(),
        6,
        "the abort must stop the turn at 6 failures"
    );
    assert!(
        client.saw_fragment(DEGENERATE_NUDGE_FRAGMENT),
        "the degenerate nudge at 3 failed iterations must be injected"
    );
}

// ── 3. Grace-call summary is NOT a breaker abort ──────────────────

/// When the budget runs out BEFORE any breaker threshold, the loop makes a
/// toolless grace call. On success that is `Ok(<model's summary>)` — a
/// different shape from `Ok(<breaker abort>)`, and the two must stay
/// distinguishable. `max_iterations: 3` puts the ceiling well under both
/// breaker thresholds, so neither can fire.
#[tokio::test]
async fn budget_exhaustion_grace_summary_is_not_a_breaker_abort() {
    let temp = tempfile::tempdir().unwrap();
    let mut responses = varying_calls("fail_probe", 3);
    responses.push(text_response("summary of partial work: 2 of 3 checks done"));
    let client = Arc::new(ScriptedClient::new(responses));
    let agent = build_agent(
        Arc::clone(&client),
        FailingTool,
        3,
        temp.path().join("grace_summary.sqlite"),
    )
    .await;

    let result = agent
        .run("do some work".to_string())
        .await
        .expect("a successful grace call returns Ok");

    // Shape 2: the model's own words, NOT the canned breaker body.
    assert_eq!(
        result.content, "summary of partial work: 2 of 3 checks done",
        "the grace call must return the model's summary"
    );
    assert!(
        !result.content.starts_with(ABORT_PREFIX),
        "a grace summary must never look like a breaker abort, got: {}",
        result.content
    );

    // 3 tool-calling turns + 1 toolless grace call. Note the degenerate
    // NUDGE does fire (it triggers at 3 failed iterations, which this
    // script reaches) — the breaker arms, but the budget ceiling is hit
    // before its abort threshold of 6, so the turn ends on the grace
    // call instead. The exact-content assertion above is what proves the
    // abort did not happen.
    assert_eq!(client.requests(), 4, "the grace call costs one extra turn");
    assert!(
        client.saw_fragment(DEGENERATE_NUDGE_FRAGMENT),
        "the degenerate nudge at 3 failed iterations still fires — the \
         budget ceiling, not the breaker, decides this turn's ending"
    );
}

// ── 4. Failing grace call is a hard error ─────────────────────────

/// Same exhaustion as above, but the grace call itself fails (a response
/// with no choices, which `process_response` rejects). The loop converts
/// that into `Err(MaxIterationsExceeded)` — shape 3, distinct from both
/// `Ok` shapes above.
#[tokio::test]
async fn budget_exhaustion_with_failing_grace_call_is_a_hard_error() {
    let temp = tempfile::tempdir().unwrap();
    let mut responses = varying_calls("fail_probe", 3);
    responses.push(empty_choices_response());
    let client = Arc::new(ScriptedClient::new(responses));
    let agent = build_agent(
        Arc::clone(&client),
        FailingTool,
        3,
        temp.path().join("grace_failed.sqlite"),
    )
    .await;

    let result = agent.run("do some work".to_string()).await;

    // Shape 3: a hard error, not an `Ok` abort and not an `Ok` summary.
    match result {
        Err(Error::MaxIterationsExceeded { max }) => {
            assert_eq!(max, 3, "the error must report the configured ceiling");
        }
        Ok(m) => panic!("a failed grace call must not return Ok, got: {m:?}"),
        Err(e) => panic!("expected MaxIterationsExceeded, got: {e:?}"),
    }
    assert_eq!(
        client.requests(),
        4,
        "the failing grace call was still attempted"
    );
}
