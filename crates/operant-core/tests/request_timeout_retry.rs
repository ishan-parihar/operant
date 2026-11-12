//! The 2026-11-12 audit's evidence pair, pinned as a regression.
//!
//! **E (expected):** a provider that is slow on the FIRST call — past the
//! agent's own `request_timeout` ceiling — but answers on the second call.
//! The turn completes; the slow call is retried, not fatal.
//!
//! **O (what happened before the fix):** the ceiling fired, the turn died
//! with `request timed out after 120s (loop request_timeout ceiling)`, and
//! because the failure was an untyped `Agent` string the cron seam's
//! bounded transient retry could not see it either — so the seat's scheduled
//! poke went dark until the next cadence tick instead of re-issuing.
//!
//! The ceiling is driven HERE, not mocked: the client really sleeps past the
//! agent's `request_timeout`, so the test exercises the exact race inside
//! `call_with_loop_timeout` and the retry arm that has to follow it.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use futures::stream::BoxStream;

use operant_core::agent::{
    AgentConfig, AgentEvent, ChatRequest, ModelClient, OperantAgent, StreamChunk,
};
use operant_core::client::{ChatResponse, Choice, MessageDelta, Role, Usage};
use operant_core::database::Database;
use operant_core::error::{Error, Result};
use operant_core::tools::ToolRegistry;

/// The ceiling the agent enforces — short enough to keep the test quick,
/// long enough that a normal answer never races it.
const CEILING: Duration = Duration::from_millis(150);

/// A client whose first `slow_calls` calls sleep PAST the agent's ceiling;
/// every later call answers immediately.
struct SlowThenFastClient {
    /// Shared with the test so the assertions can see the call count after
    /// the agent has taken the client by value.
    counter: Arc<AtomicUsize>,
    slow_calls: usize,
    slow_for: Duration,
}

fn slow_then_fast(slow_calls: usize) -> (SlowThenFastClient, Arc<AtomicUsize>) {
    let counter = Arc::new(AtomicUsize::new(0));
    let client = SlowThenFastClient {
        counter: counter.clone(),
        slow_calls,
        slow_for: CEILING * 6,
    };
    (client, counter)
}

#[async_trait]
impl ModelClient for SlowThenFastClient {
    fn provider_name(&self) -> &str {
        "slow-then-fast"
    }

    async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse> {
        let n = self.counter.fetch_add(1, Ordering::SeqCst);
        if n < self.slow_calls {
            tokio::time::sleep(self.slow_for).await;
        }
        Ok(text_response("answered"))
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

fn test_config() -> AgentConfig {
    AgentConfig {
        reasoning_effort: String::new(),
        reasoning_effort_param: Default::default(),
        context_semantic_compaction: false,
        model: "demo".to_string(),
        max_trajectories: 0,
        show_turn_attribution: true,
        max_iterations: 4,
        tool_timeout: Duration::from_secs(5),
        request_timeout: CEILING,
        model_request_timeouts: Default::default(),
        system_prompt: Some("You are a test agent.".to_string()),
        stream: false,
        context_window: 8000,
        max_tool_result_share: operant_core::context_management::DEFAULT_MAX_TOOL_RESULT_SHARE,
        max_tool_result_total_share:
            operant_core::context_management::DEFAULT_MAX_TOOL_RESULT_TOTAL_SHARE,
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

fn agent_for(client: SlowThenFastClient, db_path: std::path::PathBuf) -> OperantAgent {
    OperantAgent::new(
        test_config(),
        Box::new(client),
        ToolRegistry::new(Duration::from_secs(5)),
        Arc::new(Database::init(db_path).unwrap()),
    )
}

fn db_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "operant_request_timeout_retry_{}_{}.sqlite",
        std::process::id(),
        name
    ))
}

/// The E/O pair: one slow call, then an answer. The turn MUST complete.
#[tokio::test]
async fn a_slow_first_call_is_retried_and_the_turn_completes() {
    let (client, counter) = slow_then_fast(1);
    let agent = agent_for(client, db_path("retry"));

    let reply = agent
        .run("hello".to_string())
        .await
        .expect("the turn must survive one slow call");

    assert!(
        reply.content.contains("answered"),
        "the turn must return the model's answer, got: {}",
        reply.content
    );
    assert_eq!(
        counter.load(Ordering::SeqCst),
        2,
        "the slow call must be re-issued exactly once"
    );
}

/// The retry is VISIBLE: the host is told a retry is scheduled, so the TUI
/// can show it and the operator is not left staring at a silent turn.
#[tokio::test]
async fn a_retried_call_emits_the_retry_event() {
    let (client, _counter) = slow_then_fast(1);
    let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentEvent>(32);
    let agent = OperantAgent::with_events(
        test_config(),
        Box::new(client),
        ToolRegistry::new(Duration::from_secs(5)),
        Arc::new(Database::init(db_path("event")).unwrap()),
        tx,
    );

    agent
        .run("hello".to_string())
        .await
        .expect("the turn must survive one slow call");

    let mut saw_retry = false;
    while let Ok(event) = rx.try_recv() {
        if let AgentEvent::RetryScheduled { attempt, .. } = event {
            assert_eq!(attempt, 1, "the first retry is attempt 1");
            saw_retry = true;
        }
    }
    assert!(
        saw_retry,
        "a retried call must emit AgentEvent::RetryScheduled"
    );
}

/// A provider that NEVER answers fails the turn — but with the TYPED error,
/// so the cron seam's bounded transient retry can see it and re-arm the
/// seat's schedule. An untyped `Agent` string here is what made the poke
/// go dark.
#[tokio::test]
async fn a_persistently_slow_provider_fails_with_the_typed_timeout() {
    let (client, counter) = slow_then_fast(usize::MAX);
    let agent = agent_for(client, db_path("typed"));

    let err = agent
        .run("hello".to_string())
        .await
        .expect_err("a provider that never answers must fail the turn");

    assert!(
        matches!(err, Error::RequestTimeout { .. }),
        "the failure must stay TYPED so the recovery gates can read it, got: {err:?}"
    );
    assert!(
        err.is_transient(),
        "a slow provider is transient — the cron seam's retry gate depends on it"
    );
    assert!(
        !err.is_self_healing(),
        "a timeout is not fixed by re-prompting the model; it needs a re-issue"
    );
    assert!(
        counter.load(Ordering::SeqCst) > 1,
        "the turn must have re-issued the request before giving up"
    );
}

/// The control: with the turn's retry budget already spent, the SAME slow
/// provider must fail the turn. Without this, a broken retry arm that
/// retried forever would still pass the test above.
#[tokio::test]
async fn without_retry_budget_the_slow_provider_fails_the_turn() {
    let (client, counter) = slow_then_fast(1);
    let mut config = test_config();
    config.max_retries = 0;
    let agent = OperantAgent::new(
        config,
        Box::new(client),
        ToolRegistry::new(Duration::from_secs(5)),
        Arc::new(Database::init(db_path("no-budget")).unwrap()),
    );

    let err = agent
        .run("hello".to_string())
        .await
        .expect_err("no retry budget means the slow call is fatal");

    assert!(
        matches!(err, Error::RequestTimeout { .. }),
        "the failure must stay TYPED, got: {err:?}"
    );
    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "without budget the request must NOT be re-issued"
    );
}

/// The public-API half of the same pair: the ceiling's error is TRANSIENT
/// (the cron seam's bounded retry gate reads exactly this flag) and keeps
/// the operator-visible text.
#[test]
fn the_request_timeout_is_transient_and_keeps_its_text() {
    let err = Error::RequestTimeout {
        budget: Duration::from_secs(120),
        guidance: String::new(),
    };
    assert!(err.is_transient(), "a slow provider is retryable");
    assert!(
        !err.is_self_healing(),
        "a timeout needs a re-issue, not a re-prompt"
    );
    let display = err.to_string();
    assert!(display.contains("timed out"), "got: {display}");
    assert!(
        display.contains("loop request_timeout ceiling"),
        "got: {display}"
    );
}

/// The floor must match the ids the operator actually configures: an
/// aggregator variant suffix (`...:free`) and the dated Claude 4 generation
/// both missed the table before the fix.
#[test]
fn the_reasoning_floor_matches_real_model_ids() {
    use operant_core::reasoning_timeouts::get_reasoning_stale_timeout_floor;
    assert_eq!(
        get_reasoning_stale_timeout_floor("dots-studio/dots-3-note-preview:free"),
        Some(300)
    );
    assert_eq!(
        get_reasoning_stale_timeout_floor("deepseek/deepseek-r1:free"),
        Some(600)
    );
    // The LONGER needle wins: `o3-mini` (300s), not `o3` (600s).
    assert_eq!(
        get_reasoning_stale_timeout_floor("openai/o3-mini:free"),
        Some(300)
    );
    assert_eq!(
        get_reasoning_stale_timeout_floor("claude-sonnet-4-20250514"),
        Some(240)
    );
}

/// The operator's explicit per-model budget — the only lever that can cover
/// a LOCAL ALIAS, which the vendor-slug floor table can never match.
#[test]
fn an_explicit_per_model_budget_is_found() {
    use operant_core::reasoning_timeouts::explicit_model_timeout;
    let mut overrides = std::collections::HashMap::new();
    overrides.insert("large-stack".to_string(), Duration::from_secs(600));
    assert_eq!(
        explicit_model_timeout(&overrides, "large-stack"),
        Some(Duration::from_secs(600))
    );
    assert_eq!(
        explicit_model_timeout(&overrides, "openrouter/large-stack"),
        Some(Duration::from_secs(600))
    );
    assert_eq!(explicit_model_timeout(&overrides, "gpt-4o"), None);
    assert_eq!(
        explicit_model_timeout(&std::collections::HashMap::new(), "large-stack"),
        None
    );
}
