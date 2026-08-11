use std::collections::HashMap;

use async_trait::async_trait;
use futures::StreamExt;
use futures::stream::BoxStream;

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
            .chat(
                &request.model,
                &request.messages,
                tools,
                request.max_tokens,
                request.temperature,
            )
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
            .chat_streaming(
                &request.model,
                &request.messages,
                tools,
                request.max_tokens,
                request.temperature,
            )
            .await?;

        // Map each ChatStreamEvent into a StreamChunk.  Tool-call deltas are
        // passed through as partial ToolCall objects; the receiving agent
        // (process_stream in agent/mod.rs) merges incremental arguments
        // across events using the tool-call ID.
        //
        // OpenAI-compatible streaming providers only emit the tool-call ID
        // (and `function.name`) in the FIRST delta for each `index`; later
        // deltas carry only `function.arguments` fragments.  We therefore
        // keep a per-stream `index -> id` map (`StreamToolCallIndex`) so
        // every emitted ToolCall reuses the original ID and the agent's
        // `merge_stream_tool_call` correctly appends incremental arguments
        // instead of producing many disjoint partial calls.
        let mapped = stream.scan(StreamToolCallIndex::default(), |state, event_result| {
            let chunk = match event_result {
                Ok(event) => {
                    let choice = &event.choices.first();
                    let content = choice.and_then(|c| c.delta.content.clone());
                    let reasoning = choice.and_then(|c| c.delta.reasoning_content.clone());
                    let tool_calls = extract_tool_calls_from_stream_event(&event, state);
                    let extra_content = choice.and_then(|c| c.delta.extra_content.clone());

                    Ok(StreamChunk {
                        content,
                        reasoning,
                        tool_calls,
                        extra_content,
                        usage: event.usage,
                    })
                }
                Err(e) => Err(e),
            };
            futures::future::ready(Some(chunk))
        });

        Ok(Box::pin(mapped))
    }
}

/// Per-stream state mapping tool-call `index` to the canonical `id` issued
/// by the provider in the first delta for that index.  This guarantees that
/// continuation deltas (which omit `id`) reuse the same ID so the agent
/// merger appends arguments instead of creating duplicate partial calls.
#[derive(Default)]
struct StreamToolCallIndex {
    id_by_index: HashMap<usize, String>,
}

impl StreamToolCallIndex {
    fn resolve_id(&mut self, index: usize, delta_id: Option<String>) -> String {
        if let Some(id) = delta_id.and_then(|id| (!id.is_empty()).then_some(id)) {
            self.id_by_index.insert(index, id.clone());
            return id;
        }
        if let Some(id) = self.id_by_index.get(&index) {
            return id.clone();
        }
        // No id has ever been seen for this index.  Use a deterministic
        // fallback so future continuation deltas with the same index will
        // also resolve to the same synthesized id.
        let synthesized = format!("call_stream_{}", index);
        self.id_by_index.insert(index, synthesized.clone());
        synthesized
    }
}

/// Extract tool-call deltas from a single streaming event.
///
/// Within one event multiple deltas for the same tool-call ID are merged
/// (appending incremental arguments).  Cross-event merging happens in
/// `merge_stream_tool_call` inside `agent/mod.rs` and relies on the ID
/// being stable across events for the same tool call — which is what the
/// `StreamToolCallIndex` state below guarantees.
fn extract_tool_calls_from_stream_event(
    event: &ChatStreamEvent,
    state: &mut StreamToolCallIndex,
) -> Option<Vec<ToolCall>> {
    let mut calls: Vec<ToolCall> = Vec::new();

    for choice in &event.choices {
        let Some(deltas) = &choice.delta.tool_calls else {
            continue;
        };
        for delta in deltas {
            // Resolve the canonical id for this tool-call index.  The first
            // delta for an index carries the real id; later deltas for the
            // same index typically have `id == None` and we look it up.
            let id = state.resolve_id(delta.index, delta.id.clone());

            // `function` may be absent when a provider only emits
            // top-level `id`/`type` keys for the very first delta.  We
            // synthesize an empty function in that case so the cross-event
            // merger can still register the id.
            let (delta_name, delta_arguments) = match delta.function.as_ref() {
                Some(func) => (func.name.clone(), func.arguments.clone()),
                None => (String::new(), String::new()),
            };

            // Merge with the last call if it has the same id (some providers
            // emit several deltas for the same tool call inside a single
            // event).
            if let Some(last) = calls.last_mut()
                && last.id == id
            {
                if last.function.name.is_empty() && !delta_name.is_empty() {
                    last.function.name = delta_name;
                }
                last.function.arguments.push_str(&delta_arguments);
                continue;
            }

            calls.push(ToolCall {
                id,
                function: ToolCallFunction {
                    name: delta_name,
                    arguments: delta_arguments,
                },
            });
        }
    }

    if calls.is_empty() { None } else { Some(calls) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{
        ChatStreamEvent, StreamChoice, StreamingMessageDelta, StreamingToolCallDelta,
    };

    fn build_event(deltas: Vec<StreamingToolCallDelta>) -> ChatStreamEvent {
        ChatStreamEvent {
            id: "evt".into(),
            object: "chat.completion.chunk".into(),
            created: 0,
            model: "test".into(),
            choices: vec![StreamChoice {
                index: 0,
                delta: StreamingMessageDelta {
                    role: None,
                    content: None,
                    reasoning_content: None,
                    tool_calls: Some(deltas),
                    extra_content: None,
                },
                finish_reason: None,
            }],
            usage: None,
        }
    }

    #[test]
    fn streaming_tool_call_function_with_missing_name_deserializes() {
        // Providers stream the function name only in the FIRST delta.
        // Subsequent deltas have only `arguments` and no `name`.  The
        // ToolCallFunction struct must tolerate missing fields or the entire
        // continuation event will fail to parse, which used to leave the
        // first delta's empty arguments string as the only one delivered.
        let payload = r#"{"arguments":"{\"foo\":"}"#;
        let parsed: ToolCallFunction = serde_json::from_str(payload).expect("must deserialize");
        assert_eq!(parsed.name, "");
        assert_eq!(parsed.arguments, r#"{"foo":"#);
    }

    #[test]
    fn stream_tool_call_index_reuses_id_across_events() {
        let mut state = StreamToolCallIndex::default();

        // First event: full delta with id, name, partial args.
        let first = build_event(vec![StreamingToolCallDelta {
            index: 0,
            id: Some("call_real".into()),
            call_type: Some("function".into()),
            function: Some(ToolCallFunction {
                name: "echo".into(),
                arguments: r#"{"text":"#.into(),
            }),
        }]);
        let calls = extract_tool_calls_from_stream_event(&first, &mut state).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_real");
        assert_eq!(calls[0].function.name, "echo");
        assert_eq!(calls[0].function.arguments, r#"{"text":"#);

        // Second event: continuation with id == None.  Must reuse "call_real"
        // so the agent's cross-event merger appends, not duplicates.
        let second = build_event(vec![StreamingToolCallDelta {
            index: 0,
            id: None,
            call_type: None,
            function: Some(ToolCallFunction {
                name: String::new(),
                arguments: r#""hi"}"#.into(),
            }),
        }]);
        let calls = extract_tool_calls_from_stream_event(&second, &mut state).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_real");
        assert_eq!(calls[0].function.arguments, r#""hi"}"#);
    }

    #[test]
    fn stream_tool_call_index_synthesizes_stable_id_when_first_delta_has_no_id() {
        let mut state = StreamToolCallIndex::default();

        let first = build_event(vec![StreamingToolCallDelta {
            index: 0,
            id: None,
            call_type: None,
            function: Some(ToolCallFunction {
                name: "datetime".into(),
                arguments: String::new(),
            }),
        }]);
        let calls = extract_tool_calls_from_stream_event(&first, &mut state).unwrap();
        assert_eq!(calls.len(), 1);
        let synthesized_id = calls[0].id.clone();
        assert_eq!(synthesized_id, "call_stream_0");

        // Continuation must reuse the synthesized id so cross-event merging
        // by id still combines the deltas into a single tool call.
        let second = build_event(vec![StreamingToolCallDelta {
            index: 0,
            id: None,
            call_type: None,
            function: Some(ToolCallFunction {
                name: String::new(),
                arguments: r#"{}"#.into(),
            }),
        }]);
        let calls = extract_tool_calls_from_stream_event(&second, &mut state).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, synthesized_id);
    }

    #[test]
    fn stream_tool_call_index_handles_multiple_indexes_in_parallel() {
        let mut state = StreamToolCallIndex::default();

        // Two tool calls started in the same event with distinct indexes.
        let first = build_event(vec![
            StreamingToolCallDelta {
                index: 0,
                id: Some("call_a".into()),
                call_type: Some("function".into()),
                function: Some(ToolCallFunction {
                    name: "tool_a".into(),
                    arguments: r#"{""#.into(),
                }),
            },
            StreamingToolCallDelta {
                index: 1,
                id: Some("call_b".into()),
                call_type: Some("function".into()),
                function: Some(ToolCallFunction {
                    name: "tool_b".into(),
                    arguments: r#"{""#.into(),
                }),
            },
        ]);
        let calls = extract_tool_calls_from_stream_event(&first, &mut state).unwrap();
        assert_eq!(calls.len(), 2);

        // Continuations for both indexes — id omitted in both.
        let second = build_event(vec![
            StreamingToolCallDelta {
                index: 1,
                id: None,
                call_type: None,
                function: Some(ToolCallFunction {
                    name: String::new(),
                    arguments: r#"y":2}"#.into(),
                }),
            },
            StreamingToolCallDelta {
                index: 0,
                id: None,
                call_type: None,
                function: Some(ToolCallFunction {
                    name: String::new(),
                    arguments: r#"x":1}"#.into(),
                }),
            },
        ]);
        let calls = extract_tool_calls_from_stream_event(&second, &mut state).unwrap();
        assert_eq!(calls.len(), 2);
        // Order is preserved by the input deltas, so first is index 1, second is index 0.
        assert_eq!(calls[0].id, "call_b");
        assert_eq!(calls[1].id, "call_a");
    }

    #[test]
    fn null_name_deltas_accumulate_into_full_arguments() {
        // Regression for opencode.ai/zen: continuation deltas carry explicit
        // `name:null` (previously a hard deserialization error that made
        // parse_sse_event drop the event, discarding every argument fragment).
        // Three events, first with id+name, next two with only argument
        // fragments, must merge into one tool call with full JSON arguments.
        let events = [
            r#"{"id":"evt1","object":"chat.completion.chunk","created":0,"model":"m","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_64763002df6e4fe7b61c8004","type":"function","function":{"name":"web_search","arguments":""}}]}}]}"#,
            r#"{"id":"evt2","object":"chat.completion.chunk","created":0,"model":"m","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":null,"type":"function","function":{"name":null,"arguments":"{"}}]}}]}"#,
            r#"{"id":"evt3","object":"chat.completion.chunk","created":0,"model":"m","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":null,"type":"function","function":{"name":null,"arguments":"\"query\": \"rust async 2026\"}"}}]}}]}"#,
        ];
        let mut state = StreamToolCallIndex::default();
        let mut merged: Vec<ToolCall> = Vec::new();
        for payload in events {
            let event: ChatStreamEvent = serde_json::from_str(payload).unwrap();
            if let Some(calls) = extract_tool_calls_from_stream_event(&event, &mut state) {
                for tc in calls {
                    crate::agent::merge_stream_tool_call(&mut merged, tc);
                }
            }
        }
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].function.name, "web_search");
        assert_eq!(
            merged[0].function.arguments,
            r#"{"query": "rust async 2026"}"#
        );
    }
}
