//! A stream that ends with NO completion signal is an incomplete answer.
//!
//! A provider can close its SSE body without the terminal `[DONE]` marker
//! and without a finish reason. Until the marker was surfaced, the agent
//! could not tell that from a finished reply — it took the partial as the
//! final answer, and the operator saw a sentence cut off mid-word with no
//! explanation.
//!
//! The recovery is the generic one (jcode's incomplete-response path): the
//! cut-off text rides the continuation prompt and the answer is reassembled.
//! The CONTINUATION is only asked for when the text itself looks cut — a
//! reply that ends on punctuation is finished, and a provider that omits
//! the marker must not be charged a pointless continuation on every turn.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::{StreamExt, stream::BoxStream};

use operant_core::agent::{AgentConfig, ChatRequest, ModelClient, OperantAgent, StreamChunk};
use operant_core::client::ChatResponse;
use operant_core::database::Database;
use operant_core::error::{Error, Result};
use operant_core::tools::ToolRegistry;

/// One scripted streaming turn.
struct Turn {
    text: &'static str,
    finish_reason: &'static str,
}

/// Serves the scripted turns in order and records every request's messages.
struct ScriptedStreamClient {
    turns: Mutex<Vec<Turn>>,
    seen: Mutex<Vec<Vec<String>>>,
    calls: AtomicUsize,
}

impl ScriptedStreamClient {
    fn new(turns: Vec<Turn>) -> Arc<Self> {
        Arc::new(Self {
            turns: Mutex::new(turns),
            seen: Mutex::new(Vec::new()),
            calls: AtomicUsize::new(0),
        })
    }

    fn requests(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn messages_of(&self, index: usize) -> Vec<String> {
        self.seen.lock().unwrap()[index].clone()
    }
}

#[async_trait]
impl ModelClient for ScriptedStreamClient {
    fn provider_name(&self) -> &str {
        "scripted-stream"
    }

    async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse> {
        Err(Error::Agent(
            "this fixture serves the streaming path only".into(),
        ))
    }

    async fn chat_streaming(
        &self,
        request: ChatRequest,
    ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
        self.seen
            .lock()
            .unwrap()
            .push(request.messages.iter().map(|m| m.content.clone()).collect());
        let mut guard = self.turns.lock().unwrap();
        let turn = guard.remove(0);
        drop(guard);
        self.calls.fetch_add(1, Ordering::SeqCst);
        let mut chunk = StreamChunk::new(Some(turn.text.to_string()), None, None);
        chunk.finish_reason = Some(turn.finish_reason.to_string());
        Ok(futures::stream::iter(vec![Ok(chunk)]).boxed())
    }
}

struct ScriptedClientHandle(Arc<ScriptedStreamClient>);

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

fn test_config(model: &str) -> AgentConfig {
    AgentConfig {
        model: model.to_string(),
        max_iterations: 4,
        stream: true,
        context_window: 8_000,
        system_prompt: Some("You are a test agent.".to_string()),
        ..AgentConfig::default()
    }
}

/// A stream that stops mid-sentence with no completion signal is continued,
/// and the answer comes back whole.
#[tokio::test]
async fn a_stream_that_stops_mid_sentence_is_continued_and_reassembled() {
    let temp = tempfile::tempdir().unwrap();
    let client = ScriptedStreamClient::new(vec![
        Turn {
            text: "The first half of the answer, and then the",
            finish_reason: "incomplete",
        },
        Turn {
            text: " rest of it.",
            finish_reason: "stop",
        },
    ]);
    let agent = OperantAgent::new(
        test_config("demo-incomplete-stream"),
        Box::new(ScriptedClientHandle(Arc::clone(&client))),
        ToolRegistry::new(Duration::from_secs(5)),
        Arc::new(Database::init(temp.path().join("incomplete.sqlite")).unwrap()),
    );

    let result = agent
        .run("say something long".to_string())
        .await
        .expect("the continuation must recover the turn");

    assert_eq!(
        result.content, "The first half of the answer, and then the rest of it.",
        "the answer must be reassembled from both halves"
    );
    assert_eq!(client.requests(), 2, "one continuation, no more");
    assert!(
        client
            .messages_of(1)
            .iter()
            .any(|m| m.contains("The first half of the answer")),
        "the continuation request must carry the cut-off text, got {:?}",
        client.messages_of(1)
    );
}

/// A stream that ends with no completion signal but whose text ENDS the
/// sentence is finished — asking for a continuation there would charge the
/// operator a call for nothing.
#[tokio::test]
async fn a_stream_that_ends_the_sentence_is_not_continued() {
    let temp = tempfile::tempdir().unwrap();
    let client = ScriptedStreamClient::new(vec![Turn {
        text: "Here is a whole answer.",
        finish_reason: "incomplete",
    }]);
    let agent = OperantAgent::new(
        test_config("demo-complete-answer"),
        Box::new(ScriptedClientHandle(Arc::clone(&client))),
        ToolRegistry::new(Duration::from_secs(5)),
        Arc::new(Database::init(temp.path().join("complete.sqlite")).unwrap()),
    );

    let result = agent
        .run("say something".to_string())
        .await
        .expect("the turn must complete");

    assert_eq!(result.content, "Here is a whole answer.");
    assert_eq!(
        client.requests(),
        1,
        "a finished sentence must not cost a continuation call"
    );
}
