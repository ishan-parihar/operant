//! A tool that ran and failed says so in the agent's own log.
//!
//! The refusal paths (tool not found, bad arguments, blocked by approval,
//! denied by seat policy) each log themselves. A tool that EXECUTED and
//! returned an error did not: the failure reached the model and the
//! operator, and nothing else — so the log of a turn whose tool failed
//! could not say which tool failed, how long it took, or why.
//!
//! The line under test is the one the loop writes when a tool result comes
//! back with `success: false`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex, OnceLock};
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

/// A tool that always fails.
struct FailingTool;

#[async_trait]
impl OperantTool for FailingTool {
    fn name(&self) -> &str {
        "fail_probe"
    }

    fn description(&self) -> &str {
        "Always fails (test fixture)."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "fail_probe",
            "Always fails (test fixture).",
            serde_json::json!({"type": "object", "properties": {}}),
        )
    }

    async fn execute(&self, _args: serde_json::Value, _context: ToolContext) -> ToolResult {
        ToolResult {
            tool_call_id: String::new(),
            name: "fail_probe".to_string(),
            success: false,
            content: String::new(),
            error: Some("the fixture's failure text".to_string()),
            timed_out: false,
        }
    }
}

struct ScriptedClient {
    steps: Mutex<Vec<ChatResponse>>,
}

#[async_trait]
impl ModelClient for ScriptedClient {
    fn provider_name(&self) -> &str {
        "scripted"
    }

    async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse> {
        let mut guard = self.steps.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_empty() {
            return Ok(text_response("done"));
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

// ── Tracing capture (the pattern from tests/loop_recovery_paths.rs) ──

#[derive(Clone)]
struct LogSink(Arc<Mutex<Vec<String>>>);

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

fn install_log_capture() -> Arc<Mutex<Vec<String>>> {
    static SINK: OnceLock<Arc<Mutex<Vec<String>>>> = OnceLock::new();
    SINK.get_or_init(|| {
        let sink = Arc::new(Mutex::new(Vec::new()));
        let _ = tracing::subscriber::set_global_default(
            tracing_subscriber::fmt()
                .with_writer(LogSink(Arc::clone(&sink)))
                .with_ansi(false)
                .with_max_level(tracing::Level::WARN)
                .finish(),
        );
        Arc::clone(&sink)
    })
    .clone()
}

#[tokio::test]
async fn a_failed_tool_is_logged_with_its_name_duration_and_error() {
    let capture = install_log_capture();
    let temp = tempfile::tempdir().unwrap();
    let client = Arc::new(ScriptedClient {
        steps: Mutex::new(vec![
            tool_call_response("fail_probe", "{}"),
            text_response("the tool failed, as scripted"),
        ]),
    });
    let registry = ToolRegistry::new(Duration::from_secs(5));
    registry.register(FailingTool).await.unwrap();
    let agent = OperantAgent::new(
        AgentConfig {
            advertised_skills_token_budget: 0,
            model: "demo-tool-failure".to_string(),
            max_iterations: 3,
            stream: false,
            context_window: 8_000,
            system_prompt: Some("You are a test agent.".to_string()),
            ..AgentConfig::default()
        },
        Box::new(ScriptedClientHandle(Arc::clone(&client))),
        registry,
        Arc::new(Database::init(temp.path().join("tool_failure.sqlite")).unwrap()),
    );

    agent
        .run("call the failing tool".to_string())
        .await
        .expect("the turn must complete");

    let lines = capture.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let failure = lines
        .iter()
        .find(|line| line.contains("Tool call failed"))
        .unwrap_or_else(|| panic!("the failed tool must be logged; captured lines: {lines:#?}"));
    assert!(
        failure.contains("fail_probe"),
        "the line must name the tool, got: {failure}"
    );
    assert!(
        failure.contains("the fixture's failure text"),
        "the line must carry the error, got: {failure}"
    );
    assert!(
        failure.contains("duration_ms"),
        "the line must carry the duration, got: {failure}"
    );
}
