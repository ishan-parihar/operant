//! Scripted-model tests for the S3 background-review discipline — the
//! defect from `docs/AGENT-LOOP-FIX-EXECUTION-PLAN.md` where one long turn
//! fired up to 4 background reviews mid-loop (measured 05:10 specimen),
//! each racing the turn for provider slots, all reviewing a PARTIAL
//! transcript and concluding nothing.
//!
//! The discipline under test:
//!
//! - **Once per turn**: the in-loop cadence trigger (`run.rs` skill-nudge
//!   block) arms a `review_fired` flag at most once; the per-iteration
//!   counter bump/persist semantics are untouched.
//! - **Deferred to exit**: the actual `spawn_background_review` runs on
//!   the turn's way OUT (TextResponse / GraceCall / CircuitBreaker), so
//!   the review sees the COMPLETE transcript and never competes with
//!   the turn. Interrupt/Error exits skip it — the operator already
//!   knows why the turn stopped.
//! - **No-op early exit**: the review daemon stops after 2 consecutive
//!   rounds whose successful tool calls wrote nothing (reads only:
//!   memory_search / memory_recall / skill_view); any successful write
//!   resets the streak. The iteration cap (5) stays the outer bound.
//!
//! Every case scripts the failure or its control. Revert the once-per-turn
//! gate and `fired_once…` sees 3 review daemons (interval 10 over 30
//! iterations); revert the deferral and the review chat interleaves with
//! the turn's requests — consuming scripted responses and breaking the
//! exact-count and after-the-turn assertions; revert the no-op exit and
//! the read-only daemon burns 5 chats instead of 2.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::stream::BoxStream;

use operant_core::agent::{AgentConfig, ChatRequest, ModelClient, OperantAgent, StreamChunk};
use operant_core::client::{
    ChatResponse, Choice, Message, MessageDelta, Role, ToolCallDelta, ToolCallFunction, Usage,
};
use operant_core::database::Database;
use operant_core::error::{Error, Result};
use operant_core::schema::ToolSchema;
use operant_core::tools::{OperantTool, ToolContext, ToolRegistry, ToolResult};

/// The review harness marker — present in every request the review daemon
/// makes, absent from every main-loop request.
const REVIEW_MARK: &str = "[Background review context]";
/// Text the daemon's default response carries: round ends with no tool
/// calls, daemon stops immediately (the old no-op specimen, one round).
const NOTHING_TO_SAVE: &str = "Nothing to save";
const WORKER: &str = "worker_probe";

/// The daemon_pool label `spawn_background_review` uses — tests drain it
/// so the daemon is awaited deterministically instead of polled.
const REVIEW_DAEMON_LABEL: &str = "probe-extra";

/// `daemon_pool` is process-global (a static handle table), so tests that
/// spawn or await review daemons must not race each other's handles.
static REVIEW_TEST_SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Every LLM response triggers a models.dev price lookup
/// (`compress.rs::emit_usage_and_cost`). On a cold cache that is a REAL
/// network fetch, and `models_dev.rs`'s cold path never persists the
/// result — so a hermetic test process would do one live HTTPS fetch of
/// the catalog PER scripted response (~3s each here; on an offline CI
/// runner, a 15s timeout per response instead). Point `HOME` at the
/// test's temp dir and pre-seed the on-disk cache so the lookup is a
/// local disk read: no network, deterministic, fast.
///
/// SAFETY-owned env swap: every test in this binary holds
/// `REVIEW_TEST_SERIAL` across its whole body, so the process-global
/// `HOME` change is never observed by a concurrent test thread.
fn prime_models_dev_cache(temp: &tempfile::TempDir) {
    let home = temp.path().join("home");
    std::fs::create_dir_all(home.join(".operant")).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    std::fs::write(
        home.join(".operant").join("models_dev_cache.json"),
        format!(r#"{{"models":[],"providers":[],"cached_at":{now}}}"#),
    )
    .unwrap();
    // SAFETY: serialized by REVIEW_TEST_SERIAL (see doc comment).
    unsafe { std::env::set_var("HOME", &home) };
}

/// Scripted response: a model answer or a hard chat error.
enum ScriptedItem {
    Model(ChatResponse),
    Fail(String),
}

struct RecordedRequest {
    messages: Vec<Message>,
}

struct ScriptedClient {
    responses: Mutex<Vec<ScriptedItem>>,
    recorded: Mutex<Vec<RecordedRequest>>,
}

impl ScriptedClient {
    fn new(responses: Vec<ScriptedItem>) -> Self {
        Self {
            responses: Mutex::new(responses),
            recorded: Mutex::new(Vec::new()),
        }
    }

    fn requests_snapshot(&self) -> Vec<RecordedRequest> {
        self.recorded
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Indices (into recorded order) of the review daemon's requests.
    fn review_request_indices(&self) -> Vec<usize> {
        self.requests_snapshot()
            .iter()
            .enumerate()
            .filter(|(_, r)| r.messages.iter().any(|m| m.content.contains(REVIEW_MARK)))
            .map(|(i, _)| i)
            .collect()
    }
}

impl Clone for RecordedRequest {
    fn clone(&self) -> Self {
        Self {
            messages: self.messages.clone(),
        }
    }
}

/// Lets the shared `Arc<ScriptedClient>` survive the `Box<dyn ModelClient>`
/// hand-off (the review daemon clones the SAME client — which is exactly
/// what lets these tests record the daemon's requests too).
struct ScriptedClientHandle(Arc<ScriptedClient>);

#[async_trait]
impl ModelClient for ScriptedClientHandle {
    fn provider_name(&self) -> &str {
        "scripted"
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
        {
            let recorded = &self.0.recorded;
            recorded
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(RecordedRequest {
                    messages: request.messages,
                });
        }
        let mut guard = self.0.responses.lock().unwrap_or_else(|e| e.into_inner());
        match guard.pop_first() {
            Some(ScriptedItem::Model(r)) => Ok(r),
            Some(ScriptedItem::Fail(msg)) => Err(Error::Agent(msg)),
            // Exhausted script: a no-tool-call answer ends the daemon in
            // round 1 (the old no-op specimen) without extra scripting.
            None => Ok(text_response(NOTHING_TO_SAVE)),
        }
    }

    async fn chat_streaming(
        &self,
        _request: ChatRequest,
    ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
        Err(Error::Agent("streaming not used in this test".into()))
    }
}

/// Pop-front helper (scripts are consumed in scripted order).
trait PopFirst {
    fn pop_first(&mut self) -> Option<ScriptedItem>;
}

impl PopFirst for Vec<ScriptedItem> {
    fn pop_first(&mut self) -> Option<ScriptedItem> {
        if self.is_empty() {
            None
        } else {
            Some(self.remove(0))
        }
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
                    id: Some(format!("call_{tool}_{}", arguments.len())),
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

/// Varying-args tool calls — identical calls 6× in a row trip the
/// repeat-guardrail (a different, correct breaker) and would abort the
/// turn before the review cadence under test ever fires.
fn varying_calls(tool: &str, n: usize) -> Vec<ScriptedItem> {
    (0..n)
        .map(|i| ScriptedItem::Model(tool_call_response(tool, &format!("{{\"i\":{i}}}"))))
        .collect()
}

/// Instant success — the main turn's tool.
struct WorkerTool;

#[async_trait]
impl OperantTool for WorkerTool {
    fn name(&self) -> &str {
        WORKER
    }

    fn description(&self) -> &str {
        "Always succeeds (test fixture)."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            WORKER,
            "Always succeeds (test fixture).",
            serde_json::json!({"type": "object", "properties": {}}),
        )
    }

    async fn execute(&self, _args: serde_json::Value, _context: ToolContext) -> ToolResult {
        ToolResult::success(WORKER, serde_json::json!({"ok": true}))
    }
}

/// Fake write tool — same name as the real memory write so the daemon's
/// whitelist executes it and `is_review_write_tool` counts the round as
/// a write.
struct MemoryStoreTool;

#[async_trait]
impl OperantTool for MemoryStoreTool {
    fn name(&self) -> &str {
        "memory_store"
    }

    fn description(&self) -> &str {
        "Fake memory write (test fixture)."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "memory_store",
            "Fake memory write (test fixture).",
            serde_json::json!({"type": "object", "properties": {}}),
        )
    }

    async fn execute(&self, _args: serde_json::Value, _context: ToolContext) -> ToolResult {
        ToolResult::success("memory_store", serde_json::json!({"stored": true}))
    }
}

/// Fake read tool — succeeds but is a read, so a round of only these must
/// count toward the no-op early exit (the `{"count":0}` specimen).
struct MemorySearchTool;

#[async_trait]
impl OperantTool for MemorySearchTool {
    fn name(&self) -> &str {
        "memory_search"
    }

    fn description(&self) -> &str {
        "Fake memory read (test fixture)."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "memory_search",
            "Fake memory read (test fixture).",
            serde_json::json!({"type": "object", "properties": {}}),
        )
    }

    async fn execute(&self, _args: serde_json::Value, _context: ToolContext) -> ToolResult {
        ToolResult::success("memory_search", serde_json::json!({"count": 0}))
    }
}

fn test_config(max_iterations: usize, skill_nudge_interval: usize) -> AgentConfig {
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
        // The cadence under test. Memory review stays off — this slice is
        // about the SKILL review path.
        skill_nudge_interval,
        memory_review_interval: 0,
        max_retries: 3,
        tool_search: Default::default(),
        guardrail_exempt_tools: Vec::new(),
    }
}

async fn build_agent(
    client: Arc<ScriptedClient>,
    max_iterations: usize,
    skill_nudge_interval: usize,
    db_path: std::path::PathBuf,
) -> OperantAgent {
    let registry = ToolRegistry::new(Duration::from_secs(5));
    registry.register_dyn(Arc::new(WorkerTool)).await.unwrap();
    registry
        .register_dyn(Arc::new(MemoryStoreTool))
        .await
        .unwrap();
    registry
        .register_dyn(Arc::new(MemorySearchTool))
        .await
        .unwrap();
    OperantAgent::new(
        test_config(max_iterations, skill_nudge_interval),
        Box::new(ScriptedClientHandle(client)),
        registry,
        Arc::new(Database::init(db_path).unwrap()),
    )
}

/// The specimen shape (2026-10-03 §1 S3): a 30-iteration turn with
/// `skill_nudge_interval=10`. The old code fired a review at iterations
/// 10, 20 AND 30 — three mid-loop reviews of partial transcripts racing
/// the turn. The discipline must deliver exactly ONE review, spawned on
/// the turn's way out: after the 31st (final) main-loop request, with
/// the COMPLETE transcript in view.
#[tokio::test]
async fn thirty_iteration_turn_fires_one_review_after_the_turn_not_mid_loop() {
    let _serial = REVIEW_TEST_SERIAL.lock().await;
    let temp = tempfile::tempdir().unwrap();
    prime_models_dev_cache(&temp);

    let mut script = varying_calls(WORKER, 30);
    script.push(ScriptedItem::Model(text_response(
        "final answer after thirty iterations",
    )));
    let client = Arc::new(ScriptedClient::new(script));
    let agent = build_agent(
        Arc::clone(&client),
        60,
        10, // skill_nudge_interval: triggers at iterations 10, 20, 30
        temp.path().join("once_per_turn.sqlite"),
    )
    .await;

    let result = agent
        .run("do some work".to_string())
        .await
        .expect("the turn must complete on its own text");

    assert_eq!(result.content, "final answer after thirty iterations");

    // Await the review daemon deterministically.
    let completed =
        operant_core::daemon_pool::drain_for_label(REVIEW_DAEMON_LABEL, Duration::from_secs(5))
            .await;
    assert_eq!(completed, 1, "exactly one review daemon must spawn");

    let snapshot = client.requests_snapshot();
    // 30 tool-call requests + 1 final-text request from the main turn…
    assert_eq!(
        snapshot.len(),
        32,
        "31 main-loop requests + exactly ONE review request"
    );

    let review_indices = client.review_request_indices();
    assert_eq!(
        review_indices,
        vec![31],
        "the review request must be the LAST request — anything earlier is \
         a mid-loop spawn reviewing a partial transcript"
    );

    // No main-loop request may carry the review harness — that is the
    // mid-loop interleaving the slice removes.
    for req in &snapshot[..31] {
        assert!(
            !req.messages.iter().any(|m| m.content.contains(REVIEW_MARK)),
            "review harness leaked into a mid-loop request"
        );
    }

    // The review saw the COMPLETE transcript: the turn's final answer is
    // in view (the old mid-loop spawns reviewed without it).
    let review_req = &snapshot[31];
    assert!(
        review_req
            .messages
            .iter()
            .any(|m| m.content.contains("final answer after thirty iterations")),
        "the deferred review must review the complete transcript"
    );
}

/// Negative control for the cadence: below the interval nothing arms, no
/// review ever spawns.
#[tokio::test]
async fn below_interval_no_review_spawns() {
    let _serial = REVIEW_TEST_SERIAL.lock().await;
    let temp = tempfile::tempdir().unwrap();
    prime_models_dev_cache(&temp);

    let mut script = varying_calls(WORKER, 5);
    script.push(ScriptedItem::Model(text_response("done early")));
    let client = Arc::new(ScriptedClient::new(script));
    let agent = build_agent(
        Arc::clone(&client),
        20,
        10, // 5 iterations < 10 → cadence never fires
        temp.path().join("below_interval.sqlite"),
    )
    .await;

    let result = agent
        .run("do some work".to_string())
        .await
        .expect("turn completes normally");
    assert_eq!(result.content, "done early");

    operant_core::daemon_pool::drain_for_label(REVIEW_DAEMON_LABEL, Duration::from_millis(300))
        .await;
    assert!(
        client.review_request_indices().is_empty(),
        "no review may spawn below the cadence interval"
    );
}

/// The Interrupt/Error rule: a turn whose cadence DID arm (10 iterations,
/// interval 10) but which then dies on a chat error must NOT spawn the
/// review — the operator already knows why the turn stopped, and there
/// is no final answer for the review to evaluate.
#[tokio::test]
async fn error_exit_skips_the_armed_review() {
    let _serial = REVIEW_TEST_SERIAL.lock().await;
    let temp = tempfile::tempdir().unwrap();
    prime_models_dev_cache(&temp);

    let mut script = varying_calls(WORKER, 10);
    script.push(ScriptedItem::Fail("provider exploded".to_string()));
    let client = Arc::new(ScriptedClient::new(script));
    let agent = build_agent(
        Arc::clone(&client),
        20,
        10, // arms at iteration 10…
        temp.path().join("error_exit.sqlite"),
    )
    .await;

    let result = agent.run("do some work".to_string()).await;
    assert!(result.is_err(), "the scripted chat error must end the turn");

    operant_core::daemon_pool::drain_for_label(REVIEW_DAEMON_LABEL, Duration::from_millis(300))
        .await;
    assert!(
        client.review_request_indices().is_empty(),
        "an error exit must skip the armed review"
    );
}

/// The no-op early exit (the `{"count":0}` specimen): a review daemon
/// whose rounds only READ stops after 2 consecutive no-write rounds —
/// BEFORE its 5-round iteration cap. Five read rounds are scripted; only
/// two may be consumed.
#[tokio::test]
async fn noop_rounds_exit_the_daemon_before_the_iteration_cap() {
    let _serial = REVIEW_TEST_SERIAL.lock().await;
    let temp = tempfile::tempdir().unwrap();
    prime_models_dev_cache(&temp);

    let mut script = varying_calls(WORKER, 10);
    script.push(ScriptedItem::Model(text_response("done")));
    // Five read-only review rounds are AVAILABLE; the daemon must take
    // exactly two and stop.
    for _ in 0..5 {
        script.push(ScriptedItem::Model(tool_call_response(
            "memory_search",
            "{}",
        )));
    }
    let client = Arc::new(ScriptedClient::new(script));
    let agent = build_agent(
        Arc::clone(&client),
        20,
        10,
        temp.path().join("noop_exit.sqlite"),
    )
    .await;

    let result = agent
        .run("do some work".to_string())
        .await
        .expect("turn completes normally");
    assert_eq!(result.content, "done");

    operant_core::daemon_pool::drain_for_label(REVIEW_DAEMON_LABEL, Duration::from_secs(5)).await;

    let review_indices = client.review_request_indices();
    assert_eq!(
        review_indices,
        vec![11, 12],
        "the daemon must stop after exactly TWO read-only rounds (cap is 5)"
    );
}

/// The reset rule: one WRITE on round 1 resets the no-op streak, so the
/// daemon survives two more read-only rounds and exits on round 3 — not
/// before. Two consecutive is the verdict; a write in between must not
/// count.
#[tokio::test]
async fn a_write_round_resets_the_noop_streak() {
    let _serial = REVIEW_TEST_SERIAL.lock().await;
    let temp = tempfile::tempdir().unwrap();
    prime_models_dev_cache(&temp);

    let mut script = varying_calls(WORKER, 10);
    script.push(ScriptedItem::Model(text_response("done")));
    // Round 1: a write. Rounds 2-3: reads (two consecutive → exit at 3).
    script.push(ScriptedItem::Model(tool_call_response(
        "memory_store",
        "{}",
    )));
    for _ in 0..4 {
        script.push(ScriptedItem::Model(tool_call_response(
            "memory_search",
            "{}",
        )));
    }
    let client = Arc::new(ScriptedClient::new(script));
    let agent = build_agent(
        Arc::clone(&client),
        20,
        10,
        temp.path().join("write_resets.sqlite"),
    )
    .await;

    let result = agent
        .run("do some work".to_string())
        .await
        .expect("turn completes normally");
    assert_eq!(result.content, "done");

    operant_core::daemon_pool::drain_for_label(REVIEW_DAEMON_LABEL, Duration::from_secs(5)).await;

    let review_indices = client.review_request_indices();
    assert_eq!(
        review_indices,
        vec![11, 12, 13],
        "write round 1 resets the streak; rounds 2-3 are the two \
         consecutive no-ops that end it — exit at round 3, not before"
    );
}

/// The positive control: a review that WRITES on every round still runs
/// to its iteration cap — the early exit must never cut a productive
/// review short. Five write rounds are scripted; all five must be
/// consumed (no early exit, no over-blocking).
#[tokio::test]
async fn write_every_round_still_runs_to_the_iteration_cap() {
    let _serial = REVIEW_TEST_SERIAL.lock().await;
    let temp = tempfile::tempdir().unwrap();
    prime_models_dev_cache(&temp);

    let mut script = varying_calls(WORKER, 10);
    script.push(ScriptedItem::Model(text_response("done")));
    for _ in 0..5 {
        script.push(ScriptedItem::Model(tool_call_response(
            "memory_store",
            "{}",
        )));
    }
    let client = Arc::new(ScriptedClient::new(script));
    let agent = build_agent(
        Arc::clone(&client),
        20,
        10,
        temp.path().join("write_cap.sqlite"),
    )
    .await;

    let result = agent
        .run("do some work".to_string())
        .await
        .expect("turn completes normally");
    assert_eq!(result.content, "done");

    operant_core::daemon_pool::drain_for_label(REVIEW_DAEMON_LABEL, Duration::from_secs(5)).await;

    let review_indices = client.review_request_indices();
    assert_eq!(
        review_indices,
        vec![11, 12, 13, 14, 15],
        "a write on every round runs the full 5-round cap — unchanged"
    );
}
