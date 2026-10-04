//! Scripted-model tests for the S1 grace-output quality gate — the defect
//! from `docs/AGENT-LOOP-FIX-EXECUTION-PLAN.md` where a budget-exhausted
//! turn ends on `attempt_grace_call` (run.rs `Ok((text, _))` arm) and the
//! model's answer was returned UNVALIDATED: a 13-char reasoning-leak
//! artifact (`]<]\u{200b}minimax[>[`) — a provider's leaked internal
//! markers, not prose — counted as "answered" and was relayed to the
//! operator verbatim.
//!
//! The gate is `turn_rules::is_degenerate_final_text`: when it fires, the
//! grace arm returns an EMPTY assistant message, and one layer up
//! `gateway_runner.rs:738-762` substitutes the user-facing stopped notice
//! for empty content. So the two terminal shapes this file pins are:
//!
//! - **Degenerate grace output** → `Ok(<empty message>)` — the exact shape
//!   the existing empty-content fallback already handles (same shape as the
//!   exhausted empty ladder pinned in `tests/loop_recovery_paths.rs`).
//! - **Valid grace output** → `Ok(<model's summary>)`, verbatim — the
//!   positive control. A gate test that cannot pass on valid input measures
//!   nothing (2026-09-29 rule); this case fails if the gate over-blocks.
//!
//! Both scripts are the circuit_breaker_abort exhaustion shape: three
//! failing tool iterations against `max_iterations: 3`, then the toolless
//! grace call — so the grace branch is really reached (asserted through the
//! request count, not assumed).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::atomic::{AtomicUsize, Ordering};
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

/// The observed production artifact class: a 13-char reasoning-leak marker
/// string with a zero-width space, no whitespace, not prose.
const GARBAGE_GRACE: &str = "]<]\u{200b}minimax[>[";

/// A grace summary that must pass the gate: under 80 chars but real prose.
const VALID_GRACE: &str = "summary of partial work: 2 of 3 checks done";

/// A tool that always FAILS, so every loop iteration ends on a tool result
/// and the budget — not a text answer — terminates the turn.
struct FailingTool;

#[async_trait]
impl OperantTool for FailingTool {
    fn name(&self) -> &str {
        "fail_probe"
    }

    fn description(&self) -> &str {
        "Always fails (drives the budget-exhaustion grace path)."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "fail_probe",
            "Always fails (drives the budget-exhaustion grace path).",
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
/// counts requests so a test can assert the grace call was really made.
struct ScriptedClient {
    responses: Mutex<Vec<ChatResponse>>,
    seen: AtomicUsize,
}

impl ScriptedClient {
    fn new(responses: Vec<ChatResponse>) -> Self {
        Self {
            responses: Mutex::new(responses),
            seen: AtomicUsize::new(0),
        }
    }

    /// How many times the loop asked the model for a response.
    fn requests(&self) -> usize {
        self.seen.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl ModelClient for ScriptedClient {
    fn provider_name(&self) -> &str {
        "scripted"
    }

    async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse> {
        self.seen.fetch_add(1, Ordering::Relaxed);
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

/// `n` calls whose ARGUMENTS differ on every iteration, so neither the R35
/// identical-call breaker nor the degenerate-breaker abort threshold fires
/// before the budget ceiling — the turn ends on the grace call.
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
        // Non-streaming: the grace gate sits on the shared response path,
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

async fn build_agent(
    client: Arc<ScriptedClient>,
    max_iterations: usize,
    db_path: std::path::PathBuf,
) -> OperantAgent {
    let registry = ToolRegistry::new(Duration::from_secs(5));
    registry.register(FailingTool).await.unwrap();
    OperantAgent::new(
        test_config(max_iterations),
        Box::new(ScriptedClientHandle(client)),
        registry,
        Arc::new(Database::init(db_path).unwrap()),
    )
}

/// Lets the shared [`Arc<ScriptedClient>`] survive the `Box<dyn ModelClient>`
/// hand-off so the test can inspect the request count afterwards.
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

/// Fault-injected case: the budget runs out and the grace call returns the
/// observed reasoning-leak garbage. The gate must drop it: the turn's
/// returned message is EMPTY, which is the shape the gateway's existing
/// fallback substitutes a user-facing stopped notice for. Remove the gate
/// (return `Message::assistant(&text)` unconditionally) and this test fails
/// on the exact-content assertion.
#[tokio::test]
async fn degenerate_grace_output_returns_empty_message() {
    let temp = tempfile::tempdir().unwrap();
    let mut responses = varying_calls("fail_probe", 3);
    responses.push(text_response(GARBAGE_GRACE));
    let client = Arc::new(ScriptedClient::new(responses));
    let agent = build_agent(
        Arc::clone(&client),
        3,
        temp.path().join("grace_gate.sqlite"),
    )
    .await;

    let result = agent
        .run("do some work".to_string())
        .await
        .expect("a successful grace call still returns Ok — only the body is gated");

    assert_eq!(
        result.content, "",
        "degenerate grace output must become the empty assistant message, got: {result:?}"
    );

    // 3 tool-calling turns + 1 toolless grace call: the grace branch was
    // really reached, not bypassed into some other terminal shape.
    assert_eq!(client.requests(), 4, "the grace call costs one extra turn");
}

/// Positive control: same exhaustion, but the grace call returns a real
/// summary. The gate must pass it through VERBATIM — otherwise the gate is
/// not discriminating garbage from answers, only blocking everything.
#[tokio::test]
async fn valid_grace_output_passes_the_gate_verbatim() {
    let temp = tempfile::tempdir().unwrap();
    let mut responses = varying_calls("fail_probe", 3);
    responses.push(text_response(VALID_GRACE));
    let client = Arc::new(ScriptedClient::new(responses));
    let agent = build_agent(
        Arc::clone(&client),
        3,
        temp.path().join("grace_control.sqlite"),
    )
    .await;

    let result = agent
        .run("do some work".to_string())
        .await
        .expect("a successful grace call returns Ok");

    assert_eq!(result.content, VALID_GRACE);
    assert_eq!(client.requests(), 4, "the grace call costs one extra turn");
}
