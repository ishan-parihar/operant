//! The auto-poke: a bounded synthetic continuation after an early exit.
//!
//! A turn that runs out of room ends with a TOOLLESS wrap-up. When that
//! wrap-up comes back empty or degenerate (the live incident: a 13-char
//! reasoning-leak artifact, which the S1 quality gate threw away), the turn
//! used to stop with NOTHING — an empty answer and a notice telling the
//! operator the wrap-up "ran out of room mid-summary", which is not what
//! happened. The operator had no answer and no way to get one.
//!
//! The poke is the bounded continuation that gives the turn one more pass
//! WITH its tools. It is ported from jcode's auto-poke
//! (`crates/jcode-todo-policy/src/lib.rs` `build_auto_poke_message`,
//! `crates/jcode-tui/src/tui/app/commands_auto_poke_errors.rs`
//! `is_non_retryable_auto_poke_error`).
//!
//! What these tests pin:
//! - an empty wrap-up is followed by a poke, and the poke's answer IS the
//!   turn's answer (the operator gets a result, not an empty card);
//! - the poke bypasses the configured iteration limit (the turn continues
//!   PAST `max_iterations`);
//! - the poke is bounded — a model that keeps failing gets at most
//!   `MAX_AUTO_POKES` continuations, and then the turn ends;
//! - a poke is NOT issued when the wrap-up produced a real answer (the
//!   existing grace path is untouched).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::stream::BoxStream;

use operant_core::agent::{
    AgentConfig, ChatRequest, ModelClient, OperantAgent, StreamChunk, is_auto_poke_message,
};
use operant_core::client::{
    ChatResponse, Choice, MessageDelta, Role, ToolCallDelta, ToolCallFunction, Usage,
};
use operant_core::database::Database;
use operant_core::error::{Error, Result};
use operant_core::schema::ToolSchema;
use operant_core::tools::{OperantTool, ToolContext, ToolRegistry, ToolResult};

/// The observed production artifact: a short, whitespace-free reasoning-leak
/// marker. The S1 gate rejects it, so the wrap-up produced nothing.
const DEGENERATE_WRAP_UP: &str = "]<]\u{200b}minimax[>[";

/// The answer the poke turn produces.
const POKE_ANSWER: &str = "the task is finished: the remaining checks passed";

/// A tool that SUCCEEDS, so the turn made real progress.
///
/// The poke's gate is "there is work to continue" — an open todo list or a
/// successful tool call. A turn whose every tool failed is a stuck model,
/// and poking it would only repeat the failing calls.
struct OkTool;

#[async_trait]
impl OperantTool for OkTool {
    fn name(&self) -> &str {
        "ok_probe"
    }

    fn description(&self) -> &str {
        "Always succeeds (drives the budget-exhaustion path)."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            "ok_probe",
            "Always succeeds (drives the budget-exhaustion path).",
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
            timed_out: false,
        }
    }
}

/// A scripted client that answers by REQUEST SHAPE rather than by a fixed
/// script: the wrap-up is the request with NO tools, every other request is
/// a loop turn.
struct ShapeClient {
    /// The answers the loop turns get, in order.
    loop_answers: Mutex<Vec<ChatResponse>>,
    /// The answer every toolless wrap-up gets.
    wrap_up: Mutex<ChatResponse>,
    seen: AtomicUsize,
    toolless_seen: AtomicUsize,
}

impl ShapeClient {
    fn new(loop_answers: Vec<ChatResponse>, wrap_up: ChatResponse) -> Self {
        Self {
            loop_answers: Mutex::new(loop_answers),
            wrap_up: Mutex::new(wrap_up),
            seen: AtomicUsize::new(0),
            toolless_seen: AtomicUsize::new(0),
        }
    }

    fn requests(&self) -> usize {
        self.seen.load(Ordering::Relaxed)
    }

    fn toolless_requests(&self) -> usize {
        self.toolless_seen.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl ModelClient for ShapeClient {
    fn provider_name(&self) -> &str {
        "scripted"
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
        self.seen.fetch_add(1, Ordering::Relaxed);
        if request.tools.is_empty() {
            // The wrap-up: toolless by construction.
            self.toolless_seen.fetch_add(1, Ordering::Relaxed);
            return Ok(self
                .wrap_up
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone());
        }
        let mut guard = self.loop_answers.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_empty() {
            return Ok(text_response("the loop ran out of scripted answers"));
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

struct ShapeClientHandle(Arc<ShapeClient>);

#[async_trait]
impl ModelClient for ShapeClientHandle {
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

fn test_config(max_iterations: usize) -> AgentConfig {
    AgentConfig {
        advertised_skills_token_budget: 0,
        model: "demo".to_string(),
        max_iterations,
        tool_timeout: Duration::from_secs(5),
        request_timeout: Duration::from_secs(10),
        system_prompt: Some("You are a test agent.".to_string()),
        stream: false,
        context_window: 8000,
        max_tool_result_share: operant_core::context_management::DEFAULT_MAX_TOOL_RESULT_SHARE,
        max_tool_result_total_share:
            operant_core::context_management::DEFAULT_MAX_TOOL_RESULT_TOTAL_SHARE,
        record_trajectories: false,
        skill_nudge_interval: 0,
        memory_review_interval: 0,
        max_retries: 3,
        retry: crate::config::RetrySettings::default(),
        approval_mode: "off".to_string(),
        ..AgentConfig::default()
    }
}

async fn build_agent(
    client: Arc<ShapeClient>,
    max_iterations: usize,
    db_path: std::path::PathBuf,
) -> (OperantAgent, Arc<Database>) {
    let registry = ToolRegistry::new(Duration::from_secs(5));
    registry.register(OkTool).await.unwrap();
    let db = Arc::new(Database::init(db_path).unwrap());
    let agent = OperantAgent::new(
        test_config(max_iterations),
        Box::new(ShapeClientHandle(client)),
        registry,
        Arc::clone(&db),
    );
    (agent, db)
}

#[tokio::test]
async fn an_empty_wrap_up_is_followed_by_a_poke_that_finishes_the_work() {
    let temp = tempfile::tempdir().unwrap();
    // One tool turn burns the single iteration, the wrap-up comes back
    // degenerate, and the poke turn answers.
    let client = Arc::new(ShapeClient::new(
        vec![
            tool_call_response("ok_probe", "{\"i\":1}"),
            text_response(POKE_ANSWER),
        ],
        text_response(DEGENERATE_WRAP_UP),
    ));
    let (agent, _db) =
        build_agent(Arc::clone(&client), 1, temp.path().join("auto_poke.sqlite")).await;

    let answer = agent
        .run("do the work".to_string())
        .await
        .expect("the turn must complete");

    assert_eq!(
        answer.content, POKE_ANSWER,
        "the poke's answer is the turn's answer — the operator gets a result, \
         not an empty card"
    );
    assert_eq!(
        client.toolless_requests(),
        1,
        "exactly one toolless wrap-up must have run"
    );
    assert!(
        client.requests() >= 3,
        "the poke must have issued its own request (saw {})",
        client.requests()
    );

    // The poke is a synthetic user turn in the conversation — and it is
    // recognisable as one, so a renderer does not pass it off as something
    // the operator typed.
    let conversation = agent.conversation().await;
    let pokes: Vec<_> = conversation
        .iter()
        .filter(|m| is_auto_poke_message(&m.content))
        .collect();
    assert_eq!(
        pokes.len(),
        1,
        "exactly one poke must be in the conversation: {conversation:?}"
    );
    assert_eq!(
        pokes[0].role,
        Role::User,
        "the poke is persisted as a user turn so the model treats it as a \
         continuation"
    );
}

#[tokio::test]
async fn the_poke_runs_past_the_configured_iteration_limit() {
    let temp = tempfile::tempdir().unwrap();
    // `max_iterations: 1` — the poke's turn is iteration 2, which the
    // configured limit forbids. The poke is allowed to run it anyway.
    let client = Arc::new(ShapeClient::new(
        vec![
            tool_call_response("ok_probe", "{\"i\":1}"),
            text_response(POKE_ANSWER),
        ],
        text_response(DEGENERATE_WRAP_UP),
    ));
    let (agent, _db) = build_agent(
        Arc::clone(&client),
        1,
        temp.path().join("auto_poke_limit.sqlite"),
    )
    .await;

    let answer = agent
        .run("do the work".to_string())
        .await
        .expect("the turn must complete");
    assert_eq!(
        answer.content, POKE_ANSWER,
        "the poke must bypass the configured iteration limit"
    );
}

#[tokio::test]
async fn the_poke_is_bounded_and_the_turn_ends_when_the_budget_runs_out() {
    let temp = tempfile::tempdir().unwrap();
    // Every loop turn calls the failing tool and every wrap-up is degenerate:
    // the poke cannot rescue this turn, and it must not spin.
    let client = Arc::new(ShapeClient::new(
        vec![
            tool_call_response("ok_probe", "{\"i\":1}"),
            tool_call_response("ok_probe", "{\"i\":2}"),
            tool_call_response("ok_probe", "{\"i\":3}"),
            tool_call_response("ok_probe", "{\"i\":4}"),
            tool_call_response("ok_probe", "{\"i\":5}"),
            tool_call_response("ok_probe", "{\"i\":6}"),
            tool_call_response("ok_probe", "{\"i\":7}"),
            tool_call_response("ok_probe", "{\"i\":8}"),
        ],
        text_response(DEGENERATE_WRAP_UP),
    ));
    let (agent, _db) = build_agent(
        Arc::clone(&client),
        1,
        temp.path().join("auto_poke_bounded.sqlite"),
    )
    .await;

    let answer = agent
        .run("do the work".to_string())
        .await
        .expect("the turn must complete, even with nothing to say");

    assert!(
        answer.content.is_empty(),
        "a turn whose wrap-up never produced an answer ends empty, as before: \
         got {:?}",
        answer.content
    );
    let conversation = agent.conversation().await;
    let pokes = conversation
        .iter()
        .filter(|m| is_auto_poke_message(&m.content))
        .count();
    assert!(pokes <= 2, "the poke must be bounded (saw {pokes} pokes)");
}

#[tokio::test]
async fn a_real_wrap_up_is_not_followed_by_a_poke() {
    let temp = tempfile::tempdir().unwrap();
    let client = Arc::new(ShapeClient::new(
        vec![tool_call_response("ok_probe", "{\"i\":1}")],
        text_response("summary of the partial work: two of three checks passed"),
    ));
    let (agent, _db) =
        build_agent(Arc::clone(&client), 1, temp.path().join("no_poke.sqlite")).await;

    let answer = agent
        .run("do the work".to_string())
        .await
        .expect("the turn must complete");
    assert_eq!(
        answer.content, "summary of the partial work: two of three checks passed",
        "the wrap-up's own answer is the turn's answer"
    );
    let conversation = agent.conversation().await;
    assert!(
        !conversation
            .iter()
            .any(|m| is_auto_poke_message(&m.content)),
        "a wrap-up that produced an answer needs no poke: {conversation:?}"
    );
    assert_eq!(
        client.requests(),
        2,
        "the turn must not have spent a request on a poke"
    );
}
