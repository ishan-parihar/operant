//! Native Anthropic Messages API adapter.
//!
//! Activated by the `anthropic` feature flag.

use async_trait::async_trait;
use futures::StreamExt;
use futures::stream::BoxStream;
use reqwest::Client;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use super::super::model_client::{ChatRequest, ModelClient, StreamChunk};
use super::prompt_caching::{CacheTtl, apply_cache_control};
use crate::client::{
    ChatResponse, Choice, Message, MessageDelta, Role, ToolCall, ToolCallFunction, Usage,
};
use crate::error::{Error, Result};

/// Anthropic Messages API client.
#[derive(Clone)]
pub struct AnthropicModelClient {
    api_key: Arc<RwLock<String>>,
    base_url: String,
    http: Client,
}

impl AnthropicModelClient {
    pub fn new(api_key: String) -> Self {
        // Use a connect_timeout so a dead Anthropic endpoint doesn't hang
        // the agent forever. We deliberately do NOT set a total request
        // timeout — streaming responses (chat_streaming) can legitimately
        // run for many minutes on long generations, and a total timeout
        // would cut them off mid-stream.
        let http = Client::builder()
            .connect_timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| Client::new());
        Self {
            api_key: Arc::new(RwLock::new(api_key)),
            base_url: "https://api.anthropic.com".to_string(),
            http,
        }
    }

    pub fn with_base_url(mut self, url: String) -> Self {
        self.base_url = url;
        self
    }

    /// Convert internal messages to Anthropic format, extracting system prompt.
    /// Adds `cache_control: {type: "ephemeral"}` breakpoints on the system
    /// prompt and the last tool result to enable Anthropic prompt caching.
    /// Without these breakpoints, the frozen-prefix split (iter-39) is purely
    /// logical — the API doesn't know which blocks to cache.
    fn convert_request(&self, request: &ChatRequest) -> Value {
        let mut system: Option<String> = None;
        let mut messages: Vec<Value> = Vec::new();

        // Anthropic's API requires that ALL tool_result blocks from a single
        // assistant turn be grouped into ONE user message. Emitting a separate
        // user message per tool result causes HTTP 400 "invalid_request_error"
        // ("messages must alternate between user and assistant roles").
        //
        // Strategy: iterate through all messages and whenever we encounter a
        // run of consecutive Role::Tool messages, collect them all into a
        // single `{"role":"user","content":[{tool_result},{tool_result},…]}`.
        // A non-Tool message (or the end of the list) flushes the accumulator.
        let mut pending_tool_results: Vec<Value> = Vec::new();

        let flush_tool_results = |msgs: &mut Vec<Value>, pending: &mut Vec<Value>| {
            if !pending.is_empty() {
                msgs.push(json!({"role": "user", "content": pending.clone()}));
                pending.clear();
            }
        };

        for msg in &request.messages {
            match msg.role {
                Role::System => {
                    flush_tool_results(&mut messages, &mut pending_tool_results);
                    system = Some(msg.content.clone());
                }
                Role::User => {
                    flush_tool_results(&mut messages, &mut pending_tool_results);
                    messages.push(json!({"role": "user", "content": msg.content}));
                }
                Role::Assistant => {
                    flush_tool_results(&mut messages, &mut pending_tool_results);
                    let content = self.build_assistant_content(msg);
                    messages.push(json!({"role": "assistant", "content": content}));
                }
                Role::Tool => {
                    // Accumulate into the pending batch — do NOT flush yet.
                    // The next non-Tool message (or end of list) will flush
                    // all accumulated results into a single user message.
                    pending_tool_results.push(json!({
                        "type": "tool_result",
                        "tool_use_id": msg.tool_call_id.as_deref().unwrap_or(""),
                        "content": msg.content
                    }));
                }
            }
        }
        // Flush any trailing tool results (e.g. at the very end of the list).
        flush_tool_results(&mut messages, &mut pending_tool_results);

        let mut body = json!({
            "model": request.model,
            "messages": messages,
            "max_tokens": request.max_tokens.unwrap_or(4096),
        });

        // System prompt with cache_control breakpoint.
        // Anthropic's prompt caching requires explicit cache_control markers.
        // The system prompt is the most stable part of the request — it
        // includes the base prompt, skills, and instructions. Marking it
        // as cacheable means subsequent requests with the same system prompt
        // pay ~10x less (cache read vs fresh encoding).
        if let Some(sys) = system {
            body["system"] = json!([{
                "type": "text",
                "text": sys,
                "cache_control": {"type": "ephemeral"}
            }]);
        }

        // Apply the full system_and_3 caching strategy to messages.
        // This places cache_control breakpoints on the last 3 non-system
        // messages, reducing input token costs by ~75% on multi-turn
        // conversations (ported from hermes-agent prompt_caching.py).
        if let Some(msgs) = body["messages"].as_array_mut() {
            apply_cache_control(msgs, CacheTtl::default(), true);
        }

        if let Some(temp) = request.temperature {
            body["temperature"] = json!(temp);
        }
        if !request.tools.is_empty() {
            body["tools"] = json!(
                request
                    .tools
                    .iter()
                    .map(|t| json!({
                        "name": t.name,
                        "description": t.description,
                        "input_schema": t.parameters
                    }))
                    .collect::<Vec<_>>()
            );
        }
        if request.stream {
            body["stream"] = json!(true);
        }

        body
    }

    /// Build content array for assistant messages (text + tool_use blocks).
    fn build_assistant_content(&self, msg: &Message) -> Value {
        let mut blocks: Vec<Value> = Vec::new();
        if !msg.content.is_empty() {
            blocks.push(json!({"type": "text", "text": msg.content}));
        }
        if let Some(ref tool_calls) = msg.tool_calls {
            for tc in tool_calls {
                let input: Value =
                    serde_json::from_str(&tc.function.arguments).unwrap_or(json!({}));
                blocks.push(json!({
                    "type": "tool_use",
                    "id": tc.id,
                    "name": tc.function.name,
                    "input": input
                }));
            }
        }
        if blocks.is_empty() {
            json!("")
        } else {
            json!(blocks)
        }
    }

    /// Parse Anthropic response into our ChatResponse format.
    fn parse_response(&self, body: Value) -> Result<ChatResponse> {
        let id = body["id"].as_str().unwrap_or("").to_string();
        let model = body["model"].as_str().unwrap_or("").to_string();

        let mut content: Option<String> = None;
        let mut tool_calls: Vec<ToolCall> = Vec::new();

        if let Some(blocks) = body["content"].as_array() {
            for block in blocks {
                match block["type"].as_str() {
                    Some("text") => {
                        content = block["text"].as_str().map(|s| s.to_string());
                    }
                    Some("tool_use") => {
                        tool_calls.push(ToolCall {
                            id: block["id"].as_str().unwrap_or("").to_string(),
                            function: ToolCallFunction {
                                name: block["name"].as_str().unwrap_or("").to_string(),
                                arguments: block["input"].to_string(),
                            },
                        });
                    }
                    _ => {}
                }
            }
        }

        let usage_obj = &body["usage"];
        let usage = Usage {
            prompt_tokens: usage_obj["input_tokens"].as_u64().unwrap_or(0) as u32,
            completion_tokens: usage_obj["output_tokens"].as_u64().unwrap_or(0) as u32,
            total_tokens: (usage_obj["input_tokens"].as_u64().unwrap_or(0)
                + usage_obj["output_tokens"].as_u64().unwrap_or(0))
                as u32,
        };

        let tc = if tool_calls.is_empty() {
            None
        } else {
            Some(
                tool_calls
                    .into_iter()
                    .enumerate()
                    .map(|(i, tc)| crate::client::ToolCallDelta {
                        index: i,
                        id: Some(tc.id),
                        call_type: Some("function".to_string()),
                        function: Some(tc.function),
                    })
                    .collect(),
            )
        };

        Ok(ChatResponse {
            id,
            object: "chat.completion".to_string(),
            created: 0,
            model,
            choices: vec![Choice {
                index: 0,
                message: MessageDelta {
                    role: Some(Role::Assistant),
                    content,
                    reasoning_content: None,
                    tool_calls: tc,
                },
                finish_reason: body["stop_reason"].as_str().map(|s| s.to_string()),
            }],
            usage,
        })
    }
}

#[async_trait]
impl ModelClient for AnthropicModelClient {
    fn provider_name(&self) -> &str {
        "anthropic"
    }

    fn set_api_key(&self, api_key: &str) {
        // Blocking write is acceptable here — set_api_key is only called
        // from credential rotation in the error-handling path.
        if let Ok(mut guard) = self.api_key.write() {
            *guard = api_key.to_string();
        }
    }

    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
        let body = self.convert_request(&request);
        // Clone the key into an owned String FIRST: `RwLockReadGuard` is not
        // Send, so holding the guard (via `.read().unwrap().as_str()`) across
        // the `.send().await` below makes this future non-Send and fails to
        // compile wherever the ModelClient::chat future must be Send. On a
        // poisoned lock the guard is still readable, so recover via
        // `PoisonError::into_inner()` rather than unwrap.
        let api_key = self
            .api_key
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let resp = self
            .http
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            return Err(Error::Agent(format!(
                "Anthropic API error {status}: {text}"
            )));
        }

        let json: Value =
            serde_json::from_str(&text).map_err(|e| Error::ParseResponse(e.to_string()))?;
        self.parse_response(json)
    }

    async fn chat_streaming(
        &self,
        request: ChatRequest,
    ) -> Result<BoxStream<'static, Result<StreamChunk>>> {
        let mut req = request.clone();
        req.stream = true;
        let body = self.convert_request(&req);

        // Same Send-safety requirement as `chat`: clone the key out of the
        // lock before building the request so the non-Send guard isn't held
        // across the await.
        let api_key = self
            .api_key
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let resp = self
            .http
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await?;
            return Err(Error::Agent(format!(
                "Anthropic API error {status}: {text}"
            )));
        }

        let byte_stream = resp.bytes_stream();

        let stream = byte_stream
            .scan(
                (String::new(), HashMap::<u64, (String, String)>::new()),
                |(buffer, index_map), chunk_result| {
                    let mut chunks: Vec<Result<StreamChunk>> = Vec::new();
                    match chunk_result {
                        Ok(bytes) => {
                            buffer.push_str(&String::from_utf8_lossy(&bytes));
                            // Process complete SSE lines
                            while let Some(pos) = buffer.find("\n\n") {
                                let event_block = buffer[..pos].to_string();
                                *buffer = buffer[pos + 2..].to_string();

                                if let Some(chunk) = parse_sse_event(&event_block, index_map) {
                                    chunks.push(Ok(chunk));
                                }
                            }
                        }
                        Err(e) => chunks.push(Err(Error::Network(e.into()))),
                    }
                    futures::future::ready(Some(futures::stream::iter(chunks)))
                },
            )
            .flatten();

        Ok(Box::pin(stream))
    }
}

/// Parse a single SSE event block into a `StreamChunk`.
///
/// `index_map` carries the `(id, name)` pair for each open tool-use content
/// block, keyed by Anthropic's `index` field. It is populated on
/// `content_block_start` (tool_use), read on `content_block_delta`
/// (input_json_delta) so the partial-JSON fragment can be attributed to the
/// correct tool call, and cleared on `content_block_stop`.
fn parse_sse_event(
    block: &str,
    index_map: &mut HashMap<u64, (String, String)>,
) -> Option<StreamChunk> {
    let mut event_type = "";
    let mut data = String::new();

    for line in block.lines() {
        if let Some(rest) = line.strip_prefix("event: ") {
            event_type = rest.trim();
        } else if let Some(rest) = line.strip_prefix("data: ") {
            data = rest.to_string();
        }
    }

    if data.is_empty() {
        return None;
    }

    let json: Value = serde_json::from_str(&data).ok()?;

    match event_type {
        "content_block_delta" => {
            let delta = &json["delta"];
            match delta["type"].as_str()? {
                "text_delta" => Some(StreamChunk::new(
                    delta["text"].as_str().map(|s| s.to_string()),
                    None,
                    None,
                )),
                "input_json_delta" => {
                    // Partial JSON for tool input — emit as a tool_call
                    // argument fragment, attributed to the tool_use block
                    // identified by `index`.
                    let partial = delta["partial_json"].as_str().unwrap_or("");
                    let index = json["index"].as_u64().unwrap_or(0);
                    let (id, name) = index_map.get(&index).cloned().unwrap_or_default();
                    Some(StreamChunk::new(
                        None,
                        None,
                        Some(vec![ToolCall {
                            id,
                            function: ToolCallFunction {
                                name,
                                arguments: partial.to_string(),
                            },
                        }]),
                    ))
                }
                _ => None,
            }
        }
        "content_block_start" => {
            let block = &json["content_block"];
            let index = json["index"].as_u64().unwrap_or(0);
            match block["type"].as_str()? {
                "tool_use" => {
                    let id = block["id"].as_str().unwrap_or("").to_string();
                    let name = block["name"].as_str().unwrap_or("").to_string();
                    // Record (id, name) so later input_json_delta events for
                    // this index can be attributed to the right tool call.
                    index_map.insert(index, (id.clone(), name.clone()));
                    Some(StreamChunk::new(
                        None,
                        None,
                        Some(vec![ToolCall {
                            id,
                            function: ToolCallFunction {
                                name,
                                arguments: String::new(),
                            },
                        }]),
                    ))
                }
                _ => None,
            }
        }
        "content_block_stop" => {
            // Clean up the index map entry to avoid unbounded growth on
            // long sessions with many tool calls.
            let index = json["index"].as_u64().unwrap_or(0);
            index_map.remove(&index);
            None
        }
        // iter-247: Anthropic reports usage split across two events —
        // input_tokens on message_start, output_tokens on message_delta.
        // process_stream (agent/mod.rs) merges the two halves once both
        // have arrived, so each is emitted independently here.
        "message_start" => {
            let input_tokens = json["message"]["usage"]["input_tokens"]
                .as_u64()
                .unwrap_or(0) as u32;
            (input_tokens > 0).then_some(StreamChunk {
                content: None,
                reasoning: None,
                tool_calls: None,
                extra_content: None,
                usage: Some(Usage {
                    prompt_tokens: input_tokens,
                    completion_tokens: 0,
                    total_tokens: input_tokens,
                }),
            })
        }
        "message_delta" => {
            let output_tokens = json["usage"]["output_tokens"].as_u64().unwrap_or(0) as u32;
            (output_tokens > 0).then_some(StreamChunk {
                content: None,
                reasoning: None,
                tool_calls: None,
                extra_content: None,
                usage: Some(Usage {
                    prompt_tokens: 0,
                    completion_tokens: output_tokens,
                    total_tokens: output_tokens,
                }),
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sse(event: &str, data: &str) -> String {
        format!("event: {event}\ndata: {data}\n\n")
    }

    /// `content_block_start` for a tool_use block emits a ToolCall with the
    /// real id + name + empty args, and records (id, name) in the index map.
    #[test]
    fn parse_sse_content_block_start_tool_use_records_index() {
        let mut map = HashMap::new();
        let block = sse(
            "content_block_start",
            r#"{"index":0,"content_block":{"type":"tool_use","id":"toolu_abc","name":"Bash","input":{}}}"#,
        );
        let chunk = parse_sse_event(&block, &mut map).expect("should produce a chunk");

        let tc = &chunk.tool_calls.unwrap()[0];
        assert_eq!(tc.id, "toolu_abc");
        assert_eq!(tc.function.name, "Bash");
        assert_eq!(tc.function.arguments, "");
        assert_eq!(
            map.get(&0),
            Some(&("toolu_abc".to_string(), "Bash".to_string()))
        );
    }

    /// `input_json_delta` events are attributed to the correct tool call via
    /// the `index` field — the ToolCall emitted carries the real id + name,
    /// not empty strings. This is the regression that previously split every
    /// streamed tool call into a real call (empty args) + a phantom call
    /// (empty name) with the args.
    #[test]
    fn parse_sse_input_json_delta_carries_id_and_name_from_index_map() {
        let mut map = HashMap::new();
        // First, send content_block_start to populate the map.
        let start = sse(
            "content_block_start",
            r#"{"index":0,"content_block":{"type":"tool_use","id":"toolu_abc","name":"Bash","input":{}}}"#,
        );
        let _ = parse_sse_event(&start, &mut map);

        // Now send two input_json_delta events with partial JSON fragments.
        let d1 = sse(
            "content_block_delta",
            r#"{"index":0,"delta":{"type":"input_json_delta","partial_json":"{\"command\":"}}"#,
        );
        let d2 = sse(
            "content_block_delta",
            r#"{"index":0,"delta":{"type":"input_json_delta","partial_json":"\"ls\"}"}}"#,
        );

        let c1 = parse_sse_event(&d1, &mut map).expect("delta should produce a chunk");
        let c2 = parse_sse_event(&d2, &mut map).expect("delta should produce a chunk");

        let tc1 = &c1.tool_calls.unwrap()[0];
        assert_eq!(tc1.id, "toolu_abc", "delta must carry the real tool id");
        assert_eq!(
            tc1.function.name, "Bash",
            "delta must carry the real tool name"
        );
        assert_eq!(tc1.function.arguments, r#"{"command":"#);

        let tc2 = &c2.tool_calls.unwrap()[0];
        assert_eq!(tc2.id, "toolu_abc");
        assert_eq!(tc2.function.name, "Bash");
        assert_eq!(tc2.function.arguments, r#""ls"}"#);
    }

    /// `content_block_stop` clears the index map entry — subsequent deltas
    /// for the same index (shouldn't happen in practice, but defensive) get
    /// empty id/name rather than the stale value.
    #[test]
    fn parse_sse_content_block_stop_clears_index_map() {
        let mut map = HashMap::new();
        let start = sse(
            "content_block_start",
            r#"{"index":0,"content_block":{"type":"tool_use","id":"toolu_abc","name":"Bash","input":{}}}"#,
        );
        let _ = parse_sse_event(&start, &mut map);
        assert!(map.contains_key(&0));

        let stop = sse("content_block_stop", r#"{"index":0}"#);
        // content_block_stop returns None (no chunk emitted).
        let chunk = parse_sse_event(&stop, &mut map);
        assert!(chunk.is_none());
        assert!(!map.contains_key(&0));
    }

    /// Multiple concurrent tool calls: each gets its own index entry, and
    /// deltas are attributed to the correct tool call regardless of order.
    #[test]
    fn parse_sse_concurrent_tool_calls_correlate_by_index() {
        let mut map = HashMap::new();

        // Two content_block_start events for two different tool calls.
        let s1 = sse(
            "content_block_start",
            r#"{"index":0,"content_block":{"type":"tool_use","id":"toolu_one","name":"Bash","input":{}}}"#,
        );
        let s2 = sse(
            "content_block_start",
            r#"{"index":1,"content_block":{"type":"tool_use","id":"toolu_two","name":"FileRead","input":{}}}"#,
        );
        let _ = parse_sse_event(&s1, &mut map);
        let _ = parse_sse_event(&s2, &mut map);

        // Interleaved deltas — index 1 first, then index 0.
        let d_two = sse(
            "content_block_delta",
            r#"{"index":1,"delta":{"type":"input_json_delta","partial_json":"{\"path\":"}}"#,
        );
        let d_one = sse(
            "content_block_delta",
            r#"{"index":0,"delta":{"type":"input_json_delta","partial_json":"{\"cmd\":"}}"#,
        );

        let c_two = parse_sse_event(&d_two, &mut map).expect("delta for index 1");
        let c_one = parse_sse_event(&d_one, &mut map).expect("delta for index 0");

        let tc_two = &c_two.tool_calls.unwrap()[0];
        assert_eq!(tc_two.id, "toolu_two");
        assert_eq!(tc_two.function.name, "FileRead");

        let tc_one = &c_one.tool_calls.unwrap()[0];
        assert_eq!(tc_one.id, "toolu_one");
        assert_eq!(tc_one.function.name, "Bash");
    }

    /// `text_delta` events still produce a content StreamChunk (regression
    /// guard — the refactor shouldn't break the text path).
    #[test]
    fn parse_sse_text_delta_returns_content_chunk() {
        let mut map = HashMap::new();
        let block = sse(
            "content_block_delta",
            r#"{"index":0,"delta":{"type":"text_delta","text":"hello"}}"#,
        );
        let chunk = parse_sse_event(&block, &mut map).expect("text_delta should produce a chunk");
        assert_eq!(chunk.content.as_deref(), Some("hello"));
        assert!(chunk.tool_calls.is_none());
    }

    /// End-to-end: content_block_start + 2 deltas + stop should produce 3
    /// chunks (1 start + 2 deltas; stop emits None) that, when merged via
    /// `merge_stream_tool_call`, yield a single ToolCall with the full args.
    #[test]
    fn parse_sse_full_tool_use_sequence_merges_into_one_call() {
        use crate::agent::merge_stream_tool_call;

        let mut map = HashMap::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();

        let events = [
            sse(
                "content_block_start",
                r#"{"index":0,"content_block":{"type":"tool_use","id":"toolu_abc","name":"Bash","input":{}}}"#,
            ),
            sse(
                "content_block_delta",
                r#"{"index":0,"delta":{"type":"input_json_delta","partial_json":"{\"command\":"}}"#,
            ),
            sse(
                "content_block_delta",
                r#"{"index":0,"delta":{"type":"input_json_delta","partial_json":" \"echo hi\"}"}}"#,
            ),
            sse("content_block_stop", r#"{"index":0}"#),
        ];

        for ev in &events {
            if let Some(tcs) = parse_sse_event(ev, &mut map).and_then(|chunk| chunk.tool_calls) {
                for tc in tcs {
                    merge_stream_tool_call(&mut tool_calls, tc);
                }
            }
        }

        // Exactly one tool call — no phantom empty-name entry.
        assert_eq!(tool_calls.len(), 1, "should merge into a single tool call");
        assert_eq!(tool_calls[0].id, "toolu_abc");
        assert_eq!(tool_calls[0].function.name, "Bash");
        assert_eq!(
            tool_calls[0].function.arguments,
            r#"{"command": "echo hi"}"#
        );
    }

    /// `message_start` carries `message.usage.input_tokens` — this is the
    /// only place Anthropic streaming reports prompt token count (iter-247).
    #[test]
    fn parse_sse_message_start_extracts_input_tokens() {
        let mut map = HashMap::new();
        let block = sse(
            "message_start",
            r#"{"type":"message_start","message":{"id":"msg_1","usage":{"input_tokens":42,"output_tokens":1}}}"#,
        );
        let chunk =
            parse_sse_event(&block, &mut map).expect("message_start should produce a chunk");
        let usage = chunk.usage.expect("usage should be present");
        assert_eq!(usage.prompt_tokens, 42);
        assert_eq!(usage.completion_tokens, 0);
    }

    /// `message_delta` carries the final `usage.output_tokens` — the only
    /// place Anthropic streaming reports completion token count (iter-247).
    #[test]
    fn parse_sse_message_delta_extracts_output_tokens() {
        let mut map = HashMap::new();
        let block = sse(
            "message_delta",
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":17}}"#,
        );
        let chunk =
            parse_sse_event(&block, &mut map).expect("message_delta should produce a chunk");
        let usage = chunk.usage.expect("usage should be present");
        assert_eq!(usage.completion_tokens, 17);
        assert_eq!(usage.prompt_tokens, 0);
    }

    /// A `message_start` with no usage info (e.g. malformed/absent field)
    /// should not fabricate a zero-token usage chunk.
    #[test]
    fn parse_sse_message_start_without_usage_returns_none() {
        let mut map = HashMap::new();
        let block = sse(
            "message_start",
            r#"{"type":"message_start","message":{"id":"msg_1"}}"#,
        );
        assert!(parse_sse_event(&block, &mut map).is_none());
    }

    // ── convert_request tool-result batching tests (iter-70) ─────────────────
    //
    // Anthropic's API requires that ALL tool_result blocks from a single
    // assistant turn are grouped into ONE user message. Previously each
    // Role::Tool message was emitted as a separate {"role":"user"} message,
    // which caused HTTP 400 "invalid_request_error" when the agent ran
    // multiple tools concurrently (iter-56 concurrent 8-worker pool).

    fn make_client() -> AnthropicModelClient {
        AnthropicModelClient::new("sk-test".to_string())
    }

    fn make_request(messages: Vec<crate::client::Message>) -> ChatRequest {
        ChatRequest::new("claude-opus-4-5", messages)
    }

    /// Two consecutive Role::Tool messages (parallel tool results) must be
    /// merged into a SINGLE user message with two tool_result content blocks.
    /// Previously each Role::Tool produced a separate user message, which
    /// violated Anthropic's role-alternation rule.
    #[test]
    fn convert_request_batches_consecutive_tool_results_into_one_user_message() {
        use crate::client::Message;

        let client = make_client();
        let messages = vec![
            Message::user("run two tools"),
            // Simulated assistant with two tool_use blocks (content set by build_assistant_content)
            Message::assistant(""),
            Message::tool("call_1", r#"{"result":"search done"}"#),
            Message::tool("call_2", r#"{"result":"memory stored"}"#),
        ];
        let body = client.convert_request(&make_request(messages));
        let msgs = body["messages"].as_array().expect("messages must be array");

        // Expected shape: user → assistant → user (with 2 tool_results)
        // NOT: user → assistant → user → user
        assert_eq!(
            msgs.len(),
            3,
            "must be exactly 3 messages (user, assistant, batched-tool-user); got {}: {body}",
            msgs.len()
        );

        assert_eq!(msgs[0]["role"].as_str(), Some("user"));
        assert_eq!(msgs[1]["role"].as_str(), Some("assistant"));
        assert_eq!(msgs[2]["role"].as_str(), Some("user"));

        let content = msgs[2]["content"]
            .as_array()
            .expect("batched tool message must have array content");
        assert_eq!(
            content.len(),
            2,
            "batched tool message must contain exactly 2 tool_result blocks; got {}: {content:?}",
            content.len()
        );
        assert_eq!(content[0]["type"].as_str(), Some("tool_result"));
        assert_eq!(content[0]["tool_use_id"].as_str(), Some("call_1"));
        assert_eq!(content[1]["type"].as_str(), Some("tool_result"));
        assert_eq!(content[1]["tool_use_id"].as_str(), Some("call_2"));
    }

    /// A single Role::Tool message must still produce a valid user message
    /// with a single tool_result block (regression guard — single-tool case).
    #[test]
    fn convert_request_single_tool_result_still_wrapped_correctly() {
        use crate::client::Message;

        let client = make_client();
        let messages = vec![
            Message::user("do one thing"),
            Message::assistant(""),
            Message::tool("call_only", r#"{"ok":true}"#),
        ];
        let body = client.convert_request(&make_request(messages));
        let msgs = body["messages"].as_array().expect("messages must be array");

        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[2]["role"].as_str(), Some("user"));
        let content = msgs[2]["content"]
            .as_array()
            .expect("tool message must have array content");
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"].as_str(), Some("tool_result"));
        assert_eq!(content[0]["tool_use_id"].as_str(), Some("call_only"));
    }

    /// Tool results followed by another user turn must flush the batch
    /// before emitting the next user message. Tests that the accumulator
    /// resets correctly across turn boundaries.
    #[test]
    fn convert_request_tool_results_flushed_before_next_user_turn() {
        use crate::client::Message;

        let client = make_client();
        let messages = vec![
            Message::user("first user turn"),
            Message::assistant(""),
            Message::tool("call_a", "result a"),
            Message::tool("call_b", "result b"),
            Message::user("second user turn"),
        ];
        let body = client.convert_request(&make_request(messages));
        let msgs = body["messages"].as_array().expect("messages must be array");

        // user → assistant → batched-tool-user → user
        assert_eq!(
            msgs.len(),
            4,
            "must have 4 messages; got {}: {body}",
            msgs.len()
        );
        assert_eq!(msgs[0]["role"].as_str(), Some("user"));
        assert_eq!(msgs[0]["content"].as_str(), Some("first user turn"));
        assert_eq!(msgs[1]["role"].as_str(), Some("assistant"));
        // batched tool results
        assert_eq!(msgs[2]["role"].as_str(), Some("user"));
        assert_eq!(
            msgs[2]["content"].as_array().expect("must be array").len(),
            2
        );
        // second explicit user turn
        assert_eq!(msgs[3]["role"].as_str(), Some("user"));
        assert_eq!(msgs[3]["content"].as_str(), Some("second user turn"));
    }
}
