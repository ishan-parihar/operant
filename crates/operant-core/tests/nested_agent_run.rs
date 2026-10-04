//! Live nested sub-agent run — the child spends its OWN budget and finishes
//! on its OWN deadline.
//!
//! `sub_agent_tool.rs` already unit-tests the two STATIC properties of the
//! delegation seam: the spawn guard refuses a too-deep child before a worker
//! is built, and the worker timeout is clamped to a floor. Neither proves the
//! thing that matters at runtime — that a child which runs a real
//! `OperantAgent` loop spends its own iteration budget and finishes on its own
//! deadline, without charging a single iteration to the parent that delegated
//! to it.
//!
//! `SubAgentTool::run_child` builds the child from scratch: its own
//! `AgentConfig` (`max_iterations` from the delegation args, default 50), its
//! own `ToolRegistry`, and its own `OperantAgent`, whose `IterationBudget` is
//! constructed from that config (`agent/builders.rs:93`). A scripted
//! `ModelClient` on the PARENT therefore cannot observe the child's turns —
//! the child talks to a real `OpenAIModelClient` over the parent's HTTP client
//! (`sub_agent_tool.rs:641-667`). So the child's model is a loopback HTTP
//! server here, and every child-side assertion is made from the child's own
//! recorded request bodies.
//!
//! ## The asymmetry that makes this test able to fail
//!
//! The parent runs with `max_iterations: 8`; the delegation passes
//! `max_iterations: 1`. A child that honours its own budget makes exactly
//! TWO requests: one tool-calling iteration, then the toolless grace call that
//! `run.rs:371` issues once `IterationBudget::consume` refuses. A child that
//! shared the parent's budget — or silently fell back to the default 50 —
//! would keep sending tool-BEARING requests, so the assertions test the
//! `tools` key of each child request body, not only a count: the grace call is
//! the only shape with no `tools` array. Shared budget therefore fails on the
//! request count, on the tool-bearing count, and on the poison string a third
//! request would be answered with.
//!
//! ## Harness
//!
//! Mirrors `tests/circuit_breaker_abort.rs` and `tests/turn_anomaly_retry.rs`
//! for the parent — a scripted `ModelClient` recording
//! `(role, content, tool_call_id)` per request — and `client.rs`'s own
//! `embeddings_server` for the child: a raw loopback HTTP server that records
//! request bodies and answers them from a fixed script.
//!
//! ## Interacting paths
//!
//! - The parent's registry has a 5s tool timeout, far shorter than the child's
//!   own 30s delegation deadline. The child completing proves its deadline came
//!   from the delegation args rather than the parent's per-tool timeout.
//! - `MAX_SPAWN_DEPTH` defaults to 1 and the tool is registered at
//!   `parent_depth: 0`, so the child lands at depth 1 and is admitted — the
//!   depth refusal path is unit-covered, not re-tested here.
//! - A leaf child inherits the `builtin` toolset, so the child's scripted tool
//!   call targets the real `datetime` tool. Its nondeterministic result body is
//!   never asserted on.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::stream::BoxStream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use operant_core::agent::StreamChunk;
use operant_core::agent::{AgentConfig, AgentEvent, ChatRequest, ModelClient, OperantAgent};
use operant_core::client::{
    ChatResponse, Choice, ClientConfig, MessageDelta, OpenAIClient, Role, ToolCallDelta,
    ToolCallFunction, Usage,
};
use operant_core::database::Database;
use operant_core::error::{Error, Result};
use operant_core::tools::ToolRegistry;
use operant_core::tools::sub_agent_tool::SubAgentTool;

/// Appears only inside the CHILD's conversation (its system prompt). If it
/// ever shows up in a parent request, the child leaked into the parent.
const CHILD_GOAL_MARKER: &str = "CHILD_GOAL_MARKER";

/// The child has a fresh conversation, so this can only ever come from
/// `build_child_system_prompt`.
const CHILD_PREAMBLE_MARKER: &str = "You are a focused subagent";

/// The text the child's single grace call returns. Reaches the parent as the
/// body of ONE `delegate_task` tool result.
const CHILD_ANSWER_MARKER: &str = "CHILD_ANSWER_MARKER";

/// A child that ran past its own budget gets this. It must never reach the
/// parent.
const POISON_MARKER: &str = "CHILD_OVERRAN_ITS_OWN_BUDGET";

const PARENT_SYSTEM_PROMPT: &str = "You are the parent test agent.";

// ── Parent: scripted ModelClient ──────────────────────────────────

/// One recorded message from a parent request, reduced to the fields the
/// assertions need.
#[derive(Debug, Clone, PartialEq)]
struct Seen {
    role: Role,
    content: String,
    tool_call_id: Option<String>,
}

struct ScriptedClient {
    responses: Mutex<Vec<ChatResponse>>,
    seen: Mutex<Vec<Vec<Seen>>>,
}

impl ScriptedClient {
    fn new(responses: Vec<ChatResponse>) -> Self {
        Self {
            responses: Mutex::new(responses),
            seen: Mutex::new(Vec::new()),
        }
    }

    /// How many times the parent's loop asked its model for a response.
    fn requests(&self) -> usize {
        self.seen.lock().unwrap().len()
    }

    fn messages_of(&self, index: usize) -> Vec<Seen> {
        self.seen.lock().unwrap()[index].clone()
    }

    /// True when ANY message in ANY parent request contained `fragment`.
    fn saw(&self, fragment: &str) -> bool {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .flatten()
            .any(|m| m.content.contains(fragment))
    }
}

#[async_trait]
impl ModelClient for ScriptedClient {
    fn provider_name(&self) -> &str {
        "scripted-parent"
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
        self.seen.lock().unwrap().push(
            request
                .messages
                .iter()
                .map(|m| Seen {
                    role: m.role.clone(),
                    content: m.content.clone(),
                    tool_call_id: m.tool_call_id.clone(),
                })
                .collect(),
        );
        let mut guard = self.responses.lock().unwrap();
        if guard.is_empty() {
            return Ok(text_response("parent: script exhausted"));
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

/// Lets the shared `Arc<ScriptedClient>` survive the `Box<dyn ModelClient>`
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

// ── Child: loopback OpenAI-compatible endpoint ────────────────────

/// The child's model endpoint. Records every request body it is asked to
/// answer, so the child's iteration accounting is observable from outside.
struct MockChildModel {
    base_url: String,
    seen: Arc<Mutex<Vec<String>>>,
}

impl MockChildModel {
    async fn start(script: Vec<String>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let script = Arc::new(Mutex::new(script));

        let seen_task = Arc::clone(&seen);
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    break;
                };
                let Some((head, body)) = read_http_request(&mut sock).await else {
                    continue;
                };
                seen_task.lock().unwrap().push(body);

                // Insurance: nothing in this test should ask for embeddings,
                // but a wrong-shaped reply here would surface as an opaque
                // parse error rather than an isolation failure.
                let reply = if head.contains("/embeddings") {
                    r#"{"data":[{"index":0,"embedding":[0.1,0.2,0.3]}]}"#.to_string()
                } else {
                    let mut guard = script.lock().unwrap();
                    if guard.is_empty() {
                        text_body(POISON_MARKER)
                    } else {
                        guard.remove(0)
                    }
                };

                let head = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    reply.len()
                );
                let _ = sock.write_all(head.as_bytes()).await;
                let _ = sock.write_all(reply.as_bytes()).await;
                let _ = sock.shutdown().await;
            }
        });

        Self {
            base_url: format!("http://{addr}"),
            seen,
        }
    }

    /// Every request body the child agent sent, in order.
    fn requests(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn content_length(head: &str) -> usize {
    head.lines()
        .find(|line| line.to_ascii_lowercase().starts_with("content-length:"))
        .and_then(|line| line.split(':').nth(1))
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0)
}

/// Read one HTTP request: headers, then exactly `Content-Length` body bytes.
async fn read_http_request(sock: &mut tokio::net::TcpStream) -> Option<(String, String)> {
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8192];

    let body_start = loop {
        let n = sock.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = find_subslice(&buf, b"\r\n\r\n") {
            break pos + 4;
        }
        if buf.len() > 1 << 20 {
            return None;
        }
    };

    let head = String::from_utf8_lossy(&buf[..body_start - 4]).to_string();
    let body_len = content_length(&head);
    while buf.len() < body_start + body_len {
        let n = sock.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    }

    let body = String::from_utf8_lossy(&buf[body_start..body_start + body_len]).to_string();
    Some((head, body))
}

/// A request carried a non-empty `tools` array. The grace call
/// (`run.rs::attempt_grace_call`) builds a bare `ChatRequest`, so it is the
/// only child request shape this returns false for.
fn carried_tools(body: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| value.get("tools").cloned())
        .is_some_and(|tools| tools.as_array().is_some_and(|list| !list.is_empty()))
}

// ── Response builders ────────────────────────────────────────────

fn text_response(content: &str) -> ChatResponse {
    ChatResponse {
        id: "parent-resp".to_string(),
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

fn tool_call_response(tool: &str, call_id: &str, arguments: &str) -> ChatResponse {
    ChatResponse {
        id: "parent-resp".to_string(),
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

fn text_body(content: &str) -> String {
    serde_json::json!({
        "id": "child-resp",
        "object": "chat.completion",
        "created": 0,
        "model": "demo",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": content},
            "finish_reason": "stop",
        }],
        "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2},
    })
    .to_string()
}

fn tool_call_body(tool: &str, call_id: &str, arguments: &str) -> String {
    serde_json::json!({
        "id": "child-resp",
        "object": "chat.completion",
        "created": 0,
        "model": "demo",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "index": 0,
                    "id": call_id,
                    "type": "function",
                    "function": {"name": tool, "arguments": arguments},
                }],
            },
            "finish_reason": "tool_calls",
        }],
        "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2},
    })
    .to_string()
}

// ── Parent config ────────────────────────────────────────────────

/// The parent's budget is deliberately 8x the child's, so a child that read
/// the parent's `max_iterations` instead of its own delegation argument would
/// run visibly further.
const PARENT_MAX_ITERATIONS: usize = 8;

fn test_config() -> AgentConfig {
    AgentConfig {
        model: "demo".to_string(),
        max_iterations: PARENT_MAX_ITERATIONS,
        // 5s per tool call: shorter than the child's own 30s deadline, so the
        // child completing proves it did not inherit the parent's tool timeout.
        tool_timeout: Duration::from_secs(5),
        request_timeout: Duration::from_secs(10),
        system_prompt: Some(PARENT_SYSTEM_PROMPT.to_string()),
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

// ── The test ─────────────────────────────────────────────────────

#[tokio::test]
async fn nested_sub_agent_runs_on_its_own_budget_and_timeout() {
    let temp = tempfile::tempdir().unwrap();
    let child_model = Arc::new(
        MockChildModel::start(vec![
            // Child iteration 1: one real tool call against the child's
            // inherited `builtin` registry.
            tool_call_body("datetime", "call_child_tool_1", "{}"),
            // The toolless grace call the child makes when its OWN budget of
            // 1 is exhausted.
            text_body(&format!(
                "{CHILD_ANSWER_MARKER}: one iteration, then out of budget"
            )),
        ])
        .await,
    );

    // The parent's client_config is what SubAgentTool clones for the child, so
    // pointing it at the loopback server is what wires the child's model.
    let parent_http = OpenAIClient::new(ClientConfig {
        base_url: child_model.base_url.clone(),
        api_key: Some("test-key".to_string()),
        timeout: Duration::from_secs(10),
        ..Default::default()
    });

    let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(16);
    let database = Arc::new(Database::init(temp.path().join("nested_agent_run.sqlite")).unwrap());

    // 60s registry (delegation) deadline: the child needs ≈2 LLM turns + 1 tool
    // call (observed ≥11s under load).  This is the ceiling for the delegate
    // tool call itself — distinct from `config.tool_timeout = 5s` above, whose
    // non-inheritance into the child's tools is the actual subject under test.
    let registry = ToolRegistry::new(Duration::from_secs(60));
    registry
        .register(SubAgentTool::with_parent_tool_policy(
            &parent_http,
            "demo",
            0,
            Vec::new(),
            Default::default(),
            Default::default(),
            Arc::clone(&database),
            Some(event_tx),
        ))
        .await
        .unwrap();

    // Parent script: delegate once (child told max_iterations 1, timeout 30s),
    // then answer.
    let client = Arc::new(ScriptedClient::new(vec![
        tool_call_response(
            "delegate_task",
            "call_delegate_1",
            &format!(
                r#"{{"goal":"confirm {CHILD_GOAL_MARKER} stays in the child","max_iterations":1,"timeout_seconds":30}}"#
            ),
        ),
        text_response("parent: delegation returned, turn complete"),
    ]));

    let agent = OperantAgent::new(
        test_config(),
        Box::new(ScriptedClientHandle(Arc::clone(&client))),
        registry,
        database,
    );

    let result = agent
        .run("delegate this, then report back".to_string())
        .await
        .expect("the parent turn must complete, not error");

    // ── Parent completed on its own budget ──────────────────────
    assert_eq!(
        result.content, "parent: delegation returned, turn complete",
        "the parent must finish on its own script, got: {result:?}"
    );
    assert_eq!(
        client.requests(),
        2,
        "the parent must spend exactly two iterations: delegate, then answer"
    );

    // ── The child stopped on its OWN limit, not the parent's ────
    let child_requests = child_model.requests();
    let tool_bearing = child_requests.iter().filter(|b| carried_tools(b)).count();

    assert_eq!(
        child_requests.len(),
        2,
        "the child must run exactly one tool iteration plus one grace call. \
         Its delegation argument was max_iterations=1, so any other count means \
         it read a different budget (shared={PARENT_MAX_ITERATIONS}, default=50). \
         Recorded: {child_requests:#?}"
    );
    assert_eq!(
        tool_bearing, 1,
        "only the child's first request may carry a `tools` array; the second \
         must be the toolless grace call that exhaustion triggers. A child \
         sharing the parent's budget would send tools again. Recorded: {child_requests:#?}"
    );
    assert!(
        !carried_tools(&child_requests[1]),
        "the child's second request is its grace call and must have no tools"
    );

    // ── The child's own script ran, and its answer came home ────
    let child_first = &child_requests[0];
    assert!(
        child_first.contains(CHILD_GOAL_MARKER) && child_first.contains(CHILD_PREAMBLE_MARKER),
        "the child's own conversation must carry its goal and subagent system \
         prompt, proving a real nested agent ran: {child_first}"
    );
    assert!(
        !child_requests[1].contains(CHILD_ANSWER_MARKER),
        "the child's second request is asked to summarise, not re-plan"
    );

    // The grace call's text reached the parent as the delegation's result.
    let parent_second = client.messages_of(1);
    let tool_results: Vec<&Seen> = parent_second
        .iter()
        .filter(|m| m.role == Role::Tool)
        .collect();
    assert_eq!(
        tool_results.len(),
        1,
        "the parent must see exactly one tool result — the delegation's own. \
         Got: {parent_second:?}"
    );
    assert_eq!(
        tool_results[0].tool_call_id.as_deref(),
        Some("call_delegate_1"),
        "the single tool result must be the parent's own delegate_task call"
    );
    assert!(
        tool_results[0].content.contains(CHILD_ANSWER_MARKER),
        "the child's answer must be the delegation's payload, got: {}",
        tool_results[0].content
    );
    assert!(
        !tool_results[0].content.contains(POISON_MARKER),
        "the child must never run past its own budget, got: {}",
        tool_results[0].content
    );

    // ── Budget isolation: the child's turns never touched the parent ──
    let child_tool_ids = ["call_child_tool_1"];
    for message in client.messages_of(0).iter().chain(parent_second.iter()) {
        assert!(
            !child_tool_ids.contains(&message.tool_call_id.as_deref().unwrap_or("")),
            "a child tool call entered the parent's conversation: {message:?}"
        );
    }
    assert!(
        !client.saw(CHILD_GOAL_MARKER),
        "the child's goal must not appear in any parent request — the child has \
         a fresh conversation"
    );
    assert!(
        !client.saw(CHILD_PREAMBLE_MARKER),
        "the child's subagent system prompt must not appear in any parent request"
    );
    assert!(
        !client.saw(POISON_MARKER),
        "the poison reply must never surface in the parent"
    );

    // ── Timeout isolation: the child finished on its own deadline ──
    let mut events = Vec::new();
    while let Ok(event) = event_rx.try_recv() {
        events.push(event);
    }
    assert_eq!(
        events.len(),
        2,
        "one SubagentStarted and one SubagentStopped, got: {events:?}"
    );
    match (&events[0], &events[1]) {
        (
            AgentEvent::SubagentStarted {
                subagent_id,
                role,
                depth,
            },
            AgentEvent::SubagentStopped {
                subagent_id: stop_id,
                status,
                summary,
            },
        ) => {
            assert_eq!(*depth, 1, "the child must be spawned at depth 1");
            assert_eq!(role, "leaf", "the default role is a leaf child");
            assert_eq!(
                subagent_id, stop_id,
                "start and stop must pair on the same sub-agent id"
            );
            assert_eq!(
                status, "completed",
                "the child must finish inside its own 30s deadline, not time out"
            );
            assert!(
                summary.contains(CHILD_ANSWER_MARKER),
                "the stop summary is the child's own answer, got: {summary}"
            );
        }
        (started, stopped) => {
            panic!("expected a subagent start/stop pair, got {started:?} / {stopped:?}")
        }
    }
}
