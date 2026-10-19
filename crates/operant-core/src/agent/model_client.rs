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
    /// Reasoning effort for this turn (2026-10-14 batch, plan 03). `None`
    /// on every auxiliary request (guardian, compressor, recap, background
    /// review) — only main turns carry effort, and only when the user set
    /// one and the model hasn't rejected the field.
    pub reasoning_effort: Option<String>,
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
            reasoning_effort: None,
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

    /// Attach the reasoning effort (plan 03). Main turns only.
    pub fn with_reasoning_effort(mut self, effort: Option<String>) -> Self {
        self.reasoning_effort = effort;
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
    /// Provider-reported finish reason, present on the terminal chunk(s) of
    /// the stream (`"stop"`, `"length"`, `"tool_calls"`, ...). Surfaced to
    /// the loop so it can detect truncated responses (`"length"`) and
    /// request a continuation instead of treating a cut-off answer as final
    /// (round-4 T1, hermes `_should_treat_stop_as_truncated` parity).
    pub finish_reason: Option<String>,
    /// Count of SSE payloads this stream DROPPED because they failed to
    /// parse (corrupted framing). Present only on the stream's synthetic
    /// terminal chunk (emitted by the openai adapter after the source ends)
    /// and only when at least one payload was dropped.
    pub dropped_events: Option<u32>,
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
            finish_reason: None,
            dropped_events: None,
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

    /// Update the API key at runtime for credential rotation.
    ///
    /// Called by the credential pool when rotating to a new key. The default
    /// implementation is a no-op (safe for clients that don't support dynamic
    /// key switching). Clients that store the key as a field should override
    /// this to update it.
    ///
    /// This means "use this key now" and nothing more. It does NOT change
    /// any stored credential: the pool's own rotation path calls it with a
    /// credential it just selected, and rewriting the pool there would
    /// destroy the benched credential's secret. To REPLACE a credential's
    /// secret — a key the operator rotated in `.env` — use
    /// [`rotate_credential_value`](Self::rotate_credential_value).
    fn set_api_key(&self, _api_key: &str) {
        // Default no-op — not all clients store the key directly
    }

    /// Replace the credential this client is currently USING with a new
    /// secret.
    ///
    /// This is the operation a runtime credential rotation needs: the
    /// operator changed the key, so the client's own stored secret is
    /// stale, and a client that re-reads its credential from a pool (or
    /// from the process environment) would otherwise put the old secret
    /// back on the wire.
    ///
    /// It is deliberately NOT `set_api_key`. The pool's internal rotation
    /// path calls `set_api_key` with a DIFFERENT existing credential to
    /// switch to; if that also rewrote a pool credential's value, the
    /// benched credential would lose its secret and the pool would end up
    /// holding a duplicate. The two operations have different meanings and
    /// get different methods.
    ///
    /// The default implementation is a no-op: a client that holds its key
    /// directly and is not pool-backed has nothing to replace.
    fn rotate_credential_value(&self, _api_key: &str) {
        // Default no-op — only a pool-backed client has a stored credential
        // to replace.
    }

    /// Retarget the client at another OpenAI-compatible endpoint at runtime
    /// (the TUI's `/connect`). `api_key` is the new endpoint's credential, or
    /// `None` for a keyless endpoint. Default no-op: wrappers forward it,
    /// concrete clients that cannot be retargeted ignore it.
    fn set_endpoint(&self, _base_url: &str, _api_key: Option<&str>) {}

    /// The endpoint the client is currently using.
    ///
    /// A caller that wants to retarget only on a CHANGE needs to know the
    /// current value: `set_endpoint` with no key EMPTIES a pool-backed
    /// client's credentials, so calling it every turn for an endpoint that
    /// did not change would break the agent.
    fn base_url(&self) -> Option<String> {
        None
    }

    /// The credential the client is currently using.
    ///
    /// A caller that retargets the endpoint needs to hand the client back
    /// the key it is already using when the new endpoint's key is not
    /// known: `set_endpoint` with no key empties a pool-backed client's
    /// credentials, and an agent with no credential cannot make a request.
    fn current_api_key(&self) -> Option<String> {
        None
    }
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
