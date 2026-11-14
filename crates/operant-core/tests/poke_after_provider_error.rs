//! The poke must be REACHABLE after a provider error (2026-11-12 audit).
//!
//! **E (expected):** a turn that made real progress and then hit a
//! CLASSIFIED transient provider failure is RE-ISSUED while the turn's retry
//! budget lasts, and once that budget is spent the bounded synthetic
//! continuation — the poke — gives the turn its last pass and finishes the
//! work.
//!
//! **O (what happened before the fix):** the establishment error path
//! returned the error DEAD. The poke's own guard had already recorded that
//! error two lines earlier (so it KNEW the failure was transient), but
//! nothing ever asked it: the turn ended with an error card, the seat's work
//! died with it, and the operator's "the poke does not work on an
//! interruption" report was exactly this seam.
//!
//! The controls: a DETERMINISTIC failure still ends the turn (a second
//! identical request cannot fix "insufficient_quota", and the poke's guard
//! says so), and an UNCLASSIFIED failure ends the turn without a re-issue
//! (the operator's "an error ends the turn" contract, pinned by
//! `review_discipline::error_exit_skips_the_armed_review`).

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

const POKE_ANSWER: &str = "the work is finished after the poke";
const TOOL: &str = "ok_probe";

/// A tool that SUCCEEDS — the poke's gate ("there is work to continue": an
/// open todo list, or a turn that ran tools).
struct OkTool;

#[async_trait]
impl OperantTool for OkTool {
    fn name(&self) -> &str {
        TOOL
    }

    fn description(&self) -> &str {
        "Always succeeds (drives the poke's work gate)."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::new(
            TOOL,
            "Always succeeds (drives the poke's work gate).",
            serde_json::json!({"type": "object", "properties": {}}),
        )
    }

    async fn execute(&self, _args: serde_json::Value, _context: ToolContext) -> ToolResult {
        ToolResult {
            tool_call_id: String::new(),
            name: TOOL.to_string(),
            success: true,
            content: "ok".to_string(),
            error: None,
            timed_out: false,
        }
    }
}

/// What the scripted client does on its next turn.
///
/// The failure carries the ERROR, not a message: the classifier reads the
/// status code and the body, and the two tests here deliberately script a
/// CLASSIFIED transient failure (an overloaded 503) and a deterministic one
/// (a 400 billing rejection) — which the classifier must be able to tell
/// apart, since the poke's gate is built on that verdict.
enum Scripted {
    Model(ChatResponse),
    Fail(Error),
    /// The call hangs for this long before answering — so an interrupt can
    /// land while the request is in flight.
    Slow(Duration),
}

/// A 503 the classifier reads as `Overloaded` (retryable).
fn overloaded() -> Error {
    Error::Provider {
        status: 503,
        body: "upstream overloaded".to_string(),
        retry_after: None,
    }
}

/// A 400 the classifier reads as `ContextOverflow`: retryable, but
/// `should_compress` — the compress-then-stop class.
fn too_big() -> Error {
    Error::Provider {
        status: 400,
        body: "context length exceeded: the prompt is too long".to_string(),
        retry_after: None,
    }
}

/// A 400 the classifier reads as `Billing` (not retryable), and whose text
/// the poke's own deterministic marker also names.
fn out_of_credits() -> Error {
    Error::Provider {
        status: 400,
        body: "insufficient_quota".to_string(),
        retry_after: None,
    }
}

struct ScriptedClient {
    script: Mutex<Vec<Scripted>>,
    seen: AtomicUsize,
    /// Every user message the client was shown — the poke is one of them.
    shown: Mutex<Vec<String>>,
}

impl ScriptedClient {
    fn new(script: Vec<Scripted>) -> Arc<Self> {
        Arc::new(Self {
            script: Mutex::new(script),
            seen: AtomicUsize::new(0),
            shown: Mutex::new(Vec::new()),
        })
    }

    fn requests(&self) -> usize {
        self.seen.load(Ordering::Relaxed)
    }

    /// Whether the harness's synthetic continuation reached the model.
    fn poke_seen(&self) -> bool {
        self.shown
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .any(|m| is_auto_poke_message(m))
    }
}

struct ScriptedClientHandle(Arc<ScriptedClient>);

#[async_trait]
impl ModelClient for ScriptedClientHandle {
    fn provider_name(&self) -> &str {
        "scripted"
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
        let client = &self.0;
        client.seen.fetch_add(1, Ordering::Relaxed);
        {
            let mut shown = client.shown.lock().unwrap_or_else(|e| e.into_inner());
            for message in &request.messages {
                if message.role == Role::User {
                    shown.push(message.content.clone());
                }
            }
        }
        // The item is TAKEN before any await — holding the script lock
        // across the sleep would make the future non-Send.
        let item = {
            let mut guard = client.script.lock().unwrap_or_else(|e| e.into_inner());
            if guard.is_empty() {
                None
            } else {
                Some(guard.remove(0))
            }
        };
        match item {
            Some(Scripted::Model(response)) => Ok(response),
            Some(Scripted::Fail(error)) => Err(error),
            Some(Scripted::Slow(for_how_long)) => {
                tokio::time::sleep(for_how_long).await;
                Ok(text_response("too late"))
            }
            None => Ok(text_response("the script ran out")),
        }
    }

    async fn chat_streaming(
        &self,
        _request: ChatRequest,
    ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
        Err(Error::Agent("streaming is not used in this test".into()))
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

fn test_config() -> AgentConfig {
    AgentConfig {
        retry: Default::default(),
        advertised_skills_token_budget: 0,
        model: "demo".to_string(),
        max_iterations: 8,
        tool_timeout: Duration::from_secs(5),
        request_timeout: Duration::from_secs(10),
        system_prompt: Some("You are a test agent.".to_string()),
        stream: false,
        context_window: 8000,
        max_tool_result_share: operant_core::context_management::DEFAULT_MAX_TOOL_RESULT_SHARE,
        record_trajectories: false,
        skill_nudge_interval: 0,
        memory_review_interval: 0,
        max_retries: 3,
        approval_mode: "off".to_string(),
        ..AgentConfig::default()
    }
}

async fn build_agent(client: Arc<ScriptedClient>, db_path: std::path::PathBuf) -> OperantAgent {
    let registry = ToolRegistry::new(Duration::from_secs(5));
    registry.register(OkTool).await.unwrap();
    OperantAgent::new(
        test_config(),
        Box::new(ScriptedClientHandle(client)),
        registry,
        Arc::new(Database::init(db_path).unwrap()),
    )
}

/// The E/O pair: a CLASSIFIED transient provider failure (an overloaded 503)
/// after real progress is RE-ISSUED first, and once the turn's re-issue budget
/// is spent the bounded poke gives the turn its last pass — the poke's answer
/// is the turn's answer.
///
/// Four failures because `max_retries = 3`: the initial call plus three
/// re-issues, then the re-issue budget is spent and the poke takes over.
#[tokio::test]
async fn a_transient_provider_failure_is_followed_by_a_poke() {
    let temp = tempfile::tempdir().unwrap();
    let client = ScriptedClient::new(vec![
        // Iteration 1: the model runs a tool that SUCCEEDS (the poke's gate:
        // there is work to continue).
        Scripted::Model(tool_call_response(TOOL, "{\"i\":1}")),
        // Iterations 2-5: the provider is overloaded. The first three are
        // RE-ISSUED (the turn's retry budget); the fourth spends it.
        Scripted::Fail(overloaded()),
        Scripted::Fail(overloaded()),
        Scripted::Fail(overloaded()),
        Scripted::Fail(overloaded()),
        // The poke's turn: the model finishes the work.
        Scripted::Model(text_response(POKE_ANSWER)),
    ]);
    let agent = build_agent(
        Arc::clone(&client),
        temp.path().join("poke_after_transient.sqlite"),
    )
    .await;

    let answer = agent
        .run("do the work".to_string())
        .await
        .expect("a transient provider failure must not end the turn dead");

    assert_eq!(
        answer.content, POKE_ANSWER,
        "the poke's answer is the turn's answer"
    );
    assert!(
        client.poke_seen(),
        "the synthetic continuation must have reached the model"
    );
    assert_eq!(
        client.requests(),
        6,
        "one tool turn, four failed turns, one poke turn"
    );
}

/// The control: a DETERMINISTIC failure ends the turn. The poke's guard
/// exists so the harness does not burn refused calls on a request that
/// cannot succeed.
#[tokio::test]
async fn a_deterministic_provider_failure_ends_the_turn_without_a_poke() {
    let temp = tempfile::tempdir().unwrap();
    let client = ScriptedClient::new(vec![
        Scripted::Model(tool_call_response(TOOL, "{\"i\":1}")),
        Scripted::Fail(out_of_credits()),
    ]);
    let agent = build_agent(
        Arc::clone(&client),
        temp.path().join("poke_after_deterministic.sqlite"),
    )
    .await;

    let err = agent
        .run("do the work".to_string())
        .await
        .expect_err("a deterministic provider failure must end the turn");

    assert!(
        err.to_string().contains("insufficient_quota"),
        "the operator must see the real cause, got: {err}"
    );
    assert!(
        !client.poke_seen(),
        "a deterministic failure must not be poked"
    );
    assert_eq!(
        client.requests(),
        2,
        "no third request may follow a deterministic failure"
    );
}

/// A context overflow is compress-ONCE, then stop.
///
/// The compress arm handles the first overflow (compress + retry); once it
/// has run, `compress_attempted` is set and a SECOND overflow reaches the
/// recovery seam. The same too-big prompt cannot succeed on a re-issue — the
/// classifier marks an overflow `retryable`, but `should_compress` is the
/// policy that says "compress, do not re-issue" — so the recovery loop must
/// stop there instead of burning the turn's whole retry budget on requests
/// the provider has already rejected, and the bounded poke must not fire
/// either (it cannot shrink the prompt).
///
/// This is the regression guard for `earns_recovery`'s third condition.
#[tokio::test]
async fn a_context_overflow_is_not_re_issued_by_the_recovery_seam() {
    let temp = tempfile::tempdir().unwrap();
    let client = ScriptedClient::new(vec![
        // Iteration 1: the model runs a tool that SUCCEEDS.
        Scripted::Model(tool_call_response(TOOL, "{\"i\":1}")),
        // Iteration 2: the provider rejects the too-big prompt.
        Scripted::Fail(too_big()),
        // The compress arm's retry: it SUCCEEDS with another tool call, so the
        // turn is still alive and `compress_attempted` is now set.
        Scripted::Model(tool_call_response(TOOL, "{\"i\":2}")),
        // Iteration 4: the prompt is too big AGAIN. The compress arm is spent,
        // so this reaches the recovery seam — and must stop it.
        Scripted::Fail(too_big()),
    ]);
    let agent = build_agent(
        Arc::clone(&client),
        temp.path().join("overflow_no_reissue.sqlite"),
    )
    .await;

    let err = agent
        .run("do the work".to_string())
        .await
        .expect_err("an overflow the compress arm cannot fix must end the turn");

    assert!(
        err.to_string().contains("context length"),
        "the operator sees the provider's own cause, got: {err}"
    );
    assert!(
        !client.poke_seen(),
        "a synthetic continuation cannot shrink a too-big prompt — no poke"
    );
    assert_eq!(
        client.requests(),
        4,
        "the too-big prompt must NOT be re-issued: tool, overflow, compress-retry, \
         overflow — and nothing more"
    );
}

/// An interrupted RE-ISSUE is an interrupt, not a provider failure.
///
/// The establishment arm returns the typed `Error::Interrupted` when the
/// operator stops the turn, and the TUI renders that as "Turn stopped." — no
/// error card. The re-issue loop must do the same: without the check, an
/// interrupted re-issue surfaces as a generic agent error and the operator
/// gets an error card for a turn they stopped themselves.
#[tokio::test]
async fn an_interrupted_re_issue_surfaces_the_typed_interrupt() {
    let temp = tempfile::tempdir().unwrap();
    let client = ScriptedClient::new(vec![
        // The first call fails transiently, so the turn RE-ISSUES.
        Scripted::Fail(overloaded()),
        // The re-issue is in flight when the interrupt lands.
        Scripted::Slow(Duration::from_millis(800)),
    ]);
    let agent = build_agent(
        Arc::clone(&client),
        temp.path().join("interrupted_reissue.sqlite"),
    )
    .await;

    let flag = agent.interrupt_flag();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(80)).await;
        flag.trigger();
    });

    let err = agent
        .run("do the work".to_string())
        .await
        .expect_err("an interrupted turn must fail");
    assert!(
        matches!(err, operant_core::error::Error::Interrupted),
        "the interrupt must stay TYPED (the TUI shows 'Turn stopped.', not an error card), got: {err:?}"
    );
}
