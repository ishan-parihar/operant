//! The session transcript is written EXACTLY ONCE per message.
//!
//! The core owns the transcript: the user message is persisted before the
//! model call, and every assistant message the turn produces (the answer,
//! the wrap-up summary, the circuit-breaker notice) is persisted with it.
//! The gateway used to write the same two rows again, and the core wrote
//! the user row again on a retried turn — so a resumed session showed the
//! operator's message and the bot's answer TWICE.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::stream::BoxStream;

use operant_core::agent::{AgentConfig, ChatRequest, ModelClient, OperantAgent, StreamChunk};
use operant_core::client::{ChatResponse, Choice, MessageDelta, Role, Usage};
use operant_core::database::Database;
use operant_core::error::{Error, Result};
use operant_core::tools::ToolRegistry;

struct ScriptedClient {
    responses: Mutex<Vec<ChatResponse>>,
}

impl ScriptedClient {
    fn new(responses: Vec<ChatResponse>) -> Self {
        Self {
            responses: Mutex::new(responses),
        }
    }
}

#[async_trait]
impl ModelClient for ScriptedClient {
    fn provider_name(&self) -> &str {
        "scripted"
    }

    async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse> {
        let mut guard = self.responses.lock().unwrap_or_else(|e| e.into_inner());
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

fn test_config(model: &str) -> AgentConfig {
    AgentConfig {
        advertised_skills_token_budget: 0,
        model: model.to_string(),
        max_iterations: 2,
        stream: false,
        context_window: 8_000,
        system_prompt: Some("You are a test agent.".to_string()),
        ..AgentConfig::default()
    }
}

/// One turn, one user row, one assistant row.
#[tokio::test]
async fn a_turn_writes_one_user_row_and_one_assistant_row() {
    let temp = tempfile::tempdir().unwrap();
    let db = Arc::new(Database::init(temp.path().join("transcript.sqlite")).unwrap());
    let agent = OperantAgent::new(
        test_config("demo-transcript"),
        Box::new(ScriptedClient::new(vec![text_response("the answer")])),
        ToolRegistry::new(Duration::from_secs(5)),
        Arc::clone(&db),
    );

    agent
        .run("the question".to_string())
        .await
        .expect("the turn must complete");

    let session_id = agent.session_id().expect("the turn has a session");
    let rows = db
        .get_session_messages_full(&session_id)
        .expect("the transcript must be readable");
    let users: Vec<_> = rows.iter().filter(|row| row.role == "user").collect();
    let assistants: Vec<_> = rows.iter().filter(|row| row.role == "assistant").collect();
    assert_eq!(
        users.len(),
        1,
        "the operator's message must be in the transcript exactly once, got {users:#?}"
    );
    assert_eq!(
        assistants.len(),
        1,
        "the answer must be in the transcript exactly once, got {assistants:#?}"
    );
    assert_eq!(users[0].content.as_deref(), Some("the question"));
    assert_eq!(assistants[0].content.as_deref(), Some("the answer"));
}
