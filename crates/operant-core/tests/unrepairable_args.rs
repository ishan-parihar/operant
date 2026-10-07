//! Scripted-model tests for the S4 unrepairable-args honesty fix — the
//! defect from `docs/AGENT-LOOP-FIX-EXECUTION-PLAN.md` where tool-call
//! arguments no repair could fix were silently substituted with `"{}"`
//! and executed anyway, so the model saw a guaranteed schema-validation
//! failure ("Missing required field …") instead of the truth, and retried
//! into a substitution cascade (measured specimen: 1,938 substitutions →
//! 522 validation failures → dedupe skips in one window).
//!
//! The contract after B2:
//!
//! - `repair_tool_call_arguments` returns `RepairOutcome::Unrepairable`
//!   for destroyed args (the Python-literal `None` token, structureless
//!   garbage); only genuinely repaired or already-valid args come back
//!   `Fixed`.
//! - On `Unrepairable`, `execute_tools` does NOT execute: the call is
//!   answered with ONE named `ToolResult::error_with_name` — "Arguments
//!   could not be parsed and were NOT executed. Reply with corrected
//!   arguments on the next turn…" — and never reaches phase 2.
//! - The refusal is per-CALL, not per-tool: the tool stays visible in
//!   every later request's schema list, so a corrected retry executes
//!   normally on the next iteration.
//!
//! Every case scripts the failure (the fault-injection rule) and every
//! refusal claim has a control proving the gate does not over-block:
//! parseable and repairable args must still execute through the same
//! path, and the same tool must recover on the very next turn. Revert
//! any piece of B2 and one of these fails on an exact-content assertion.

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

/// The two halves of the B2 refusal text — both must reach the model.
const REFUSAL_MARK: &str = "could not be parsed and were NOT executed";
const REPLY_MARK: &str = "Reply with corrected arguments on the next turn";
/// The pre-B2 cascade signature: `"{}"` executed → schema validation noise.
const MISSING_MARK: &str = "Missing required field";
const PROBE: &str = "unrepairable_probe";

/// Counts REAL executions. The S4 refusal must never reach it — the
/// pre-B2 code also never reached the tool impl (validation fired first),
/// so the transcript content, not the counter alone, is what separates
/// the two eras; the counter catches the other regression direction:
/// dummy `{}` args flowing all the way into a schemaless tool.
struct ProbeTool {
    entered: Arc<AtomicUsize>,
}

#[async_trait]
impl OperantTool for ProbeTool {
    fn name(&self) -> &str {
        PROBE
    }

    fn description(&self) -> &str {
        "Echoes its `action` arg; counts executions (drives the S4 gate)."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            PROBE,
            "Echoes its `action` arg; counts executions (drives the S4 gate).",
            serde_json::json!({
                "type": "object",
                "properties": {"action": {"type": "string"}},
                "required": ["action"],
            }),
        )
    }

    async fn execute(&self, args: serde_json::Value, _context: ToolContext) -> ToolResult {
        self.entered.fetch_add(1, Ordering::SeqCst);
        // (The registry stamps the real tool_call_id onto the result.)
        match args.get("action").and_then(|a| a.as_str()) {
            Some(action) => ToolResult::success("probe", format!("ran_action:{action}")),
            None => ToolResult::error("probe", "probe needs an action"),
        }
    }
}

/// What the agent actually sent the model: the schema list and the
/// message transcript. The refusal must appear HERE — the model only
/// learns from what lands in the transcript.
struct RecordedRequest {
    tools: Vec<String>,
    messages: Vec<Message>,
}

/// Scripted non-streaming client. Each call pops the next response and
/// records the full request so tests assert what the model was shown.
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
            .iter()
            .map(|r| RecordedRequest {
                tools: r.tools.clone(),
                messages: r.messages.clone(),
            })
            .collect()
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

fn tool_call_response(call_id: &str, arguments: &str) -> ChatResponse {
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
                    id: Some(call_id.to_string()),
                    call_type: Some("function".to_string()),
                    function: Some(ToolCallFunction {
                        name: PROBE.to_string(),
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

fn test_config(max_iterations: usize) -> AgentConfig {
    AgentConfig {
        model: "demo".to_string(),
        max_iterations,
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

/// Tool-role messages of one recorded request — the tool results the
/// model actually saw, in order.
fn tool_messages(req: &RecordedRequest) -> Vec<&Message> {
    req.messages
        .iter()
        .filter(|m| m.role == Role::Tool)
        .collect()
}

fn has_tool(req: &RecordedRequest, name: &str) -> bool {
    req.tools.iter().any(|t| t == name)
}

/// S4 cascade collapse: a scripted model emits the Python-literal `None`
/// token as arguments. Pre-B2 that was normalised to `"{}"`, executed,
/// and answered with "Missing required field: action" — the cascade's
/// first link. Now: ONE named not-executed error, the probe never runs,
/// and the very next iteration is a clean text turn (two model requests
/// total — no substitution loop to feed).
#[tokio::test]
async fn unrepairable_none_args_answered_once_never_executed() {
    let temp = tempfile::tempdir().unwrap();
    let client = Arc::new(ScriptedClient::new(vec![
        tool_call_response("call_none_1", "None"),
        text_response("recovered with text"),
    ]));
    let probe = Arc::new(ProbeTool {
        entered: Arc::new(AtomicUsize::new(0)),
    });
    let registry = ToolRegistry::new(Duration::from_secs(30));
    registry.register_dyn(probe.clone()).await.unwrap();
    let agent = build_agent(
        Arc::clone(&client),
        6,
        temp.path().join("unrepairable_none.sqlite"),
        registry,
    )
    .await;

    let result = agent
        .run("do some work".to_string())
        .await
        .expect("the refusal must end the turn normally, not as an error");

    // The turn completed on the model's text — one honest error, then a
    // clean iteration. This is the cascade collapse: exactly two model
    // requests, nothing left to substitute or dedupe.
    assert_eq!(result.content, "recovered with text");
    let requests = client.requests();
    assert_eq!(requests.len(), 2, "expected 2 model requests");

    // NEVER executed — no `{}`-shim execution, no validation round.
    assert_eq!(
        probe.entered.load(Ordering::SeqCst),
        0,
        "unrepairable args must never reach the tool impl"
    );

    // The model's next turn saw exactly ONE tool result: the named
    // not-executed error, both halves of the message.
    let tool_msgs = tool_messages(&requests[1]);
    assert_eq!(tool_msgs.len(), 1, "exactly one tool result expected");
    assert!(
        tool_msgs[0].content.contains(REFUSAL_MARK),
        "refusal text missing: {}",
        tool_msgs[0].content
    );
    assert!(
        tool_msgs[0].content.contains(REPLY_MARK),
        "corrective-instruction text missing: {}",
        tool_msgs[0].content
    );
    assert!(
        !tool_msgs[0].content.contains(MISSING_MARK),
        "the pre-S4 schema-validation round must be gone, got: {}",
        tool_msgs[0].content
    );

    // The refusal is per-CALL: the probe stays visible so a corrected
    // retry is possible on the next turn.
    assert!(
        has_tool(&requests[1], PROBE),
        "the refused tool must stay in the schema list"
    );
}

/// Positive control: a normal parseable call executes through the same
/// path — the S4 gate must not over-block.
#[tokio::test]
async fn parseable_args_execute_normally() {
    let temp = tempfile::tempdir().unwrap();
    let client = Arc::new(ScriptedClient::new(vec![
        tool_call_response("call_ok_1", r#"{"action":"pwd"}"#),
        text_response("all done"),
    ]));
    let probe = Arc::new(ProbeTool {
        entered: Arc::new(AtomicUsize::new(0)),
    });
    let registry = ToolRegistry::new(Duration::from_secs(30));
    registry.register_dyn(probe.clone()).await.unwrap();
    let agent = build_agent(
        Arc::clone(&client),
        6,
        temp.path().join("parseable_args.sqlite"),
        registry,
    )
    .await;

    let result = agent
        .run("do some work".to_string())
        .await
        .expect("normal call path must end the turn normally");
    assert_eq!(result.content, "all done");

    // It REALLY executed — the counter is the proof.
    assert_eq!(probe.entered.load(Ordering::SeqCst), 1);

    let requests = client.requests();
    assert_eq!(requests.len(), 2);
    let tool_msgs = tool_messages(&requests[1]);
    assert_eq!(tool_msgs.len(), 1);
    assert!(
        tool_msgs[0].content.contains("ran_action:pwd"),
        "probe output missing from transcript: {}",
        tool_msgs[0].content
    );
}

/// Repairable args (trailing comma — the iter-123 truncation family)
/// still take the Fixed path and execute with the repaired payload.
/// This is the over-block control for the repair half of the gate.
#[tokio::test]
async fn repairable_trailing_comma_args_execute_after_repair() {
    let temp = tempfile::tempdir().unwrap();
    let client = Arc::new(ScriptedClient::new(vec![
        tool_call_response("call_fix_1", r#"{"action":"pwd",}"#),
        text_response("repaired and done"),
    ]));
    let probe = Arc::new(ProbeTool {
        entered: Arc::new(AtomicUsize::new(0)),
    });
    let registry = ToolRegistry::new(Duration::from_secs(30));
    registry.register_dyn(probe.clone()).await.unwrap();
    let agent = build_agent(
        Arc::clone(&client),
        6,
        temp.path().join("repairable_args.sqlite"),
        registry,
    )
    .await;

    let result = agent
        .run("do some work".to_string())
        .await
        .expect("repaired-call path must end the turn normally");
    assert_eq!(result.content, "repaired and done");

    assert_eq!(
        probe.entered.load(Ordering::SeqCst),
        1,
        "repairable args must execute after repair"
    );

    let requests = client.requests();
    assert_eq!(requests.len(), 2);
    let tool_msgs = tool_messages(&requests[1]);
    assert_eq!(tool_msgs.len(), 1);
    assert!(
        tool_msgs[0].content.contains("ran_action:pwd"),
        "repaired payload must reach the tool: {}",
        tool_msgs[0].content
    );
    assert!(!tool_msgs[0].content.contains(REFUSAL_MARK));
}

/// The refusal's promise is real: the SAME tool, called again on the
/// very next iteration with corrected arguments, executes normally.
/// Pre-S4 the model was stuck in the validation-failure loop instead.
#[tokio::test]
async fn corrected_retry_after_refusal_executes() {
    let temp = tempfile::tempdir().unwrap();
    let client = Arc::new(ScriptedClient::new(vec![
        tool_call_response("call_none_1", "None"),
        tool_call_response("call_ok_2", r#"{"action":"pwd"}"#),
        text_response("recovered properly"),
    ]));
    let probe = Arc::new(ProbeTool {
        entered: Arc::new(AtomicUsize::new(0)),
    });
    let registry = ToolRegistry::new(Duration::from_secs(30));
    registry.register_dyn(probe.clone()).await.unwrap();
    let agent = build_agent(
        Arc::clone(&client),
        6,
        temp.path().join("corrected_retry.sqlite"),
        registry,
    )
    .await;

    let result = agent
        .run("do some work".to_string())
        .await
        .expect("recovery after refusal must end the turn normally");
    assert_eq!(result.content, "recovered properly");

    let requests = client.requests();

    // Only the corrected call executed — the refused one never did.
    assert_eq!(probe.entered.load(Ordering::SeqCst), 1);

    assert_eq!(requests.len(), 3);

    // First refusal: one honest error, no validation noise.
    let first = tool_messages(&requests[1]);
    assert_eq!(first.len(), 1);
    assert!(first[0].content.contains(REFUSAL_MARK));
    assert!(!first[0].content.contains(MISSING_MARK));

    // Then the corrected retry executes. Transcripts accumulate, so the
    // third request shows the refusal followed by the real result.
    let second = tool_messages(&requests[2]);
    assert_eq!(second.len(), 2, "refusal, then the corrected result");
    assert!(second[0].content.contains(REFUSAL_MARK));
    assert!(second[1].content.contains("ran_action:pwd"));
}
