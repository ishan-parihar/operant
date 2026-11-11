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

    fn set_api_key(&self, api_key: &str) {
        // Credential-pool rotation parity: swap the runtime key so the next
        // request uses the new credential without rebuilding the client.
        self.inner.set_api_key(api_key);
    }

    fn set_endpoint(&self, base_url: &str, api_key: Option<&str>) {
        self.inner.set_base_url(base_url);
        // An empty key clears the rotation override, so a keyless endpoint
        // never inherits the previous provider's key.
        self.inner.set_api_key(api_key.unwrap_or(""));
    }

    fn base_url(&self) -> Option<String> {
        Some(self.inner.base_url())
    }

    fn current_api_key(&self) -> Option<String> {
        // `effective_api_key` falls back to the literal string "default"
        // when there is no key. That placeholder is NOT a credential: the
        // retarget path seeds a pool credential with this value, and a
        // credential whose secret is the word "default" is a
        // credential that cannot authenticate. Report "no key" instead.
        let key = self.inner.effective_api_key();
        if key.is_empty() || key == "default" {
            None
        } else {
            Some(key)
        }
    }

    async fn chat(&self, request: ChatRequest) -> Result<crate::client::ChatResponse> {
        let tools = if request.tools.is_empty() {
            None
        } else {
            Some(request.tools.as_slice())
        };
        self.inner
            .chat_with_effort(
                &request.model,
                &request.messages,
                tools,
                request.max_tokens,
                request.temperature,
                request.reasoning_effort.as_deref(),
                request.reasoning_shape,
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
            .chat_streaming_with_effort(
                &request.model,
                &request.messages,
                tools,
                request.max_tokens,
                request.temperature,
                request.reasoning_effort.as_deref(),
                request.reasoning_shape,
            )
            .await?;
        Ok(map_sse_stream(stream))
    }
}

/// Map a provider's SSE stream into the agent's chunks, with the terminal
/// chunk that reports how the stream ENDED.
///
/// Split out of `chat_streaming` so the end of a stream — the part that
/// separates "the provider answered" from "the body just stopped" — is
/// testable without an HTTP server.
fn map_sse_stream(
    stream: crate::client::ChatStreamResponse,
) -> BoxStream<'static, Result<StreamChunk>> {
    // Corrupted-SSE accounting: clone the drop counter before the
    // response is moved into the mapped stream; the terminal chunk
    // below reports the total so the agent can surface stream
    // corruption instead of silently rendering a truncated turn.
    let dropped = stream.dropped_counter();
    // Whether the provider's terminal `[DONE]` marker was seen, and
    // whether ANY chunk carried a finish reason. Together they separate
    // "the provider said this stream is over" from "the body just ended"
    // — the difference between a complete answer and one that was cut
    // off mid-sentence with no signal at all.
    let saw_done = stream.done_flag();
    let saw_finish_reason = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let saw_finish_reason_for_scan = std::sync::Arc::clone(&saw_finish_reason);

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
    let mapped = stream.scan(
        StreamToolCallIndex::default(),
        move |state, event_result| {
            let chunk = match event_result {
                Ok(event) => {
                    let choice = &event.choices.first();
                    let content = choice.and_then(|c| c.delta.content.clone());
                    let reasoning = choice.and_then(|c| c.delta.reasoning_content.clone());
                    let tool_calls = extract_tool_calls_from_stream_event(&event, state);
                    let extra_content = choice.and_then(|c| c.delta.extra_content.clone());
                    let finish_reason = choice.and_then(|c| c.finish_reason.clone());
                    if finish_reason.is_some() {
                        saw_finish_reason_for_scan
                            .store(true, std::sync::atomic::Ordering::Relaxed);
                    }

                    Ok(StreamChunk {
                        content,
                        reasoning,
                        tool_calls,
                        extra_content,
                        usage: event.usage,
                        finish_reason,
                        dropped_events: None,
                        model: Some(event.model.clone()),
                    })
                }
                Err(e) => Err(e),
            };
            futures::future::ready(Some(chunk))
        },
    );

    // Terminal chunk: carries the stream's dropped-payload count (set
    // only when > 0) so `process_stream` can emit StreamCorrupted at
    // stream end. Chained after the source so it never precedes real
    // content.
    let fin = futures::stream::once(async move {
        let n = dropped.load(std::sync::atomic::Ordering::Relaxed) as u32;
        // The stream ended with NO completion signal at all: the body
        // closed without the terminal `[DONE]` marker and without a
        // finish reason. The answer may be cut off mid-sentence, and
        // until now the agent had no way to know — it treated the
        // partial as a complete answer. "incomplete" is the same
        // vocabulary jcode uses for an incomplete response; the agent
        // decides whether to ask for a continuation.
        let incomplete = !saw_done.load(std::sync::atomic::Ordering::Relaxed)
            && !saw_finish_reason.load(std::sync::atomic::Ordering::Relaxed);
        Ok(StreamChunk {
            content: None,
            reasoning: None,
            tool_calls: None,
            extra_content: None,
            usage: None,
            finish_reason: incomplete.then(|| "incomplete".to_string()),
            dropped_events: (n > 0).then_some(n),
            model: None,
        })
    });
    Box::pin(mapped.chain(fin))
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
    /// RT-472: the operator's `agent.reasoning_effort_param` never reached
    /// the wire — the adapter hardcoded `Auto` — so "none" still sent
    /// `reasoning_effort` on every non-openrouter host, and the forced
    /// branches of `apply_reasoning_field` had NO production caller at all.
    ///
    /// BEHAVIORAL: the request's shape must reach the SENT body — this
    /// drives the real adapter against a local server and reads the body it
    /// sent.
    #[tokio::test]
    async fn the_requests_shape_reaches_the_sent_body() {
        use std::sync::Arc;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let body_seen = Arc::new(std::sync::Mutex::new(String::new()));
        let body_for_server = Arc::clone(&body_seen);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind a loopback port");
        let addr = listener.local_addr().expect("the bound address");
        tokio::spawn(async move {
            // Two requests: one per shape.
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let mut buf = vec![0u8; 16 * 1024];
                let read = socket.read(&mut buf).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..read]).to_string();
                let body = head.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
                *body_for_server.lock().unwrap() = body;
                let response = r#"{"id":"1","object":"chat.completion","created":0,"model":"demo","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;
                let out = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    response.len(),
                    response
                );
                let _ = socket.write_all(out.as_bytes()).await;
                let _ = socket.flush().await;
            }
        });

        let client = OpenAIClient::new(crate::client::ClientConfig {
            base_url: format!("http://{addr}/v1"),
            api_key: Some("k".to_string()),
            ..crate::client::ClientConfig::default()
        });
        let adapter = OpenAIModelClient::new(client);

        // The "None" shape: no reasoning field at all.
        let request =
            ChatRequest::new("demo".to_string(), vec![crate::client::Message::user("hi")])
                .with_reasoning_effort(Some("high".to_string()))
                .with_reasoning_shape(crate::client::ReasoningShape::None);
        adapter
            .chat(request)
            .await
            .expect("the request must succeed");
        let body = body_seen.lock().unwrap().clone();
        assert!(
            !body.contains("reasoning_effort")
                && !body.contains("\"reasoning\"")
                && !body.contains("thinking"),
            "the None shape must send no reasoning field: {body}"
        );

        // The forced object shape.
        let request =
            ChatRequest::new("demo".to_string(), vec![crate::client::Message::user("hi")])
                .with_reasoning_effort(Some("high".to_string()))
                .with_reasoning_shape(crate::client::ReasoningShape::ReasoningObject);
        adapter
            .chat(request)
            .await
            .expect("the request must succeed");
        let body = body_seen.lock().unwrap().clone();
        assert!(
            body.contains("\"reasoning\""),
            "the forced object shape must reach the sent body: {body}"
        );
    }

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

    /// A stream whose body ends WITHOUT the terminal `[DONE]` marker and
    /// without a finish reason carries NO completion signal: the provider
    /// simply stopped. The answer may be cut off mid-sentence, and until the
    /// marker was surfaced the agent had no way to tell that from a finished
    /// reply — it took the partial as final.
    #[tokio::test]
    async fn a_stream_that_ends_without_a_completion_signal_is_marked_incomplete() {
        use futures::StreamExt;
        let body = "data: {\"id\":\"1\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"half a senten\"},\"finish_reason\":null}]}\n\n";
        let stream = crate::client::ChatStreamResponse::new(futures::stream::iter(vec![Ok(
            bytes::Bytes::from(body),
        )]));
        let chunks: Vec<_> = map_sse_stream(stream).collect().await;

        let terminal = chunks
            .last()
            .expect("the terminal chunk is always emitted")
            .as_ref()
            .expect("the terminal chunk is not an error");
        assert_eq!(
            terminal.finish_reason.as_deref(),
            Some("incomplete"),
            "a stream with no completion signal must say so"
        );
    }

    /// The terminal `[DONE]` marker IS a completion signal: a stream that
    /// ends with it is finished, whatever the chunks said.
    #[tokio::test]
    async fn a_stream_that_ends_with_the_done_marker_is_not_marked_incomplete() {
        use futures::StreamExt;
        let body = "data: {\"id\":\"1\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"a whole answer\"},\"finish_reason\":null}]}\n\ndata: [DONE]\n\n";
        let stream = crate::client::ChatStreamResponse::new(futures::stream::iter(vec![Ok(
            bytes::Bytes::from(body),
        )]));
        let chunks: Vec<_> = map_sse_stream(stream).collect().await;

        let terminal = chunks
            .last()
            .expect("the terminal chunk is always emitted")
            .as_ref()
            .expect("the terminal chunk is not an error");
        assert_eq!(
            terminal.finish_reason, None,
            "the marker said the stream is over — nothing is incomplete"
        );
    }

    /// A finish reason on a chunk IS a completion signal too: a provider
    /// that omits `[DONE]` but reports `stop` finished its answer.
    #[tokio::test]
    async fn a_stream_that_reports_a_finish_reason_is_not_marked_incomplete() {
        use futures::StreamExt;
        let body = "data: {\"id\":\"1\",\"object\":\"chat.completion.chunk\",\"created\":0,\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"a whole answer\"},\"finish_reason\":\"stop\"}]}\n\n";
        let stream = crate::client::ChatStreamResponse::new(futures::stream::iter(vec![Ok(
            bytes::Bytes::from(body),
        )]));
        let chunks: Vec<_> = map_sse_stream(stream).collect().await;

        let terminal = chunks
            .last()
            .expect("the terminal chunk is always emitted")
            .as_ref()
            .expect("the terminal chunk is not an error");
        assert_eq!(
            terminal.finish_reason, None,
            "the chunk's own finish reason said the answer is done"
        );
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
