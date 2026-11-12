//! Regression tests for the absolute + percentage context caps.
//!
//! | case | what it pins |
//! |---|---|
//! | soft cap | crossing it compacts the history at the turn boundary — a `CompactionStarted` event fires |
//! | soft cap control | a history UNDER the soft cap is not compacted: the cap is what triggers the cut |
//! | hard cap | a turn that grows past the hard cap MID-TURN compacts the request list before the next provider call |
//!
//! The hard-cap case measures the request the turn boundary actually sent
//! (phase 1) and then arms the cap just above it (phase 2). The cap is
//! therefore relative to the real request — the workspace context, the
//! system prompt and the turn context are all in that number — instead of
//! guessing at them.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::stream::BoxStream;

use operant_core::agent::{
    AgentConfig, AgentEvent, ChatRequest, ModelClient, OperantAgent, StreamChunk,
};
use operant_core::client::{
    ChatResponse, Choice, Message, MessageDelta, Role, ToolCallDelta, ToolCallFunction, Usage,
};
use operant_core::context_management::{ContextCaps, estimate_total_tokens};
use operant_core::database::Database;
use operant_core::error::{Error, Result};
use operant_core::schema::ToolSchema;
use operant_core::tools::{OperantTool, ToolContext, ToolRegistry, ToolResult};

const FLOOD: &str = "flood_probe";

// ── Scripted model ──────────────────────────────────────────────

struct ScriptedClient {
    responses: Mutex<Vec<ChatResponse>>,
    seen: Mutex<Vec<Vec<Message>>>,
}

impl ScriptedClient {
    fn new(responses: Vec<ChatResponse>) -> Self {
        Self {
            responses: Mutex::new(responses),
            seen: Mutex::new(Vec::new()),
        }
    }

    /// The message list of request `index`, as the model received it.
    fn request(&self, index: usize) -> Vec<Message> {
        self.seen.lock().unwrap()[index].clone()
    }

    fn requests(&self) -> usize {
        self.seen.lock().unwrap().len()
    }
}

#[async_trait]
impl ModelClient for ScriptedClient {
    fn provider_name(&self) -> &str {
        "scripted"
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
        self.seen.lock().unwrap().push(request.messages.clone());
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

/// A tool whose body is big enough to push the request over a cap, and
/// small enough to stay under the tool-result spend guard.
struct FloodTool {
    executed: Arc<std::sync::atomic::AtomicUsize>,
}

#[async_trait]
impl OperantTool for FloodTool {
    fn name(&self) -> &str {
        FLOOD
    }

    fn description(&self) -> &str {
        "Returns a large body (test fixture)."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            FLOOD,
            "Returns a large body (test fixture).",
            serde_json::json!({"type": "object", "properties": {}}),
        )
    }

    async fn execute(&self, _args: serde_json::Value, _context: ToolContext) -> ToolResult {
        self.executed
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        ToolResult::success(FLOOD, serde_json::json!({"body": "x".repeat(4_000)}))
    }
}

// ── Agent construction ──────────────────────────────────────────

fn test_config(model: &str) -> AgentConfig {
    AgentConfig {
        reasoning_effort: String::new(),
        reasoning_effort_param: Default::default(),
        context_semantic_compaction: false,
        model: model.to_string(),
        max_trajectories: 0,
        show_turn_attribution: true,
        max_iterations: 4,
        tool_timeout: Duration::from_secs(5),
        request_timeout: Duration::from_secs(10),
        model_request_timeouts: Default::default(),
        system_prompt: Some("You are a test agent.".to_string()),
        stream: false,
        context_window: 1_000_000,
        max_tool_result_share: operant_core::context_management::DEFAULT_MAX_TOOL_RESULT_SHARE,
        max_tool_result_total_share: operant_core::context_management::DEFAULT_MAX_TOOL_RESULT_TOTAL_SHARE,
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
        guardrail_exempt_tools: Vec::new(),
        ponytail: Default::default(),
    }
}

/// Build an agent with the given caps, an optional tool, and a channel that
/// carries the turn's events.
async fn build_agent(
    client: Arc<ScriptedClient>,
    caps: ContextCaps,
    db_name: &str,
    with_flood_tool: bool,
) -> (OperantAgent, tokio::sync::mpsc::Receiver<AgentEvent>) {
    let temp = tempfile::tempdir().unwrap();
    let registry = ToolRegistry::new(Duration::from_secs(5));
    if with_flood_tool {
        registry
            .register(FloodTool {
                executed: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            })
            .await
            .unwrap();
    }
    let (event_tx, event_rx) = tokio::sync::mpsc::channel(64);
    let agent = OperantAgent::with_events(
        test_config("demo-context-caps"),
        Box::new(ScriptedClientHandle(Arc::clone(&client))),
        registry,
        Arc::new(Database::init(temp.path().join(db_name)).unwrap()),
        event_tx,
    )
    .with_context_caps(caps);
    (agent, event_rx)
}

/// Seed a history of `count` user messages, each `size` characters long.
async fn seed_history(agent: &OperantAgent, count: usize, size: usize) {
    for i in 0..count {
        agent
            .add_message(Message::user(format!("history {i} {}", "x".repeat(size))))
            .await;
    }
}

fn drain_events(rx: &mut tokio::sync::mpsc::Receiver<AgentEvent>) -> Vec<AgentEvent> {
    let mut events = Vec::new();
    while let Ok(event) = rx.try_recv() {
        events.push(event);
    }
    events
}

fn compactions(events: &[AgentEvent]) -> usize {
    events
        .iter()
        .filter(|e| matches!(e, AgentEvent::CompactionStarted { .. }))
        .count()
}

/// The installed operator configuration supplies the caps even when the
/// host does not make a separate builder call. ACP and delegated children
/// used to silently fall back to `ContextCaps::default()` here.
#[tokio::test]
async fn configured_caps_are_used_without_an_explicit_builder_override() {
    let mut config = operant_core::config::runtime_config();
    config.agent.context_window = 8_000;
    config.agent.context_soft_cap_tokens = 2_000;
    config.agent.context_soft_cap_percent = 0.0;
    config.agent.context_hard_cap_tokens = 5_000;
    config.agent.context_hard_cap_percent = 0.0;
    operant_core::config::install_runtime_config(config);

    let client = Arc::new(ScriptedClient::new(vec![text_response("answered")]));
    let temp = tempfile::tempdir().unwrap();
    let registry = ToolRegistry::new(Duration::from_secs(5));
    let (event_tx, mut rx) = tokio::sync::mpsc::channel(64);
    let mut agent_config = test_config("demo-context-caps");
    agent_config.context_window = 8_000;
    let agent = OperantAgent::with_events(
        agent_config,
        Box::new(ScriptedClientHandle(Arc::clone(&client))),
        registry,
        Arc::new(Database::init(temp.path().join("configured_caps.sqlite")).unwrap()),
        event_tx,
    );
    seed_history(&agent, 40, 500).await;

    agent
        .run("now answer".to_string())
        .await
        .expect("the turn must complete");
    let events = drain_events(&mut rx);
    assert_eq!(
        compactions(&events),
        1,
        "the installed 2k soft cap must compact the ~5k history without an explicit builder override"
    );
}

/// Both axes disabled — the agent behaves exactly as it did before the caps
/// existed.
fn caps_off() -> ContextCaps {
    ContextCaps {
        soft_tokens: 0,
        soft_percent: 0.0,
        hard_tokens: 0,
        hard_percent: 0.0,
    }
}

// ── 1. Soft cap: crossing it compacts at the turn boundary ──────

/// A history over the soft cap must be compacted BEFORE the request goes
/// out — that is the "auto-compact at turn end, before the hard cap"
/// contract. The observable is the compaction event the ladder emits.
#[tokio::test]
async fn soft_cap_compacts_the_history_at_the_turn_boundary() {
    let client = Arc::new(ScriptedClient::new(vec![text_response("answered")]));
    let (agent, mut rx) = build_agent(
        Arc::clone(&client),
        ContextCaps {
            soft_tokens: 2_000,
            soft_percent: 0.0,
            hard_tokens: 500_000,
            hard_percent: 0.0,
        },
        "soft_cap.sqlite",
        false,
    )
    .await;
    // ~5 000 tokens of history — over the 2 000-token soft cap.
    seed_history(&agent, 40, 500).await;

    let result = agent
        .run("now answer".to_string())
        .await
        .expect("the turn must complete");
    assert_eq!(result.content, "answered");

    let events = drain_events(&mut rx);
    assert_eq!(
        compactions(&events),
        1,
        "crossing the soft cap must compact the history once, at the turn boundary"
    );
    let (before, after) = events
        .iter()
        .find_map(|e| match e {
            AgentEvent::CompactionCompleted {
                tokens_before,
                tokens_after,
                ..
            } => Some((*tokens_before, *tokens_after)),
            _ => None,
        })
        .expect("the ladder must report what it compacted");
    assert!(
        after <= before,
        "the compaction must not grow the request ({before} -> {after})"
    );
}

// ── 2. Soft cap control: under the cap nothing is compacted ─────

/// The control for the case above: the SAME history, with the soft cap
/// raised above it, is sent as-is. Without this, a test that only proves
/// "some compaction happened" would pass even if the soft cap were never
/// consulted.
#[tokio::test]
async fn history_under_the_soft_cap_is_not_compacted() {
    let client = Arc::new(ScriptedClient::new(vec![text_response("answered")]));
    let (agent, mut rx) = build_agent(
        Arc::clone(&client),
        ContextCaps {
            soft_tokens: 500_000,
            soft_percent: 0.0,
            hard_tokens: 900_000,
            hard_percent: 0.0,
        },
        "soft_cap_control.sqlite",
        false,
    )
    .await;
    seed_history(&agent, 40, 500).await;

    agent
        .run("now answer".to_string())
        .await
        .expect("the turn must complete");

    let events = drain_events(&mut rx);
    assert_eq!(
        compactions(&events),
        0,
        "a history under the soft cap must not be compacted"
    );
}

// ── 3. Hard cap: a turn that grows past it compacts mid-turn ───

/// The mid-turn case: the turn boundary sent a request UNDER the hard cap,
/// the tool then added ~1 000 tokens, and the NEXT request had to be
/// compacted back under the cap before it went out.
///
/// The cap is armed relative to the request the boundary actually sent
/// (phase 1 measures it, phase 2 arms it just above), so the test does not
/// depend on how big the system prompt, the workspace context or the turn
/// context happen to be in this environment.
#[tokio::test]
async fn hard_cap_compacts_the_request_mid_turn() {
    // Phase 1 — measure the request the turn boundary sends, with the caps
    // off so nothing is compacted.
    let probe = Arc::new(ScriptedClient::new(vec![text_response("probe")]));
    let (probe_agent, _probe_rx) = build_agent(
        Arc::clone(&probe),
        caps_off(),
        "hard_cap_probe.sqlite",
        false,
    )
    .await;
    seed_history(&probe_agent, 12, 1_000).await;
    probe_agent
        .run("now answer".to_string())
        .await
        .expect("the probe turn must complete");
    let baseline = estimate_total_tokens(&probe.request(0));
    assert!(
        baseline > 1_000,
        "the seeded history must be visible in the request, got {baseline} tokens"
    );

    // Phase 2 — arm the hard cap just above the measured request, then let
    // the tool push the turn over it.
    let hard = baseline + 200;
    let client = Arc::new(ScriptedClient::new(vec![
        tool_call_response(FLOOD, "{}"),
        text_response("answered after the mid-turn compaction"),
    ]));
    let (agent, mut rx) = build_agent(
        Arc::clone(&client),
        ContextCaps {
            soft_tokens: 0,
            soft_percent: 0.0,
            hard_tokens: hard,
            hard_percent: 0.0,
        },
        "hard_cap.sqlite",
        true,
    )
    .await;
    seed_history(&agent, 12, 1_000).await;

    let result = agent
        .run("now answer".to_string())
        .await
        .expect("the turn must complete — the cap compacts, it does not abort");
    assert_eq!(result.content, "answered after the mid-turn compaction");
    assert_eq!(client.requests(), 2, "one tool turn plus the answer");

    let first = estimate_total_tokens(&client.request(0));
    let second = estimate_total_tokens(&client.request(1));
    assert!(
        first >= baseline,
        "the turn-boundary request must NOT have been compacted ({first} < {baseline}) — \
         the cap is a mid-turn guard, and this assertion keeps the test honest about \
         which path fired"
    );
    assert!(
        second < hard,
        "the request after the tool ran must be under the hard cap ({second} >= {hard})"
    );

    let events = drain_events(&mut rx);
    assert_eq!(
        compactions(&events),
        1,
        "exactly one compaction — the mid-turn one"
    );
}
