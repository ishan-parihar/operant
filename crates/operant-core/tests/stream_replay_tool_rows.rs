//! A replayed stream must re-announce its tool calls.
//!
//! When a stream dies mid-read the loop re-issues the request. The replayed
//! stream re-emits every ToolStart the dropped one announced, under the SAME
//! call ids — and the agent's dedup set (which exists so one call is not
//! announced twice WITHIN a stream) still held those ids. The replay's
//! ToolStart was therefore suppressed as a duplicate, and the recovered turn
//! ran its tools invisibly: no row for a call the operator never saw, and
//! for a call that HAD been announced, a row frozen with the truncated
//! arguments of the dropped attempt.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use futures::{StreamExt, stream::BoxStream};

use operant_core::agent::{
    AgentConfig, AgentEvent, ChatRequest, ModelClient, OperantAgent, StreamChunk,
};
use operant_core::client::ChatResponse;
use operant_core::database::Database;
use operant_core::error::{Error, Result};
use operant_core::schema::ToolSchema;
use operant_core::tools::{OperantTool, ToolContext, ToolRegistry, ToolResult};

/// A tool that always succeeds — the point of the test is the ROW, not the
/// tool's behaviour.
struct ProbeTool;

#[async_trait]
impl OperantTool for ProbeTool {
    fn name(&self) -> &str {
        "probe"
    }

    fn description(&self) -> &str {
        "Always succeeds (test fixture)."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "probe",
            "Always succeeds (test fixture).",
            serde_json::json!({"type": "object", "properties": {}}),
        )
    }

    async fn execute(&self, _args: serde_json::Value, _context: ToolContext) -> ToolResult {
        ToolResult::success("probe", serde_json::json!({"ok": true}))
    }
}

/// The first attempt announces the call and then the stream dies; the second
/// (the replay) announces the SAME call again and then answers.
struct DroppingStreamClient {
    attempts: AtomicUsize,
}

impl DroppingStreamClient {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            attempts: AtomicUsize::new(0),
        })
    }
}

fn tool_call_chunk() -> StreamChunk {
    let mut chunk = StreamChunk::new(None, None, None);
    chunk.tool_calls = Some(vec![operant_core::client::ToolCall {
        id: "call_probe".to_string(),
        function: operant_core::client::ToolCallFunction {
            name: "probe".to_string(),
            arguments: "{}".to_string(),
        },
    }]);
    chunk
}

#[async_trait]
impl ModelClient for DroppingStreamClient {
    fn provider_name(&self) -> &str {
        "dropping-stream"
    }

    async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse> {
        Err(Error::Agent(
            "this fixture serves the streaming path only".into(),
        ))
    }

    async fn chat_streaming(
        &self,
        _request: ChatRequest,
    ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
        let attempt = self.attempts.fetch_add(1, Ordering::SeqCst);
        if attempt == 0 {
            // The call is announced, then the stream dies mid-read.
            Ok(futures::stream::iter(vec![
                Ok(tool_call_chunk()),
                Err(Error::Provider {
                    status: 503,
                    body: "upstream unavailable".to_string(),
                    retry_after: None,
                }),
            ])
            .boxed())
        } else if attempt == 1 {
            // The replay: the same call, announced again.
            Ok(futures::stream::iter(vec![Ok(tool_call_chunk())]).boxed())
        } else {
            // The turn's next request: the answer.
            Ok(futures::stream::iter(vec![Ok(StreamChunk::new(
                Some("done".to_string()),
                None,
                None,
            ))])
            .boxed())
        }
    }
}

struct ClientHandle(Arc<DroppingStreamClient>);

#[async_trait]
impl ModelClient for ClientHandle {
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
async fn a_replayed_stream_re_announces_its_tool_call() {
    let temp = tempfile::tempdir().unwrap();
    let client = DroppingStreamClient::new();
    let registry = ToolRegistry::new(Duration::from_secs(5));
    registry.register(ProbeTool).await.unwrap();
    let (event_tx, mut event_rx) = tokio::sync::mpsc::channel::<AgentEvent>(64);
    let agent = OperantAgent::with_events(
        AgentConfig {
            advertised_skills_token_budget: 0,
            memory_injection_budget_tokens: 0,
            model: "demo-replay".to_string(),
            max_iterations: 4,
            stream: true,
            context_window: 8_000,
            system_prompt: Some("You are a test agent.".to_string()),
            ..AgentConfig::default()
        },
        Box::new(ClientHandle(Arc::clone(&client))),
        registry,
        Arc::new(Database::init(temp.path().join("replay.sqlite")).unwrap()),
        event_tx,
    );

    let result = agent
        .run("use the probe".to_string())
        .await
        .expect("the replay must recover the turn");

    assert_eq!(
        result.content, "done",
        "the turn must end on the replay's answer"
    );
    assert_eq!(
        client.attempts.load(Ordering::SeqCst),
        3,
        "the dropped stream, its replay, and the request that carried the answer"
    );

    let mut tool_starts = 0usize;
    while let Ok(event) = event_rx.try_recv() {
        if matches!(event, AgentEvent::ToolStart { .. }) {
            tool_starts += 1;
        }
    }
    // THREE, not two: the dropped attempt's streaming announcement, the
    // REPLAY's streaming announcement (the one the fix restores), and the
    // execute-time re-announcement that flips the row from Queued to
    // Running. Without the fix the replay's announcement is suppressed as a
    // duplicate of the dropped attempt's, and the count is two — the
    // recovered turn ran its tool with no streaming row at all.
    assert_eq!(
        tool_starts, 3,
        "the replay must re-announce the call — the dropped attempt's \
         announcement, the replay's, and the execute-time re-announcement"
    );
}
