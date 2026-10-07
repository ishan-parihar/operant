//! Wave 1 leg W1.2 — the three-engine parity harness.
//!
//! Pins TODAY's behavior of every turn engine in this workspace against one
//! shared scripted-provider matrix, so each later migration leg can prove
//! behavior preservation by re-running these tests through the facade:
//!
//! | engine | entry |
//! |---|---|
//! | Loop A | `operant_core::agent::OperantAgent::run` (gateway path) |
//! | Loop B | `operant_runtime::agent::Agent::turn_streamed` (runtime agent) |
//! | Loop C | `operant_runtime::agent::loop_::run_tool_call_loop` (CLI loop) |
//!
//! All three consume the SAME script (`Vec<Step>`). `Step` is rendered per
//! engine because core and providers define TWO DIFFERENT `ChatResponse`
//! types (core: OpenAI-style `choices`/`MessageDelta`; providers:
//! `{text, tool_calls, …}`) — the script is shared, the wire shape is not.
//!
//! Scenario matrix:
//!
//! | scenario | Loop A | Loop B | Loop C |
//! |---|---|---|---|
//! | empty×3→answer | retry ladder, final text | same ladder | same ladder |
//! | empty×4 | Ok(empty), TextResponse exit | Ok("") | Ok("") |
//! | budget exhaustion | grace LLM call → Ok(summary) | hard `bail!` | final summary LLM call |
//! | identical repeat ×3 | guardrail Warn event (no-effect tool) | NO guardrail | `[Loop Detection]` in history |
//! | context overflow | compress + reinject todos + events | error propagates (no compressor) | deterministic trim in-loop; facade compressor re-injects todos (W1.4 flip) |
//!
//! Cells where engines intentionally differ carry a `// PINNED DIVERGENCE
//! (Wave 1)` comment. Post-migration, each pin must still hold OR the
//! migration leg must consciously update it with justification.
//!
//! NOTE on Loop C compression: the compressor itself now lives in the
//! reconciled facade (`agent/reconciled.rs`, the core pair's runtime front
//! door) driven from the `loop_::run` wrapper (loop_/run.rs), which builds
//! real providers from a full `Config` and is not test-callable without
//! heavy plumbing; the loop proper is pinned via direct-drive (the pattern
//! from `operant-runtime/src/agent/loop_/tests.rs`) plus a direct unit-pin
//! of the facade's public API.
//!
//! NOTE on exit-event history: iter-609/610 (the foreign fleet's stale-baseline
//! landing) reverted the A2 surface — `Done { message }` without a reason,//! `ToolResult` without `timed_out`. iter-612 restored it: `Done` carries
//! `reason: TurnExitReason` again and `ToolResult` requires `timed_out: bool`.
//! The pins below target the RESTORED state (the current tree); any later
//! migration leg that changes exit-event shapes must update them consciously,
//! with justification.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use futures_util::stream::BoxStream;
use parking_lot::Mutex;

// ── Loop A: operant-core (gateway path) ─────────────────────────
use operant_core::agent::{
    AgentConfig, AgentEvent, ChatRequest, ModelClient, OperantAgent, StreamChunk,
};
use operant_core::client::{
    ChatResponse as CoreChatResponse, Choice, MessageDelta, Role, ToolCallDelta, ToolCallFunction,
    Usage,
};
use operant_core::database::Database;
use operant_core::error::{Error, Result};
use operant_core::schema::ToolSchema;
use operant_core::tools::todo_tool::{TODO_INJECTION_HEADER, TodoTool};
use operant_core::tools::{OperantTool, ToolContext, ToolRegistry, ToolResult};

// ── Loops B & C: operant-runtime + operant-providers ────────────
use operant_config::schema::{AgentConfig as RtAgentConfig, MemoryConfig};
use operant_memory::Memory;
use operant_providers::{
    ChatRequest as RtChatRequest, ChatResponse as RtChatResponse, Provider, ToolCall,
};
use operant_runtime::agent::dispatcher::{NativeToolDispatcher, ToolDispatcher};
use operant_runtime::agent::{Agent, TurnEvent};
use operant_runtime::observability::{NoopObserver, Observer};
use operant_runtime::tools::{Tool, ToolResult as RtToolResult};

// ═════════════════════════════════════════════════════════════════
// Shared script — ONE spec, three renderings
// ═════════════════════════════════════════════════════════════════

/// One scripted provider turn. `Empty` follows `turn_rules::AssistantTurn::
/// is_empty` semantics: no text, no reasoning, no tool calls.
#[derive(Clone)]
enum Step {
    Empty,
    Text(&'static str),
    /// (tool name, JSON arguments)
    Tool(&'static str, &'static str),
    /// A provider context-overflow error (400 + context-length body).
    OverflowError,
}

/// The overflow body proven (operant-core tests/loop_recovery_paths.rs) to
/// classify as `FailoverReason::ContextOverflow` with `should_compress`.
const OVERFLOW_BODY: &str = "This model's maximum context length is 8192 tokens, however you \
                            requested 9017 tokens.";

/// The always-success probe tool, named after a `NO_EFFECT_TOOL_NAMES`
/// entry so Loop A's repeat guardrail takes the Warn path (3rd identical
/// call warns and still executes) instead of the side-effect Skip path.
const PROBE: &str = "env_probe";

// ═════════════════════════════════════════════════════════════════
// Loop A harness (operant-core) — mirrors tests/grace_output_gate.rs
// ═════════════════════════════════════════════════════════════════

struct CoreScriptedClient {
    steps: Mutex<Vec<Step>>,
    seen: AtomicUsize,
    /// Message contents of every request, for behavior assertions.
    requests: Mutex<Vec<Vec<String>>>,
}

impl CoreScriptedClient {
    fn new(steps: Vec<Step>) -> Arc<Self> {
        Arc::new(Self {
            steps: Mutex::new(steps),
            seen: AtomicUsize::new(0),
            requests: Mutex::new(Vec::new()),
        })
    }

    fn request_count(&self) -> usize {
        self.seen.load(Ordering::Relaxed)
    }

    /// Message contents of the n-th request (0-based).
    fn messages_of(&self, n: usize) -> Vec<String> {
        self.requests.lock()[n].clone()
    }
}

/// Lets the shared `Arc<CoreScriptedClient>` survive the
/// `Box<dyn ModelClient>` hand-off (grace_output_gate.rs pattern).
struct CoreHandle(Arc<CoreScriptedClient>);

#[async_trait]
impl ModelClient for CoreHandle {
    fn provider_name(&self) -> &str {
        "scripted"
    }

    async fn chat(&self, request: ChatRequest) -> Result<CoreChatResponse> {
        let client = &self.0;
        client.seen.fetch_add(1, Ordering::Relaxed);
        client
            .requests
            .lock()
            .push(request.messages.iter().map(|m| m.content.clone()).collect());
        let step = client.steps.lock().remove(0);
        Ok(match step {
            Step::Empty => core_text(""),
            Step::Text(t) => core_text(t),
            Step::Tool(name, args) => core_tool_call(name, args),
            Step::OverflowError => {
                return Err(Error::Provider {
                    status: 400,
                    body: OVERFLOW_BODY.to_string(),
                    retry_after: None,
                });
            }
        })
    }

    async fn chat_streaming(
        &self,
        _request: ChatRequest,
    ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
        Err(Error::Agent("streaming not used in this harness".into()))
    }
}

fn core_text(content: &str) -> CoreChatResponse {
    CoreChatResponse {
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

fn core_tool_call(tool: &str, arguments: &str) -> CoreChatResponse {
    CoreChatResponse {
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
        ..core_text("")
    }
}

/// Always-succeeds probe (core `OperantTool`), counting executions.
struct CoreEnvProbe {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl OperantTool for CoreEnvProbe {
    fn name(&self) -> &str {
        PROBE
    }

    fn description(&self) -> &str {
        "Parity probe: always succeeds (a no-effect tool name)."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            PROBE,
            "Parity probe: always succeeds (a no-effect tool name).",
            serde_json::json!({"type": "object", "properties": {}}),
        )
    }

    async fn execute(&self, _args: serde_json::Value, _context: ToolContext) -> ToolResult {
        self.calls.fetch_add(1, Ordering::Relaxed);
        ToolResult {
            tool_call_id: String::new(),
            name: PROBE.to_string(),
            success: true,
            content: "probe ok".to_string(),
            error: None,
            timed_out: false,
        }
    }
}

fn core_config(max_iterations: usize) -> AgentConfig {
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

/// Builds Loop A with the `todo` tool registered plus (optionally) the
/// counting probe, and an event channel. Returns (agent, event receiver).
async fn core_agent(
    client: Arc<CoreScriptedClient>,
    max_iterations: usize,
    db_path: std::path::PathBuf,
    probe_calls: Option<Arc<AtomicUsize>>,
) -> (OperantAgent, tokio::sync::mpsc::Receiver<AgentEvent>) {
    let registry = ToolRegistry::new(Duration::from_secs(5));
    if let Some(calls) = probe_calls {
        registry.register(CoreEnvProbe { calls }).await.unwrap();
    }
    registry.register(TodoTool).await.unwrap();
    let (tx, rx) = tokio::sync::mpsc::channel(512);
    let agent = OperantAgent::with_events(
        core_config(max_iterations),
        Box::new(CoreHandle(client)),
        registry,
        Arc::new(Database::init(db_path).unwrap()),
        tx,
    );
    (agent, rx)
}

fn drain_events(mut rx: tokio::sync::mpsc::Receiver<AgentEvent>) -> Vec<AgentEvent> {
    let mut out = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        out.push(ev);
    }
    out
}

// ═════════════════════════════════════════════════════════════════
// Loops B & C harness (operant-runtime) — mirrors agent/tests.rs
// ═════════════════════════════════════════════════════════════════

struct RtScriptedProvider {
    steps: Mutex<Vec<Step>>,
    requests: Mutex<Vec<Vec<String>>>,
    next_id: AtomicUsize,
}

impl RtScriptedProvider {
    fn new(steps: Vec<Step>) -> Arc<Self> {
        Arc::new(Self {
            steps: Mutex::new(steps),
            requests: Mutex::new(Vec::new()),
            next_id: AtomicUsize::new(0),
        })
    }

    fn request_count(&self) -> usize {
        self.requests.lock().len()
    }

    /// Message contents of the n-th request (0-based).
    fn messages_of(&self, n: usize) -> Vec<String> {
        self.requests.lock()[n].clone()
    }

    fn render(&self, step: Step) -> RtChatResponse {
        match step {
            Step::Empty => rt_text(""),
            Step::Text(t) => rt_text(t),
            Step::Tool(name, args) => {
                let id = self.next_id.fetch_add(1, Ordering::Relaxed);
                RtChatResponse {
                    text: Some(String::new()),
                    tool_calls: vec![ToolCall {
                        id: format!("call_{id}"),
                        name: name.to_string(),
                        arguments: args.to_string(),
                        extra_content: None,
                    }],
                    usage: None,
                    reasoning_content: None,
                }
            }
            Step::OverflowError => unreachable!("overflow errors are returned, not rendered"),
        }
    }

    async fn next(&self, request: RtChatRequest<'_>) -> anyhow::Result<RtChatResponse> {
        self.requests
            .lock()
            .push(request.messages.iter().map(|m| m.content.clone()).collect());
        let step = self.steps.lock().remove(0);
        match step {
            Step::OverflowError => Err(anyhow::anyhow!("{OVERFLOW_BODY}")),
            other => Ok(self.render(other)),
        }
    }
}

fn rt_text(text: &str) -> RtChatResponse {
    RtChatResponse {
        text: Some(text.to_string()),
        tool_calls: vec![],
        usage: None,
        reasoning_content: None,
    }
}

/// Provider-by-value handle so the test keeps the `Arc` for assertions after
/// the provider moves into the agent (same idea as Loop A's `CoreHandle`).
struct RtHandle(Arc<RtScriptedProvider>);

#[async_trait]
impl Provider for RtHandle {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: Option<f64>,
    ) -> anyhow::Result<String> {
        Ok("fallback".into())
    }

    async fn chat(
        &self,
        request: RtChatRequest<'_>,
        _model: &str,
        _temperature: Option<f64>,
    ) -> anyhow::Result<RtChatResponse> {
        self.0.next(request).await
    }
}

#[async_trait]
impl Provider for RtScriptedProvider {
    async fn chat_with_system(
        &self,
        _system_prompt: Option<&str>,
        _message: &str,
        _model: &str,
        _temperature: Option<f64>,
    ) -> anyhow::Result<String> {
        Ok("fallback".into())
    }

    async fn chat(
        &self,
        request: RtChatRequest<'_>,
        _model: &str,
        _temperature: Option<f64>,
    ) -> anyhow::Result<RtChatResponse> {
        self.next(request).await
    }
}

/// Always-succeeds probe (runtime `Tool`), counting executions.
struct RtEnvProbe {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl Tool for RtEnvProbe {
    fn name(&self) -> &str {
        PROBE
    }

    fn description(&self) -> &str {
        "Parity probe: always succeeds."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<RtToolResult> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Ok(RtToolResult {
            success: true,
            output: "probe ok".to_string(),
            error: None,
        })
    }
}

fn rt_probe_tools() -> (Vec<Box<dyn Tool>>, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    (
        vec![Box::new(RtEnvProbe {
            calls: Arc::clone(&calls),
        }) as Box<dyn Tool>],
        calls,
    )
}

fn rt_memory() -> Arc<dyn Memory> {
    let cfg = MemoryConfig {
        backend: "none".into(),
        ..MemoryConfig::default()
    };
    Arc::from(operant_memory::create_memory(&cfg, &std::env::temp_dir(), None).unwrap())
}

/// Loop B: `Agent::turn_streamed` via `AgentBuilder` (agent/tests.rs pattern).
/// The caller keeps the `Arc<RtScriptedProvider>` for request assertions.
fn rt_agent(provider: Arc<RtScriptedProvider>, max_tool_iterations: usize) -> Agent {
    let (tools, _calls) = rt_probe_tools();
    Agent::builder()
        .provider(Box::new(RtHandle(provider)))
        .tools(tools)
        .memory(rt_memory())
        .observer(Arc::from(NoopObserver {}) as Arc<dyn Observer>)
        .tool_dispatcher(Box::new(NativeToolDispatcher) as Box<dyn ToolDispatcher>)
        .workspace_dir(std::env::temp_dir())
        .config(RtAgentConfig {
            max_tool_iterations,
            ..RtAgentConfig::default()
        })
        .build()
        .unwrap()
}

async fn rt_turn(agent: &mut Agent, message: &str) -> anyhow::Result<String> {
    let (tx, _rx) = tokio::sync::mpsc::channel::<TurnEvent>(256);
    agent.turn_streamed(message, tx, None).await
}

// ═════════════════════════════════════════════════════════════════
// Scenario 1 — empty ×3 → answer: the shared retry ladder
// ═════════════════════════════════════════════════════════════════

const S1_ANSWER: &str = "the recovered answer, served after three empty turns";

#[tokio::test]
async fn s1_empty_recovery_loop_a() {
    let temp = tempfile::tempdir().unwrap();
    let client = CoreScriptedClient::new(vec![
        Step::Empty,
        Step::Empty,
        Step::Empty,
        Step::Text(S1_ANSWER),
    ]);
    let (agent, rx) =
        core_agent(Arc::clone(&client), 8, temp.path().join("s1a.sqlite"), None).await;

    let result = agent.run("say something".to_string()).await.unwrap();

    assert_eq!(result.content, S1_ANSWER);
    assert_eq!(client.request_count(), 4, "three empties + one answer");
    assert!(
        drain_events(rx).iter().any(|ev| matches!(
            ev,
            AgentEvent::Done {
                message: m,
                ..
            } if m.content == S1_ANSWER
        )),
        "recovered turn must end with a Done carrying the answer"
    );
}

#[tokio::test]
async fn s1_empty_recovery_loop_b() {
    let provider = RtScriptedProvider::new(vec![
        Step::Empty,
        Step::Empty,
        Step::Empty,
        Step::Text(S1_ANSWER),
    ]);
    let mut agent = rt_agent(Arc::clone(&provider), 8);

    let out = rt_turn(&mut agent, "say something").await.unwrap();

    assert_eq!(out, S1_ANSWER);
    assert_eq!(provider.request_count(), 4, "three empties + one answer");
}

#[tokio::test]
async fn s2_empty_exhaustion_loop_a() {
    let temp = tempfile::tempdir().unwrap();
    let client = CoreScriptedClient::new(vec![Step::Empty; 4]);
    let (agent, rx) =
        core_agent(Arc::clone(&client), 8, temp.path().join("s2a.sqlite"), None).await;

    // PIN: the exhausted ladder returns Ok with an EMPTY body (the gateway
    // layer substitutes the stopped-early notice for empty content) — also
    // pinned in core's loop_recovery_paths.rs. `Done` again carries
    // `reason: TurnExitReason` (restored by iter-612; the `..` in the
    // match below keeps the pin body-only), so the observable pin stays
    // the empty Ok + the Done event itself.
    let result = agent.run("say something".to_string()).await.unwrap();

    assert_eq!(result.content, "");
    assert_eq!(
        client.request_count(),
        4,
        "three nudges, then the terminal empty"
    );
    assert!(
        drain_events(rx).iter().any(|ev| matches!(
            ev,
            AgentEvent::Done {
                message: m,
                ..
            } if m.content.is_empty()
        )),
        "exhausted-empty must end Done-with-empty-message, not an error or breaker"
    );
}

#[tokio::test]
async fn s2_empty_exhaustion_loop_b() {
    let provider = RtScriptedProvider::new(vec![Step::Empty; 4]);
    let mut agent = rt_agent(Arc::clone(&provider), 8);

    // PIN (same terminal shape as Loop A): Ok("") — the 4th empty becomes
    // the final response instead of erroring.
    let out = rt_turn(&mut agent, "say something").await.unwrap();

    assert_eq!(out, "");
    assert_eq!(provider.request_count(), 4);
}

// Scenario 3 — budget exhaustion (tool rounds with VARYING args so no
// repeat guardrail fires; repeat behavior is scenario 4's subject)

const S3_GRACE: &str = "grace summary: partial work completed before the budget ran out";

#[tokio::test]
async fn s3_budget_exhaustion_loop_a() {
    let temp = tempfile::tempdir().unwrap();
    let client = CoreScriptedClient::new(vec![
        Step::Tool(PROBE, r#"{"i":1}"#),
        Step::Tool(PROBE, r#"{"i":2}"#),
        Step::Tool(PROBE, r#"{"i":3}"#),
        Step::Text(S3_GRACE),
    ]);
    let probe_calls = Arc::new(AtomicUsize::new(0));
    let (agent, _rx) = core_agent(
        Arc::clone(&client),
        3,
        temp.path().join("s3a.sqlite"),
        Some(Arc::clone(&probe_calls)),
    )
    .await;

    // PINNED DIVERGENCE (Wave 1): Loop A does NOT bail on budget exhaustion —
    // it spends one extra GRACE LLM call and returns that (gate-validated)
    // summary as the answer, exiting `Done { reason: GraceCall }` (reason
    // observable on the event again since the iter-612 restore; the pin
    // remains the returned grace text plus the extra request).
    let result = agent.run("do some work".to_string()).await.unwrap();

    assert_eq!(result.content, S3_GRACE);
    assert_eq!(client.request_count(), 4, "3 tool rounds + 1 grace call");
    assert_eq!(probe_calls.load(Ordering::Relaxed), 3);
}

#[tokio::test]
async fn s3_budget_exhaustion_loop_b() {
    let provider = RtScriptedProvider::new(vec![
        Step::Tool(PROBE, r#"{"i":1}"#),
        Step::Tool(PROBE, r#"{"i":2}"#),
        Step::Tool(PROBE, r#"{"i":3}"#),
    ]);
    let (tools, probe_calls) = rt_probe_tools();
    let mut agent = Agent::builder()
        .provider(Box::new(RtHandle(Arc::clone(&provider))))
        .tools(tools)
        .memory(rt_memory())
        .observer(Arc::from(NoopObserver {}) as Arc<dyn Observer>)
        .tool_dispatcher(Box::new(NativeToolDispatcher) as Box<dyn ToolDispatcher>)
        .workspace_dir(std::env::temp_dir())
        .config(RtAgentConfig {
            max_tool_iterations: 3,
            ..RtAgentConfig::default()
        })
        .build()
        .unwrap();

    // PINNED DIVERGENCE (Wave 1): Loop B hard-bails on budget exhaustion —
    // no grace call, no partial answer, an Err the caller must handle.
    let err = rt_turn(&mut agent, "do some work").await.unwrap_err();

    assert!(
        err.to_string()
            .contains("Agent exceeded maximum tool iterations (3)"),
        "actual: {err}"
    );
    assert_eq!(probe_calls.load(Ordering::Relaxed), 3);
    assert_eq!(provider.request_count(), 3);
}

// Scenario 4 — identical tool call ×3 (repeat guardrails)

const S4_FINAL: &str = "done after three identical probes";
const S4_ARGS: &str = r#"{"x":1}"#;

#[tokio::test]
async fn s4_repeat_guardrail_loop_a() {
    let temp = tempfile::tempdir().unwrap();
    let client = CoreScriptedClient::new(vec![
        Step::Tool(PROBE, S4_ARGS),
        Step::Tool(PROBE, S4_ARGS),
        Step::Tool(PROBE, S4_ARGS),
        Step::Text(S4_FINAL),
    ]);
    let probe_calls = Arc::new(AtomicUsize::new(0));
    let (agent, rx) = core_agent(
        Arc::clone(&client),
        4,
        temp.path().join("s4a.sqlite"),
        Some(Arc::clone(&probe_calls)),
    )
    .await;

    let result = agent.run("probe repeatedly".to_string()).await.unwrap();

    assert_eq!(result.content, S4_FINAL);
    assert_eq!(client.request_count(), 4);
    // `env_probe` is a NO-EFFECT tool name: the guardrail WARNS on the 3rd
    // identical call (REPEAT_WARN_THRESHOLD = 3) and still executes it.
    assert_eq!(
        probe_calls.load(Ordering::Relaxed),
        3,
        "a no-effect tool's warned call still executes"
    );
    // The Warn surfaces as a Content event (stream.rs GuardrailDecision::Warn).
    let warned = drain_events(rx)
        .into_iter()
        .filter_map(|ev| match ev {
            AgentEvent::Content { text } if text.contains(PROBE) => Some(text),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(
        warned
            .iter()
            .any(|t| t.contains("has been called with identical arguments 3 times this turn")),
        "the guardrail warning must be visible in events, got: {warned:?}"
    );
}

#[tokio::test]
async fn s4_repeat_guardrail_loop_b() {
    let provider = RtScriptedProvider::new(vec![
        Step::Tool(PROBE, S4_ARGS),
        Step::Tool(PROBE, S4_ARGS),
        Step::Tool(PROBE, S4_ARGS),
        Step::Text(S4_FINAL),
    ]);
    let (tools, probe_calls) = rt_probe_tools();
    let mut agent = Agent::builder()
        .provider(Box::new(RtHandle(Arc::clone(&provider))))
        .tools(tools)
        .memory(rt_memory())
        .observer(Arc::from(NoopObserver {}) as Arc<dyn Observer>)
        .tool_dispatcher(Box::new(NativeToolDispatcher) as Box<dyn ToolDispatcher>)
        .workspace_dir(std::env::temp_dir())
        .config(RtAgentConfig {
            max_tool_iterations: 4,
            ..RtAgentConfig::default()
        })
        .build()
        .unwrap();

    // PINNED DIVERGENCE (Wave 1): Loop B has NO repeat guardrail — the 3rd
    // identical call executes like the first: no warning, no synthetic
    // result, no extra round-trip (exact counts; no guardrail or
    // loop-detection text anywhere in the recorded requests).
    let out = rt_turn(&mut agent, "probe repeatedly").await.unwrap();

    assert_eq!(out, S4_FINAL);
    assert_eq!(probe_calls.load(Ordering::Relaxed), 3);
    assert_eq!(provider.request_count(), 4);
    for n in 0..4 {
        for message in provider.messages_of(n) {
            assert!(
                !message.contains("[Loop Detection]") && !message.contains("[GUARDRAIL]"),
                "Loop B must not inject any repeat-guardrail feedback, got: {message}"
            );
        }
    }
}

const S5_AFTER: &str = "answer served after the compression retry";
const S5_TODO_ARGS: &str = r#"{"todos":[{"id":"p1","content":"parity probe todo","status":"in_progress"}],"sessionId":"default"}"#;

#[tokio::test]
async fn s5_compression_loop_a() {
    let temp = tempfile::tempdir().unwrap();
    let client = CoreScriptedClient::new(vec![
        Step::Tool("todo", S5_TODO_ARGS),
        Step::OverflowError,
        Step::Text(S5_AFTER),
    ]);
    let (agent, rx) =
        core_agent(Arc::clone(&client), 8, temp.path().join("s5a.sqlite"), None).await;

    // PIN: overflow is recovered IN-LOOP — compress, refund, retry once —
    // and the turn ends on the retry's answer.
    let result = agent
        .run("write a todo then answer".to_string())
        .await
        .unwrap();

    assert_eq!(result.content, S5_AFTER);
    assert_eq!(client.request_count(), 3);
    let events = drain_events(rx);
    assert!(
        events
            .iter()
            .any(|ev| matches!(ev, AgentEvent::CompactionStarted { .. })),
        "CompactionStarted must be emitted"
    );
    assert!(
        events
            .iter()
            .any(|ev| matches!(ev, AgentEvent::CompactionCompleted { .. })),
        "CompactionCompleted must be emitted"
    );
    // PINNED DIVERGENCE (Wave 1): Loop A REINJECTS the active todo list
    // after compression — the retry request carries the todo snapshot.
    let retry = client.messages_of(2);
    assert!(
        retry.iter().any(|m| m.contains(TODO_INJECTION_HEADER)),
        "the compressed retry must carry the todo snapshot, got: {retry:?}"
    );
}

#[tokio::test]
async fn s5_compression_loop_b() {
    let provider = RtScriptedProvider::new(vec![Step::OverflowError]);
    let mut agent = rt_agent(Arc::clone(&provider), 8);

    // PINNED DIVERGENCE (Wave 1): Loop B has NO compressor (its only context
    // management is the line-count `trim_history`) — a context-overflow
    // error propagates to the caller on the first request, no retry.
    let err = rt_turn(&mut agent, "answer over a huge context")
        .await
        .unwrap_err();

    assert!(
        err.to_string().contains("maximum context length"),
        "actual: {err}"
    );
    assert_eq!(provider.request_count(), 1);
}
