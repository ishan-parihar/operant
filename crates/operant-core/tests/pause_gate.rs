//! Pause-gate tests (omp `pause.ts` parity, P1 #5 of the agentic-core
//! upgrade plan).
//!
//! The global estop (`operant pause`) was pause-NEW-work only: cron
//! dispatch, kanban dispatch and the gateway's inbound turn all checked
//! the sentinel, but an agent turn already in flight ran to completion.
//! omp's pause gate freezes every agent at its two action boundaries —
//! before each model call and before each tool batch. run.rs now polls
//! `estop::is_engaged()` at exactly those two boundaries; these tests pin
//! that contract against a real `OperantAgent` driven by a scripted
//! client:
//!
//! 1. engaged → the turn does NOT reach the model within the hold window;
//! 2. disengage mid-hold → the turn proceeds and completes normally;
//! 3. unengaged → zero extra latency (the gate is one stat when clear).
//!
//! The estop used is the REAL global one (no root override exists on the
//! gate path — that is the point of the contract): engage before spawn,
//! assert the hold, disengage, and unconditionally disengage in a guard
//! so a failing test can never leave a paused sentinel behind for the
//! rest of the suite.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::stream::BoxStream;

use operant_core::agent::{AgentConfig, ChatRequest, ModelClient, OperantAgent, StreamChunk};
use operant_core::client::{ChatResponse, Message};
use operant_core::database::Database;
use operant_core::error::{Error, Result};
use operant_core::tools::ToolRegistry;

/// Scripted client: returns one fixed text reply, records the timestamp of
/// the first `chat()` call.
struct OneShotClient {
    called_at: Arc<Mutex<Option<std::time::Instant>>>,
}

#[async_trait]
impl ModelClient for OneShotClient {
    fn provider_name(&self) -> &str {
        "oneshot"
    }

    async fn chat(&self, _req: ChatRequest) -> Result<ChatResponse> {
        *self.called_at.lock().unwrap() = Some(std::time::Instant::now());
        let payload = serde_json::json!({
            "id": "chatcmpl-pause",
            "object": "chat.completion",
            "created": 0,
            "model": "oneshot",
            "choices": [{"message": {"role": "assistant", "content": "done"}}],
            "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
        });
        Ok(serde_json::from_value(payload).unwrap())
    }

    async fn chat_streaming(
        &self,
        _req: ChatRequest,
    ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
        Err(Error::Agent("streaming not used".into()))
    }
}

fn test_agent(client: OneShotClient) -> OperantAgent {
    let db = Database::init(
        std::env::temp_dir().join(format!("test_pause_gate_{}.sqlite", std::process::id())),
    )
    .unwrap();
    // Non-streaming: the mock serves the `chat` path, matching the
    // circuit-breaker harness's shape.
    let mut config = AgentConfig::default();
    config.stream = false;
    OperantAgent::new(
        config,
        Box::new(client),
        ToolRegistry::new(Duration::from_secs(5)),
        Arc::new(db),
    )
}

/// Guard: whatever happens inside the test body, the sentinel is cleared —
/// a leftover engaged estop would silently pause every later test's turns.
struct DisengageGuard;
impl Drop for DisengageGuard {
    fn drop(&mut self) {
        let _ = operant_core::estop::disengage();
    }
}

/// Engaged estop holds the turn BEFORE the first model call; disengaging
/// releases it and the turn completes with the scripted reply.
#[tokio::test(flavor = "multi_thread")]
async fn pause_gate_holds_then_releases() {
    let _guard = DisengageGuard;
    operant_core::estop::disengage().unwrap();
    operant_core::estop::engage(Some("pause-gate test")).unwrap();
    let called_at = Arc::new(Mutex::new(None));
    let agent = test_agent(OneShotClient {
        called_at: called_at.clone(),
    });

    let started = std::time::Instant::now();
    let handle = tokio::spawn(async move { agent.run("hello".to_string()).await });

    // Hold window: the gate polls at 1s; give it 1.5s to (wrongly) reach
    // the model if the gate is missing. With the gate, chat() is never
    // called while engaged.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(
        called_at.lock().unwrap().is_none(),
        "the model must not be reached while the estop is engaged"
    );

    operant_core::estop::disengage().unwrap();
    let reply = tokio::time::timeout(Duration::from_secs(10), handle)
        .await
        .expect("turn completes after disengage")
        .unwrap()
        .unwrap();
    assert!(reply.content.contains("done"));
    // The gate held for at least the engaged window before releasing.
    let called = called_at.lock().unwrap().unwrap();
    assert!(
        called.duration_since(started) >= Duration::from_millis(1400),
        "the turn must have been held at the boundary, not run through"
    );
}

/// Unengaged: the gate costs one stat — the model is reached immediately.
#[tokio::test(flavor = "multi_thread")]
async fn pause_gate_clear_costs_nothing() {
    let _guard = DisengageGuard;
    operant_core::estop::disengage().unwrap();
    let called_at = Arc::new(Mutex::new(None));
    let agent = test_agent(OneShotClient {
        called_at: called_at.clone(),
    });

    let reply = tokio::time::timeout(Duration::from_secs(10), agent.run("hello".to_string()))
        .await
        .expect("turn completes promptly")
        .unwrap();
    assert!(reply.content.contains("done"));
    assert!(called_at.lock().unwrap().is_some());
}
