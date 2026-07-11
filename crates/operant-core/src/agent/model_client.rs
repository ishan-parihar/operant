//! Provider-agnostic model client trait and request types.
//!
//! Defines [`ModelClient`] — the core abstraction every LLM provider adapter
//! implements — along with [`ChatRequest`] and [`StreamChunk`] used by all
//! providers.

use async_trait::async_trait;
use futures::stream::BoxStream;

use crate::client::{ChatResponse, Message, ToolCall, Usage};
use crate::error::Result;
use crate::schema::ToolSchema;

// ---------------------------------------------------------------------------
// ChatRequest
// ---------------------------------------------------------------------------

/// Unified request for all model clients.
#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub tools: Vec<ToolSchema>,
    pub stream: bool,
}

impl ChatRequest {
    /// Create a new non-streaming request without tools.
    pub fn new(model: impl Into<String>, messages: Vec<Message>) -> Self {
        Self {
            model: model.into(),
            messages,
            max_tokens: None,
            temperature: None,
            tools: Vec::new(),
            stream: false,
        }
    }

    /// Attach tool schemas.
    pub fn with_tools(mut self, tools: Vec<ToolSchema>) -> Self {
        self.tools = tools;
        self
    }

    /// Enable or disable streaming.
    pub fn with_stream(mut self, stream: bool) -> Self {
        self.stream = stream;
        self
    }
}

// ---------------------------------------------------------------------------
// StreamChunk
// ---------------------------------------------------------------------------

/// One chunk from a streaming response — maps naturally from SSE events.
///
/// Each chunk carries an optional text delta, an optional reasoning delta, and
/// an optional list of (potentially partial) tool-call deltas.  The receiving
/// agent is responsible for merging incremental tool-call arguments across
/// chunks.
#[derive(Debug, Clone)]
pub struct StreamChunk {
    pub content: Option<String>,
    pub reasoning: Option<String>,
    pub tool_calls: Option<Vec<ToolCall>>,
    /// Provider-specific extra data (e.g. Google Gemini thought_signature)
    pub extra_content: Option<serde_json::Value>,
    /// Token usage, present only on the chunk(s) that carry it (e.g. the
    /// final usage-only OpenAI chunk, or Anthropic's message_start/
    /// message_delta events). Callers merge across chunks — see
    /// `process_stream` in `agent/mod.rs`.
    pub usage: Option<Usage>,
}

impl StreamChunk {
    pub fn new(
        content: Option<String>,
        reasoning: Option<String>,
        tool_calls: Option<Vec<ToolCall>>,
    ) -> Self {
        Self {
            content,
            reasoning,
            tool_calls,
            extra_content: None,
            usage: None,
        }
    }
}

// ---------------------------------------------------------------------------
// ModelClient trait
// ---------------------------------------------------------------------------

/// Provider-agnostic LLM client.
///
/// Every concrete provider (OpenAI, Anthropic, etc.) implements this trait.
/// The agent uses `Arc<dyn ModelClient>` so the client can be cheaply cloned
/// and shared across concurrent tasks (e.g. session distillation).
#[async_trait]
pub trait ModelClient: Send + Sync {
    /// Non-streaming chat completion.
    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse>;

    /// Streaming chat completion.
    ///
    /// Returns a boxed stream of [`StreamChunk`]s, or an error if the stream
    /// itself could not be established.  Individual chunk errors are embedded
    /// in the stream items.
    async fn chat_streaming(
        &self,
        request: ChatRequest,
    ) -> Result<BoxStream<'static, Result<StreamChunk>>>;

    /// Human-readable provider name (e.g. `"openai"`, `"anthropic"`).
    fn provider_name(&self) -> &str;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::ToolSchema;

    #[test]
    fn test_chat_request_new() {
        let req = ChatRequest::new("gpt-4", vec![Message::user("hello")]);
        assert_eq!(req.model, "gpt-4");
        assert_eq!(req.messages.len(), 1);
        assert!(req.tools.is_empty());
        assert!(!req.stream);
    }

    #[test]
    fn test_chat_request_with_tools() {
        let tools = vec![ToolSchema::new(
            "get_weather",
            "Get weather for a city",
            serde_json::json!({ "type": "object" }),
        )];
        let req = ChatRequest::new("gpt-4", vec![]).with_tools(tools);
        assert_eq!(req.tools.len(), 1);
        assert_eq!(req.tools[0].name, "get_weather");
    }

    #[test]
    fn test_chat_request_with_stream() {
        let req = ChatRequest::new("gpt-4", vec![]).with_stream(true);
        assert!(req.stream);

        let req = ChatRequest::new("gpt-4", vec![]).with_stream(false);
        assert!(!req.stream);
    }

    #[test]
    fn test_chat_request_clone() {
        let req = ChatRequest::new("gpt-4", vec![Message::user("hi")]).with_stream(true);
        let cloned = req.clone();
        assert_eq!(cloned.model, req.model);
        assert_eq!(cloned.messages.len(), req.messages.len());
        assert_eq!(cloned.stream, req.stream);
    }

    #[test]
    fn test_stream_chunk_creation() {
        let chunk = StreamChunk::new(
            Some("Hello".to_string()),
            Some("thinking...".to_string()),
            None,
        );
        assert_eq!(chunk.content.as_deref(), Some("Hello"));
        assert_eq!(chunk.reasoning.as_deref(), Some("thinking..."));
        assert!(chunk.tool_calls.is_none());
    }

    #[test]
    fn test_stream_chunk_with_tool_calls() {
        let calls = vec![ToolCall {
            id: "call_1".to_string(),
            function: crate::client::ToolCallFunction {
                name: "get_weather".to_string(),
                arguments: r#"{"city":"London"}"#.to_string(),
            },
        }];
        let chunk = StreamChunk::new(None, None, Some(calls.clone()));
        assert!(chunk.content.is_none());
        assert!(chunk.reasoning.is_none());
        let retrieved = chunk.tool_calls.unwrap();
        assert_eq!(retrieved.len(), 1);
        assert_eq!(retrieved[0].function.name, "get_weather");
    }

    #[test]
    fn test_stream_chunk_clone() {
        let chunk = StreamChunk::new(
            Some("content".to_string()),
            None,
            Some(vec![ToolCall {
                id: "c1".to_string(),
                function: crate::client::ToolCallFunction {
                    name: "test".to_string(),
                    arguments: "{}".to_string(),
                },
            }]),
        );
        let cloned = chunk.clone();
        assert_eq!(cloned.content, chunk.content);
        assert_eq!(cloned.reasoning, chunk.reasoning);
    }

    #[test]
    fn test_chat_request_empty_messages() {
        let req = ChatRequest::new("gpt-4", vec![]);
        assert!(req.messages.is_empty());
    }
}
