//! RED TEAM of the two-span retry policy AS WIRED (iters 2290/2291).
//!
//! The policy module has unit tests that drive `next_delay` with an injected
//! clock. Those tests prove the arithmetic. They do NOT prove the wiring:
//! that the ladder reaches the large span at all, that the per-call budget is
//! really refilled, that the turn-level cap still bounds an infinite policy,
//! or that the stop surfaces the typed error the cron seam reads.
//!
//! Every test here drives the REAL agent against a scripted client, so a
//! regression in `reissue_with_budget` fails one of these even when the policy
//! module is green. That is the point: the module tests and the wiring tests
//! are different claims.
//!
//! The tests are deliberately slow (a 1-second large span, a 2-second ceiling)
//! because the alternative is mocking `Instant::now()`, which would test the
//! mock. Three seconds per test is the honest price of a real clock.

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
use operant_core::config::RetrySettings;
use operant_core::database::Database;
use operant_core::error::{Error, Result};
use operant_core::tools::ToolRegistry;

/// A client that fails the first `failures` calls with a retryable transport
/// error, then answers. The error is `Provider { retryable: true }`, which is
/// what a 5xx becomes — so it earns a re-issue rather than ending the turn.
struct FlakyClient {
    counter: Arc<AtomicUsize>,
    failures: usize,
    /// The turn's wall clock, injected so a test can make the WALL CLOCK the
    /// binding constraint instead of the policy's ceiling. Without this the
    /// default 20-minute wall clock never fires and a test cannot tell which
    /// bound stopped the ladder.
    wall_clock_secs: u64,
}

impl FlakyClient {
    fn new(failures: usize) -> (Self, Arc<AtomicUsize>) {
        Self::with_wall(failures, 20 * 60)
    }

    fn with_wall(failures: usize, wall_clock_secs: u64) -> (Self, Arc<AtomicUsize>) {
        let counter = Arc::new(AtomicUsize::new(0));
        let client = Self {
            counter: counter.clone(),
            failures,
            wall_clock_secs,
        };
        (client, counter)
    }
}

#[async_trait]
impl ModelClient for FlakyClient {
    fn provider_name(&self) -> &str {
        "flaky"
    }

    async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse> {
        let n = self.counter.fetch_add(1, Ordering::SeqCst);
        if n < self.failures {
            return Err(Error::Provider {
                status: 502,
                body: "502 Bad Gateway".to_string(),
                retry_after: None,
            });
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

fn config(retry: RetrySettings) -> AgentConfig {
    AgentConfig {
        advertised_skills_token_budget: 0,
        memory_injection_budget_tokens: 0,
        reasoning_effort: String::new(),
        reasoning_effort_param: Default::default(),
        context_semantic_compaction: false,
        model: "demo".to_string(),
        max_trajectories: 0,
        show_turn_attribution: true,
        max_iterations: 4,
        tool_timeout: Duration::from_secs(5),
        // Generous: these tests are about the RETRY span, not the request
        // ceiling. A short one would fire first and the test would pass for
        // the wrong reason.
        request_timeout: Duration::from_secs(30),
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
        retry,
        tool_search: Default::default(),
        guardrail_exempt_tools: Vec::new(),
        ponytail: Default::default(),
    }
}

fn agent(cfg: AgentConfig, client: FlakyClient, name: &str) -> OperantAgent {
    let path = std::env::temp_dir().join(format!(
        "operant_rt_retry_{}_{}.sqlite",
        std::process::id(),
        name
    ));
    // A previous run's file would carry a stale schema; start clean.
    let _ = std::fs::remove_file(&path);
    OperantAgent::new(
        cfg,
        Box::new(client),
        ToolRegistry::new(Duration::from_secs(5)),
        Arc::new(Database::init(path).unwrap()),
    )
}

/// A policy with a 1-second large span and no ceiling: the numbers the tests
/// below drive, small enough that the real clock is not the bottleneck.
fn fast_policy() -> RetrySettings {
    RetrySettings {
        small_span_retries: 3,
        large_span_secs: 1,
        max_span_secs: 0,
        announce_large_span: true,
    }
}

// ── 1. the wiring claim: the LARGE span is reachable ──────────────────

/// The policy says the small span is 3 immediate re-issues, then a 1-second
/// wait. A wiring bug that refilled the per-call budget but never let the
/// attempt counter reach 3 would still pass the module's unit tests (they call
/// `next_delay` directly) while making the whole second half of the policy
/// unreachable in production. This test is the one that would catch it.
#[tokio::test]
async fn the_ladder_reaches_the_large_span_and_then_answers() {
    // Fail 4 times: 3 small-span (immediate) + 1 that must WAIT the large span.
    let (client, counter) = FlakyClient::new(4);
    let agent = agent(config(fast_policy()), client, "large_span");

    let started = std::time::Instant::now();
    let reply = agent
        .run("hello".to_string())
        .await
        .expect("the turn must survive 4 failures");
    let elapsed = started.elapsed();

    assert!(reply.content.contains("answered"), "got: {}", reply.content);
    assert_eq!(
        counter.load(Ordering::SeqCst),
        5,
        "4 failures then an answer is 5 calls"
    );
    // The 4th call is the large-span one, so the ladder must have slept about
    // a second. A wiring bug that never reaches the large span finishes in
    // milliseconds; one that sleeps on EVERY attempt takes 4+ seconds.
    assert!(
        elapsed >= Duration::from_millis(900),
        "the large span must actually WAIT (took {elapsed:?}) — if this is \
         instant the ladder never left the small span, and the whole second \
         half of the policy is dead code"
    );
    assert!(
        elapsed < Duration::from_secs(4),
        "only ONE large-span wait belongs in this run (took {elapsed:?})"
    );
}

// ── 2. the ceiling claim: an infinite policy really is infinite ─────

/// `max_span_secs = 0` is documented as INFINITE. The only thing that may stop
/// such a ladder is the TURN's wall clock — a different knob. This test pins
/// that the policy does not stop on its own: 5 failures with a 1s span and no
/// ceiling must still be retrying (and then answer), not giving up.
#[tokio::test]
async fn an_infinite_policy_does_not_stop_on_its_own() {
    let (client, counter) = FlakyClient::new(5);
    let agent = agent(config(fast_policy()), client, "infinite");

    let reply = agent
        .run("hello".to_string())
        .await
        .expect("an infinite policy must keep retrying until the provider answers");
    assert!(reply.content.contains("answered"));
    assert_eq!(
        counter.load(Ordering::SeqCst),
        6,
        "the ladder must have re-issued past the small span without stopping"
    );
}

// ── 3. the ceiling is honoured when the operator sets one ───────────

/// With a ceiling, the ladder stops and the failure surfaces TYPED, so the
/// cron seam's transient-retry gate can read it. A ladder that stopped with an
/// untyped `Agent` string would leave the seat dark (the 2026-11-12 defect).
#[tokio::test]
async fn a_ceiling_stops_the_ladder_and_the_error_stays_typed() {
    let (client, counter) = FlakyClient::new(usize::MAX);
    let mut retry = fast_policy();
    retry.small_span_retries = 0; // straight to the large span
    retry.max_span_secs = 2; // stop after ~2 seconds of retrying
    let agent = agent(config(retry), client, "ceiling");

    let started = std::time::Instant::now();
    let err = agent
        .run("hello".to_string())
        .await
        .expect_err("a ceiling must stop the ladder");
    let elapsed = started.elapsed();

    assert!(
        matches!(err, Error::Provider { .. }),
        "the surfaced error must stay TYPED so the cron seam can read it, got: {err:?}"
    );
    assert!(
        err.is_transient(),
        "a 5xx is transient; the cron seam's retry gate depends on it"
    );
    // Two 1-second waits reach the 2-second ceiling. A ladder that ignored the
    // ceiling would run until the 20-minute wall clock (the test would hang);
    // one that stopped immediately would take no time at all.
    assert!(
        elapsed >= Duration::from_millis(1800) && elapsed < Duration::from_secs(6),
        "the ladder must stop near the ceiling, took {elapsed:?}"
    );
    assert!(
        counter.load(Ordering::SeqCst) <= 4,
        "the ceiling bounds the ATTEMPT count too, took {}",
        counter.load(Ordering::SeqCst)
    );
}

// ── 4. the turn-level cap still bounds an infinite policy ───────────

/// `TURN_MAX_RETRY_FAILURES` (12) is the backstop for a provider that fails
/// forever: the per-call budget is refilled by every large-span attempt, so
/// without the turn cap an infinite policy would retry until the wall clock.
/// This test pins that the refill does NOT defeat the cap.
#[tokio::test]
async fn the_turn_cap_bounds_an_infinite_policy() {
    let (client, counter) = FlakyClient::new(usize::MAX);
    let mut retry = fast_policy();
    // Zero small span so every attempt is a large-span one, which is the case
    // that refills the per-call budget every time.
    retry.small_span_retries = 0;
    retry.large_span_secs = 1;
    let agent = agent(config(retry), client, "turn_cap");

    let err =
        match tokio::time::timeout(Duration::from_secs(20), agent.run("hello".to_string())).await {
            Ok(r) => r.expect_err("the turn cap must stop a provider that never answers"),
            Err(_) => panic!(
                "the turn cap does not bound an infinite policy: still retrying after 20s \
             ({} calls). The per-call budget is refilled by every large-span \
             attempt, so TURN_MAX_RETRY_FAILURES must be the bound — it is not.",
                counter.load(Ordering::SeqCst)
            ),
        };

    // PROBE: print the real numbers before asserting.
    let calls = counter.load(Ordering::SeqCst);
    assert!(
        calls <= 20,
        "the turn-level cap must bound the ladder well before the wall clock, took {calls}"
    );
    assert!(
        calls >= 10,
        "the cap must not fire early either, took {calls}"
    );
    assert!(
        matches!(err, Error::Provider { .. }),
        "the error must stay typed, got: {err:?}"
    );
}

// ── 5. the announcement claim ───────────────────────────────────────

/// Every large-span retry must be ANNOUNCED. An infinite silent retry is the
/// hang the policy exists to kill. This test reads the event channel, which is
/// what the TUI pill and the gateway log are fed from.
#[tokio::test]
async fn every_large_span_retry_is_announced_with_its_delay() {
    let (client, _counter) = FlakyClient::new(4);
    let cfg = config(fast_policy());
    let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentEvent>(64);
    let path = std::env::temp_dir().join(format!(
        "operant_rt_retry_{}_announce.sqlite",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    let agent = OperantAgent::with_events(
        cfg,
        Box::new(client),
        ToolRegistry::new(Duration::from_secs(5)),
        Arc::new(Database::init(path).unwrap()),
        tx,
    );

    agent
        .run("hello".to_string())
        .await
        .expect("the turn must complete");

    let mut events = Vec::new();
    while let Ok(event) = rx.try_recv() {
        events.push(event);
    }
    let retry_events: Vec<String> = events
        .iter()
        .filter_map(|e| match e {
            AgentEvent::RetryScheduled {
                attempt,
                max_attempts,
                reason,
            } => Some(format!("{attempt}/{max_attempts} {reason}")),
            _ => None,
        })
        .collect();

    assert!(
        !retry_events.is_empty(),
        "a retried turn must emit RetryScheduled (saw {events:?})"
    );
    // The large-span announcement is the one that carries the delay, so the
    // operator's pill can say "retrying in Ns" instead of a bare attempt count.
    assert!(
        retry_events
            .iter()
            .any(|r| r.contains("large span: retrying in")),
        "the large-span announcement must carry the delay: {retry_events:?}"
    );
}

// ── 6. the small span is NOT announced ──────────────────────────────

/// The control for the test above: the immediate re-issues must NOT be
/// announced. A ladder that announced every 0-second retry would spam the pill
/// and the log with rows that carry no information.
#[tokio::test]
async fn the_small_span_is_not_announced() {
    let (client, _counter) = FlakyClient::new(2);
    let cfg = config(fast_policy());
    let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentEvent>(64);
    let path = std::env::temp_dir().join(format!(
        "operant_rt_retry_{}_small_quiet.sqlite",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    let agent = OperantAgent::with_events(
        cfg,
        Box::new(client),
        ToolRegistry::new(Duration::from_secs(5)),
        Arc::new(Database::init(path).unwrap()),
        tx,
    );

    agent
        .run("hello".to_string())
        .await
        .expect("the turn must complete");

    let mut saw_large = false;
    while let Ok(event) = rx.try_recv() {
        if let AgentEvent::RetryScheduled { reason, .. } = event
            && reason.contains("large span")
        {
            saw_large = true;
        }
    }
    assert!(
        !saw_large,
        "a run whose failures all land in the small span must not announce a large span"
    );
}

// ── 7. the interrupt claim ──────────────────────────────────────────

/// The wait must be interruptible: a user who stops the turn during a 5-minute
/// wait gets "Turn stopped", not an error card five minutes later. The interrupt
/// is a separate task because `run()` consumes the flag.
#[tokio::test]
async fn an_interrupt_during_the_large_span_wait_ends_the_turn() {
    let (client, _counter) = FlakyClient::new(usize::MAX);
    let mut retry = fast_policy();
    retry.small_span_retries = 0;
    retry.large_span_secs = 30; // a wait long enough to interrupt inside
    let cfg = config(retry);
    let path = std::env::temp_dir().join(format!(
        "operant_rt_retry_{}_interrupt.sqlite",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    let agent = Arc::new(OperantAgent::new(
        cfg,
        Box::new(client),
        ToolRegistry::new(Duration::from_secs(5)),
        Arc::new(Database::init(path).unwrap()),
    ));

    let flag = agent.interrupt_flag();
    let turn = {
        let agent = agent.clone();
        tokio::spawn(async move { agent.run("hello".to_string()).await })
    };

    // Wait until the turn is inside its large-span sleep, then interrupt.
    tokio::time::sleep(Duration::from_millis(300)).await;
    flag.trigger();

    let started = std::time::Instant::now();
    let result = tokio::time::timeout(Duration::from_secs(10), turn)
        .await
        .expect("the interrupt must end the turn well before the 30s wait expires");

    let elapsed = started.elapsed();
    let inner = result.expect("the task must not panic");
    assert!(
        matches!(inner, Err(Error::Interrupted)),
        "an interrupt during the wait must surface Interrupted, got: {inner:?}"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "the interrupt must cut the wait short, took {elapsed:?}"
    );
}
