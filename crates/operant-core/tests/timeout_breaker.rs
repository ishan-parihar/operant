//! Scripted-model tests for the S2 per-tool consecutive-timeout circuit
//! breaker — the defect from `docs/AGENT-LOOP-FIX-EXECUTION-PLAN.md` where
//! a tool timeout was just an ordinary retryable error, so one wedged tool
//! could burn the entire turn (measured specimen: 11 `aft_bash` timeouts
//! in a single turn, each retried).
//!
//! The breaker, all TURN-LOCAL (reset at the top of every `run()`):
//!
//! - `ToolResult::timed_out` — set only by `ToolResult::timeout`, called
//!   from the registry's `execute_with_timeout` Err arm AND the three
//!   stream-dispatch timeout arms (so the flag survives whichever wrapper
//!   fires first — registry timeout or stream wrapper).
//! - 2nd consecutive timeout of a tool → one system nudge telling the
//!   model the tool is gone for the turn.
//! - 3rd → masked: hidden from the request's schema list
//!   (`tools_for_turn`) and refused at dispatch (`execute_tools`
//!   preflight) — a 4th call is answered by a named "disabled" error
//!   WITHOUT executing, so no fourth timeout burn.
//! - Only a SUCCESS resets a tool's streak; non-timeout failures neither
//!   advance nor reset it.
//!
//! Every case scripts the failure (the fault-injection rule) and every
//! threshold claim has a control that fails if the breaker over-blocks:
//! single-timeout and success-interleaved turns must leave the tool
//! visible. Remove any piece of the breaker and one of these fails on an
//! exact-content assertion.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::stream::BoxStream;

use operant_core::agent::{AgentConfig, ChatRequest, ModelClient, OperantAgent, StreamChunk};
use operant_core::client::{
    ChatResponse, Choice, Message, MessageDelta, Role, ToolCallDelta, ToolCallFunction, Usage,
};
use operant_core::database::Database;
use operant_core::error::{Error, Result};
use operant_core::schema::ToolSchema;
use operant_core::tools::{OperantTool, ToolContext, ToolRegistry, ToolResult};

const NUDGE_MARK: &str = "timed out twice in a row";
const REFUSAL_MARK: &str = "disabled for the rest of this turn after repeated timeouts";
const SLOW: &str = "slow_probe";
const FLAKY: &str = "flaky_probe";
const OK: &str = "ok_probe";
const FAIL: &str = "fail_probe";

/// Sleeps far past every timeout in play, so both the registry's
/// `execute_with_timeout` (short registry timeout) and the stream wrapper
/// (short config timeout) fire deterministically depending on which the
/// test shortens. Counts ENTERED executions — the refusal path must never
/// reach it a 4th time.
struct SlowTool {
    entered: Arc<AtomicUsize>,
}

#[async_trait]
impl OperantTool for SlowTool {
    fn name(&self) -> &str {
        SLOW
    }

    fn description(&self) -> &str {
        "Blocks past the timeout (drives the S2 breaker)."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            SLOW,
            "Blocks past the timeout (drives the S2 breaker).",
            serde_json::json!({"type": "object", "properties": {}}),
        )
    }

    async fn execute(&self, _args: serde_json::Value, _context: ToolContext) -> ToolResult {
        self.entered.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(200)).await;
        ToolResult::success(SLOW, serde_json::json!({"done": true}))
    }
}

fn slow_tool() -> Arc<SlowTool> {
    Arc::new(SlowTool {
        entered: Arc::new(AtomicUsize::new(0)),
    })
}

/// The reset fixture: blocks past the timeout when the call asks to
/// (`{"mode":"block"}`), returns instantly otherwise — so ONE tool can
/// produce real timeouts and a real SUCCESS on demand, and the per-tool
/// reset ("that tool's streak") can be proven with the tool's own
/// success, not a sibling tool's.
struct FlakyTool {
    entered: Arc<AtomicUsize>,
}

#[async_trait]
impl OperantTool for FlakyTool {
    fn name(&self) -> &str {
        FLAKY
    }

    fn description(&self) -> &str {
        "Blocks on demand, else succeeds (drives the S2 reset case)."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            FLAKY,
            "Blocks on demand, else succeeds (drives the S2 reset case).",
            serde_json::json!({"type": "object", "properties": {}}),
        )
    }

    async fn execute(&self, args: serde_json::Value, _context: ToolContext) -> ToolResult {
        self.entered.fetch_add(1, Ordering::SeqCst);
        if args.get("mode").and_then(|m| m.as_str()) == Some("block") {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        ToolResult::success(FLAKY, serde_json::json!({"quick": true}))
    }
}

/// Instant success — the reset/reset-control fixture.
struct OkTool;

#[async_trait]
impl OperantTool for OkTool {
    fn name(&self) -> &str {
        OK
    }

    fn description(&self) -> &str {
        "Always succeeds (test fixture)."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            OK,
            "Always succeeds (test fixture).",
            serde_json::json!({"type": "object", "properties": {}}),
        )
    }

    async fn execute(&self, _args: serde_json::Value, _context: ToolContext) -> ToolResult {
        ToolResult {
            tool_call_id: String::new(),
            name: OK.to_string(),
            success: true,
            content: "42".to_string(),
            error: None,
            timed_out: false,
        }
    }
}

/// Instant failure that is NOT a timeout — proves only real timeouts
/// advance the streak.
struct FailingTool {
    entered: AtomicUsize,
}

#[async_trait]
impl OperantTool for FailingTool {
    fn name(&self) -> &str {
        FAIL
    }

    fn description(&self) -> &str {
        "Always fails without timing out (test fixture)."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            FAIL,
            "Always fails without timing out (test fixture).",
            serde_json::json!({"type": "object", "properties": {}}),
        )
    }

    async fn execute(&self, _args: serde_json::Value, _context: ToolContext) -> ToolResult {
        self.entered.fetch_add(1, Ordering::SeqCst);
        ToolResult {
            tool_call_id: String::new(),
            name: FAIL.to_string(),
            success: false,
            content: String::new(),
            error: Some("probe failure (test fixture)".to_string()),
            timed_out: false,
        }
    }
}

/// What the loop actually handed the model on one request — the schema
/// list is where the mask must be visible, the messages are where the
/// nudge and the refusal error must be visible.
struct RecordedRequest {
    tools: Vec<String>,
    messages: Vec<Message>,
}

/// Scripted non-streaming client. Each call pops the next response and
/// records the full request (tool names + messages) so tests assert what
/// the model was actually shown.
struct ScriptedClient {
    responses: Mutex<Vec<ChatResponse>>,
    recorded: Mutex<Vec<RecordedRequest>>,
}

impl ScriptedClient {
    fn new(responses: Vec<ChatResponse>) -> Self {
        Self {
            responses: Mutex::new(responses),
            recorded: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<RecordedRequest> {
        self.recorded
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

impl Clone for RecordedRequest {
    fn clone(&self) -> Self {
        Self {
            tools: self.tools.clone(),
            messages: self.messages.clone(),
        }
    }
}

#[async_trait]
impl ModelClient for ScriptedClient {
    fn provider_name(&self) -> &str {
        "scripted"
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
        let record = RecordedRequest {
            tools: request.tools.iter().map(|t| t.name.clone()).collect(),
            messages: request.messages,
        };
        self.recorded
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(record);
        let mut guard = self.responses.lock().unwrap_or_else(|e| e.into_inner());
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
                    id: Some(format!("call_{}_{}", tool, arguments.len())),
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

/// `n` calls whose ARGUMENTS differ on every iteration, so neither the R35
/// identical-call breaker nor the R4 guardrail skips them.
fn varying_calls(tool: &str, n: usize) -> Vec<ChatResponse> {
    (1..=n)
        .map(|i| tool_call_response(tool, &format!("{{\"i\":{i}}}")))
        .collect()
}

fn test_config(max_iterations: usize) -> AgentConfig {
    AgentConfig {
        model: "demo".to_string(),
        max_iterations,
        // Generous stream-level wrapper: the registry's own (short) timeout
        // fires first in these tests, exercising the `tools.rs` writer. The
        // stream-level case is covered by its own test below.
        tool_timeout: Duration::from_secs(5),
        request_timeout: Duration::from_secs(10),
        system_prompt: Some("You are a test agent.".to_string()),
        stream: false,
        context_window: 8000,
        max_tool_result_share: operant_core::context_management::DEFAULT_MAX_TOOL_RESULT_SHARE,
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
    }
}

async fn build_agent(
    client: Arc<ScriptedClient>,
    max_iterations: usize,
    db_path: std::path::PathBuf,
    registry: ToolRegistry,
) -> OperantAgent {
    OperantAgent::new(
        test_config(max_iterations),
        Box::new(ScriptedClientHandle(client)),
        registry,
        Arc::new(Database::init(db_path).unwrap()),
    )
}

/// Lets the shared [`Arc<ScriptedClient>`] survive the `Box<dyn ModelClient>`
/// hand-off so the test can inspect the recorded requests afterwards.
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

/// How many times the nudge text appears in the final request's message
/// list (the last request carries the complete final conversation, so
/// counting there counts nudges once each, not once per subsequent
/// request).
fn nudge_count(req: &RecordedRequest) -> usize {
    req.messages
        .iter()
        .filter(|m| m.content.contains(NUDGE_MARK))
        .count()
}

fn mentions(req: &RecordedRequest, needle: &str) -> bool {
    req.messages.iter().any(|m| m.content.contains(needle))
}

fn has_tool(req: &RecordedRequest, name: &str) -> bool {
    req.tools.iter().any(|t| t == name)
}

/// The specimen shape (2026-10-03 §1 S2): a wedged tool the model keeps
/// calling. 2 timeouts → one nudge; 3rd → masked; a 4th attempt is refused
/// WITHOUT executing (no 4th timeout burn) and the turn still completes
/// on the model's own text. This is the behavioral delta the slice must
/// deliver: 2 timeouts, one nudge, mask on the 3rd, turn continues.
#[tokio::test]
async fn three_consecutive_timeouts_mask_tool_and_refuse_fourth_call() {
    let temp = tempfile::tempdir().unwrap();
    let mut responses = varying_calls(SLOW, 4);
    responses.push(text_response("done with other work"));
    let client = Arc::new(ScriptedClient::new(responses));
    // 10ms registry timeout: the INNER `execute_with_timeout` fires
    // deterministically long before the 5s stream wrapper — the `tools.rs`
    // writer, the path production runs hit.
    let slow = slow_tool();
    let registry = ToolRegistry::new(Duration::from_millis(10));
    registry.register_dyn(slow.clone()).await.unwrap();
    let agent = build_agent(
        Arc::clone(&client),
        6,
        temp.path().join("breaker_mask.sqlite"),
        registry,
    )
    .await;

    let result = agent
        .run("do some work".to_string())
        .await
        .expect("the breaker must end the turn normally, not as an error");

    // The turn completed on the model's text — the breaker nudges and
    // masks, it never kills the turn.
    assert_eq!(result.content, "done with other work");

    let requests = client.requests();
    // 4 tool-calling iterations + the final text response = 5 requests.
    assert_eq!(requests.len(), 5, "expected 5 model requests");

    // The breaker fired: masked from the request built after the 3rd
    // timeout, and stayed masked for the rest of the turn.
    assert!(has_tool(&requests[0], SLOW), "first request lists the tool");
    assert!(has_tool(&requests[1], SLOW));
    assert!(has_tool(&requests[2], SLOW));
    assert!(
        !has_tool(&requests[3], SLOW),
        "masked tool must be absent from the 4th request's schema list"
    );
    assert!(
        !has_tool(&requests[4], SLOW),
        "mask persists to the end of the turn"
    );

    // The nudge fired exactly once (at the 2nd timeout), and the model
    // saw it — it is in every request from the 3rd on.
    assert!(mentions(&requests[2], NUDGE_MARK), "nudge must be visible");
    assert_eq!(
        nudge_count(&requests[4]),
        1,
        "exactly one nudge for the whole turn"
    );

    // The 4th call was refused at dispatch: the model received a named
    // disabled-error result for it.
    assert!(
        mentions(&requests[4], REFUSAL_MARK),
        "the refused 4th call must produce a named disabled-error result"
    );

    // The 4th attempt never entered the tool — the refusal is pre-execution,
    // so no fourth timeout burn.
    assert_eq!(
        slow.entered.load(Ordering::SeqCst),
        3,
        "exactly 3 real executions: the masked 4th call was refused before execute"
    );
}

/// Positive control for the reset (per-tool: the tool's OWN success
/// clears its streak). block, block (nudge), QUICK SUCCESS (reset),
/// block, block (nudge again). If the success did not reset, the 4th
/// block would be the 3rd consecutive timeout — the tool would be
/// masked, the 5th call refused, and the second nudge would never fire.
#[tokio::test]
async fn success_between_timeouts_resets_the_streak() {
    let temp = tempfile::tempdir().unwrap();
    let responses = vec![
        tool_call_response(FLAKY, "{\"mode\":\"block\",\"i\":1}"),
        tool_call_response(FLAKY, "{\"mode\":\"block\",\"i\":2}"),
        tool_call_response(FLAKY, "{\"mode\":\"quick\",\"i\":3}"),
        tool_call_response(FLAKY, "{\"mode\":\"block\",\"i\":4}"),
        tool_call_response(FLAKY, "{\"mode\":\"block\",\"i\":5}"),
        text_response("finished anyway"),
    ];
    let client = Arc::new(ScriptedClient::new(responses));
    let flaky = Arc::new(FlakyTool {
        entered: Arc::new(AtomicUsize::new(0)),
    });
    let registry = ToolRegistry::new(Duration::from_millis(10));
    registry.register_dyn(flaky.clone()).await.unwrap();
    let agent = build_agent(
        Arc::clone(&client),
        8,
        temp.path().join("breaker_reset.sqlite"),
        registry,
    )
    .await;

    let result = agent
        .run("do some work".to_string())
        .await
        .expect("turn completes normally");
    assert_eq!(result.content, "finished anyway");

    let requests = client.requests();
    assert_eq!(requests.len(), 6);
    for (i, req) in requests.iter().enumerate() {
        assert!(
            has_tool(req, FLAKY),
            "request {i} must still list the tool — its own success reset the streak"
        );
    }
    // 2nd timeout nudged; the two post-success timeouts nudged AGAIN
    // (streak was reset to 0, so they count 1 then 2 — not 3 then 4).
    assert_eq!(
        nudge_count(&requests[5]),
        2,
        "reset proven: the streak re-reaches 2 instead of hitting the mask at 3"
    );
    assert!(
        !mentions(&requests[5], REFUSAL_MARK),
        "the tool was never masked, so no refusal exists"
    );
    // Five real executions — every call ran; none was refused.
    assert_eq!(flaky.entered.load(Ordering::SeqCst), 5);
}

/// Regression (the healthy-turn case): one timeout, then the model moves
/// to another tool and answers. Nothing may be masked and the turn must
/// end on the model's text verbatim.
#[tokio::test]
async fn single_timeout_does_not_mask_and_turn_completes_normally() {
    let temp = tempfile::tempdir().unwrap();
    let responses = vec![
        tool_call_response(SLOW, "{\"i\":1}"),
        tool_call_response(OK, "{\"i\":2}"),
        text_response("all done"),
    ];
    let client = Arc::new(ScriptedClient::new(responses));
    let slow = slow_tool();
    let registry = ToolRegistry::new(Duration::from_millis(10));
    registry.register_dyn(slow.clone()).await.unwrap();
    registry.register(OkTool).await.unwrap();
    let agent = build_agent(
        Arc::clone(&client),
        4,
        temp.path().join("breaker_single.sqlite"),
        registry,
    )
    .await;

    let result = agent
        .run("do some work".to_string())
        .await
        .expect("a single timeout must not break the turn");
    assert_eq!(result.content, "all done");

    let requests = client.requests();
    assert_eq!(requests.len(), 3);
    for (i, req) in requests.iter().enumerate() {
        assert!(
            has_tool(req, SLOW),
            "request {i}: one timeout must not mask anything"
        );
    }
    assert_eq!(nudge_count(&requests[2]), 0, "no nudge below threshold 2");
}

/// Only REAL timeouts advance the streak: two plain failures between two
/// timeouts must leave the streak at 2 (nudge, still visible) — if
/// failures counted, the second timeout would be the 4th strike and the
/// tool would be masked.
#[tokio::test]
async fn non_timeout_failures_do_not_advance_the_streak() {
    let temp = tempfile::tempdir().unwrap();
    let responses = vec![
        tool_call_response(SLOW, "{\"i\":1}"),
        tool_call_response(FAIL, "{\"i\":2}"),
        tool_call_response(FAIL, "{\"i\":3}"),
        tool_call_response(SLOW, "{\"i\":4}"),
        text_response("worked around it"),
    ];
    let client = Arc::new(ScriptedClient::new(responses));
    let slow = slow_tool();
    let registry = ToolRegistry::new(Duration::from_millis(10));
    registry.register_dyn(slow.clone()).await.unwrap();
    registry
        .register(FailingTool {
            entered: AtomicUsize::new(0),
        })
        .await
        .unwrap();
    let agent = build_agent(
        Arc::clone(&client),
        6,
        temp.path().join("breaker_failures.sqlite"),
        registry,
    )
    .await;

    let result = agent
        .run("do some work".to_string())
        .await
        .expect("turn completes normally");
    assert_eq!(result.content, "worked around it");

    let requests = client.requests();
    assert_eq!(requests.len(), 5);
    for (i, req) in requests.iter().enumerate() {
        assert!(
            has_tool(req, SLOW),
            "request {i}: plain failures must never advance the streak to the mask"
        );
    }
    assert_eq!(
        nudge_count(&requests[4]),
        1,
        "the second real timeout lands on streak 2 (nudge), not 4 (mask)"
    );
    assert!(
        !mentions(&requests[4], REFUSAL_MARK),
        "no mask, so no refusal"
    );
    assert_eq!(slow.entered.load(Ordering::SeqCst), 2);
}

/// The stream-dispatch timeout arms (the OUTER wrapper that fires when it
/// expires before the registry's own timeout) must mark the flag too — a
/// timeout result built there without `timed_out` would slip past the
/// breaker entirely. Registry timeout is generous here; the 10ms
/// `tool_timeout` wrapper is what fires.
#[tokio::test]
async fn stream_level_timeout_result_still_advances_the_breaker() {
    let temp = tempfile::tempdir().unwrap();
    let mut responses = varying_calls(SLOW, 3);
    responses.push(text_response("done"));
    let client = Arc::new(ScriptedClient::new(responses));

    // Registry timeout 5s (never fires); the 10ms stream wrapper does.
    let slow = slow_tool();
    let registry = ToolRegistry::new(Duration::from_secs(5));
    registry.register_dyn(slow.clone()).await.unwrap();
    let mut config = test_config(6);
    config.tool_timeout = Duration::from_millis(10);
    let agent = OperantAgent::new(
        config,
        Box::new(ScriptedClientHandle(Arc::clone(&client))),
        registry,
        Arc::new(Database::init(temp.path().join("breaker_stream.sqlite")).unwrap()),
    );

    let result = agent
        .run("do some work".to_string())
        .await
        .expect("turn completes normally");
    assert_eq!(result.content, "done");

    let requests = client.requests();
    assert_eq!(requests.len(), 4);
    assert!(has_tool(&requests[0], SLOW));
    assert!(has_tool(&requests[1], SLOW));
    assert!(has_tool(&requests[2], SLOW));
    assert!(
        !has_tool(&requests[3], SLOW),
        "stream-level timeouts must drive the mask exactly like registry ones"
    );
    assert_eq!(nudge_count(&requests[3]), 1);
}
