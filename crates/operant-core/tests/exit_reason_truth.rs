//! Scripted-model tests for the S5 exit-truth fix — the defect from
//! `docs/AGENT-LOOP-FIX-EXECUTION-PLAN.md` §1 A2 where a turn that ended
//! early (grace call after budget/wall-clock exhaustion, or a circuit-
//! breaker abort) was indistinguishable from a normal answer: the loop
//! never stamped the real reason, and the gateway relayed a degenerate
//! partial as if it were the final answer (the 25-day silence-after-
//! degenerate-warning specimen).
//!
//! The fix (`AgentEvent::Done { message, reason }` + `TurnExitReason`
//! gained `GraceCall`/`CircuitBreaker`) is pinned here on BOTH observable
//! surfaces:
//!
//! 1. the `Done` event itself (what the gateway consumes), via a real
//!    `OperantAgent` built with `OperantAgent::with_events`;
//! 2. the `TurnDiagnostics` line (`Turn ended: reason=…`), via the same
//!    process-global tracing sink `tests/loop_recovery_paths.rs` uses.
//!
//! Three tests — a fault-injected grace garbage turn, a scripted
//! circuit-breaker turn, and the positive control (a normal text turn
//! still reports `text_response` and delivers the answer unmodified —
//! without it a "stamp everything non-normal" bug would pass here while
//! lying in the other direction).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use futures::stream::BoxStream;

use operant_core::agent::{
    AgentConfig, AgentEvent, ChatRequest, ModelClient, OperantAgent, StreamChunk, TurnExitReason,
};
use operant_core::client::{
    ChatResponse, Choice, MessageDelta, Role, ToolCallDelta, ToolCallFunction, Usage,
};
use operant_core::database::Database;
use operant_core::error::{Error, Result};
use operant_core::schema::ToolSchema;
use operant_core::tools::{OperantTool, ToolContext, ToolRegistry, ToolResult};

// ── Scripted model ──────────────────────────────────────────────

struct ScriptedClient {
    responses: Mutex<Vec<ChatResponse>>,
    requests: Mutex<usize>,
}

impl ScriptedClient {
    fn new(responses: Vec<ChatResponse>) -> Self {
        Self {
            responses: Mutex::new(responses),
            requests: Mutex::new(0),
        }
    }

    fn request_count(&self) -> usize {
        *self.requests.lock().unwrap()
    }
}

#[async_trait]
impl ModelClient for ScriptedClient {
    fn provider_name(&self) -> &str {
        "scripted"
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
        drop(request);
        *self.requests.lock().unwrap() += 1;
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

/// A tool that always succeeds — the R35 breaker exists precisely because
/// identical calls can keep SUCCEEDING; without a succeeding tool the
/// script would trip the failure breaker first (mirrors
/// `tests/circuit_breaker_abort.rs`).
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
            content: "ok".to_string(),
            error: None,
        }
    }
}

// ── Tracing capture (mirrors tests/loop_recovery_paths.rs) ──────

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

/// Process-global sink; every test shares the one installation (same
/// contract as `tests/loop_recovery_paths.rs`).
fn install_log_capture() -> Arc<Mutex<Vec<String>>> {
    static SINK: OnceLock<Arc<Mutex<Vec<String>>>> = OnceLock::new();
    SINK.get_or_init(|| {
        let sink = Arc::new(Mutex::new(Vec::new()));
        let _ = tracing::subscriber::set_global_default(
            tracing_subscriber::fmt()
                .with_writer(LogSink(Arc::clone(&sink)))
                .finish(),
        );
        sink
    })
    .clone()
}

fn test_config(model: &str, max_iterations: usize) -> AgentConfig {
    AgentConfig {
        model: model.to_string(),
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

/// Runs one turn with the event channel wired and returns the turn's
/// result plus every `AgentEvent` the loop emitted. Draining with
/// `try_recv` after `run()` is enough: `emit` awaits the channel send, so
/// every event is already buffered by the time `run()` returns.
async fn run_with_events(
    responses: Vec<ChatResponse>,
    max_iterations: usize,
    model: &str,
) -> (
    Result<operant_core::client::Message>,
    Vec<AgentEvent>,
    Arc<ScriptedClient>,
) {
    let temp = tempfile::tempdir().unwrap();
    let client = Arc::new(ScriptedClient::new(responses));
    let registry = ToolRegistry::new(Duration::from_secs(5));
    registry.register(OkTool).await.unwrap();
    let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(64);
    let agent = OperantAgent::with_events(
        test_config(model, max_iterations),
        Box::new(ScriptedClientHandle(Arc::clone(&client))),
        registry,
        Arc::new(Database::init(temp.path().join("exit_reason_truth.sqlite")).unwrap()),
        event_tx,
    );
    let result = agent.run("do some work".to_string()).await;
    let mut events = Vec::new();
    while let Ok(event) = event_rx.try_recv() {
        events.push(event);
    }
    (result, events, client)
}

/// The `Done` event of the turn, of which every exit emits exactly one.
fn done_event(events: &[AgentEvent]) -> AgentEvent {
    let dones: Vec<&AgentEvent> = events
        .iter()
        .filter(|e| matches!(e, AgentEvent::Done { .. }))
        .collect();
    assert_eq!(
        dones.len(),
        1,
        "every run-loop exit must emit exactly one Done event, got {} in {events:#?}",
        dones.len()
    );
    dones[0].clone()
}

fn saw_reason_line(capture: &Arc<Mutex<Vec<String>>>, model: &str, reason: &str) -> bool {
    capture.lock().unwrap().iter().any(|line| {
        line.contains(&format!("Turn ended: reason={reason}"))
            && line.contains(&format!("model={model}"))
    })
}

// ── 1. Fault-injected: degenerate grace turn reports grace_call ──

/// The exact S5 specimen: budget exhaustion (max_iterations: 1 → one tool
/// iteration, then the budget gate routes to the grace call), the grace
/// call returns degenerate reasoning-leak garbage, and A1's S1 gate turns
/// that into the empty message. Before this fix the turn was
/// indistinguishable from a normal answer; now the `Done` event MUST
/// carry `reason: GraceCall` and the diagnostics line MUST say
/// `reason=grace_call` — never `text_response`.
#[tokio::test]
async fn degenerate_grace_turn_reports_grace_call() {
    let capture = install_log_capture();
    const MODEL: &str = "demo-grace-degenerate";
    let (result, events, client) = run_with_events(
        vec![
            tool_call_response("ok_probe", "{}"),
            text_response("]<]\u{200b}minimax[>["),
        ],
        1,
        MODEL,
    )
    .await;

    let result = result.expect("a successful grace call still returns Ok — only the body is gated");
    assert_eq!(
        client.request_count(),
        2,
        "the tool iteration plus the toolless grace call — nothing else"
    );
    assert_eq!(
        result.content, "",
        "S1 gate: degenerate grace output must still become the empty message"
    );

    let done = done_event(&events);
    let AgentEvent::Done { message, reason } = done else {
        unreachable!("done_event only returns Done")
    };
    assert_eq!(
        reason,
        TurnExitReason::GraceCall,
        "a grace-exit turn must not report text_response"
    );
    assert_eq!(
        message.content, "",
        "the gated (empty) message is what the event carries"
    );
    assert!(
        saw_reason_line(&capture, MODEL, "grace_call"),
        "TurnDiagnostics must say reason=grace_call"
    );
    assert!(
        !saw_reason_line(&capture, MODEL, "text_response"),
        "a grace-exit turn must never log reason=text_response"
    );
}

// ── 2. Fault-injected: circuit-breaker abort reports circuit_breaker ──

/// Six identical succeeding tool calls trip the R35 breaker at streak 6;
/// the loop returns the canned abort message. The `Done` event MUST carry
/// `reason: CircuitBreaker` and the diagnostics line MUST say
/// `reason=circuit_breaker` — an abort must never read as an answer.
#[tokio::test]
async fn circuit_breaker_abort_reports_circuit_breaker() {
    let capture = install_log_capture();
    const MODEL: &str = "demo-circuit-breaker";
    let (result, events, client) = run_with_events(
        (0..6)
            .map(|_| tool_call_response("ok_probe", "{}"))
            .collect(),
        6,
        MODEL,
    )
    .await;

    let result = result.expect("the R35 breaker aborts with Ok, not Err");
    assert_eq!(
        client.request_count(),
        6,
        "the abort stops the turn at streak 6"
    );
    assert!(
        result.content.starts_with("⚠️ I stopped early:"),
        "expected a breaker abort body, got: {}",
        result.content
    );

    let done = done_event(&events);
    let AgentEvent::Done { message, reason } = done else {
        unreachable!("done_event only returns Done")
    };
    assert_eq!(
        reason,
        TurnExitReason::CircuitBreaker,
        "a breaker abort must not report text_response"
    );
    assert_eq!(
        message.content, result.content,
        "the Done event carries the operator-facing abort message"
    );
    assert!(
        saw_reason_line(&capture, MODEL, "circuit_breaker"),
        "TurnDiagnostics must say reason=circuit_breaker"
    );
}

// ── 3. Positive control: a normal text turn ─────────────────────

/// A plain answer still reports `reason=text_response` and the `Done`
/// message carries the model's text UNMODIFIED. Without this test a
/// regression that stamps every exit as early-stop passes 1 and 2 while
/// lying in the opposite direction on the healthy path.
#[tokio::test]
async fn normal_text_turn_reports_text_response_and_unmodified_message() {
    let capture = install_log_capture();
    const MODEL: &str = "demo-text-normal";
    let (result, events, _) = run_with_events(vec![text_response("the answer")], 5, MODEL).await;

    let result = result.expect("a normal turn returns Ok");
    assert_eq!(result.content, "the answer");

    let done = done_event(&events);
    let AgentEvent::Done { message, reason } = done else {
        unreachable!("done_event only returns Done")
    };
    assert_eq!(reason, TurnExitReason::TextResponse);
    assert_eq!(
        message.content, "the answer",
        "the healthy path delivers the model's answer verbatim"
    );
    assert!(
        saw_reason_line(&capture, MODEL, "text_response"),
        "TurnDiagnostics must say reason=text_response"
    );
}
