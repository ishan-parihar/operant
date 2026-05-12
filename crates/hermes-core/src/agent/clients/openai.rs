use async_trait::async_trait;
use futures::StreamExt;
use futures::stream::BoxStream;

use crate::client::{
    ChatStreamEvent, OpenAIClient, ToolCall, ToolCallFunction,
};
use crate::error::Result;
use super::super::model_client::{ChatRequest, ModelClient, StreamChunk};

/// Adapter that wraps [`OpenAIClient`] and implements [`ModelClient`].
///
/// This is the primary way the agent talks to OpenAI-compatible backends.
/// The underlying `OpenAIClient` is **not** modified.
#[derive(Clone)]
pub struct OpenAIModelClient {
    inner: OpenAIClient,
}

impl OpenAIModelClient {
    pub fn new(client: OpenAIClient) -> Self {
        Self { inner: client }
    }

    /// Access the underlying [`OpenAIClient`] for direct use in tools
    /// that need the concrete type (e.g. audio generation).
    pub fn inner(&self) -> &OpenAIClient {
        &self.inner
    }
}

#[async_trait]
impl ModelClient for OpenAIModelClient {
    fn provider_name(&self) -> &str {
        "openai"
    }

    async fn chat(&self, request: ChatRequest) -> Result<crate::client::ChatResponse> {
        let tools = if request.tools.is_empty() {
            None
        } else {
            Some(request.tools.as_slice())
        };
        self.inner
            .chat(&request.model, &request.messages, tools)
            .await
    }

    async fn chat_streaming(
        &self,
        request: ChatRequest,
    ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
        let tools = if request.tools.is_empty() {
            None
        } else {
            Some(request.tools.as_slice())
        };

        let stream = self
            .inner
            .chat_streaming(&request.model, &request.messages, tools)
            .await?;

        // Map each ChatStreamEvent into a StreamChunk.  Tool-call deltas are
        // passed through as partial ToolCall objects; the receiving agent
        // (process_stream in agent/mod.rs) merges incremental arguments.
        let mapped = stream.map(|event_result| match event_result {
            Ok(event) => {
                let content = event
                    .choices
                    .first()
                    .and_then(|c| c.delta.content.clone());

                let reasoning = event
                    .choices
                    .first()
                    .and_then(|c| c.delta.reasoning_content.clone());

                let tool_calls = extract_tool_calls_from_stream_event(&event);

                Ok(StreamChunk {
                    content,
                    reasoning,
                    tool_calls,
                })
            }
            Err(e) => Err(e),
        });

        Ok(Box::pin(mapped))
    }
}

/// Extract tool-call deltas from a single streaming event.
///
/// Within one event multiple deltas for the same tool-call ID are merged
/// (appending incremental arguments).  Cross-event merging happens in
/// `merge_stream_tool_call` inside `agent/mod.rs`.
fn extract_tool_calls_from_stream_event(event: &ChatStreamEvent) -> Option<Vec<ToolCall>> {
    let mut calls: Vec<ToolCall> = Vec::new();

    for choice in &event.choices {
        let Some(deltas) = &choice.delta.tool_calls else {
            continue;
        };
        for delta in deltas {
            let Some(ref func) = delta.function else {
                continue;
            };
            let id = delta
                .id
                .clone()
                .unwrap_or_else(|| format!("call_stream_{}", delta.index));

            // Merge with the last call if it has the same ID (streaming
            // providers often send multiple deltas for the same tool call
            // in a single event).
            if let Some(last) = calls.last_mut() {
                if last.id == id {
                    last.function.arguments.push_str(&func.arguments);
                    continue;
                }
            }

            calls.push(ToolCall {
                id,
                function: ToolCallFunction {
                    name: func.name.clone(),
                    arguments: func.arguments.clone(),
                },
            });
        }
    }

    if calls.is_empty() {
        None
    } else {
        Some(calls)
    }
}
