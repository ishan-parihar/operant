//! Path test for the deterministic tool-loop anomaly retry.
//!
//! The unit tests in `turn_end_heuristics` prove the detectors in
//! isolation. This proves the WIRING: a turn whose only tool result is
//! empty re-enters the loop through the existing continuation shape
//! (an injected user `Message` + `continue`), and the model gets a
//! second chance instead of the turn ending on a broken observation.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::stream::BoxStream;

use operant_core::agent::{AgentConfig, OperantAgent};
use operant_core::agent::{ChatRequest, ModelClient, StreamChunk};
use operant_core::client::{
    ChatResponse, Choice, MessageDelta, Role, ToolCallDelta, ToolCallFunction, Usage,
};
use operant_core::database::Database;
use operant_core::error::{Error, Result};
use operant_core::schema::ToolSchema;
use operant_core::tools::{OperantTool, ToolContext, ToolRegistry, ToolResult};
use operant_core::turn_end_heuristics::ANOMALY_RETRY_PREFIX;

/// A tool that "succeeds" but returns nothing — the empty-result
/// anomaly the detector exists for.
struct EmptyResultTool;

#[async_trait]
impl OperantTool for EmptyResultTool {
    fn name(&self) -> &str {
        "empty_result"
    }

    fn description(&self) -> &str {
        "Returns an empty body on purpose (test fixture)."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "empty_result",
            "Returns an empty body on purpose (test fixture).",
            serde_json::json!({"type": "object", "properties": {}}),
        )
    }

    async fn execute(&self, _args: serde_json::Value, _context: ToolContext) -> ToolResult {
        ToolResult {
            tool_call_id: String::new(),
            name: "empty_result".to_string(),
            success: true,
            content: String::new(),
            error: None,
        }
    }
}

/// A tool that returns a clean, usable body — the control case.
struct OkResultTool;

#[async_trait]
impl OperantTool for OkResultTool {
    fn name(&self) -> &str {
        "ok_result"
    }

    fn description(&self) -> &str {
        "Returns a normal body (test fixture)."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "ok_result",
            "Returns a normal body (test fixture).",
            serde_json::json!({"type": "object", "properties": {}}),
        )
    }

    async fn execute(&self, _args: serde_json::Value, _context: ToolContext) -> ToolResult {
        ToolResult {
            tool_call_id: String::new(),
            name: "ok_result".to_string(),
            success: true,
            content: "42".to_string(),
            error: None,
        }
    }
}

/// Scripted non-streaming client. Each call pops the next response and
/// records the request messages so the test can assert what the model
/// actually saw.
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

    fn requests(&self) -> usize {
        self.seen.lock().unwrap().len()
    }

    fn messages_of(&self, index: usize) -> Vec<String> {
        self.seen.lock().unwrap()[index].clone()
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

fn tool_call_response(tool: &str) -> ChatResponse {
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
                    id: Some("call_1".to_string()),
                    call_type: Some("function".to_string()),
                    function: Some(ToolCallFunction {
                        name: tool.to_string(),
                        arguments: "{}".to_string(),
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

fn test_config() -> AgentConfig {
    AgentConfig {
        model: "demo".to_string(),
        max_iterations: 6,
        tool_timeout: Duration::from_secs(5),
        request_timeout: Duration::from_secs(10),
        system_prompt: Some("You are a test agent.".to_string()),
        // Non-streaming: the retry block lives on the shared response path,
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
    db_path: std::path::PathBuf,
) -> OperantAgent {
    let registry = ToolRegistry::new(Duration::from_secs(5));
    registry.register(tool).await.unwrap();
    OperantAgent::new(
        test_config(),
        Box::new(ScriptedClientHandle(client)),
        registry,
        Arc::new(Database::init(db_path).unwrap()),
    )
}

/// Lets the shared [`Arc<ScriptedClient>`` survive the `Box<dyn ModelClient>`
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

#[tokio::test]
async fn empty_tool_result_re_enters_the_loop_with_feedback() {
    let temp = tempfile::tempdir().unwrap();
    let client = Arc::new(ScriptedClient::new(vec![
        tool_call_response("empty_result"),
        text_response("first attempt, built on an empty observation"),
        text_response("corrected answer after re-running the tool"),
    ]));
    let agent = build_agent(
        Arc::clone(&client),
        EmptyResultTool,
        temp.path().join("anomaly_retry.sqlite"),
    )
    .await;

    let result = agent.run("what is the answer?".to_string()).await.unwrap();

    // The model got a THIRD turn: tool call → answer (anomaly detected) →
    // feedback injected → answer. Without the retry this would have
    // returned the second response.
    assert_eq!(client.requests(), 3, "the turn must re-enter the loop");
    assert_eq!(
        result.content, "corrected answer after re-running the tool",
        "the turn must end on the post-retry response"
    );

    // The feedback reached the model as a user message on the retry —
    // the same injection shape the truncation continuation uses.
    let third = client.messages_of(2);
    assert!(
        third
            .iter()
            .any(|m| m.trim_start().starts_with(ANOMALY_RETRY_PREFIX)),
        "the retry request must carry the anomaly feedback message, got: {third:?}"
    );
    assert!(
        third.iter().any(|m| m.contains("attempt 1/3")),
        "the feedback must state the capped attempt number"
    );
}

#[tokio::test]
async fn clean_tool_result_does_not_re_enter_the_loop() {
    let temp = tempfile::tempdir().unwrap();
    let client = Arc::new(ScriptedClient::new(vec![
        tool_call_response("ok_result"),
        text_response("the answer is 42"),
    ]));
    let agent = build_agent(
        Arc::clone(&client),
        OkResultTool,
        temp.path().join("anomaly_clean.sqlite"),
    )
    .await;

    let result = agent.run("what is the answer?".to_string()).await.unwrap();

    assert_eq!(result.content, "the answer is 42");
    assert_eq!(
        client.requests(),
        2,
        "a clean tool result must not cost an extra turn"
    );
    let second = client.messages_of(1);
    assert!(
        !second
            .iter()
            .any(|m| m.trim_start().starts_with(ANOMALY_RETRY_PREFIX)),
        "no feedback message should be injected on a clean turn"
    );
}
