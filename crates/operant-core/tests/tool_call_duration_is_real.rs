//! **The `ObserverEvent::ToolCall` duration must be the tool's real wall-clock
//! time, not a structural zero.**
//!
//! The defect: `crates/operant-core/src/agent/run.rs` emitted
//! `ObserverEvent::ToolCall { duration: Duration::from_millis(0), .. }` — a
//! hardcoded zero — while the timing was being measured correctly at all three
//! `execute_tools` sites in `agent/stream.rs`, each of which already fed
//! `TurnEndBus::record_tool_duration`, and then thrown away on the path to the
//! observer.
//!
//! So `ConsoleObserver`'s `duration_ms` field (`observer.rs:302`) was
//! structurally always `0`, and an operator asking "which tool made this turn
//! take 40 seconds?" got an answer with no durations in it. `turn_end.rs`'s own
//! contract test already pinned that all three branches time their call — the
//! timing existed and was discarded one hop later.
//!
//! Why the bus cannot be reused: `TurnEndBus::tool_durations_ms` is a `Vec<u64>`
//! in completion order that deliberately discards the tool name
//! (`turn_end.rs:127`). `ObserverEvent::ToolCall` pairs ONE tool name with ONE
//! duration, so a name-less vector cannot fill it without an index→tool map
//! that goes stale the moment a call is dropped or reordered. The fix is a
//! per-call-ID map on the agent.
//!
//! **The proof matters more than the assertion.** A test that only checks
//! `duration > Duration::ZERO` would also pass if the code timestamped the
//! *arrival of the result* rather than the *start of execution*, or reported a
//! constant that merely cleared the threshold. These tests sleep INSIDE the
//! tool and require the report to cover that sleep, plus an instant-tool control
//! that fails if the report is a fixed number.

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
use operant_core::observer::{Observer, ObserverEvent, ObserverMetric};
use operant_core::schema::ToolSchema;
use operant_core::tools::{OperantTool, ToolContext, ToolRegistry, ToolResult};

const SLEEPY: &str = "sleepy_probe";
const SIZED: &str = "sized_probe";
const QUICK: &str = "quick_probe";

/// How long the slow tool sleeps. The assertion floor sits well below it (a
/// mismeasured start reports near-zero) and the ceiling well above (the wrapper
/// cannot report more than the tool took plus scheduling slack).
const SLEEP_MS: u64 = 220;
/// A duration below this cannot have covered the sleep.
const MIN_EXPECTED_MS: u64 = 120;

/// Records every observer event it is handed.
struct RecordingObserver {
    events: Mutex<Vec<ObserverEvent>>,
}

impl RecordingObserver {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            events: Mutex::new(Vec::new()),
        })
    }

    fn tool_calls(&self) -> Vec<(String, Duration, bool)> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                ObserverEvent::ToolCall {
                    tool,
                    duration,
                    success,
                    ..
                } => Some((tool.clone(), *duration, *success)),
                _ => None,
            })
            .collect()
    }

    /// The payload sizes as reported for one tool (input bytes, output bytes).
    fn byte_counts(&self, tool: &str) -> Vec<(usize, usize)> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                ObserverEvent::ToolCall {
                    tool: name,
                    input_bytes,
                    output_bytes,
                    ..
                } if name == tool => Some((*input_bytes, *output_bytes)),
                _ => None,
            })
            .collect()
    }
}

impl Observer for RecordingObserver {
    fn name(&self) -> &str {
        "recording-observer"
    }

    fn record_event(&self, event: &ObserverEvent) {
        if let Ok(mut events) = self.events.lock() {
            events.push(event.clone());
        }
    }

    fn record_metric(&self, _metric: &ObserverMetric) {}
}

/// Sleeps `SLEEP_MS`, then succeeds. The sleep is the measurement: any correct
/// implementation must report at least most of it.
struct SleepyTool {
    entered: Arc<AtomicUsize>,
}

#[async_trait]
impl OperantTool for SleepyTool {
    fn name(&self) -> &str {
        SLEEPY
    }

    fn description(&self) -> &str {
        "Sleeps, then succeeds (drives the duration measurement)."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            SLEEPY,
            "Sleeps, then succeeds (drives the duration measurement).",
            serde_json::json!({"type": "object", "properties": {}}),
        )
    }

    async fn execute(&self, _args: serde_json::Value, _context: ToolContext) -> ToolResult {
        self.entered.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(SLEEP_MS)).await;
        ToolResult::success(SLEEPY, serde_json::json!({"slept_ms": SLEEP_MS}))
    }
}

/// Returns immediately — the control that proves the report is not a fixed
/// constant that merely cleared the threshold by tuning.
struct QuickTool;

#[async_trait]
impl OperantTool for QuickTool {
    fn name(&self) -> &str {
        QUICK
    }

    fn description(&self) -> &str {
        "Returns instantly (the duration control)."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            QUICK,
            "Returns instantly (the duration control).",
            serde_json::json!({"type": "object", "properties": {}}),
        )
    }

    async fn execute(&self, _args: serde_json::Value, _context: ToolContext) -> ToolResult {
        ToolResult::success(QUICK, serde_json::json!({"instant": true}))
    }
}

/// One tool call, then a final answer. Replay-safe: the last response repeats,
/// so an unexpected extra turn still terminates.
struct ScriptedClient {
    responses: Mutex<Vec<ChatResponse>>,
}

impl ScriptedClient {
    fn new(tool_name: &str) -> Arc<Self> {
        Self::new_with(tool_name, tool_call_response)
    }

    /// Same as `new`, but the caller shapes the tool-call response (here, to
    /// send arguments of a distinctive length).
    fn new_with(tool_name: &str, first: impl FnOnce(&str) -> ChatResponse) -> Arc<Self> {
        let responses = vec![first(tool_name), text_response("done")];
        Arc::new(Self {
            responses: Mutex::new(responses),
        })
    }

    fn pop(&self) -> ChatResponse {
        let mut responses = self.responses.lock().unwrap();
        if responses.len() > 1 {
            responses.remove(0)
        } else {
            responses[0].clone()
        }
    }
}

fn tool_call_response(tool_name: &str) -> ChatResponse {
    tool_call_response_with_args(tool_name, "{}")
}

fn tool_call_response_with_args(tool_name: &str, args: &str) -> ChatResponse {
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
                    id: Some("call-1".to_string()),
                    call_type: Some("function".to_string()),
                    function: Some(ToolCallFunction {
                        name: tool_name.to_string(),
                        arguments: args.to_string(),
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

struct ScriptedClientHandle(Arc<ScriptedClient>);

#[async_trait]
impl ModelClient for ScriptedClientHandle {
    fn provider_name(&self) -> &str {
        "scripted"
    }

    async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse> {
        Ok(self.0.pop())
    }

    async fn chat_streaming(
        &self,
        _request: ChatRequest,
    ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
        Err(Error::Config("streaming not used in this test".to_string()))
    }
}

fn test_config() -> AgentConfig {
    AgentConfig {
        reasoning_effort: String::new(),
        reasoning_effort_param: Default::default(),
        context_semantic_compaction: false,
        model: "demo".to_string(),
        max_trajectories: 0,
        max_iterations: 3,
        tool_timeout: Duration::from_secs(30),
        request_timeout: Duration::from_secs(30),
        system_prompt: Some("You are a test agent.".to_string()),
        stream: false,
        context_window: 8000,
        max_tool_result_share: operant_core::context_management::DEFAULT_MAX_TOOL_RESULT_SHARE,
        max_healing_attempts: 1,
        fallback_models: Vec::new(),
        fallback_on_errors: false,
        loop_detection_enabled: false,
        approval_mode: "off".to_string(),
        approval_allowlist: Vec::new(),
        approval_allowlist_path: None,
        record_trajectories: false,
        skill_nudge_interval: 0,
        memory_review_interval: 0,
        max_retries: 1,
        tool_search: Default::default(),
        guardrail_exempt_tools: Vec::new(),
        ponytail: Default::default(),
    }
}

/// Both tools are registered in every agent: the scripted model decides which
/// one is called, and registering only one would let a registration accident
/// masquerade as a duration measurement.
async fn build_agent(
    tool_name: &str,
    observer: Arc<RecordingObserver>,
    db_path: std::path::PathBuf,
) -> OperantAgent {
    let registry = ToolRegistry::new(Duration::from_secs(30));
    registry
        .register_dyn(Arc::new(SleepyTool {
            entered: Arc::new(AtomicUsize::new(0)),
        }))
        .await
        .unwrap();
    registry.register_dyn(Arc::new(QuickTool)).await.unwrap();

    OperantAgent::new(
        test_config(),
        Box::new(ScriptedClientHandle(ScriptedClient::new(tool_name))),
        registry,
        Arc::new(Database::init(db_path).unwrap()),
    )
    .with_observer(observer)
}

#[tokio::test]
async fn a_slow_tools_observer_duration_covers_the_time_it_actually_took() {
    let dir = tempfile::tempdir().unwrap();
    let observer = RecordingObserver::new();
    let agent = build_agent(SLEEPY, observer.clone(), dir.path().join("db.sqlite")).await;

    agent
        .run("call the slow tool once".to_string())
        .await
        .expect("the turn must complete");

    let calls = observer.tool_calls();
    let slow: Vec<_> = calls.iter().filter(|(t, _, _)| t == SLEEPY).collect();
    assert_eq!(slow.len(), 1, "expected exactly one ToolCall for {SLEEPY}");

    let (_, duration, success) = slow[0];
    assert!(success, "the tool succeeded, so the event must say so");

    let ms = duration.as_millis() as u64;
    assert!(
        ms >= MIN_EXPECTED_MS,
        "the reported duration must cover the tool's {SLEEP_MS}ms sleep — got {ms}ms. \
         A near-zero value means the observer is still being fed a constant (the \
         iter-1505 defect), or it timestamps the RESULT rather than the START"
    );
    assert!(
        ms <= SLEEP_MS + 5_000,
        "the reported duration cannot exceed the tool's own time plus scheduling \
         slack — got {ms}ms for a {SLEEP_MS}ms tool, so the clock is being read twice"
    );
}

#[tokio::test]
async fn an_instant_tool_reports_a_tiny_duration_not_the_slow_tools_one() {
    // The control: if the report were a fixed constant that merely cleared the
    // first test's threshold, the instant tool would report it too.
    let dir = tempfile::tempdir().unwrap();
    let observer = RecordingObserver::new();
    let agent = build_agent(QUICK, observer.clone(), dir.path().join("db.sqlite")).await;

    agent
        .run("call the quick tool once".to_string())
        .await
        .expect("the turn must complete");

    let calls = observer.tool_calls();
    let quick: Vec<_> = calls.iter().filter(|(t, _, _)| t == QUICK).collect();
    assert_eq!(quick.len(), 1, "expected exactly one ToolCall for {QUICK}");

    let (_, duration, _) = quick[0];
    assert!(
        duration.as_millis() < MIN_EXPECTED_MS as u128,
        "an instant tool must report a small duration — got {duration:?}, which means \
         the report is not measuring this tool at all"
    );
}

#[test]
fn the_emit_site_must_not_be_fed_a_constant_zero() {
    // A source-level pin on the exact defect: the one emit site in run.rs was
    // `duration: Duration::from_millis(0)`. The fix reads a measured value.
    let run_rs = include_str!("../src/agent/run.rs");
    assert!(
        !run_rs.contains("duration: Duration::from_millis(0)"),
        "run.rs is still emitting a constant-zero ToolCall duration — the observer \
         must read the duration recorded by execute_tools"
    );
    assert!(
        run_rs.contains("self.tool_duration_for(&result.tool_call_id)"),
        "run.rs must read the recorded duration for the tool call it is reporting"
    );

    // ...and every branch that records a turn_end timing must also record the
    // per-call duration, or the observer goes back to zero on that path. Count
    // both kinds: the fn definition also matches `record_tool_duration(`.
    let stream_rs = include_str!("../src/agent/stream.rs");
    let bus_records = stream_rs.matches("bus.record_tool_duration(").count();
    let self_records = stream_rs.matches("self.record_tool_duration(").count();
    assert_eq!(
        bus_records, 3,
        "expected three turn_end timing sites in stream.rs (single, serialized, \
         concurrent) — found {bus_records}; turn_end.rs's own contract pins this, so a \
         changed count means a branch was added or removed and both recordings must \
         follow it"
    );
    assert_eq!(
        self_records, 3,
        "expected the per-call duration recorded at all three execution sites, one \
         beside each turn_end record — found {self_records}. A branch that records to \
         the bus but not per-call leaves the observer reading Duration::ZERO on that \
         path"
    );
}

// ---------------------------------------------------------------------------
// The payload sizes both directions (iter-1551)
//
// `ObserverEvent::ToolCall` grew `input_bytes`/`output_bytes`. They are only
// worth having if they describe THIS call, so every assertion below compares
// against a number derived from the fixture rather than a threshold:
//
// * `input_bytes` == the exact length of the arguments the scripted model
//   sent. Those are rendered into a string at the `ToolCallStart` emit site
//   (`stream.rs:915`) — the only place the raw text exists — so the size must
//   be parked there and read back by call id at the completion emit
//   (`run.rs:2207`).
// * `output_bytes` == the exact length of the JSON the tool returned.
//
// The two are deliberately different lengths, so a swapped-field report
// (input quoted as output) fails too — which a `> 0` assertion cannot detect.
// ---------------------------------------------------------------------------

/// The arguments the scripted model sends. Long enough that it cannot collide
/// with the result's length by accident.
const SIZED_ARGS: &str = r#"{"pad":"0123456789ABCDEF"}"#;

fn sized_tool_call_response(tool_name: &str) -> ChatResponse {
    tool_call_response_with_args(tool_name, SIZED_ARGS)
}

/// Returns a fixed, distinctive payload. Its serialized length is what
/// `output_bytes` must equal.
struct SizedTool;

#[async_trait]
impl OperantTool for SizedTool {
    fn name(&self) -> &str {
        SIZED
    }

    fn description(&self) -> &str {
        "Returns a fixed payload of known length (drives the byte counts)."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            SIZED,
            "Returns a fixed payload of known length (drives the byte counts).",
            serde_json::json!({"type": "object", "properties": {}}),
        )
    }

    async fn execute(&self, _args: serde_json::Value, _context: ToolContext) -> ToolResult {
        ToolResult::success(SIZED, serde_json::json!({"p": "abcdef"}))
    }
}

async fn build_sized_agent(
    observer: Arc<RecordingObserver>,
    db_path: std::path::PathBuf,
) -> OperantAgent {
    let registry = ToolRegistry::new(Duration::from_secs(30));
    registry.register_dyn(Arc::new(SizedTool)).await.unwrap();

    OperantAgent::new(
        test_config(),
        Box::new(ScriptedClientHandle(ScriptedClient::new_with(
            SIZED,
            sized_tool_call_response,
        ))),
        registry,
        Arc::new(Database::init(db_path).unwrap()),
    )
    .with_observer(observer)
}

#[tokio::test]
async fn the_observer_sees_the_real_byte_count_in_both_directions() {
    let dir = tempfile::tempdir().unwrap();
    let observer = RecordingObserver::new();
    let agent = build_sized_agent(observer.clone(), dir.path().join("db.sqlite")).await;

    agent
        .run("call the sized tool once".to_string())
        .await
        .expect("the turn must complete");

    let calls = observer.tool_calls();
    let sized: Vec<_> = calls.iter().filter(|(t, ..)| t == SIZED).collect();
    assert_eq!(sized.len(), 1, "expected exactly one ToolCall for {SIZED}");
    let (_, duration, success) = sized[0];
    assert!(success, "the tool succeeded");
    assert!(duration.as_millis() < 120, "the sized tool is instant");

    let counts = observer.byte_counts(SIZED);
    assert_eq!(counts.len(), 1, "one byte-count report for {SIZED}");
    let in_out = counts[0];

    assert_eq!(
        in_out.0,
        SIZED_ARGS.len(),
        "input_bytes must be the byte length of the arguments the model sent \
         ({SIZED_ARGS:?}) — got {}. A zero here means the size was never parked at the \
         ToolCallStart site, or was parked under a different call id",
        in_out.0
    );
    assert_eq!(
        in_out.1,
        serde_json::to_string(&serde_json::json!({"p": "abcdef"}))
            .expect("the fixture serializes")
            .len(),
        "output_bytes must be the byte length of the JSON the tool returned — got {}",
        in_out.1
    );

    assert_ne!(
        in_out.0, in_out.1,
        "the fixture's two lengths are different on purpose; a swapped report (the \
         output length quoted as the input length) still passes a > 0 check"
    );
}

#[test]
fn neither_byte_count_is_a_hardcoded_zero() {
    // The two failure modes this field can silently regress into: a literal in
    // the initializer, or an input size read nowhere.
    let run_rs = include_str!("../src/agent/run.rs");
    assert!(
        !run_rs.contains("input_bytes: 0"),
        "run.rs is emitting a constant-zero input_bytes"
    );
    assert!(
        run_rs.contains("self.tool_input_bytes_for(&result.tool_call_id)"),
        "run.rs must read the parked input size for the call it is reporting"
    );
    assert!(
        run_rs.contains("output_bytes: result.content.len()"),
        "run.rs must report the result's own content length"
    );

    let stream_rs = include_str!("../src/agent/stream.rs");
    let parked = stream_rs.matches("self.record_tool_input_bytes(").count();
    assert!(
        parked >= 1,
        "stream.rs must park the input size where the arguments are rendered — \
         found no record_tool_input_bytes call"
    );
    assert!(
        stream_rs.contains("map.insert(tool_call_id.to_string(), bytes)"),
        "the parked size must be keyed by tool call id, not appended to a list \
         that goes stale when a call is dropped or reordered"
    );
}
