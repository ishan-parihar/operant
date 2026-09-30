//! Path test for memory as a real multi-turn feedback channel.
//!
//! A memory provider that is merely *called* proves nothing: the call sites
//! can be wired and the data can still never reach the next turn. So the
//! stub below derives its `prefetch()` output from the payload the previous
//! `sync_turn()` received, and the assertions check CONTENT — the string
//! built out of turn 1's assistant text must appear in the messages turn 2
//! actually sends to the model. The same turn-1 request is asserted NOT to
//! contain it, which is what rules out "the stub just echoes a constant".
//!
//! The `shutdown_memory_executor()` after each `run()` is not optional: the
//! post-turn `sync_turn` is submitted to a background FIFO executor, so
//! without the drain turn 2 can prefetch before turn 1's write lands.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::stream::BoxStream;

use operant_core::agent::{AgentConfig, ChatRequest, ModelClient, OperantAgent, StreamChunk};
use operant_core::client::{ChatResponse, Choice, MessageDelta, Role, Usage};
use operant_core::database::Database;
use operant_core::error::{Error, Result};
use operant_core::memory_provider::MemoryProvider;
use operant_core::tools::ToolRegistry;

const TURN_1_QUERY: &str = "what is turn one about?";
const TURN_2_QUERY: &str = "what is turn two about?";
const TURN_1_ANSWER: &str = "TURN-1-ANSWER";
const TURN_2_ANSWER: &str = "TURN-2-ANSWER";

/// Unique enough that finding it in a request message can only mean the
/// prefetch output travelled there.
const RECALL_MARKER: &str = "RECALL-OF-PREVIOUS-TURN";

/// Stub provider. Records all three feedback-channel calls and derives its
/// recall output from the last synced turn — never from the query — so a
/// non-empty recall proves the write reached the read.
struct RecordingProvider {
    prefetch_queries: Mutex<Vec<String>>,
    synced: Mutex<Vec<(String, String)>>,
    queued: Mutex<Vec<String>>,
}

impl RecordingProvider {
    fn new() -> Self {
        Self {
            prefetch_queries: Mutex::new(Vec::new()),
            synced: Mutex::new(Vec::new()),
            queued: Mutex::new(Vec::new()),
        }
    }

    /// The single source of truth for the recalled string, shared by the
    /// provider and the assertions so they cannot drift apart.
    fn recall_line(assistant: &str) -> String {
        format!("{RECALL_MARKER}: previous assistant said `{assistant}`")
    }

    fn prefetch_count(&self) -> usize {
        self.prefetch_queries.lock().unwrap().len()
    }

    fn last_synced(&self) -> Option<(String, String)> {
        self.synced.lock().unwrap().last().cloned()
    }

    fn synced_count(&self) -> usize {
        self.synced.lock().unwrap().len()
    }

    fn queued(&self) -> Vec<String> {
        self.queued.lock().unwrap().clone()
    }
}

#[async_trait]
impl MemoryProvider for RecordingProvider {
    fn name(&self) -> &str {
        "recording_stub"
    }

    fn is_available(&self) -> bool {
        true
    }

    async fn initialize(&self, _session_id: &str) -> Result<()> {
        Ok(())
    }

    async fn prefetch(&self, query: &str) -> String {
        self.prefetch_queries
            .lock()
            .unwrap()
            .push(query.to_string());
        match self.synced.lock().unwrap().last() {
            Some((_, assistant)) => Self::recall_line(assistant),
            None => String::new(),
        }
    }

    async fn sync_turn(&self, user: &str, assistant: &str) -> Result<()> {
        self.synced
            .lock()
            .unwrap()
            .push((user.to_string(), assistant.to_string()));
        Ok(())
    }

    fn queue_prefetch(&self, query: &str) {
        self.queued.lock().unwrap().push(query.to_string());
    }
}

/// Scripted client. Returns the responses in order and keeps every request's
/// message contents so the test can inspect what the model actually saw.
struct ScriptedClient {
    responses: Mutex<Vec<ChatResponse>>,
    seen: Mutex<Vec<Vec<String>>>,
}

impl ScriptedClient {
    fn new(responses: Vec<ChatResponse>) -> Self {
        Self {
            responses: Mutex::new(responses),
            seen: Mutex::new(Vec::new()),
        }
    }

    fn messages_of(&self, index: usize) -> Vec<String> {
        self.seen.lock().unwrap()[index].clone()
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
            .push(request.messages.iter().map(|m| m.content.clone()).collect());
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

/// Keeps the shared `Arc<ScriptedClient>` alive across the
/// `Box<dyn ModelClient>` hand-off so the test can read the recorded
/// requests afterwards.
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

fn test_config() -> AgentConfig {
    AgentConfig {
        model: "demo".to_string(),
        max_iterations: 6,
        tool_timeout: Duration::from_secs(5),
        request_timeout: Duration::from_secs(10),
        system_prompt: Some("You are a test agent.".to_string()),
        // Non-streaming: the memory hooks sit on the shared post-turn path,
        // which both modes enter.
        stream: false,
        context_window: 8000,
        max_tool_result_share: operant_core::context_management::DEFAULT_MAX_TOOL_RESULT_SHARE,
        max_healing_attempts: 1,
        fallback_models: Vec::new(),
        fallback_on_errors: false,
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

#[tokio::test]
async fn memory_is_a_multi_turn_feedback_channel() {
    let temp = tempfile::tempdir().unwrap();
    let client = Arc::new(ScriptedClient::new(vec![
        text_response(TURN_1_ANSWER),
        text_response(TURN_2_ANSWER),
    ]));
    let provider = Arc::new(RecordingProvider::new());
    let db = Database::init(temp.path().join("memory_feedback.sqlite")).unwrap();

    let agent = OperantAgent::new(
        test_config(),
        Box::new(ScriptedClientHandle(Arc::clone(&client))),
        ToolRegistry::new(Duration::from_secs(5)),
        Arc::new(db),
    )
    .with_memory_provider(Arc::clone(&provider) as Arc<dyn MemoryProvider>);

    // ── Turn 1 ────────────────────────────────────────────────────────
    let first = agent.run(TURN_1_QUERY.to_string()).await.unwrap();
    assert_eq!(first.content, TURN_1_ANSWER);
    // Drain the background FIFO executor, else turn 1's write is still in
    // flight when turn 2 prefetches.
    agent.shutdown_memory_executor().await;

    assert_eq!(
        provider.synced_count(),
        1,
        "turn 1 must reach sync_turn exactly once"
    );
    assert_eq!(
        provider.last_synced(),
        Some((TURN_1_QUERY.to_string(), TURN_1_ANSWER.to_string())),
        "sync_turn must carry the turn's own user query and assistant text"
    );
    assert_eq!(
        provider.prefetch_count(),
        1,
        "turn 1 must prefetch (against an empty store)"
    );
    assert_eq!(
        provider.queued(),
        vec![TURN_1_QUERY.to_string()],
        "queue_prefetch must fire after turn 1, keyed on that turn's query"
    );

    // ── Turn 2 ────────────────────────────────────────────────────────
    let second = agent.run(TURN_2_QUERY.to_string()).await.unwrap();
    assert_eq!(second.content, TURN_2_ANSWER);
    // The executor was already taken by the turn-1 drain; this is a no-op
    // kept for symmetry so a future second turn can't leak a worker.
    agent.shutdown_memory_executor().await;

    assert_eq!(
        provider.prefetch_count(),
        2,
        "turn 2 must prefetch again, this time against a populated store"
    );
    assert_eq!(
        provider.queued(),
        vec![TURN_1_QUERY.to_string(), TURN_2_QUERY.to_string()],
        "queue_prefetch must fire once per completed turn, in order"
    );

    // ── (a) CONTENT: turn 1's assistant text changed turn 2's context ──
    let expected = RecordingProvider::recall_line(TURN_1_ANSWER);
    let turn_two_request = client.messages_of(1);
    assert!(
        turn_two_request.iter().any(|m| m.contains(&expected)),
        "turn 2's request must carry the prefetch line derived from turn 1's \
         sync_turn payload.\n  expected to find: {expected}\n  \
         request messages: {turn_two_request:#?}"
    );

    // The recall line cannot predate the write that produced it, so its
    // presence in turn 2 and absence in turn 1 is a real delta — not a
    // constant the stub happens to emit on every turn.
    let turn_one_request = client.messages_of(0);
    assert!(
        !turn_one_request.iter().any(|m| m.contains(RECALL_MARKER)),
        "turn 1 prefetched an empty store, so no recall line may appear in \
         its request.\n  request messages: {turn_one_request:#?}"
    );
    assert!(
        !turn_two_request
            .iter()
            .any(|m| m.contains(&RecordingProvider::recall_line(TURN_2_ANSWER))),
        "turn 2's recall must be derived from turn 1, not from turn 2's own \
         (not yet written) answer"
    );
}
