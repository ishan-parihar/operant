use std::collections::HashMap;

use async_trait::async_trait;
use futures::stream::BoxStream;
use futures::StreamExt;
use tracing::debug;

use super::super::model_client::{ChatRequest, ModelClient, StreamChunk};
use crate::client::{ChatStreamEvent, OpenAIClient, ToolCall, ToolCallFunction};
use crate::error::Result;

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

        let mut tracker = StreamToolCallTracker::new();
        let mapped = stream.map(move |event_result| match event_result {
            Ok(event) => {
                let choice = &event.choices.first();
                let content = choice.and_then(|c| c.delta.content.clone());
                let reasoning = choice.and_then(|c| c.delta.reasoning_content.clone());
                let tool_calls = tracker.process_event(&event);
                let extra_content = choice.and_then(|c| c.delta.extra_content.clone());

                Ok(StreamChunk {
                    content,
                    reasoning,
                    tool_calls,
                    extra_content,
                })
            }
            Err(e) => Err(e),
        });

        Ok(Box::pin(mapped))
    }
}

struct StreamToolCallTracker {
    index_map: HashMap<usize, (String, String)>,
}

impl StreamToolCallTracker {
    fn new() -> Self {
        Self {
            index_map: HashMap::new(),
        }
    }

    fn process_event(&mut self, event: &ChatStreamEvent) -> Option<Vec<ToolCall>> {
        let mut calls: Vec<ToolCall> = Vec::new();

        for choice in &event.choices {
            let Some(deltas) = &choice.delta.tool_calls else {
                continue;
            };
            for delta in deltas {
                let Some(ref func) = delta.function else {
                    continue;
                };
                let index = delta.index;

                if let Some(ref stream_id) = delta.id {
                    debug!(index = %index, stream_id = %stream_id, "Received delta with id");
                    self.index_map
                        .entry(index)
                        .or_insert_with(|| (stream_id.clone(), func.name.clone()))
                        .0 = stream_id.clone();
                } else {
                    debug!(index = %index, "Received delta without id");
                }

                let entry_id = self.index_map.get(&index).map(|e| e.0.clone());
                let entry_name = self.index_map.get(&index).map(|e| e.1.clone());
                let id = delta
                    .id
                    .clone()
                    .or(entry_id)
                    .unwrap_or_else(|| format!("call_stream_{}", index));

                let name = entry_name.unwrap_or_else(|| func.name.clone());

                if let Some(last) = calls.last_mut() {
                    if last.id == id {
                        last.function.arguments.push_str(&func.arguments);
                        continue;
                    }
                }

                calls.push(ToolCall {
                    id,
                    function: ToolCallFunction {
                        name,
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
}
