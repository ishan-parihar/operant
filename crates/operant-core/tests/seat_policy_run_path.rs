//! Real-path tests for P1 enforcement (permission-genome wave-2 slice E):
//! the seat-policy `decide()` precedence engine is LIVE on the agent run
//! path.
//!
//! Every test drives the REAL `OperantAgent::run` → `execute_tools` guard —
//! scripted model, real agent, real `SeatPolicyDb` over a tempdir, real
//! permission channel drained by the test the way the gateway drains it.
//! Nothing here re-implements the guard: the fault-injection procedure from
//! the slice-E report (hardcode a lockdown into the guard regardless of
//! source → the no-source test goes red → restore → green) keeps that honest
//! the same way `cron_session_isolation.rs`'s negative control does.
//!
//! The two stub tools matter: the guard only classifies by NAME, so a stub
//! named `bash` sits on the hardcoded dangerous list without running a
//! shell, and `seat_probe` is a safe name no list mentions — which is what
//! makes the lockdown escalation below prove the policy genome bites
//! beyond today's dangerous list (ungoverned, `seat_probe` runs silent).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::stream::BoxStream;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use operant_core::agent::{
    AgentConfig, ChatRequest, ModelClient, OperantAgent, StreamChunk, ToolPermissionRequest,
    ToolPermissionResponse,
};
use operant_core::client::{
    ChatResponse, Choice, MessageDelta, Role, ToolCallDelta, ToolCallFunction, Usage,
};
use operant_core::database::Database;
use operant_core::error::{Error, Result};
use operant_core::org::seat_policy::{SeatMode, SeatPolicy, SeatPolicySource};
use operant_core::org::seat_policy_db::SeatPolicyDb;
use operant_core::schema::ToolSchema;
use operant_core::tools::{OperantTool, ToolContext, ToolRegistry, ToolResult};

/// Marker `seat_probe` returns when it actually executes.
const PROBE_MARKER: &str = "SEAT_PROBE_RAN";
/// Marker the `bash` stub returns when it actually executes.
const BASH_MARKER: &str = "BASH_STUB_RAN";

// ── Stub tools ─────────────────────────────────────────────────────────

/// Safe-named stub: not on the permission-gated list, so ungoverned it runs
/// without a prompt — exactly the tool a lockdown seat must newly escalate.
struct SeatProbeTool;

#[derive(JsonSchema, Deserialize)]
struct ProbeArgs {
    message: String,
}

#[async_trait]
impl OperantTool for SeatProbeTool {
    fn name(&self) -> &str {
        "seat_probe"
    }
    fn description(&self) -> &str {
        "Test stub — returns a marker when it executes."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<ProbeArgs>("seat_probe", "Test stub for the seat-policy run path")
    }
    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let parsed: ProbeArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("seat_probe", format!("bad args: {e}")),
        };
        ToolResult::success(
            "seat_probe",
            json!({ "echoed": parsed.message, "marker": PROBE_MARKER }),
        )
    }
}

/// Dangerous-named stub: sits on the hardcoded dangerous list by NAME, so
/// the ungoverned guard prompts for it, but executing it runs no shell.
struct BashStubTool;

#[derive(JsonSchema, Deserialize)]
struct BashArgs {
    command: String,
}

#[async_trait]
impl OperantTool for BashStubTool {
    fn name(&self) -> &str {
        "bash"
    }
    fn description(&self) -> &str {
        "Test stub — named bash so the guard treats it as dangerous."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<BashArgs>("bash", "Dangerous-named test stub")
    }
    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let parsed: BashArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error("bash", format!("bad args: {e}")),
        };
        ToolResult::success(
            "bash",
            json!({ "marker": BASH_MARKER, "command": parsed.command }),
        )
    }
}

// ── Scripted model ─────────────────────────────────────────────────────

/// Queue-driven client: every `chat` pops the next scripted response and
/// records the message list it was handed, so a test can assert on the tool
/// results the loop fed back (the `seen` log is the whole conversation).
struct ScriptedClient {
    responses: Mutex<Vec<ChatResponse>>,
    seen: Mutex<Vec<String>>,
}

impl ScriptedClient {
    fn new(responses: Vec<ChatResponse>) -> Arc<Self> {
        Arc::new(Self {
            responses: Mutex::new(responses),
            seen: Mutex::new(Vec::new()),
        })
    }

    /// Did any recorded request message contain `fragment`?
    fn saw(&self, fragment: &str) -> bool {
        self.seen
            .lock()
            .unwrap()
            .iter()
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
            .extend(request.messages.iter().map(|m| m.content.clone()));
        let mut guard = self.responses.lock().unwrap();
        if guard.is_empty() {
            return Ok(text_response("out of script"));
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

// ── World ──────────────────────────────────────────────────────────────

/// An agent over a tempdir database with both stub tools registered, a
/// scripted model, and a permission channel the test drains. When `seat` is
/// `Some((session_id, policy))`, a real `SeatPolicyDb` carries that row and
/// is attached as the agent's seat-policy source; `None` is the pre-slice-E
/// world — no source at all.
struct World {
    _dir: tempfile::TempDir,
    client: Arc<ScriptedClient>,
    agent: Arc<OperantAgent>,
    permission_rx: tokio::sync::mpsc::Receiver<ToolPermissionRequest>,
}

async fn world(responses: Vec<ChatResponse>, seat: Option<(&str, SeatPolicy)>) -> World {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = Arc::new(Database::init(dir.path().join("agent.sqlite")).expect("Database::init"));

    let registry = ToolRegistry::new(Duration::from_secs(5));
    registry.register(SeatProbeTool).await.unwrap();
    registry.register(BashStubTool).await.unwrap();

    let client = ScriptedClient::new(responses);
    let mut agent = OperantAgent::new(
        test_config(),
        Box::new(ScriptedClientHandle(Arc::clone(&client))),
        registry,
        db,
    );

    let (permission_tx, permission_rx) = tokio::sync::mpsc::channel::<ToolPermissionRequest>(8);
    agent = agent.with_permissions(permission_tx);

    if let Some((session_id, policy)) = seat {
        let store = SeatPolicyDb::init(dir.path().join("seat_policies.sqlite"))
            .expect("SeatPolicyDb::init");
        store
            .upsert(session_id, &policy)
            .expect("upsert seat policy");
        let source: Arc<dyn SeatPolicySource> = Arc::new(store);
        agent = agent.with_seat_policy_source(Some(source));
        let agent = Arc::new(agent);
        agent.set_session_id(session_id);
        return World {
            _dir: dir,
            client,
            agent,
            permission_rx,
        };
    }

    World {
        _dir: dir,
        client,
        agent: Arc::new(agent),
        permission_rx,
    }
}

/// Same shape `tests/circuit_breaker_abort.rs::test_config` established, but
/// with the production approval mode (`smart`) so the smart gate runs ahead
/// of the seat consultation exactly as it does live.
fn test_config() -> AgentConfig {
    AgentConfig {
        model: "demo".to_string(),
        max_iterations: 4,
        tool_timeout: Duration::from_secs(5),
        request_timeout: Duration::from_secs(10),
        system_prompt: Some("You are a test agent.".to_string()),
        // Non-streaming: the scripted client answers through `chat`.
        stream: false,
        context_window: 8000,
        max_tool_result_share: operant_core::context_management::DEFAULT_MAX_TOOL_RESULT_SHARE,
        max_healing_attempts: 1,
        fallback_models: Vec::new(),
        fallback_on_errors: false,
        approval_mode: "smart".to_string(),
        approval_allowlist: Vec::new(),
        approval_allowlist_path: None,
        record_trajectories: false,
        skill_nudge_interval: 0,
        memory_review_interval: 0,
        max_retries: 3,
        tool_search: Default::default(),
    }
}

/// One tool call, then a closing text answer — the minimal real-path script.
fn probe_script() -> Vec<ChatResponse> {
    vec![
        tool_call_response("seat_probe", r#"{"message":"hello"}"#),
        text_response("done"),
    ]
}

// ── Tests ──────────────────────────────────────────────────────────────

/// A lockdown seat escalates even a SAFE tool through the real guard — the
/// escalation the ungoverned path never produces — and the prompt carries
/// the policy's own why-sentence as the danger explanation.
#[tokio::test]
async fn lockdown_seat_escalates_a_tool_call_through_the_real_guard() {
    let mut w = world(
        probe_script(),
        Some((
            "seat-lockdown",
            SeatPolicy {
                mode: SeatMode::Lockdown,
                allow: vec![],
                deny: vec![],
            },
        )),
    )
    .await;

    let run = tokio::spawn({
        let agent = Arc::clone(&w.agent);
        async move { agent.run("go".to_string()).await }
    });

    let request = tokio::time::timeout(Duration::from_secs(10), w.permission_rx.recv())
        .await
        .expect("permission request within 10s")
        .expect("permission channel live");
    assert_eq!(
        request.tool_name, "seat_probe",
        "lockdown must escalate the safe-named tool too"
    );
    assert!(
        request.danger_explanation.contains("lockdown mode")
            && request.danger_explanation.contains("seat_probe"),
        "the prompt must carry the policy's why-sentence, got: {:?}",
        request.danger_explanation
    );

    // Approve once — the escalated call must then actually execute.
    request
        .response_tx
        .send(ToolPermissionResponse::AllowOnce)
        .expect("respond AllowOnce");

    run.await.expect("join run").expect("run completes");
    assert!(
        w.client.saw(PROBE_MARKER),
        "after AllowOnce the tool must have executed"
    );
}

/// A yolo seat runs the same tool with NO permission prompt at all — the
/// Run verdict must actually skip the channel, not enter it quietly.
#[tokio::test]
async fn yolo_seat_runs_the_tool_with_no_permission_prompt() {
    let mut w = world(
        probe_script(),
        Some((
            "seat-yolo",
            SeatPolicy {
                mode: SeatMode::Yolo,
                allow: vec![],
                deny: vec![],
            },
        )),
    )
    .await;

    w.agent.run("go".to_string()).await.expect("run completes");

    assert!(
        w.client.saw(PROBE_MARKER),
        "the yolo seat's tool call must have executed"
    );
    assert!(
        w.permission_rx.try_recv().is_err(),
        "a yolo Run verdict must never enter the permission channel"
    );
}

/// An agent with NO source attached (and a session id, so only the missing
/// source keeps it ungoverned) behaves exactly as today: a safe tool runs
/// with no prompt. This is the test the slice-E fault injection turns red.
#[tokio::test]
async fn no_source_agent_behaves_as_today_safe_tool_runs_unprompted() {
    // Session id set, source absent — the ungoverned-by-construction world
    // every non-gateway agent still lives in.
    let mut w = world(probe_script(), None).await;
    w.agent.set_session_id("seat-ungoverned");

    w.agent.run("go".to_string()).await.expect("run completes");

    assert!(
        w.client.saw(PROBE_MARKER),
        "ungoverned safe tool must run, as it always has"
    );
    assert!(
        w.permission_rx.try_recv().is_err(),
        "ungoverned safe tool must not prompt — today's behavior, byte-identical"
    );
}

/// An agent with NO source still prompts for a dangerous-named tool with
/// today's hardcoded danger sentence — the ungoverned dangerous path is
/// untouched — and a declined response still denies the call.
#[tokio::test]
async fn no_source_agent_behaves_as_today_dangerous_tool_prompts() {
    let mut w = world(
        vec![
            tool_call_response("bash", r#"{"command":"echo hi"}"#),
            text_response("done"),
        ],
        None,
    )
    .await;

    let run = tokio::spawn({
        let agent = Arc::clone(&w.agent);
        async move { agent.run("go".to_string()).await }
    });

    let request = tokio::time::timeout(Duration::from_secs(10), w.permission_rx.recv())
        .await
        .expect("permission request within 10s")
        .expect("permission channel live");
    assert_eq!(request.tool_name, "bash");
    assert_eq!(
        request.danger_explanation, "This runs a shell command on your system",
        "the ungoverned arm must keep today's hardcoded danger sentence, got: {:?}",
        request.danger_explanation
    );

    request
        .response_tx
        .send(ToolPermissionResponse::Deny)
        .expect("respond Deny");

    run.await.expect("join run").expect("run completes");
    assert!(
        w.client.saw("Permission denied by user"),
        "a declined prompt must deny the call as today"
    );
    assert!(
        !w.client.saw(BASH_MARKER),
        "the denied tool must not have executed"
    );
}

/// A seat deny row denies outright — shaped like a declined permission,
/// carrying the policy's why, and never entering the permission channel
/// (deny is not escalate). Deny beats yolo, per the precedence engine.
#[tokio::test]
async fn seat_deny_row_denies_without_prompting() {
    let mut w = world(
        probe_script(),
        Some((
            "seat-deny",
            SeatPolicy {
                mode: SeatMode::Yolo,
                allow: vec![],
                deny: vec!["seat_probe".to_string()],
            },
        )),
    )
    .await;

    w.agent.run("go".to_string()).await.expect("run completes");

    assert!(
        !w.client.saw(PROBE_MARKER),
        "a seat deny row must keep the tool from executing"
    );
    assert!(
        w.client.saw("Permission denied by seat policy"),
        "the deny must be shaped like a declined permission, naming the seat policy"
    );
    assert!(
        w.client.saw("matches the seat's deny list"),
        "the policy's why-sentence must travel with the deny"
    );
    assert!(
        w.permission_rx.try_recv().is_err(),
        "deny is not escalate — the channel must never see the call"
    );
}
